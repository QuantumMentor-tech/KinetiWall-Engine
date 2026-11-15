//! Tests for Step 36: Unified Playback Configuration, Backend Capability Negotiation & Safe Runtime Defaults.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use kinetiwall_catalog::{AvailabilityState, WallpaperId, WallpaperType};
use kinetiwall_playback::{
    AudioReactivityOptions, BackendCapabilities, BackendOptions, CommonPlaybackConfig,
    ConfigUpdateResult, DesiredState, DisplayTargetId, EffectivePlaybackConfig, HwAccelPreference,
    LoopMode, Negotiator, NormalizedVolume, PlaybackAssetInfo, PlaybackConfig, PlaybackError,
    PlaybackErrorKind, PlaybackRequest, PlaybackSessionId, PlayerAdapter, PlayerAvailability,
    PlayerCapabilities, PlayerInstanceId, PlayerReport, PlayerSupervisor, PrepareProgress,
    RuntimeConfigUpdate, ScalingMode, SessionContext, StartMode, StopAck, VideoBackendOptions,
    WallpaperPlaybackLookup, WebBackendOptions,
};

// -------------------------------------------------------------------------------------------------
// Test Fixtures
// -------------------------------------------------------------------------------------------------

struct MockLookup {
    wallpaper_id: WallpaperId,
    wallpaper_type: WallpaperType,
    availability: AvailabilityState,
}

impl WallpaperPlaybackLookup for MockLookup {
    fn playback_info(&self, id: WallpaperId) -> Option<PlaybackAssetInfo> {
        if id == self.wallpaper_id {
            Some(PlaybackAssetInfo {
                wallpaper_type: self.wallpaper_type,
                availability: self.availability.clone(),
            })
        } else {
            None
        }
    }
}

struct ConfigurableMockAdapter {
    wallpaper_type: WallpaperType,
    capabilities: BackendCapabilities,
    pub is_alive: Arc<AtomicBool>,
    pub start_count: Arc<AtomicU32>,
    pub last_applied_volume: Arc<Mutex<Option<f32>>>,
    pub last_applied_mute: Arc<Mutex<Option<bool>>>,
    pub last_applied_pause: Arc<Mutex<Option<bool>>>,
    pub simulated_reports: Arc<Mutex<Vec<(PlaybackSessionId, PlayerReport)>>>,
}

impl ConfigurableMockAdapter {
    fn new(wallpaper_type: WallpaperType, capabilities: BackendCapabilities) -> Self {
        Self {
            wallpaper_type,
            capabilities,
            is_alive: Arc::new(AtomicBool::new(true)),
            start_count: Arc::new(AtomicU32::new(0)),
            last_applied_volume: Arc::new(Mutex::new(None)),
            last_applied_mute: Arc::new(Mutex::new(None)),
            last_applied_pause: Arc::new(Mutex::new(None)),
            simulated_reports: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl PlayerAdapter for ConfigurableMockAdapter {
    fn capabilities(&self) -> PlayerCapabilities {
        PlayerCapabilities {
            wallpaper_type: self.wallpaper_type,
            supports_cancellation: true,
            reports_observed_state: true,
        }
    }

    fn availability(&self) -> PlayerAvailability {
        if self.capabilities.is_runtime_available {
            PlayerAvailability::Available
        } else {
            PlayerAvailability::Unavailable {
                reason: "Runtime unavailable".into(),
            }
        }
    }

    fn prepare(&mut self, _ctx: &SessionContext) -> Result<PrepareProgress, PlaybackError> {
        Ok(PrepareProgress::Ready)
    }

    fn start(&mut self, ctx: &SessionContext) -> Result<(), PlaybackError> {
        self.start_count.fetch_add(1, Ordering::SeqCst);
        let mut reports = self.simulated_reports.lock().unwrap();
        reports.push((ctx.session_id, PlayerReport::Playing));
        Ok(())
    }

    fn cancel(
        &mut self,
        _ctx: &SessionContext,
    ) -> Result<kinetiwall_playback::CancelAck, PlaybackError> {
        Ok(kinetiwall_playback::CancelAck::Accepted)
    }

    fn stop(&mut self, _ctx: &SessionContext) -> Result<StopAck, PlaybackError> {
        Ok(StopAck::Stopped)
    }

    fn shutdown(&mut self) -> Result<(), PlaybackError> {
        Ok(())
    }

    fn poll_reports(&mut self) -> Vec<(PlaybackSessionId, PlayerReport)> {
        let mut queue = self.simulated_reports.lock().unwrap();
        let reports = queue.clone();
        queue.clear();
        reports
    }

    fn is_alive(&mut self, _ctx: &SessionContext) -> bool {
        self.is_alive.load(Ordering::SeqCst)
    }

    fn backend_capabilities(&self) -> BackendCapabilities {
        self.capabilities.clone()
    }

    fn apply_runtime_config(
        &mut self,
        _ctx: &SessionContext,
        update: &RuntimeConfigUpdate,
    ) -> Result<(), PlaybackError> {
        if let Some(vol) = update.volume {
            *self.last_applied_volume.lock().unwrap() = Some(vol);
        }
        if let Some(muted) = update.muted {
            *self.last_applied_mute.lock().unwrap() = Some(muted);
        }
        if let Some(paused) = update.paused {
            *self.last_applied_pause.lock().unwrap() = Some(paused);
        }
        Ok(())
    }
}

// -------------------------------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------------------------------

#[test]
fn test_default_playback_configuration_and_serialization() {
    let default_cfg = PlaybackConfig::default();
    assert_eq!(default_cfg.common.volume.value(), 1.0);
    assert!(!default_cfg.common.muted);
    assert_eq!(default_cfg.common.loop_mode, LoopMode::Infinite);
    assert_eq!(default_cfg.common.start_mode, StartMode::AutoPlay);
    assert_eq!(default_cfg.common.scaling_mode, ScalingMode::Fill);
    assert_eq!(
        default_cfg.common.hardware_acceleration,
        HwAccelPreference::Auto
    );
    assert_eq!(default_cfg.backend, BackendOptions::None);

    // Serialization / Deserialization idempotency
    let json = serde_json::to_string(&default_cfg).expect("serialize");
    let deserialized: PlaybackConfig = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(default_cfg, deserialized);
}

#[test]
fn test_volume_clamping_and_bounds_validation() {
    // Valid values
    assert!(NormalizedVolume::new(0.0).is_ok());
    assert!(NormalizedVolume::new(0.5).is_ok());
    assert!(NormalizedVolume::new(1.0).is_ok());

    // Invalid bounds
    assert!(NormalizedVolume::new(-0.01).is_err());
    assert!(NormalizedVolume::new(1.01).is_err());
    assert!(NormalizedVolume::new(f32::NAN).is_err());
    assert!(NormalizedVolume::new(f32::INFINITY).is_err());

    // Clamping behavior
    let (v_low, adj_low) = NormalizedVolume::clamped(-10.0);
    assert_eq!(v_low.value(), 0.0);
    assert!(adj_low);

    let (v_high, adj_high) = NormalizedVolume::clamped(5.5);
    assert_eq!(v_high.value(), 1.0);
    assert!(adj_high);

    let (v_nan, adj_nan) = NormalizedVolume::clamped(f32::NAN);
    assert_eq!(v_nan.value(), NormalizedVolume::DEFAULT);
    assert!(adj_nan);

    let (v_ok, adj_ok) = NormalizedVolume::clamped(0.75);
    assert_eq!(v_ok.value(), 0.75);
    assert!(!adj_ok);
}

#[test]
fn test_audio_reactivity_options_bounds() {
    // Out of range clamping
    let (clamped, adj) = AudioReactivityOptions::clamped(5.0, -0.5);
    assert!(adj);
    assert_eq!(clamped.sensitivity, 3.0);
    assert_eq!(clamped.smoothing, 0.0);

    let (clamped2, adj2) = AudioReactivityOptions::clamped(0.01, 1.2);
    assert!(adj2);
    assert_eq!(clamped2.sensitivity, 0.1);
    assert_eq!(clamped2.smoothing, 0.95);

    let (valid, adj3) = AudioReactivityOptions::clamped(1.5, 0.7);
    assert!(!adj3);
    assert_eq!(valid.sensitivity, 1.5);
    assert_eq!(valid.smoothing, 0.7);
}

#[test]
fn test_negotiation_full_support_video_and_web() {
    let video_caps = BackendCapabilities::for_video(true);
    let video_req = PlaybackConfig::video_default();

    let rep = Negotiator::negotiate(&video_req, &video_caps).expect("video negotiate");
    assert!(rep.adjustments.is_empty());
    assert_eq!(
        rep.effective_config,
        EffectivePlaybackConfig {
            common: CommonPlaybackConfig::default(),
            backend: BackendOptions::Video(VideoBackendOptions::default()),
        }
    );

    let web_caps = BackendCapabilities::for_web(true);
    let web_req = PlaybackConfig {
        common: CommonPlaybackConfig::default(),
        backend: BackendOptions::Web(WebBackendOptions {
            audio_reactivity: Some(AudioReactivityOptions::default()),
            background_color: Some("#000000".into()),
        }),
    };
    let web_rep = Negotiator::negotiate(&web_req, &web_caps).expect("web negotiate");
    assert!(web_rep.adjustments.is_empty());
}

#[test]
fn test_negotiation_missing_runtime_fails() {
    let unavailable_caps = BackendCapabilities::for_video(false);
    let req = PlaybackConfig::video_default();

    let err = Negotiator::negotiate(&req, &unavailable_caps).unwrap_err();
    assert_eq!(err.kind, PlaybackErrorKind::BackendUnavailable);
}

#[test]
fn test_negotiation_unsupported_wallpaper_type() {
    let video_caps = BackendCapabilities::for_video(true);
    let web_req = PlaybackConfig::web_default();

    let err = Negotiator::negotiate(&web_req, &video_caps).unwrap_err();
    assert_eq!(err.kind, PlaybackErrorKind::UnsupportedCapability);
}

#[test]
fn test_negotiation_unsupported_optional_features_fallback() {
    // Backend without volume, without loop control, without audio reactivity
    let mut limited_caps = BackendCapabilities::for_web(true);
    limited_caps.supports_audio_reactivity = false;

    let requested = PlaybackConfig {
        common: CommonPlaybackConfig {
            volume: NormalizedVolume::new(0.5).unwrap(),
            muted: true,
            loop_mode: LoopMode::Once,
            start_mode: StartMode::Paused,
            scaling_mode: ScalingMode::Stretch,
            hardware_acceleration: HwAccelPreference::ForceHardware,
        },
        backend: BackendOptions::Web(WebBackendOptions {
            audio_reactivity: Some(AudioReactivityOptions::default()),
            background_color: None,
        }),
    };

    let rep = Negotiator::negotiate(&requested, &limited_caps).expect("negotiate with fallback");
    assert!(!rep.adjustments.is_empty());

    // Volume defaulted to 1.0
    assert_eq!(rep.effective_config.common.volume.value(), 1.0);
    // Mute disabled
    assert!(!rep.effective_config.common.muted);
    // Loop mode defaulted to Infinite
    assert_eq!(rep.effective_config.common.loop_mode, LoopMode::Infinite);
    // Scaling mode defaulted to Fill
    assert_eq!(rep.effective_config.common.scaling_mode, ScalingMode::Fill);
    // Audio reactivity safely disabled without aborting
    match &rep.effective_config.backend {
        BackendOptions::Web(opts) => {
            assert!(opts.audio_reactivity.is_none());
        }
        _ => panic!("Expected web options"),
    }
}

#[test]
fn test_supervisor_runtime_config_update_success() {
    let wallpaper_id = WallpaperId::new();
    let lookup = MockLookup {
        wallpaper_id,
        wallpaper_type: WallpaperType::Video,
        availability: AvailabilityState::Available,
    };

    let mut orch = kinetiwall_playback::PlaybackOrchestrator::new();
    let adapter =
        ConfigurableMockAdapter::new(WallpaperType::Video, BackendCapabilities::for_video(true));
    let vol_ref = Arc::clone(&adapter.last_applied_volume);
    let mute_ref = Arc::clone(&adapter.last_applied_mute);
    orch.register_adapter(Box::new(adapter)).expect("register");

    let mut supervisor = PlayerSupervisor::new(orch);
    let target = DisplayTargetId(1);
    let req = PlaybackRequest::new(wallpaper_id, WallpaperType::Video, target);

    let instance_id = supervisor.play_wallpaper(&lookup, req).expect("play");
    supervisor.poll_and_supervise(&lookup);

    // Apply dynamic volume update
    let update = RuntimeConfigUpdate::set_volume(0.42);
    let res = supervisor.update_runtime_config(target, instance_id, update);
    assert_eq!(
        res,
        ConfigUpdateResult::Applied {
            effective_volume: Some(0.42),
            effective_muted: None,
            effective_paused: None,
        }
    );
    assert_eq!(*vol_ref.lock().unwrap(), Some(0.42));

    // Apply dynamic mute update
    let mute_update = RuntimeConfigUpdate::set_muted(true);
    let res_mute = supervisor.update_runtime_config(target, instance_id, mute_update);
    assert_eq!(
        res_mute,
        ConfigUpdateResult::Applied {
            effective_volume: None,
            effective_muted: Some(true),
            effective_paused: None,
        }
    );
    assert_eq!(*mute_ref.lock().unwrap(), Some(true));
}

#[test]
fn test_supervisor_runtime_config_update_stale_instance_rejected() {
    let wallpaper_id = WallpaperId::new();
    let lookup = MockLookup {
        wallpaper_id,
        wallpaper_type: WallpaperType::Video,
        availability: AvailabilityState::Available,
    };

    let mut orch = kinetiwall_playback::PlaybackOrchestrator::new();
    let adapter =
        ConfigurableMockAdapter::new(WallpaperType::Video, BackendCapabilities::for_video(true));
    orch.register_adapter(Box::new(adapter)).expect("register");

    let mut supervisor = PlayerSupervisor::new(orch);
    let target = DisplayTargetId(1);
    let req = PlaybackRequest::new(wallpaper_id, WallpaperType::Video, target);

    let _real_instance_id = supervisor.play_wallpaper(&lookup, req).expect("play");

    // Pass an obsolete / different instance id
    let stale_instance_id = PlayerInstanceId::new();
    let update = RuntimeConfigUpdate::set_volume(0.3);
    let res = supervisor.update_runtime_config(target, stale_instance_id, update);

    assert_eq!(res, ConfigUpdateResult::StaleInstance);
}

#[test]
fn test_supervisor_runtime_config_preserved_across_recovery() {
    let wallpaper_id = WallpaperId::new();
    let lookup = MockLookup {
        wallpaper_id,
        wallpaper_type: WallpaperType::Video,
        availability: AvailabilityState::Available,
    };

    let mut orch = kinetiwall_playback::PlaybackOrchestrator::new();
    let adapter =
        ConfigurableMockAdapter::new(WallpaperType::Video, BackendCapabilities::for_video(true));
    let is_alive = Arc::clone(&adapter.is_alive);
    let start_count = Arc::clone(&adapter.start_count);
    orch.register_adapter(Box::new(adapter)).expect("register");

    let mut supervisor = PlayerSupervisor::new(orch);
    let target = DisplayTargetId(1);
    let req = PlaybackRequest::new(wallpaper_id, WallpaperType::Video, target);

    let instance_id = supervisor.play_wallpaper(&lookup, req).expect("play");
    supervisor.poll_and_supervise(&lookup);
    assert_eq!(start_count.load(Ordering::SeqCst), 1);

    // User tunes volume to 0.35
    let update = RuntimeConfigUpdate::set_volume(0.35);
    let res = supervisor.update_runtime_config(target, instance_id, update);
    assert!(matches!(res, ConfigUpdateResult::Applied { .. }));

    // Child crashes!
    is_alive.store(false, Ordering::SeqCst);
    supervisor.poll_and_supervise(&lookup);

    // Supervisor auto-recovers the crashed player
    assert_eq!(start_count.load(Ordering::SeqCst), 2);
    let target_status = supervisor.target_status(target).expect("target status");
    assert_ne!(target_status.instance_id, instance_id);
    assert_eq!(target_status.desired, DesiredState::Playing);

    // Verify the target's preserved config retains volume 0.35!
    let preserved_vol = target_status
        .config
        .as_ref()
        .expect("preserved config")
        .common
        .volume
        .value();
    assert!((preserved_vol - 0.35).abs() < f32::EPSILON);
}

#[test]
fn test_supervisor_runtime_config_unsupported_setting_rejected() {
    let wallpaper_id = WallpaperId::new();
    let lookup = MockLookup {
        wallpaper_id,
        wallpaper_type: WallpaperType::Web,
        availability: AvailabilityState::Available,
    };

    let mut orch = kinetiwall_playback::PlaybackOrchestrator::new();
    // Web backend does not support dynamic volume
    let adapter =
        ConfigurableMockAdapter::new(WallpaperType::Web, BackendCapabilities::for_web(true));
    orch.register_adapter(Box::new(adapter)).expect("register");

    let mut supervisor = PlayerSupervisor::new(orch);
    let target = DisplayTargetId(0);
    let req = PlaybackRequest::new(wallpaper_id, WallpaperType::Web, target);

    let instance_id = supervisor.play_wallpaper(&lookup, req).expect("play");
    supervisor.poll_and_supervise(&lookup);

    // Attempt dynamic volume update on web wallpaper
    let update = RuntimeConfigUpdate::set_volume(0.5);
    let res = supervisor.update_runtime_config(target, instance_id, update);
    assert_eq!(
        res,
        ConfigUpdateResult::UnsupportedSetting {
            setting: "volume".into()
        }
    );
}
