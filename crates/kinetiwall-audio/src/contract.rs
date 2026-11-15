//! Strongly typed data contracts, settings, and lifecycle states for audio-reactive wallpapers.
//!
//! Enforces:
//! 1. Opt-in activation (`enabled: false` by default).
//! 2. Bounded, finite numerical guarantees (clamped `0.0..=1.0`).
//! 3. Fixed-size visualization payloads without unbounded heap allocations.
//! 4. Separation of user preference from observed runtime capture states.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Number of acoustic frequency bands extracted for wallpaper visualizers.
/// 0: Sub-bass (20-60 Hz)
/// 1: Bass (60-250 Hz)
/// 2: Low-mid (250-500 Hz)
/// 3: Mid (500-1500 Hz)
/// 4: Upper-mid (1500-3000 Hz)
/// 5: Presence (3000-6000 Hz)
/// 6: Brilliance (6000-12000 Hz)
/// 7: Highs (12000-20000 Hz)
pub const NUM_FREQUENCY_BANDS: usize = 8;

/// User preferences for audio reactivity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioReactivitySettings {
    /// Explicit opt-in toggle. Default is strictly `false`.
    pub enabled: bool,
    /// Amplitude multiplier for visualization sensitivity (range: 0.1 to 5.0, default 1.0).
    pub sensitivity: f32,
    /// Temporal smoothing factor (0.0 = instant/raw, 0.95 = heavy smoothing, default 0.5).
    pub smoothing: f32,
    /// Target frame rate for visualization updates (10 to 60 FPS, default 30).
    pub target_fps: u32,
}

impl Default for AudioReactivitySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            sensitivity: 1.0,
            smoothing: 0.5,
            target_fps: 30,
        }
    }
}

impl AudioReactivitySettings {
    /// Validates and normalizes settings into strictly bounded ranges.
    pub fn sanitized(&self) -> Self {
        Self {
            enabled: self.enabled,
            sensitivity: if self.sensitivity.is_finite() {
                self.sensitivity.clamp(0.1, 5.0)
            } else {
                1.0
            },
            smoothing: if self.smoothing.is_finite() {
                self.smoothing.clamp(0.0, 0.95)
            } else {
                0.5
            },
            target_fps: self.target_fps.clamp(10, 60),
        }
    }
}

/// Compact, versionable visualization message published to wallpaper renderers.
/// Contains no raw audio samples, microphone feeds, or unbounded buffers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisualizerFrame {
    /// Monotonically increasing frame sequence number.
    pub sequence: u64,
    /// Timestamp in milliseconds since UNIX epoch or system start.
    pub timestamp_ms: u64,
    /// Normalized root-mean-square (RMS) energy in `0.0..=1.0`.
    pub rms: f32,
    /// Normalized peak amplitude with transient decay in `0.0..=1.0`.
    pub peak: f32,
    /// 8 normalized acoustic frequency bands, each in `0.0..=1.0`.
    pub frequency_bands: [f32; NUM_FREQUENCY_BANDS],
    /// True when the signal is at or below the ambient noise floor (-60 dBFS).
    pub is_silent: bool,
}

impl VisualizerFrame {
    /// Generates a valid silence frame.
    pub fn silence(sequence: u64, timestamp_ms: u64) -> Self {
        Self {
            sequence,
            timestamp_ms,
            rms: 0.0,
            peak: 0.0,
            frequency_bands: [0.0; NUM_FREQUENCY_BANDS],
            is_silent: true,
        }
    }

    /// Enforces finite values and range boundaries [0.0, 1.0] on all fields.
    pub fn sanitize(&mut self) {
        if !self.rms.is_finite() {
            self.rms = 0.0;
        } else {
            self.rms = self.rms.clamp(0.0, 1.0);
        }

        if !self.peak.is_finite() {
            self.peak = 0.0;
        } else {
            self.peak = self.peak.clamp(0.0, 1.0);
        }

        for band in &mut self.frequency_bands {
            if !band.is_finite() {
                *band = 0.0;
            } else {
                *band = band.clamp(0.0, 1.0);
            }
        }
    }
}

/// Observed lifecycle state of the audio capture subsystem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "detail")]
pub enum AudioCaptureState {
    /// Audio capture is explicitly disabled by user preference or policy.
    Disabled,
    /// Capture resources, COM interfaces, or audio endpoints are initializing.
    Initializing,
    /// Loopback capture is active and streaming samples.
    Active,
    /// Capture is temporarily suspended (e.g. wallpaper paused or screen locked).
    Suspended,
    /// No supported audio rendering endpoint is available.
    Unavailable(String),
    /// An unrecoverable capture or stream failure occurred.
    Failed(String),
    /// Resources are being drained and released.
    Stopping,
}

impl AudioCaptureState {
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active)
    }

    pub fn display_label(&self) -> &'static str {
        match self {
            Self::Disabled => "Audio reactivity disabled",
            Self::Initializing => "Initializing audio capture...",
            Self::Active => "Audio capture active",
            Self::Suspended => "Audio capture suspended",
            Self::Unavailable(_) => "Audio endpoint unavailable",
            Self::Failed(_) => "Audio capture error",
            Self::Stopping => "Stopping audio capture...",
        }
    }
}

/// Typed error conditions for the audio subsystem.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AudioCaptureError {
    #[error("Audio reactivity is not enabled")]
    NotEnabled,

    #[error("No supported audio output endpoint detected: {0}")]
    DeviceUnavailable(String),

    #[error("Audio capture initialization failed: {0}")]
    InitializationFailed(String),

    #[error("Unsupported audio format ({0})")]
    UnsupportedFormat(String),

    #[error("Audio stream error: {0}")]
    StreamError(String),

    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),

    #[error("Audio capture cancelled")]
    Cancelled,
}
