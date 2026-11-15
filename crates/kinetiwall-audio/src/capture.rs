//! Audio capture backend abstractions for native Windows and deterministic simulation.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::contract::AudioCaptureError;
use crate::wasapi::WasapiLoopbackSession;

/// Abstraction for an audio capture provider.
pub trait AudioCaptureBackend: Send + 'static {
    /// Starts the capture stream.
    fn start(&mut self) -> Result<(), AudioCaptureError>;

    /// Reads available mono samples (or returns None if no new samples are available).
    fn read_samples(&mut self) -> Result<Option<Vec<f32>>, AudioCaptureError>;

    /// Returns the effective sample rate in Hertz.
    fn sample_rate(&self) -> u32;

    /// Halts capture and releases active stream handles.
    fn stop(&mut self);

    /// Checks whether capture is currently active.
    fn is_active(&self) -> bool;
}

/// Native Windows WASAPI loopback capture backend.
pub struct NativeWasapiBackend {
    session: Option<WasapiLoopbackSession>,
    sample_rate: u32,
    active: bool,
}

impl Default for NativeWasapiBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl NativeWasapiBackend {
    pub fn new() -> Self {
        Self {
            session: None,
            sample_rate: 48000,
            active: false,
        }
    }
}

impl AudioCaptureBackend for NativeWasapiBackend {
    fn start(&mut self) -> Result<(), AudioCaptureError> {
        if self.active && self.session.is_some() {
            return Ok(());
        }

        let session = WasapiLoopbackSession::open_default()?;
        self.sample_rate = session.sample_rate();
        self.session = Some(session);
        self.active = true;
        Ok(())
    }

    fn read_samples(&mut self) -> Result<Option<Vec<f32>>, AudioCaptureError> {
        if let Some(session) = &mut self.session {
            session.read_packet()
        } else {
            Ok(None)
        }
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn stop(&mut self) {
        self.session = None;
        self.active = false;
    }

    fn is_active(&self) -> bool {
        self.active
    }
}

/// Deterministic simulated capture source for CI and automated testing.
pub enum SimulationMode {
    Silence,
    Tone { freq_hz: f32, amplitude: f32 },
    Impulse { interval_samples: usize },
    WhiteNoise { amplitude: f32 },
}

pub struct SimulatedAudioBackend {
    mode: SimulationMode,
    sample_rate: u32,
    chunk_size: usize,
    phase: f32,
    sample_counter: usize,
    running: Arc<AtomicBool>,
}

impl SimulatedAudioBackend {
    pub fn new(mode: SimulationMode) -> Self {
        Self {
            mode,
            sample_rate: 48000,
            chunk_size: 256,
            phase: 0.0,
            sample_counter: 0,
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn set_mode(&mut self, mode: SimulationMode) {
        self.mode = mode;
    }
}

impl AudioCaptureBackend for SimulatedAudioBackend {
    fn start(&mut self) -> Result<(), AudioCaptureError> {
        self.running.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn read_samples(&mut self) -> Result<Option<Vec<f32>>, AudioCaptureError> {
        if !self.running.load(Ordering::SeqCst) {
            return Ok(None);
        }

        let mut samples = Vec::with_capacity(self.chunk_size);
        match self.mode {
            SimulationMode::Silence => {
                samples.resize(self.chunk_size, 0.0f32);
            }
            SimulationMode::Tone { freq_hz, amplitude } => {
                let step = 2.0 * std::f32::consts::PI * freq_hz / self.sample_rate as f32;
                for _ in 0..self.chunk_size {
                    samples.push(self.phase.sin() * amplitude);
                    self.phase = (self.phase + step) % (2.0 * std::f32::consts::PI);
                }
            }
            SimulationMode::Impulse { interval_samples } => {
                for _ in 0..self.chunk_size {
                    if self.sample_counter.is_multiple_of(interval_samples) {
                        samples.push(1.0f32);
                    } else {
                        samples.push(0.0f32);
                    }
                    self.sample_counter += 1;
                }
            }
            SimulationMode::WhiteNoise { amplitude } => {
                // Linear congruential pseudo-random generator
                let mut seed = (self.sample_counter as u32)
                    .wrapping_mul(1664525)
                    .wrapping_add(1013904223);
                for _ in 0..self.chunk_size {
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    let norm = (seed as f32 / u32::MAX as f32) * 2.0 - 1.0;
                    samples.push(norm * amplitude);
                    self.sample_counter += 1;
                }
            }
        }

        Ok(Some(samples))
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
    }

    fn is_active(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}
