//! Comprehensive automated test suite for Step 35:
//! Unified Player Orchestration, Health Monitoring & Crash Recovery.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use kinetiwall_catalog::{AvailabilityState, WallpaperId, WallpaperType};
use kinetiwall_playback::adapter::{
    CancelAck, PlayerAdapter, PlayerReport, PrepareProgress, SessionContext, StopAck,
};
use kinetiwall_playback::health::PlayerHealthStatus;
use kinetiwall_playback::lifecycle::{DesiredState, SessionState};
use kinetiwall_playback::orchestrator::PlaybackOrchestrator;
use kinetiwall_playback::recovery::{RecoveryOutcome, RecoveryPolicy};
use kinetiwall_playback::supervisor::{AudioCoordinator, PlayerSupervisor};
use kinetiwall_playback::types::{
    DisplayTargetId, PlaybackAssetInfo, PlaybackError, PlaybackErrorKind, PlaybackRequest,
    PlaybackSessionId, PlayerAvailability, PlayerCapabilities, WallpaperPlaybackLookup,
};

// -------------------------------------------------------------------------------------------------
// Test Doubles / Fixtures
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

struct ControllableAdapter {
    wallpaper_type: WallpaperType,
    is_alive: Arc<AtomicBool>,
    simulated_reports: Arc<std::sync::Mutex<Vec<(PlaybackSessionId, PlayerReport)>>>,
    start_count: Arc<AtomicU32>,
    stop_count: Arc<AtomicU32>,
    auto_confirm_playing: bool,
}

impl ControllableAdapter {
    fn new(wallpaper_type: WallpaperType) -> Self {
        Self {
            wallpaper_type,
            is_alive: Arc::new(AtomicBool::new(true)),
            simulated_reports: Arc::new(std::sync::Mutex::new(Vec::new())),
            start_count: Arc::new(AtomicU32::new(0)),
            stop_count: Arc::new(AtomicU32::new(0)),
            auto_confirm_playing: true,
        }
    }
}

impl PlayerAdapter for ControllableAdapter {
    fn capabilities(&self) -> PlayerCapabilities {
        PlayerCapabilities {
            wallpaper_type: self.wallpaper_type,
            supports_cancellation: true,
            reports_observed_state: true,
        }
    }

    fn availability(&self) -> PlayerAvailability {
        PlayerAvailability::Available
    }

    fn prepare(&mut self, _ctx: &SessionContext) -> Result<PrepareProgress, PlaybackError> {
        Ok(PrepareProgress::Ready)
    }

    fn start(&mut self, ctx: &SessionContext) -> Result<(), PlaybackError> {
        self.start_count.fetch_add(1, Ordering::SeqCst);
        if self.auto_confirm_playing {
            self.simulated_reports
                .lock()
                .unwrap()
                .push((ctx.session_id, PlayerReport::Playing));
        }
        Ok(())
    }

    fn cancel(&mut self, _ctx: &SessionContext) -> Result<CancelAck, PlaybackError> {
        Ok(CancelAck::Accepted)
    }

    fn stop(&mut self, _ctx: &SessionContext) -> Result<StopAck, PlaybackError> {
        self.stop_count.fetch_add(1, Ordering::SeqCst);
        Ok(StopAck::Stopped)
    }

    fn shutdown(&mut self) -> Result<(), PlaybackError> {
        self.stop_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn poll_reports(&mut self) -> Vec<(PlaybackSessionId, PlayerReport)> {
        let mut list = self.simulated_reports.lock().unwrap();
        let drained = list.clone();
        list.clear();
        drained
    }

    fn is_alive(&mut self, _ctx: &SessionContext) -> bool {
        self.is_alive.load(Ordering::SeqCst)
    }
}

struct MockAudioCoordinator {
    started_count: Arc<AtomicU32>,
    stopped_count: Arc<AtomicU32>,
    failed_count: Arc<AtomicU32>,
}

impl MockAudioCoordinator {
    fn new() -> (Self, Arc<AtomicU32>, Arc<AtomicU32>, Arc<AtomicU32>) {
        let started = Arc::new(AtomicU32::new(0));
        let stopped = Arc::new(AtomicU32::new(0));
        let failed = Arc::new(AtomicU32::new(0));
        (
            Self {
                started_count: Arc::clone(&started),
                stopped_count: Arc::clone(&stopped),
                failed_count: Arc::clone(&failed),
            },
            started,
            stopped,
            failed,
        )
    }
}

impl AudioCoordinator for MockAudioCoordinator {
    fn on_session_started(&mut self, _id: WallpaperId, _type: WallpaperType) {
        self.started_count.fetch_add(1, Ordering::SeqCst);
    }

    fn on_session_stopped(&mut self) {
        self.stopped_count.fetch_add(1, Ordering::SeqCst);
    }

    fn on_session_failed(&mut self) {
        self.failed_count.fetch_add(1, Ordering::SeqCst);
    }
}

// -------------------------------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------------------------------

#[test]
fn test_supervisor_start_and_normal_observation() {
    let wallpaper_id = WallpaperId::new();
    let lookup = MockLookup {
        wallpaper_id,
        wallpaper_type: WallpaperType::Video,
        availability: AvailabilityState::Available,
    };

    let mut orch = PlaybackOrchestrator::new();
    let adapter = ControllableAdapter::new(WallpaperType::Video);
    orch.register_adapter(Box::new(adapter)).expect("register");

    let mut supervisor = PlayerSupervisor::new(orch);
    let target = DisplayTargetId(1);

    let req = PlaybackRequest::new(wallpaper_id, WallpaperType::Video, target);

    // 1. Play wallpaper
    let instance_id = supervisor.play_wallpaper(&lookup, req).expect("play");

    let status = supervisor.target_status(target).expect("target status");
    assert_eq!(status.instance_id, instance_id);
    assert_eq!(status.desired, DesiredState::Playing);
    assert_eq!(status.observed, SessionState::Starting);
    assert!(status.health.is_healthy());

    // 2. Poll and supervise: processes pending Playing event
    let reports = supervisor.poll_and_supervise(&lookup);
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].status, PlayerHealthStatus::Healthy);

    let updated = supervisor.target_status(target).expect("updated status");
    assert_eq!(updated.observed, SessionState::Playing);
}

#[test]
fn test_supervisor_stop_target_and_cleanup() {
    let wallpaper_id = WallpaperId::new();
    let lookup = MockLookup {
        wallpaper_id,
        wallpaper_type: WallpaperType::Web,
        availability: AvailabilityState::Available,
    };

    let mut orch = PlaybackOrchestrator::new();
    let adapter = ControllableAdapter::new(WallpaperType::Web);
    let stop_count = Arc::clone(&adapter.stop_count);
    orch.register_adapter(Box::new(adapter)).expect("register");

    let mut supervisor = PlayerSupervisor::new(orch);
    let target = DisplayTargetId(0);

    let req = PlaybackRequest::new(wallpaper_id, WallpaperType::Web, target);

    supervisor.play_wallpaper(&lookup, req).expect("play");
    assert!(supervisor.target_status(target).is_some());

    // Stop target
    let stop_res = supervisor.stop_target(target).expect("stop");
    assert_eq!(stop_res, kinetiwall_playback::StopOutcome::Stopped);
    assert!(supervisor.target_status(target).is_none());
    assert!(stop_count.load(Ordering::SeqCst) >= 1);

    // Repeated stop is idempotent
    let stop_again = supervisor.stop_target(target).expect("stop again");
    assert_eq!(stop_again, kinetiwall_playback::StopOutcome::AlreadyStopped);
}

#[test]
fn test_supervisor_detects_process_crash_and_executes_recovery() {
    let wallpaper_id = WallpaperId::new();
    let lookup = MockLookup {
        wallpaper_id,
        wallpaper_type: WallpaperType::Video,
        availability: AvailabilityState::Available,
    };

    let mut orch = PlaybackOrchestrator::new();
    let adapter = ControllableAdapter::new(WallpaperType::Video);
    let is_alive = Arc::clone(&adapter.is_alive);
    let start_count = Arc::clone(&adapter.start_count);
    orch.register_adapter(Box::new(adapter)).expect("register");

    let mut supervisor = PlayerSupervisor::new(orch);
    let target = DisplayTargetId(2);

    let req = PlaybackRequest::new(wallpaper_id, WallpaperType::Video, target);

    let initial_instance = supervisor.play_wallpaper(&lookup, req).expect("play");
    supervisor.poll_and_supervise(&lookup);
    assert_eq!(start_count.load(Ordering::SeqCst), 1);

    // Simulate sudden process crash
    is_alive.store(false, Ordering::SeqCst);

    // Poll and supervise: should detect crash and perform automatic recovery!
    let reports = supervisor.poll_and_supervise(&lookup);
    assert_eq!(reports.len(), 1);

    // A replacement instance was launched with fresh generation token
    let new_target = supervisor.target_status(target).expect("target status");
    assert_ne!(new_target.instance_id, initial_instance);
    assert_eq!(new_target.desired, DesiredState::Playing);
    assert_eq!(
        start_count.load(Ordering::SeqCst),
        2,
        "Replacement player launched"
    );
}

#[test]
fn test_supervisor_recovery_circuit_breaker_budget_exhaustion() {
    let wallpaper_id = WallpaperId::new();
    let lookup = MockLookup {
        wallpaper_id,
        wallpaper_type: WallpaperType::Video,
        availability: AvailabilityState::Available,
    };

    let mut orch = PlaybackOrchestrator::new();
    let adapter = ControllableAdapter::new(WallpaperType::Video);
    let is_alive = Arc::clone(&adapter.is_alive);
    orch.register_adapter(Box::new(adapter)).expect("register");

    let mut supervisor = PlayerSupervisor::new(orch);
    supervisor.set_recovery_policy(RecoveryPolicy {
        max_retries: 2,
        window_duration: Duration::from_secs(60),
        initial_backoff: Duration::from_millis(1),
        max_backoff: Duration::from_millis(5),
    });

    let target = DisplayTargetId(0);
    let req = PlaybackRequest::new(wallpaper_id, WallpaperType::Video, target);

    supervisor.play_wallpaper(&lookup, req).expect("play");

    // Persistent failure mode: process is dead on every attempt
    is_alive.store(false, Ordering::SeqCst);

    // Attempt 1 -> Recovered (retries: 1)
    supervisor.poll_and_supervise(&lookup);
    assert_eq!(
        supervisor
            .target_status(target)
            .unwrap()
            .recovery_budget
            .attempts(),
        1
    );

    // Attempt 2 -> Recovered (retries: 2)
    supervisor.poll_and_supervise(&lookup);
    assert_eq!(
        supervisor
            .target_status(target)
            .unwrap()
            .recovery_budget
            .attempts(),
        2
    );

    // Attempt 3 -> Budget Exhausted! Circuit breaker prevents infinite restart loop
    let reports = supervisor.poll_and_supervise(&lookup);
    assert!(supervisor.target_status(target).unwrap().health.is_dead());
    assert_eq!(
        supervisor
            .target_status(target)
            .unwrap()
            .recovery_budget
            .attempts(),
        2
    );
    assert!(reports[0].status.is_dead());
}

#[test]
fn test_supervisor_non_retryable_failure_does_not_restart() {
    let wallpaper_id = WallpaperId::new();
    let lookup = MockLookup {
        wallpaper_id,
        wallpaper_type: WallpaperType::Web,
        availability: AvailabilityState::Available,
    };

    let mut orch = PlaybackOrchestrator::new();
    let adapter = ControllableAdapter::new(WallpaperType::Web);
    let reports_queue = Arc::clone(&adapter.simulated_reports);
    let start_count = Arc::clone(&adapter.start_count);
    orch.register_adapter(Box::new(adapter)).expect("register");

    let mut supervisor = PlayerSupervisor::new(orch);
    let target = DisplayTargetId(0);
    let req = PlaybackRequest::new(wallpaper_id, WallpaperType::Web, target);

    supervisor.play_wallpaper(&lookup, req).expect("play");
    let session_id = supervisor.target_status(target).unwrap().session_id;

    // Simulate backend reporting a deterministic non-retryable failure (e.g. AssetUnavailable)
    reports_queue.lock().unwrap().push((
        session_id,
        PlayerReport::Failed {
            kind: PlaybackErrorKind::AssetUnavailable,
        },
    ));

    supervisor.poll_and_supervise(&lookup);

    // Status is Dead, and start_count did not increase (no recovery attempted)
    let status = supervisor.target_status(target).unwrap();
    assert!(status.health.is_dead());
    assert_eq!(start_count.load(Ordering::SeqCst), 1);
}

#[test]
fn test_supervisor_user_stop_prevents_recovery() {
    let wallpaper_id = WallpaperId::new();
    let lookup = MockLookup {
        wallpaper_id,
        wallpaper_type: WallpaperType::Video,
        availability: AvailabilityState::Available,
    };

    let mut orch = PlaybackOrchestrator::new();
    let adapter = ControllableAdapter::new(WallpaperType::Video);
    orch.register_adapter(Box::new(adapter)).expect("register");

    let mut supervisor = PlayerSupervisor::new(orch);
    let target = DisplayTargetId(1);
    let req = PlaybackRequest::new(wallpaper_id, WallpaperType::Video, target);

    supervisor.play_wallpaper(&lookup, req).expect("play");

    // User explicitly stops playback
    supervisor.stop_target(target).expect("stop");

    // Recovery outcome must be IneligibleNotDesired
    let outcome = supervisor.execute_recovery(target, &lookup);
    assert_eq!(outcome, RecoveryOutcome::IneligibleNotDesired);
}

#[test]
fn test_audio_reactivity_lifecycle_coordination() {
    let wallpaper_id = WallpaperId::new();
    let lookup = MockLookup {
        wallpaper_id,
        wallpaper_type: WallpaperType::Web,
        availability: AvailabilityState::Available,
    };

    let mut orch = PlaybackOrchestrator::new();
    let adapter = ControllableAdapter::new(WallpaperType::Web);
    let is_alive = Arc::clone(&adapter.is_alive);
    orch.register_adapter(Box::new(adapter)).expect("register");

    let mut supervisor = PlayerSupervisor::new(orch);
    let (coord, started, stopped, failed) = MockAudioCoordinator::new();
    supervisor.set_audio_coordinator(Box::new(coord));

    let target = DisplayTargetId(0);
    let req = PlaybackRequest::new(wallpaper_id, WallpaperType::Web, target);

    // 1. Play session -> audio coordinator informed of session start
    supervisor.play_wallpaper(&lookup, req).expect("play");
    assert_eq!(started.load(Ordering::SeqCst), 1);
    assert_eq!(stopped.load(Ordering::SeqCst), 0);
    assert_eq!(failed.load(Ordering::SeqCst), 0);

    // 2. Process crashes -> audio coordinator informed of failure
    is_alive.store(false, Ordering::SeqCst);
    supervisor.poll_and_supervise(&lookup);
    assert_eq!(failed.load(Ordering::SeqCst), 1);

    // 3. Normal stop -> audio coordinator informed of stop
    supervisor.stop_target(target).expect("stop");
    assert_eq!(stopped.load(Ordering::SeqCst), 1);
}

#[test]
fn test_supervisor_shutdown_is_clean_and_idempotent() {
    let wallpaper_id = WallpaperId::new();
    let lookup = MockLookup {
        wallpaper_id,
        wallpaper_type: WallpaperType::Video,
        availability: AvailabilityState::Available,
    };

    let mut orch = PlaybackOrchestrator::new();
    let adapter = ControllableAdapter::new(WallpaperType::Video);
    let stop_count = Arc::clone(&adapter.stop_count);
    orch.register_adapter(Box::new(adapter)).expect("register");

    let mut supervisor = PlayerSupervisor::new(orch);
    let target = DisplayTargetId(0);
    let req = PlaybackRequest::new(wallpaper_id, WallpaperType::Video, target);

    supervisor.play_wallpaper(&lookup, req).expect("play");

    // 1. First shutdown releases all targets and sessions
    let res1 = supervisor.shutdown();
    assert!(res1.is_ok());
    assert!(supervisor.is_shutting_down());
    assert!(supervisor.all_targets().is_empty());
    assert!(stop_count.load(Ordering::SeqCst) >= 1);

    // 2. Play during shutdown is rejected
    let req2 = PlaybackRequest::new(wallpaper_id, WallpaperType::Video, target);
    let play_err = supervisor.play_wallpaper(&lookup, req2).unwrap_err();
    assert_eq!(play_err.kind, PlaybackErrorKind::ShuttingDown);

    // 3. Second shutdown is a safe no-op
    let res2 = supervisor.shutdown();
    assert!(res2.is_ok());
}
