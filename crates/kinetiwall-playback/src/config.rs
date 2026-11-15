//! Unified playback configuration contracts, bounded types, safe runtime defaults,
//! and dynamic runtime update models.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Error produced when validating a configuration value strictly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigValidationError {
    pub setting: String,
    pub detail: String,
}

impl fmt::Display for ConfigValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Invalid config for '{}': {}", self.setting, self.detail)
    }
}

impl std::error::Error for ConfigValidationError {}

/// Normalized audio volume bounded in `[0.0, 1.0]`.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NormalizedVolume(f32);

impl NormalizedVolume {
    pub const MIN: f32 = 0.0;
    pub const MAX: f32 = 1.0;
    pub const DEFAULT: f32 = 1.0;

    /// Attempts to construct a `NormalizedVolume`. Rejects NaN, Inf, and out-of-range values.
    pub fn new(val: f32) -> Result<Self, ConfigValidationError> {
        if val.is_nan() || val.is_infinite() {
            return Err(ConfigValidationError {
                setting: "volume".into(),
                detail: "Volume must be a finite number".into(),
            });
        }
        if !(Self::MIN..=Self::MAX).contains(&val) {
            return Err(ConfigValidationError {
                setting: "volume".into(),
                detail: format!("Volume {} outside valid range [0.0, 1.0]", val),
            });
        }
        Ok(Self(val))
    }

    /// Clamps an arbitrary float into `[0.0, 1.0]` safely (NaN falls back to default).
    /// Returns `(NormalizedVolume, was_adjusted)`.
    #[must_use]
    pub fn clamped(val: f32) -> (Self, bool) {
        if val.is_nan() || val.is_infinite() {
            (Self(Self::DEFAULT), true)
        } else if val < Self::MIN {
            (Self(Self::MIN), true)
        } else if val > Self::MAX {
            (Self(Self::MAX), true)
        } else {
            (Self(val), false)
        }
    }

    #[must_use]
    pub fn value(self) -> f32 {
        self.0
    }
}

impl Default for NormalizedVolume {
    fn default() -> Self {
        Self(Self::DEFAULT)
    }
}

/// Loop playback policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LoopMode {
    #[default]
    Infinite,
    Once,
    Count(u32),
}

/// Startup playback mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StartMode {
    #[default]
    AutoPlay,
    Paused,
}

/// Wallpaper display scaling mode (aligned with KW-DAT-004 specification).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ScalingMode {
    #[default]
    Fill,
    Fit,
    Stretch,
    Center,
}

/// Hardware acceleration preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HwAccelPreference {
    #[default]
    Auto,
    Disabled,
    ForceHardware,
}

/// Settings genuinely shared across player backends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CommonPlaybackConfig {
    pub volume: NormalizedVolume,
    pub muted: bool,
    pub loop_mode: LoopMode,
    pub start_mode: StartMode,
    pub scaling_mode: ScalingMode,
    pub hardware_acceleration: HwAccelPreference,
}

/// Hardware decoder mode for video backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VideoHwDec {
    #[default]
    AutoSafe,
    Disabled,
    D3d11va,
}

/// Video-player specific options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct VideoBackendOptions {
    pub hwdec: VideoHwDec,
    pub audio_track: Option<u32>,
}

/// Audio reactivity visualizer options for web wallpapers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioReactivityOptions {
    /// Bounded in [0.1, 3.0], default 1.0.
    pub sensitivity: f32,
    /// Bounded in [0.0, 0.95], default 0.65.
    pub smoothing: f32,
}

impl AudioReactivityOptions {
    pub const MIN_SENSITIVITY: f32 = 0.1;
    pub const MAX_SENSITIVITY: f32 = 3.0;
    pub const DEFAULT_SENSITIVITY: f32 = 1.0;

    pub const MIN_SMOOTHING: f32 = 0.0;
    pub const MAX_SMOOTHING: f32 = 0.95;
    pub const DEFAULT_SMOOTHING: f32 = 0.65;

    #[must_use]
    pub fn clamped(sensitivity: f32, smoothing: f32) -> (Self, bool) {
        let mut adjusted = false;

        let sens = if sensitivity.is_nan() || sensitivity.is_infinite() {
            adjusted = true;
            Self::DEFAULT_SENSITIVITY
        } else if sensitivity < Self::MIN_SENSITIVITY {
            adjusted = true;
            Self::MIN_SENSITIVITY
        } else if sensitivity > Self::MAX_SENSITIVITY {
            adjusted = true;
            Self::MAX_SENSITIVITY
        } else {
            sensitivity
        };

        let smooth = if smoothing.is_nan() || smoothing.is_infinite() {
            adjusted = true;
            Self::DEFAULT_SMOOTHING
        } else if smoothing < Self::MIN_SMOOTHING {
            adjusted = true;
            Self::MIN_SMOOTHING
        } else if smoothing > Self::MAX_SMOOTHING {
            adjusted = true;
            Self::MAX_SMOOTHING
        } else {
            smoothing
        };

        (
            Self {
                sensitivity: sens,
                smoothing: smooth,
            },
            adjusted,
        )
    }
}

impl Default for AudioReactivityOptions {
    fn default() -> Self {
        Self {
            sensitivity: Self::DEFAULT_SENSITIVITY,
            smoothing: Self::DEFAULT_SMOOTHING,
        }
    }
}

/// Web-player specific options.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct WebBackendOptions {
    pub audio_reactivity: Option<AudioReactivityOptions>,
    pub background_color: Option<String>,
}

/// Typed backend-specific configuration options.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BackendOptions {
    #[default]
    None,
    Video(VideoBackendOptions),
    Web(WebBackendOptions),
}

/// Complete requested playback configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PlaybackConfig {
    #[serde(default)]
    pub common: CommonPlaybackConfig,
    #[serde(default)]
    pub backend: BackendOptions,
}

impl PlaybackConfig {
    #[must_use]
    pub fn new(common: CommonPlaybackConfig, backend: BackendOptions) -> Self {
        Self { common, backend }
    }

    #[must_use]
    pub fn video_default() -> Self {
        Self {
            common: CommonPlaybackConfig::default(),
            backend: BackendOptions::Video(VideoBackendOptions::default()),
        }
    }

    #[must_use]
    pub fn web_default() -> Self {
        Self {
            common: CommonPlaybackConfig::default(),
            backend: BackendOptions::Web(WebBackendOptions::default()),
        }
    }
}

/// Effective, validated, and negotiated configuration guaranteed ready for backend application.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectivePlaybackConfig {
    pub common: CommonPlaybackConfig,
    pub backend: BackendOptions,
}

impl EffectivePlaybackConfig {
    #[must_use]
    pub fn from_config(config: PlaybackConfig) -> Self {
        Self {
            common: config.common,
            backend: config.backend,
        }
    }
}

/// Dynamic runtime configuration update payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RuntimeConfigUpdate {
    pub volume: Option<f32>,
    pub muted: Option<bool>,
    pub paused: Option<bool>,
}

impl RuntimeConfigUpdate {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.volume.is_none() && self.muted.is_none() && self.paused.is_none()
    }

    #[must_use]
    pub fn set_volume(volume: f32) -> Self {
        Self {
            volume: Some(volume),
            muted: None,
            paused: None,
        }
    }

    #[must_use]
    pub fn set_muted(muted: bool) -> Self {
        Self {
            volume: None,
            muted: Some(muted),
            paused: None,
        }
    }

    #[must_use]
    pub fn set_paused(paused: bool) -> Self {
        Self {
            volume: None,
            muted: None,
            paused: Some(paused),
        }
    }
}

/// Diagnostic result of a dynamic configuration update attempt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ConfigUpdateResult {
    Applied {
        effective_volume: Option<f32>,
        effective_muted: Option<bool>,
        effective_paused: Option<bool>,
    },
    NoChange,
    TargetNotFound,
    StaleInstance,
    TargetNotActive,
    UnsupportedSetting {
        setting: String,
    },
    BackendFailed {
        error: String,
    },
}
