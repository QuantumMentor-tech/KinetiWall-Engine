//! Audio Reactivity Service coordinator managing capture lifecycle, rate-limiting, and distribution.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tracing::{error, info};

use crate::capture::AudioCaptureBackend;
use crate::contract::{
    AudioCaptureError, AudioCaptureState, AudioReactivitySettings, VisualizerFrame,
};
use crate::dsp::AudioFeatureExtractor;

/// Service coordinating opt-in audio reactivity, background capture, and rate-limited frame emission.
pub struct AudioReactivityService {
    settings: AudioReactivitySettings,
    state: Arc<Mutex<AudioCaptureState>>,
    backend_factory: Option<Box<dyn Fn() -> Box<dyn AudioCaptureBackend> + Send>>,
    worker_handle: Option<JoinHandle<()>>,
    stop_signal: Arc<AtomicBool>,
    suspend_signal: Arc<AtomicBool>,
    latest_frame: Arc<Mutex<Option<VisualizerFrame>>>,
    frame_broadcaster: Arc<Mutex<Vec<SyncSender<VisualizerFrame>>>>,
}

impl AudioReactivityService {
    /// Creates a new service with the provided capture backend factory.
    pub fn new<F>(factory: F) -> Self
    where
        F: Fn() -> Box<dyn AudioCaptureBackend> + Send + 'static,
    {
        Self {
            settings: AudioReactivitySettings::default(),
            state: Arc::new(Mutex::new(AudioCaptureState::Disabled)),
            backend_factory: Some(Box::new(factory)),
            worker_handle: None,
            stop_signal: Arc::new(AtomicBool::new(false)),
            suspend_signal: Arc::new(AtomicBool::new(false)),
            latest_frame: Arc::new(Mutex::new(None)),
            frame_broadcaster: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Returns current observed capture state.
    pub fn state(&self) -> AudioCaptureState {
        self.state.lock().unwrap().clone()
    }

    /// Returns the current configuration settings.
    pub fn settings(&self) -> &AudioReactivitySettings {
        &self.settings
    }

    /// Updates configuration settings, applying sensitivity and smoothing dynamically.
    pub fn update_settings(&mut self, new_settings: AudioReactivitySettings) {
        let sanitized = new_settings.sanitized();
        let was_enabled = self.settings.enabled;
        self.settings = sanitized;

        if !was_enabled && self.settings.enabled {
            let _ = self.start();
        } else if was_enabled && !self.settings.enabled {
            self.stop();
        }
    }

    /// Subscribes to bounded visualization frames (drops stale frames if consumer lags).
    pub fn subscribe(&self) -> Receiver<VisualizerFrame> {
        let (tx, rx) = sync_channel(2);
        self.frame_broadcaster.lock().unwrap().push(tx);
        rx
    }

    /// Returns the most recently computed visualizer frame without blocking.
    pub fn latest_frame(&self) -> Option<VisualizerFrame> {
        self.latest_frame.lock().unwrap().clone()
    }

    /// Starts audio capture if explicitly enabled.
    pub fn start(&mut self) -> Result<(), AudioCaptureError> {
        if !self.settings.enabled {
            *self.state.lock().unwrap() = AudioCaptureState::Disabled;
            return Err(AudioCaptureError::NotEnabled);
        }

        // Idempotent start
        if self.worker_handle.is_some() {
            return Ok(());
        }

        *self.state.lock().unwrap() = AudioCaptureState::Initializing;
        self.stop_signal.store(false, Ordering::SeqCst);
        self.suspend_signal.store(false, Ordering::SeqCst);

        let factory = match &self.backend_factory {
            Some(f) => f,
            None => {
                let err = "No backend factory configured".to_string();
                *self.state.lock().unwrap() = AudioCaptureState::Failed(err.clone());
                return Err(AudioCaptureError::InitializationFailed(err));
            }
        };

        let mut backend = factory();
        if let Err(e) = backend.start() {
            let state_val = match &e {
                AudioCaptureError::DeviceUnavailable(msg) => {
                    AudioCaptureState::Unavailable(msg.clone())
                }
                other => AudioCaptureState::Failed(other.to_string()),
            };
            *self.state.lock().unwrap() = state_val;
            return Err(e);
        }

        *self.state.lock().unwrap() = AudioCaptureState::Active;

        let state_arc = Arc::clone(&self.state);
        let stop_arc = Arc::clone(&self.stop_signal);
        let suspend_arc = Arc::clone(&self.suspend_signal);
        let latest_frame_arc = Arc::clone(&self.latest_frame);
        let broadcaster_arc = Arc::clone(&self.frame_broadcaster);
        let settings = self.settings.clone();

        let handle = thread::Builder::new()
            .name("kinetiwall-audio-worker".to_string())
            .spawn(move || {
                let sample_rate = backend.sample_rate();
                let mut extractor = AudioFeatureExtractor::new(
                    sample_rate,
                    settings.sensitivity,
                    settings.smoothing,
                );

                let frame_interval = Duration::from_secs_f32(1.0 / settings.target_fps as f32);
                let mut next_tick = Instant::now() + frame_interval;

                info!(sample_rate, "Audio reactivity worker thread started");

                while !stop_arc.load(Ordering::SeqCst) {
                    // Check suspension
                    if suspend_arc.load(Ordering::SeqCst) {
                        thread::sleep(Duration::from_millis(50));
                        continue;
                    }

                    // Read captured audio buffer
                    match backend.read_samples() {
                        Ok(Some(samples)) => {
                            let timestamp_ms = SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .map(|d| d.as_millis() as u64)
                                .unwrap_or(0);

                            let frame = extractor.process_mono_samples(&samples, timestamp_ms);

                            // Store latest frame
                            *latest_frame_arc.lock().unwrap() = Some(frame.clone());

                            // Broadcast to subscribers (non-blocking drop-oldest)
                            let mut senders = broadcaster_arc.lock().unwrap();
                            senders.retain(|tx| {
                                let _ = tx.try_send(frame.clone());
                                true
                            });
                        }
                        Ok(None) => {
                            // Sleep briefly to prevent spin
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(e) => {
                            error!(error = %e, "Audio capture backend read error");
                            *state_arc.lock().unwrap() = AudioCaptureState::Failed(e.to_string());
                            break;
                        }
                    }

                    // Enforce bounded frame rate
                    let now = Instant::now();
                    if now < next_tick {
                        thread::sleep(next_tick - now);
                    }
                    next_tick = Instant::now() + frame_interval;
                }

                backend.stop();
                info!("Audio reactivity worker thread exited cleanly");
            })
            .map_err(|e| AudioCaptureError::InitializationFailed(e.to_string()))?;

        self.worker_handle = Some(handle);
        Ok(())
    }

    /// Suspends capture temporarily (e.g. screen lock or wallpaper pause).
    pub fn suspend(&mut self) {
        if self.worker_handle.is_some() {
            self.suspend_signal.store(true, Ordering::SeqCst);
            *self.state.lock().unwrap() = AudioCaptureState::Suspended;
        }
    }

    /// Resumes suspended capture.
    pub fn resume(&mut self) -> Result<(), AudioCaptureError> {
        if !self.settings.enabled {
            return Err(AudioCaptureError::NotEnabled);
        }
        if self.worker_handle.is_some() {
            self.suspend_signal.store(false, Ordering::SeqCst);
            *self.state.lock().unwrap() = AudioCaptureState::Active;
        } else {
            self.start()?;
        }
        Ok(())
    }

    /// Stops audio capture and terminates worker thread.
    pub fn stop(&mut self) {
        *self.state.lock().unwrap() = AudioCaptureState::Stopping;
        self.stop_signal.store(true, Ordering::SeqCst);

        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }

        *self.state.lock().unwrap() = AudioCaptureState::Disabled;
    }

    /// Explicitly shuts down and clears all broadcasters and resources.
    pub fn shutdown(&mut self) {
        self.stop();
        self.frame_broadcaster.lock().unwrap().clear();
        *self.latest_frame.lock().unwrap() = None;
    }
}

impl Drop for AudioReactivityService {
    fn drop(&mut self) {
        self.shutdown();
    }
}
