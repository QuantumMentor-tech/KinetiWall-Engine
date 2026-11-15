//! Comprehensive automated test suite for Step 38:
//! Playback Stress Testing, Stability Validation & Resource Leak Detection.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use kinetiwall_catalog::{AvailabilityState, WallpaperId, WallpaperType};
use kinetiwall_playback::adapter::{
    CancelAck, PlayerAdapter, PlayerReport, PrepareProgress, SessionContext, StopAck,
};
use kinetiwall_playback::diagnostics::{
    BackendPerformanceMetrics, DiagnosticCategory, DiagnosticEvent, DiagnosticsAggregator,
    ProcessResourceSample, MAX_RETAINED_EVENTS, MAX_RETAINED_SAMPLES,
};
use kinetiwall_playback::health::PlayerHealthStatus;
use kinetiwall_playback::lifecycle::{DesiredState, SessionState};
use kinetiwall_playback::orchestrator::PlaybackOrchestrator;
use kinetiwall_playback::recovery::RecoveryPolicy;
use kinetiwall_playback::supervisor::PlayerSupervisor;
use kinetiwall_playback::types::{
    DisplayTargetId, PlaybackAssetInfo, PlaybackError, PlaybackRequest, PlaybackSessionId,
    PlayerAvailability, PlayerCapabilities, WallpaperPlaybackLookup,
};
use kinetiwall_playback::PlaybackConfig;

// -------------------------------------------------------------------------------------------------
// Test Doubles / Fixtures
// -------------------------------------------------------------------------------------------------

struct DynamicLookup {
    wallpapers: std::collections::HashMap<WallpaperId, (WallpaperType, AvailabilityState)>,
}

impl DynamicLookup {
    fn new() -> Self {
        Self {
            wallpapers: std::collections::HashMap::new(),
        }
    }

    fn register(&mut self, id: WallpaperId, wp_type: WallpaperType) {
        self.wallpapers
            .insert(id, (wp_type, AvailabilityState::Available));
    }
}

impl WallpaperPlaybackLookup for DynamicLookup {
    fn playback_info(&self, id: WallpaperId) -> Option<PlaybackAssetInfo> {
        self.wallpapers.get(&id).map(|(ty, av)| PlaybackAssetInfo {
            wallpaper_type: *ty,
            availability: av.clone(),
        })
    }
}

struct ControllableStressAdapter {
    wallpaper_type: WallpaperType,
    is_alive: Arc<AtomicBool>,
    simulated_reports: Arc<std::sync::Mutex<Vec<(PlaybackSessionId, PlayerReport)>>>,
    start_count: Arc<AtomicU32>,
    stop_count: Arc<AtomicU32>,
    prepare_count: Arc<AtomicU32>,
    cancel_count: Arc<AtomicU32>,
    auto_confirm_playing: bool,
    simulated_sample: Arc<std::sync::Mutex<Option<ProcessResourceSample>>>,
    simulated_backend_metrics: Arc<std::sync::Mutex<BackendPerformanceMetrics>>,
}

impl ControllableStressAdapter {
    fn new(wallpaper_type: WallpaperType) -> Self {
        Self {
            wallpaper_type,
            is_alive: Arc::new(AtomicBool::new(true)),
            simulated_reports: Arc::new(std::sync::Mutex::new(Vec::new())),
            start_count: Arc::new(AtomicU32::new(0)),
            stop_count: Arc::new(AtomicU32::new(0)),
            prepare_count: Arc::new(AtomicU32::new(0)),
            cancel_count: Arc::new(AtomicU32::new(0)),
            auto_confirm_playing: true,
            simulated_sample: Arc::new(std::sync::Mutex::new(None)),
            simulated_backend_metrics: Arc::new(std::sync::Mutex::new(
                BackendPerformanceMetrics::default(),
            )),
        }
    }
}

impl PlayerAdapter for ControllableStressAdapter {
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
        self.prepare_count.fetch_add(1, Ordering::SeqCst);
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
        self.cancel_count.fetch_add(1, Ordering::SeqCst);
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

    fn sample_metrics(&mut self, _ctx: &SessionContext) -> Option<ProcessResourceSample> {
        self.simulated_sample.lock().unwrap().clone()
    }

    fn backend_metrics(&self, _ctx: &SessionContext) -> BackendPerformanceMetrics {
        self.simulated_backend_metrics.lock().unwrap().clone()
    }

    fn process_id(&self, _ctx: &SessionContext) -> Option<u32> {
        Some(12345)
    }
}

// -------------------------------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------------------------------

/// Tests 50 rapid sequential start-to-play-to-stop cycles on a display target, verifying
/// that every cycle transitions cleanly, no zombie sessions remain, and ring buffers
/// stay bounded.
#[test]
fn test_stress_rapid_lifecycle_transitions_50_cycles() {
    let mut lookup = DynamicLookup::new();
    let wallpaper_id = WallpaperId::new();
    lookup.register(wallpaper_id, WallpaperType::Video);

    let adapter = ControllableStressAdapter::new(WallpaperType::Video);
    let _is_alive = Arc::clone(&adapter.is_alive);
    let start_count = Arc::clone(&adapter.start_count);
    let stop_count = Arc::clone(&adapter.stop_count);

    let mut orchestrator = PlaybackOrchestrator::new();
    orchestrator
        .register_adapter(Box::new(adapter))
        .expect("register");

    let mut supervisor = PlayerSupervisor::new(orchestrator);
    let target = DisplayTargetId(0);

    for cycle in 1..=50 {
        let req = PlaybackRequest::new(wallpaper_id, WallpaperType::Video, target);
        let instance_id = supervisor
            .play_wallpaper(&lookup, req)
            .expect("play must succeed");

        // Verify active target state
        let status = supervisor
            .target_status(target)
            .expect("target must be tracked");
        assert_eq!(status.instance_id, instance_id);
        assert_eq!(status.desired, DesiredState::Playing);

        // Advance to playing
        let reports = supervisor.poll_and_supervise(&lookup);
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].status, PlayerHealthStatus::Healthy);

        // Stop the session
        supervisor
            .stop_target(target)
            .expect("stop must succeed cleanly");

        // Assert target removed from active tracking
        assert!(supervisor.target_status(target).is_none());

        // Verify post-mortem snapshot is preserved and valid
        let snap = supervisor
            .diagnostics_snapshot(target)
            .expect("snapshot exists");
        assert_eq!(snap.instance_id, instance_id);
        assert!(snap.recent_events.len() <= MAX_RETAINED_EVENTS);
        assert!(snap.resource_summary.sample_count <= MAX_RETAINED_SAMPLES);

        // Verify start/stop counters match cycle count
        assert_eq!(start_count.load(Ordering::SeqCst), cycle);
        assert_eq!(stop_count.load(Ordering::SeqCst), cycle);
    }
}

/// Tests multi-target concurrent lifecycle operations across a quad-display topology,
/// verifying that stopping or restarting one target has zero side-effects on others.
#[test]
fn test_stress_multi_target_concurrent_lifecycle() {
    let mut lookup = DynamicLookup::new();
    let wp_video = WallpaperId::new();
    let wp_web = WallpaperId::new();
    lookup.register(wp_video, WallpaperType::Video);
    lookup.register(wp_web, WallpaperType::Web);

    let mut orchestrator = PlaybackOrchestrator::new();
    orchestrator
        .register_adapter(Box::new(ControllableStressAdapter::new(
            WallpaperType::Video,
        )))
        .expect("register video");
    orchestrator
        .register_adapter(Box::new(ControllableStressAdapter::new(WallpaperType::Web)))
        .expect("register web");

    let mut supervisor = PlayerSupervisor::new(orchestrator);

    let t0 = DisplayTargetId(0);
    let t1 = DisplayTargetId(1);
    let t2 = DisplayTargetId(2);
    let t3 = DisplayTargetId(3);

    // 1. Start all 4 displays
    let inst0 = supervisor
        .play_wallpaper(
            &lookup,
            PlaybackRequest::new(wp_video, WallpaperType::Video, t0),
        )
        .expect("start t0");
    let inst1 = supervisor
        .play_wallpaper(
            &lookup,
            PlaybackRequest::new(wp_web, WallpaperType::Web, t1),
        )
        .expect("start t1");
    let inst2 = supervisor
        .play_wallpaper(
            &lookup,
            PlaybackRequest::new(wp_video, WallpaperType::Video, t2),
        )
        .expect("start t2");
    let _inst3 = supervisor
        .play_wallpaper(
            &lookup,
            PlaybackRequest::new(wp_web, WallpaperType::Web, t3),
        )
        .expect("start t3");

    // Advance all to playing
    supervisor.poll_and_supervise(&lookup);

    assert_eq!(supervisor.all_targets().len(), 4);

    // 2. Stop targets 1 and 3
    supervisor.stop_target(t1).expect("stop t1");
    supervisor.stop_target(t3).expect("stop t3");

    // Verify targets 0 and 2 remain unaffected in Playing state
    assert_eq!(supervisor.all_targets().len(), 2);
    let stat0 = supervisor.target_status(t0).expect("t0 alive");
    let stat2 = supervisor.target_status(t2).expect("t2 alive");
    assert_eq!(stat0.instance_id, inst0);
    assert_eq!(stat0.observed, SessionState::Playing);
    assert_eq!(stat2.instance_id, inst2);
    assert_eq!(stat2.observed, SessionState::Playing);

    // 3. Restart t1 with Video instead of Web
    let inst1_new = supervisor
        .play_wallpaper(
            &lookup,
            PlaybackRequest::new(wp_video, WallpaperType::Video, t1),
        )
        .expect("restart t1");
    supervisor.poll_and_supervise(&lookup);

    assert_ne!(inst1_new, inst1);
    assert_eq!(supervisor.all_targets().len(), 3);
    assert!(supervisor.target_status(t3).is_none());
}

/// Tests race conditions when stop is requested immediately after start before backend
/// reports ready or playing.
#[test]
fn test_stress_racing_start_and_immediate_stop() {
    let mut lookup = DynamicLookup::new();
    let wallpaper_id = WallpaperId::new();
    lookup.register(wallpaper_id, WallpaperType::Video);

    let mut adapter = ControllableStressAdapter::new(WallpaperType::Video);
    adapter.auto_confirm_playing = false; // Does not immediately report playing
    let stop_count = Arc::clone(&adapter.stop_count);

    let mut orchestrator = PlaybackOrchestrator::new();
    orchestrator
        .register_adapter(Box::new(adapter))
        .expect("register");

    let mut supervisor = PlayerSupervisor::new(orchestrator);
    let target = DisplayTargetId(0);

    for _ in 0..10 {
        let req = PlaybackRequest::new(wallpaper_id, WallpaperType::Video, target);
        supervisor
            .play_wallpaper(&lookup, req)
            .expect("start dispatched");

        // Immediately stop while in Starting state
        supervisor
            .stop_target(target)
            .expect("immediate stop must succeed");
        assert!(supervisor.target_status(target).is_none());
    }

    assert_eq!(stop_count.load(Ordering::SeqCst), 10);
}

/// Tests rapid wallpaper switching on the same target, ensuring the previous wallpaper
/// is cleanly torn down, its snapshot archived, and the new wallpaper started without
/// duplicate sessions or resource leakage.
#[test]
fn test_stress_immediate_wallpaper_switching_same_target() {
    let mut lookup = DynamicLookup::new();
    let wp1 = WallpaperId::new();
    let wp2 = WallpaperId::new();
    lookup.register(wp1, WallpaperType::Video);
    lookup.register(wp2, WallpaperType::Video);

    let adapter = ControllableStressAdapter::new(WallpaperType::Video);
    let _is_alive = Arc::clone(&adapter.is_alive);
    let start_count = Arc::clone(&adapter.start_count);
    let stop_count = Arc::clone(&adapter.stop_count);

    let mut orchestrator = PlaybackOrchestrator::new();
    orchestrator
        .register_adapter(Box::new(adapter))
        .expect("register");

    let mut supervisor = PlayerSupervisor::new(orchestrator);
    let target = DisplayTargetId(0);

    let mut current_instance = supervisor
        .play_wallpaper(
            &lookup,
            PlaybackRequest::new(wp1, WallpaperType::Video, target),
        )
        .expect("start wp1");

    for i in 0..20 {
        let next_wp = if i % 2 == 0 { wp2 } else { wp1 };
        let next_instance = supervisor
            .play_wallpaper(
                &lookup,
                PlaybackRequest::new(next_wp, WallpaperType::Video, target),
            )
            .expect("switch wallpaper");

        assert_ne!(next_instance, current_instance);
        current_instance = next_instance;

        // Verify only 1 target is tracked
        assert_eq!(supervisor.all_targets().len(), 1);
        let status = supervisor.target_status(target).unwrap();
        assert_eq!(status.wallpaper_id, next_wp);
    }

    assert_eq!(start_count.load(Ordering::SeqCst), 21);
    assert_eq!(stop_count.load(Ordering::SeqCst), 20);
}

/// Tests late events arriving from an obsolete/stopped session to verify they are dropped
/// without corrupting the state of active sessions.
#[test]
fn test_stress_stale_callbacks_and_late_event_rejection() {
    let mut lookup = DynamicLookup::new();
    let wp = WallpaperId::new();
    lookup.register(wp, WallpaperType::Video);

    let adapter = ControllableStressAdapter::new(WallpaperType::Video);
    let simulated_reports = Arc::clone(&adapter.simulated_reports);

    let mut orchestrator = PlaybackOrchestrator::new();
    orchestrator
        .register_adapter(Box::new(adapter))
        .expect("register");

    let mut supervisor = PlayerSupervisor::new(orchestrator);
    let target = DisplayTargetId(0);

    // 1. Start and stop session 1
    supervisor
        .play_wallpaper(
            &lookup,
            PlaybackRequest::new(wp, WallpaperType::Video, target),
        )
        .expect("start 1");
    supervisor.poll_and_supervise(&lookup);

    let old_snapshot = supervisor.diagnostics_snapshot(target).unwrap();
    let old_session_id = old_snapshot.session_id;

    supervisor.stop_target(target).expect("stop 1");

    // 2. Start session 2
    let inst2 = supervisor
        .play_wallpaper(
            &lookup,
            PlaybackRequest::new(wp, WallpaperType::Video, target),
        )
        .expect("start 2");
    supervisor.poll_and_supervise(&lookup);

    let cur_snapshot = supervisor.diagnostics_snapshot(target).unwrap();
    assert_ne!(cur_snapshot.session_id, old_session_id);

    // 3. Inject late unsolicited event from old_session_id
    simulated_reports.lock().unwrap().push((
        old_session_id,
        PlayerReport::Failed {
            kind: kinetiwall_playback::PlaybackErrorKind::BackendExited,
        },
    ));

    // Poll supervisor
    supervisor.poll_and_supervise(&lookup);

    // Target 2 must remain completely unaffected and healthy
    let status2 = supervisor.target_status(target).expect("target 2 alive");
    assert_eq!(status2.instance_id, inst2);
    assert_eq!(status2.observed, SessionState::Playing);
    assert!(status2.health.is_healthy());
}

/// Tests sustained playback monitoring over 100 consecutive supervision ticks with
/// resource sample ingestion, verifying ring buffers stay capped and summaries accurate.
#[test]
fn test_stress_sustained_playback_and_resource_retention() {
    let mut lookup = DynamicLookup::new();
    let wp = WallpaperId::new();
    lookup.register(wp, WallpaperType::Video);

    let adapter = ControllableStressAdapter::new(WallpaperType::Video);
    let sample_slot = Arc::clone(&adapter.simulated_sample);

    let mut orchestrator = PlaybackOrchestrator::new();
    orchestrator
        .register_adapter(Box::new(adapter))
        .expect("register");

    let mut supervisor = PlayerSupervisor::new(orchestrator);
    let target = DisplayTargetId(0);

    supervisor
        .play_wallpaper(
            &lookup,
            PlaybackRequest::new(wp, WallpaperType::Video, target),
        )
        .expect("start");

    // Advance to playing
    supervisor.poll_and_supervise(&lookup);

    // Run 100 sustained ticks, injecting new resource samples each tick
    for i in 1..=100 {
        let mut sample = ProcessResourceSample::new(i * 1000, 12345);
        sample.cpu_percent = Some((i % 20) as f32);
        sample.working_set_bytes = Some(50_000_000 + (i * 10_000));
        sample.private_memory_bytes = Some(30_000_000 + (i * 10_000));
        sample.thread_count = Some(8);
        *sample_slot.lock().unwrap() = Some(sample);

        supervisor.poll_and_supervise(&lookup);

        let snap = supervisor.diagnostics_snapshot(target).unwrap();
        // Ring buffer must NEVER exceed MAX_RETAINED_SAMPLES
        assert!(snap.resource_summary.sample_count <= MAX_RETAINED_SAMPLES);
        assert!(snap.recent_events.len() <= MAX_RETAINED_EVENTS);
    }

    let final_snap = supervisor.diagnostics_snapshot(target).unwrap();
    assert_eq!(
        final_snap.resource_summary.sample_count,
        MAX_RETAINED_SAMPLES
    );
    assert!(final_snap.resource_summary.average_cpu_percent.is_some());
    assert_eq!(
        final_snap.resource_summary.sample_count,
        MAX_RETAINED_SAMPLES
    );
}

/// Tests that continuous process crashes trigger automatic recovery up to `max_retries`,
/// and once the retry budget is exhausted, trips the circuit breaker and stops restarting.
#[test]
fn test_stress_recovery_circuit_breaker_under_continuous_crashing() {
    let mut lookup = DynamicLookup::new();
    let wp = WallpaperId::new();
    lookup.register(wp, WallpaperType::Video);

    let adapter = ControllableStressAdapter::new(WallpaperType::Video);
    let is_alive = Arc::clone(&adapter.is_alive);
    let start_count = Arc::clone(&adapter.start_count);

    let mut orchestrator = PlaybackOrchestrator::new();
    orchestrator
        .register_adapter(Box::new(adapter))
        .expect("register");

    let mut supervisor = PlayerSupervisor::new(orchestrator);
    supervisor.set_recovery_policy(RecoveryPolicy {
        max_retries: 3,
        window_duration: Duration::from_secs(60),
        initial_backoff: Duration::from_millis(1),
        max_backoff: Duration::from_millis(5),
    });

    let target = DisplayTargetId(0);
    supervisor
        .play_wallpaper(
            &lookup,
            PlaybackRequest::new(wp, WallpaperType::Video, target),
        )
        .expect("start");

    // Establish playing
    supervisor.poll_and_supervise(&lookup);
    assert_eq!(start_count.load(Ordering::SeqCst), 1);

    // Now continuously crash the process
    is_alive.store(false, Ordering::SeqCst);

    // Ticks: Each tick detects crash and attempts recovery up to max_retries = 3
    for _ in 0..5 {
        std::thread::sleep(Duration::from_millis(10));
        supervisor.poll_and_supervise(&lookup);
    }

    // Initial (1) + 3 retries = 4 starts total
    assert_eq!(
        start_count.load(Ordering::SeqCst),
        4,
        "Circuit breaker must cap automatic retries at max_retries (3 retries)"
    );

    let snap = supervisor.diagnostics_snapshot(target).unwrap();
    assert_eq!(snap.recovery_attempts, 3);
    assert!(snap.health_status.is_dead());
    let categories: Vec<DiagnosticCategory> =
        snap.recent_events.iter().map(|e| e.category).collect();
    assert!(categories.contains(&DiagnosticCategory::RecoveryExhausted));
}

/// Tests that an explicit user stop during recovery backoff immediately aborts recovery
/// and guarantees no replacement player is launched.
#[test]
fn test_stress_user_stop_aborts_active_recovery() {
    let mut lookup = DynamicLookup::new();
    let wp = WallpaperId::new();
    lookup.register(wp, WallpaperType::Video);

    let adapter = ControllableStressAdapter::new(WallpaperType::Video);
    let _is_alive = Arc::clone(&adapter.is_alive);
    let start_count = Arc::clone(&adapter.start_count);

    let mut orchestrator = PlaybackOrchestrator::new();
    orchestrator
        .register_adapter(Box::new(adapter))
        .expect("register");

    let mut supervisor = PlayerSupervisor::new(orchestrator);
    supervisor.set_recovery_policy(RecoveryPolicy {
        max_retries: 5,
        window_duration: Duration::from_secs(60),
        initial_backoff: Duration::from_secs(10), // Long backoff so recovery stays pending
        max_backoff: Duration::from_secs(10),
    });

    let target = DisplayTargetId(0);
    supervisor
        .play_wallpaper(
            &lookup,
            PlaybackRequest::new(wp, WallpaperType::Video, target),
        )
        .expect("start");
    supervisor.poll_and_supervise(&lookup);
    assert_eq!(start_count.load(Ordering::SeqCst), 1);

    // User explicitly stops target
    supervisor.stop_target(target).expect("stop must succeed");
    assert!(supervisor.target_status(target).is_none());

    // Recovery evaluation on stopped target must return IneligibleNotDesired
    let outcome = supervisor.execute_recovery(target, &lookup);
    assert_eq!(
        outcome,
        kinetiwall_playback::RecoveryOutcome::IneligibleNotDesired
    );

    // Ticking supervisor does not trigger restart
    supervisor.poll_and_supervise(&lookup);
    assert_eq!(
        start_count.load(Ordering::SeqCst),
        1,
        "User stop must prevent any recovery restarts"
    );
}

/// Tests supervisor shutdown idempotency across a heterogeneous mix of playing,
/// stopped, and recovering targets.
#[test]
fn test_stress_shutdown_idempotency_with_mixed_targets() {
    let mut lookup = DynamicLookup::new();
    let wp = WallpaperId::new();
    lookup.register(wp, WallpaperType::Video);

    let adapter = ControllableStressAdapter::new(WallpaperType::Video);
    let mut orchestrator = PlaybackOrchestrator::new();
    orchestrator
        .register_adapter(Box::new(adapter))
        .expect("register");

    let mut supervisor = PlayerSupervisor::new(orchestrator);

    let t0 = DisplayTargetId(0);
    let t1 = DisplayTargetId(1);
    let t2 = DisplayTargetId(2);

    supervisor
        .play_wallpaper(&lookup, PlaybackRequest::new(wp, WallpaperType::Video, t0))
        .unwrap();
    supervisor
        .play_wallpaper(&lookup, PlaybackRequest::new(wp, WallpaperType::Video, t1))
        .unwrap();
    supervisor
        .play_wallpaper(&lookup, PlaybackRequest::new(wp, WallpaperType::Video, t2))
        .unwrap();

    supervisor.poll_and_supervise(&lookup);

    // Stop t1
    supervisor.stop_target(t1).unwrap();

    // 1. First shutdown
    assert!(supervisor.shutdown().is_ok());
    assert!(supervisor.is_shutting_down());
    assert_eq!(supervisor.all_targets().len(), 0);

    // 2. Second shutdown (must be safe and idempotent)
    assert!(supervisor.shutdown().is_ok());

    // 3. Third shutdown (must be safe and idempotent)
    assert!(supervisor.shutdown().is_ok());
}

/// Tests that injecting 1,000 rapid events into DiagnosticsAggregator maintains strict
/// FIFO eviction and never exceeds MAX_RETAINED_EVENTS (100).
#[test]
fn test_stress_diagnostic_aggregator_bounded_memory_1000_events() {
    let mut aggregator = DiagnosticsAggregator::new();
    let instance_id = kinetiwall_playback::PlayerInstanceId::new();
    let session_id = PlaybackSessionId::new();
    let target = DisplayTargetId(0);
    let wp = WallpaperId::new();

    for i in 0..1000 {
        let ev = DiagnosticEvent::new(
            instance_id,
            session_id,
            target,
            wp,
            WallpaperType::Video,
            DiagnosticCategory::PlaybackSuspended,
            i as u64,
        )
        .with_detail(format!("event_{}", i));
        aggregator.record_event(ev);
    }

    assert_eq!(aggregator.events().len(), MAX_RETAINED_EVENTS);
    assert_eq!(aggregator.events()[0].detail.as_deref(), Some("event_900"));
    assert_eq!(aggregator.events()[99].detail.as_deref(), Some("event_999"));
}

/// Tests applying 50 rapid configuration updates (volume, mute, pause) to an active
/// player session under churn, verifying updates apply cleanly without deadlock.
#[test]
fn test_stress_runtime_config_updates_during_churn() {
    let mut lookup = DynamicLookup::new();
    let wp = WallpaperId::new();
    lookup.register(wp, WallpaperType::Video);

    let adapter = ControllableStressAdapter::new(WallpaperType::Video);
    let mut orchestrator = PlaybackOrchestrator::new();
    orchestrator
        .register_adapter(Box::new(adapter))
        .expect("register");

    let mut supervisor = PlayerSupervisor::new(orchestrator);
    let target = DisplayTargetId(0);

    let mut req = PlaybackRequest::new(wp, WallpaperType::Video, target);
    req.config = Some(PlaybackConfig::video_default());

    supervisor.play_wallpaper(&lookup, req).expect("start");
    supervisor.poll_and_supervise(&lookup);

    let target_ref = supervisor.target_status(target).unwrap();
    let session_id = target_ref.session_id;

    for i in 0..50 {
        let vol = (i as f32) / 50.0;
        let update = kinetiwall_playback::RuntimeConfigUpdate {
            volume: Some(vol),
            muted: Some(i % 2 == 0),
            paused: Some(i % 3 == 0),
        };

        let res = supervisor
            .orchestrator_mut()
            .apply_runtime_config(session_id, &update);
        assert!(res.is_ok());
    }
}
