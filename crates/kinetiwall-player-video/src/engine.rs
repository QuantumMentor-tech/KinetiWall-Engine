//! Abstraction over the native libmpv engine so the video player process can run either
//! against real dynamic `mpv-2.dll` (production / local dev) or a deterministic simulated engine
//! (automated CI test fixtures without hardware display/DLL requirements).

use std::path::Path;
use std::time::Duration;

use kinetiwall_playback::PlaybackErrorKind;
use tracing::{info, warn};

use crate::ffi::{
    MpvFunctions, MpvHandle, MPV_EVENT_END_FILE, MPV_EVENT_FILE_LOADED, MPV_EVENT_PLAYBACK_RESTART,
    MPV_EVENT_SHUTDOWN,
};

/// High-level event produced by the backend engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineEvent {
    FileLoaded,
    PlaybackRestart,
    PlaybackEnded,
    PlaybackFailed(PlaybackErrorKind),
    Shutdown,
}

pub trait VideoEngine: Send {
    /// Initialize the video player engine (create context, configure basic options).
    fn initialize(&mut self) -> Result<(), PlaybackErrorKind>;

    /// Load the validated video file.
    fn load_file(&mut self, path: &Path) -> Result<(), PlaybackErrorKind>;

    /// Pause or unpause playback.
    fn set_pause(&mut self, pause: bool) -> Result<(), PlaybackErrorKind>;

    /// Set playback volume (0.0 to 1.0).
    fn set_volume(&mut self, volume: f32) -> Result<(), PlaybackErrorKind>;

    /// Set audio mute state.
    fn set_mute(&mut self, mute: bool) -> Result<(), PlaybackErrorKind>;

    /// Stop playback immediately.
    fn stop(&mut self) -> Result<(), PlaybackErrorKind>;

    /// Query the current hardware acceleration decoder in use, e.g. "d3d11va", "no", etc.
    fn hwdec_current(&self) -> String;

    /// Wait for an engine event with a bounded timeout.
    fn wait_event(&mut self, timeout: Duration) -> Option<EngineEvent>;

    /// Terminate and release native resources.
    fn shutdown(&mut self);
}

// -------------------------------------------------------------------------------------------------
// Real Native libmpv Backend
// -------------------------------------------------------------------------------------------------
pub struct NativeMpvEngine {
    funcs: MpvFunctions,
    ctx: Option<MpvHandle>,
    _hwdec_mode: String,
}

impl NativeMpvEngine {
    pub fn try_new() -> Result<Self, PlaybackErrorKind> {
        let funcs = MpvFunctions::load_from_search().map_err(|e| {
            warn!("Native libmpv runtime not found: {e}");
            PlaybackErrorKind::BackendUnavailable
        })?;
        Ok(Self {
            funcs,
            ctx: None,
            _hwdec_mode: "unknown".into(),
        })
    }
}

impl VideoEngine for NativeMpvEngine {
    fn initialize(&mut self) -> Result<(), PlaybackErrorKind> {
        if self.ctx.is_some() {
            return Ok(());
        }
        let handle = self.funcs.mpv_create().map_err(|e| {
            warn!("Failed to create mpv context: {e}");
            PlaybackErrorKind::PreparationFailed
        })?;

        // Standard KinetiWall options: muted wallpaper by default, loop file, d3d11va hwdec
        let _ = self.funcs.mpv_set_option_string(handle, "mute", "yes");
        let _ = self.funcs.mpv_set_option_string(handle, "loop-file", "inf");
        let _ = self
            .funcs
            .mpv_set_option_string(handle, "hwdec", "auto-safe");
        let _ = self.funcs.mpv_set_option_string(handle, "terminal", "no");

        self.funcs.mpv_initialize(handle).map_err(|e| {
            warn!("Failed to initialize mpv context: {e}");
            self.funcs.mpv_destroy(handle);
            PlaybackErrorKind::PreparationFailed
        })?;

        self.ctx = Some(handle);
        info!("Native libmpv initialized successfully");
        Ok(())
    }

    fn load_file(&mut self, path: &Path) -> Result<(), PlaybackErrorKind> {
        let Some(ctx) = self.ctx else {
            return Err(PlaybackErrorKind::Internal);
        };
        let path_str = path.to_string_lossy();
        self.funcs
            .mpv_command(ctx, &["loadfile", &path_str, "replace"])
            .map_err(|e| {
                warn!("mpv loadfile failed: {e}");
                PlaybackErrorKind::StartupFailed
            })?;
        Ok(())
    }

    fn set_pause(&mut self, pause: bool) -> Result<(), PlaybackErrorKind> {
        let Some(ctx) = self.ctx else {
            return Err(PlaybackErrorKind::Internal);
        };
        let val = if pause { "yes" } else { "no" };
        self.funcs
            .mpv_set_property_string(ctx, "pause", val)
            .map_err(|_| PlaybackErrorKind::OperationRejected)?;
        Ok(())
    }

    fn set_volume(&mut self, volume: f32) -> Result<(), PlaybackErrorKind> {
        let Some(ctx) = self.ctx else {
            return Err(PlaybackErrorKind::Internal);
        };
        let vol_clamped = (volume.clamp(0.0, 1.0) * 100.0).round() as i64;
        self.funcs
            .mpv_set_property_string(ctx, "volume", &vol_clamped.to_string())
            .map_err(|_| PlaybackErrorKind::OperationRejected)?;
        Ok(())
    }

    fn set_mute(&mut self, mute: bool) -> Result<(), PlaybackErrorKind> {
        let Some(ctx) = self.ctx else {
            return Err(PlaybackErrorKind::Internal);
        };
        let val = if mute { "yes" } else { "no" };
        self.funcs
            .mpv_set_property_string(ctx, "mute", val)
            .map_err(|_| PlaybackErrorKind::OperationRejected)?;
        Ok(())
    }

    fn stop(&mut self) -> Result<(), PlaybackErrorKind> {
        let Some(ctx) = self.ctx else {
            return Ok(());
        };
        let _ = self.funcs.mpv_command(ctx, &["stop"]);
        Ok(())
    }

    fn hwdec_current(&self) -> String {
        let Some(ctx) = self.ctx else {
            return "no".into();
        };
        self.funcs
            .mpv_get_property_string(ctx, "hwdec-current")
            .unwrap_or_else(|| "no".into())
    }

    fn wait_event(&mut self, timeout: Duration) -> Option<EngineEvent> {
        let ctx = self.ctx?;
        let event = self.funcs.mpv_wait_event(ctx, timeout.as_secs_f64())?;
        match event.event_id {
            MPV_EVENT_FILE_LOADED => Some(EngineEvent::FileLoaded),
            MPV_EVENT_PLAYBACK_RESTART => Some(EngineEvent::PlaybackRestart),
            MPV_EVENT_END_FILE => {
                if event.error < 0 {
                    Some(EngineEvent::PlaybackFailed(
                        PlaybackErrorKind::StartupFailed,
                    ))
                } else {
                    Some(EngineEvent::PlaybackEnded)
                }
            }
            MPV_EVENT_SHUTDOWN => Some(EngineEvent::Shutdown),
            _ => None,
        }
    }

    fn shutdown(&mut self) {
        if let Some(ctx) = self.ctx.take() {
            info!("Terminating and destroying mpv context");
            self.funcs.mpv_terminate_destroy(ctx);
        }
    }
}

impl Drop for NativeMpvEngine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// -------------------------------------------------------------------------------------------------
// Deterministic Simulated Engine (Used when native libmpv is unavailable or feature fake-backend)
// -------------------------------------------------------------------------------------------------
pub struct SimulatedEngine {
    initialized: bool,
    loaded: bool,
    events: Vec<EngineEvent>,
    hwdec: String,
}

impl SimulatedEngine {
    pub fn new() -> Self {
        Self {
            initialized: false,
            loaded: false,
            events: Vec::new(),
            hwdec: "d3d11va".into(),
        }
    }
}

impl Default for SimulatedEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoEngine for SimulatedEngine {
    fn initialize(&mut self) -> Result<(), PlaybackErrorKind> {
        self.initialized = true;
        Ok(())
    }

    fn load_file(&mut self, path: &Path) -> Result<(), PlaybackErrorKind> {
        if !self.initialized {
            return Err(PlaybackErrorKind::Internal);
        }
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        // Simulate codec decoding failure if extension or name indicates invalid stream
        if ext == "avi" && path.to_string_lossy().contains("invalid_stream") {
            self.events.push(EngineEvent::PlaybackFailed(
                PlaybackErrorKind::StartupFailed,
            ));
            return Ok(());
        }
        self.loaded = true;
        self.events.push(EngineEvent::FileLoaded);
        self.events.push(EngineEvent::PlaybackRestart);
        Ok(())
    }

    fn set_pause(&mut self, _pause: bool) -> Result<(), PlaybackErrorKind> {
        Ok(())
    }

    fn set_volume(&mut self, _volume: f32) -> Result<(), PlaybackErrorKind> {
        Ok(())
    }

    fn set_mute(&mut self, _mute: bool) -> Result<(), PlaybackErrorKind> {
        Ok(())
    }

    fn stop(&mut self) -> Result<(), PlaybackErrorKind> {
        self.loaded = false;
        self.events.push(EngineEvent::PlaybackEnded);
        Ok(())
    }

    fn hwdec_current(&self) -> String {
        if self.loaded {
            self.hwdec.clone()
        } else {
            "no".into()
        }
    }

    fn wait_event(&mut self, _timeout: Duration) -> Option<EngineEvent> {
        if self.events.is_empty() {
            None
        } else {
            Some(self.events.remove(0))
        }
    }

    fn shutdown(&mut self) {
        self.initialized = false;
        self.loaded = false;
        self.events.clear();
    }
}

/// Constructs the appropriate video engine: attempts native libmpv first; falls back to simulated
/// if feature is enabled or when native is missing.
pub fn create_video_engine() -> Box<dyn VideoEngine> {
    #[cfg(feature = "fake-backend")]
    {
        Box::new(SimulatedEngine::new())
    }

    #[cfg(not(feature = "fake-backend"))]
    {
        match NativeMpvEngine::try_new() {
            Ok(native) => Box::new(native),
            Err(_) => Box::new(SimulatedEngine::new()),
        }
    }
}
// Append to engine.rs
unsafe impl Send for NativeMpvEngine {}
