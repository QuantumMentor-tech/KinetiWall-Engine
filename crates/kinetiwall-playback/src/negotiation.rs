//! Backend capability discovery, structured descriptions, negotiation logic,
//! and safe fallback resolution.

use kinetiwall_catalog::WallpaperType;
use serde::{Deserialize, Serialize};

use crate::config::{
    AudioReactivityOptions, BackendOptions, CommonPlaybackConfig, EffectivePlaybackConfig,
    HwAccelPreference, LoopMode, NormalizedVolume, PlaybackConfig, WebBackendOptions,
};
use crate::types::{PlaybackError, PlaybackErrorKind};

/// Structured capability descriptor for a playback backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendCapabilities {
    pub supported_types: Vec<WallpaperType>,
    pub supports_volume: bool,
    pub supports_mute: bool,
    pub supports_loop_control: bool,
    pub supports_pause: bool,
    pub supports_dynamic_volume: bool,
    pub supports_dynamic_mute: bool,
    pub supports_dynamic_pause: bool,
    pub supports_hw_accel_selection: bool,
    pub supports_audio_reactivity: bool,
    pub supports_custom_scaling: bool,
    pub is_runtime_available: bool,
    pub runtime_name: String,
}

impl BackendCapabilities {
    /// Capabilities for native/simulated video player backend (libmpv).
    #[must_use]
    pub fn for_video(runtime_available: bool) -> Self {
        Self {
            supported_types: vec![WallpaperType::Video],
            supports_volume: true,
            supports_mute: true,
            supports_loop_control: true,
            supports_pause: true,
            supports_dynamic_volume: true,
            supports_dynamic_mute: true,
            supports_dynamic_pause: true,
            supports_hw_accel_selection: true,
            supports_audio_reactivity: false,
            supports_custom_scaling: true,
            is_runtime_available: runtime_available,
            runtime_name: "libmpv".into(),
        }
    }

    /// Capabilities for native/simulated web player backend (WebView2).
    #[must_use]
    pub fn for_web(runtime_available: bool) -> Self {
        Self {
            supported_types: vec![WallpaperType::Web],
            supports_volume: false,
            supports_mute: false,
            supports_loop_control: false,
            supports_pause: false,
            supports_dynamic_volume: false,
            supports_dynamic_mute: false,
            supports_dynamic_pause: false,
            supports_hw_accel_selection: false,
            supports_audio_reactivity: true,
            supports_custom_scaling: false,
            is_runtime_available: runtime_available,
            runtime_name: "WebView2".into(),
        }
    }

    /// Capabilities for test / mock adapters.
    #[must_use]
    pub fn for_mock(wallpaper_type: WallpaperType) -> Self {
        Self {
            supported_types: vec![wallpaper_type],
            supports_volume: true,
            supports_mute: true,
            supports_loop_control: true,
            supports_pause: true,
            supports_dynamic_volume: true,
            supports_dynamic_mute: true,
            supports_dynamic_pause: true,
            supports_hw_accel_selection: true,
            supports_audio_reactivity: wallpaper_type == WallpaperType::Web,
            supports_custom_scaling: true,
            is_runtime_available: true,
            runtime_name: "MockAdapter".into(),
        }
    }

    #[must_use]
    pub fn default_for(wallpaper_type: WallpaperType) -> Self {
        match wallpaper_type {
            WallpaperType::Video => Self::for_video(true),
            WallpaperType::Web => Self::for_web(true),
        }
    }

    #[must_use]
    pub fn supports_type(&self, wallpaper_type: WallpaperType) -> bool {
        self.supported_types.contains(&wallpaper_type)
    }
}

/// A specific adjustment made during capability negotiation to reconcile requested settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigAdjustment {
    pub setting: String,
    pub requested: String,
    pub effective: String,
    pub reason: String,
}

/// Comprehensive report of capability negotiation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NegotiationReport {
    pub effective_config: EffectivePlaybackConfig,
    pub adjustments: Vec<ConfigAdjustment>,
    pub warnings: Vec<String>,
}

/// Core negotiation engine: resolves requested configuration against verified backend capabilities.
pub struct Negotiator;

impl Negotiator {
    /// Negotiates requested options against backend capabilities, producing an `EffectivePlaybackConfig`.
    ///
    /// # Safety and Fallback Invariants:
    /// - If the backend runtime is unavailable, fails immediately with `PlaybackErrorKind::BackendUnavailable`.
    /// - If the wallpaper type is unsupported, fails with `PlaybackErrorKind::UnsupportedCapability`.
    /// - Out-of-bounds numbers are clamped to valid safe ranges and recorded in adjustments.
    /// - Unsupported optional features (e.g. audio reactivity on a backend without support) are disabled cleanly
    ///   with recorded diagnostics rather than crashing or aborting playback.
    pub fn negotiate(
        requested: &PlaybackConfig,
        capabilities: &BackendCapabilities,
    ) -> Result<NegotiationReport, PlaybackError> {
        // 1. Verify runtime availability
        if !capabilities.is_runtime_available {
            return Err(PlaybackError::new(
                PlaybackErrorKind::BackendUnavailable,
                format!(
                    "Backend runtime '{}' is currently unavailable on this system",
                    capabilities.runtime_name
                ),
            ));
        }

        let mut adjustments = Vec::new();
        let mut warnings = Vec::new();

        // 2. Validate common options
        let mut effective_common = CommonPlaybackConfig::default();

        // Volume
        let (clamped_vol, vol_adjusted) =
            NormalizedVolume::clamped(requested.common.volume.value());
        if vol_adjusted {
            adjustments.push(ConfigAdjustment {
                setting: "volume".into(),
                requested: format!("{:.2}", requested.common.volume.value()),
                effective: format!("{:.2}", clamped_vol.value()),
                reason: "Volume clamped to valid range [0.0, 1.0]".into(),
            });
        }
        if !capabilities.supports_volume && clamped_vol.value() != NormalizedVolume::DEFAULT {
            adjustments.push(ConfigAdjustment {
                setting: "volume".into(),
                requested: format!("{:.2}", clamped_vol.value()),
                effective: format!("{:.2}", NormalizedVolume::DEFAULT),
                reason: format!(
                    "Backend '{}' does not support volume control; defaulted",
                    capabilities.runtime_name
                ),
            });
            effective_common.volume = NormalizedVolume::default();
        } else {
            effective_common.volume = clamped_vol;
        }

        // Mute
        if !capabilities.supports_mute && requested.common.muted {
            adjustments.push(ConfigAdjustment {
                setting: "muted".into(),
                requested: "true".into(),
                effective: "false".into(),
                reason: format!(
                    "Backend '{}' does not support mute control",
                    capabilities.runtime_name
                ),
            });
            effective_common.muted = false;
        } else {
            effective_common.muted = requested.common.muted;
        }

        // Loop mode
        if !capabilities.supports_loop_control && requested.common.loop_mode != LoopMode::Infinite {
            adjustments.push(ConfigAdjustment {
                setting: "loop_mode".into(),
                requested: format!("{:?}", requested.common.loop_mode),
                effective: format!("{:?}", LoopMode::Infinite),
                reason: format!(
                    "Backend '{}' does not support loop mode selection; defaulted to infinite",
                    capabilities.runtime_name
                ),
            });
            effective_common.loop_mode = LoopMode::Infinite;
        } else {
            effective_common.loop_mode = requested.common.loop_mode;
        }

        // Start mode
        if !capabilities.supports_pause
            && requested.common.start_mode == crate::config::StartMode::Paused
        {
            warnings.push(format!(
                "Backend '{}' does not support paused startup; wallpaper will auto-play",
                capabilities.runtime_name
            ));
            effective_common.start_mode = crate::config::StartMode::AutoPlay;
        } else {
            effective_common.start_mode = requested.common.start_mode;
        }

        // Scaling mode
        if !capabilities.supports_custom_scaling
            && requested.common.scaling_mode != crate::config::ScalingMode::Fill
        {
            adjustments.push(ConfigAdjustment {
                setting: "scaling_mode".into(),
                requested: format!("{:?}", requested.common.scaling_mode),
                effective: format!("{:?}", crate::config::ScalingMode::Fill),
                reason: format!(
                    "Backend '{}' manages its own layout; custom scaling ignored",
                    capabilities.runtime_name
                ),
            });
            effective_common.scaling_mode = crate::config::ScalingMode::Fill;
        } else {
            effective_common.scaling_mode = requested.common.scaling_mode;
        }

        // Hardware acceleration
        if !capabilities.supports_hw_accel_selection
            && requested.common.hardware_acceleration != HwAccelPreference::Auto
        {
            adjustments.push(ConfigAdjustment {
                setting: "hardware_acceleration".into(),
                requested: format!("{:?}", requested.common.hardware_acceleration),
                effective: format!("{:?}", HwAccelPreference::Auto),
                reason: format!(
                    "Backend '{}' does not support hardware acceleration toggles",
                    capabilities.runtime_name
                ),
            });
            effective_common.hardware_acceleration = HwAccelPreference::Auto;
        } else {
            effective_common.hardware_acceleration = requested.common.hardware_acceleration;
        }

        // 3. Negotiate backend-specific options
        let effective_backend = match &requested.backend {
            BackendOptions::None => BackendOptions::None,
            BackendOptions::Video(video_opts) => {
                if !capabilities.supports_type(WallpaperType::Video) {
                    return Err(PlaybackError::new(
                        PlaybackErrorKind::UnsupportedCapability,
                        format!(
                            "Backend '{}' does not support Video wallpaper options",
                            capabilities.runtime_name
                        ),
                    ));
                }
                let mut resolved = video_opts.clone();
                if !capabilities.supports_hw_accel_selection
                    && resolved.hwdec != crate::config::VideoHwDec::AutoSafe
                {
                    adjustments.push(ConfigAdjustment {
                        setting: "video.hwdec".into(),
                        requested: format!("{:?}", resolved.hwdec),
                        effective: format!("{:?}", crate::config::VideoHwDec::AutoSafe),
                        reason: "Hardware decoder selection unavailable; defaulted to AutoSafe"
                            .into(),
                    });
                    resolved.hwdec = crate::config::VideoHwDec::AutoSafe;
                }
                BackendOptions::Video(resolved)
            }
            BackendOptions::Web(web_opts) => {
                if !capabilities.supports_type(WallpaperType::Web) {
                    return Err(PlaybackError::new(
                        PlaybackErrorKind::UnsupportedCapability,
                        format!(
                            "Backend '{}' does not support Web wallpaper options",
                            capabilities.runtime_name
                        ),
                    ));
                }
                let mut resolved = WebBackendOptions {
                    audio_reactivity: None,
                    background_color: web_opts.background_color.clone(),
                };

                if let Some(ar) = &web_opts.audio_reactivity {
                    if !capabilities.supports_audio_reactivity {
                        adjustments.push(ConfigAdjustment {
                            setting: "web.audio_reactivity".into(),
                            requested: "enabled".into(),
                            effective: "disabled".into(),
                            reason: format!(
                                "Backend '{}' does not support audio reactivity",
                                capabilities.runtime_name
                            ),
                        });
                    } else {
                        let (clamped_ar, ar_adjusted) =
                            AudioReactivityOptions::clamped(ar.sensitivity, ar.smoothing);
                        if ar_adjusted {
                            adjustments.push(ConfigAdjustment {
                                setting: "web.audio_reactivity".into(),
                                requested: format!(
                                    "sens={:.2}, smooth={:.2}",
                                    ar.sensitivity, ar.smoothing
                                ),
                                effective: format!(
                                    "sens={:.2}, smooth={:.2}",
                                    clamped_ar.sensitivity, clamped_ar.smoothing
                                ),
                                reason: "Audio reactivity parameters clamped to valid ranges"
                                    .into(),
                            });
                        }
                        resolved.audio_reactivity = Some(clamped_ar);
                    }
                }
                BackendOptions::Web(resolved)
            }
        };

        Ok(NegotiationReport {
            effective_config: EffectivePlaybackConfig {
                common: effective_common,
                backend: effective_backend,
            },
            adjustments,
            warnings,
        })
    }
}
