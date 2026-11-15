//! Comprehensive automated test suite for Step 37:
//! Playback Performance Profiling, Runtime Diagnostics & Resource Monitoring.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use kinetiwall_catalog::{AvailabilityState, WallpaperId, WallpaperType};
use kinetiwall_playback::adapter::{
    CancelAck, PlayerAdapter, PlayerReport, PrepareProgress, SessionContext, StopAck,
};
use kinetiwall_playback::diagnostics::{
    BackendPerformanceMetrics, DiagnosticCategory, DiagnosticEvent, DiagnosticsAggregator,
    LifecycleTimings, ProcessResourceSample, MAX_RETAINED_EVENTS, MAX_RETAINED_SAMPLES,
};
use kinetiwall_playback::orchestrator::PlaybackOrchestrator;
use kinetiwall_playback::recovery::RecoveryPolicy;
use kinetiwall_playback::supervisor::PlayerSupervisor;
use kinetiwall_playback::types::{
    DisplayTargetId, PlaybackAssetInfo, PlaybackError, PlaybackRequest, PlaybackSessionId,
    PlayerAvailability, PlayerCapabilities, WallpaperPlaybackLookup,
};
use kinetiwall_playback::PlayerInstanceId;

// -------------------------------------------------------------------------------------------------
// Test Doubles / Fixtures
// -------------------------------------------------------------------------------------------------

struct MockLookup {
    wallpaper_id: WallpaperId,
    wallpaper_type: WallpaperType,
}

impl WallpaperPlaybackLookup for MockLookup {
    fn playback_info(&self, id: WallpaperId) -> Option<PlaybackAssetInfo> {
        if id == self.wallpaper_id {
            Some(PlaybackAssetInfo {
                wallpaper_type: self.wallpaper_type,
                availability: AvailabilityState::Available,
            })
        } else {
            None
        }
    }
}

struct ControllableDiagnosticsAdapter {
    wallpaper_type: WallpaperType,
    is_alive: Arc<AtomicBool>,
    simulated_reports: Arc<std::sync::Mutex<Vec<(PlaybackSessionId, PlayerReport)>>>,
    start_count: Arc<AtomicU32>,
    auto_confirm_playing: bool,
    simulated_sample: Arc<std::sync::Mutex<Option<ProcessResourceSample>>>,
    simulated_backend_metrics: Arc<std::sync::Mutex<BackendPerformanceMetrics>>,
}

impl ControllableDiagnosticsAdapter {
    fn new(wallpaper_type: WallpaperType) -> Self {
        Self {
            wallpaper_type,
            is_alive: Arc::new(AtomicBool::new(true)),
            simulated_reports: Arc::new(std::sync::Mutex::new(Vec::new())),
            start_count: Arc::new(AtomicU32::new(0)),
            auto_confirm_playing: true,
            simulated_sample: Arc::new(std::sync::Mutex::new(None)),
            simulated_backend_metrics: Arc::new(std::sync::Mutex::new(
                BackendPerformanceMetrics::default(),
            )),
        }
    }
}

impl PlayerAdapter for ControllableDiagnosticsAdapter {
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
        Ok(StopAck::Stopped)
    }

    fn shutdown(&mut self) -> Result<(), PlaybackError> {
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
        Some(4242)
    }
}

// -------------------------------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------------------------------

#[test]
fn test_diagnostic_event_and_category_semantics() {
    let instance_id = PlayerInstanceId::new();
    let session_id = PlaybackSessionId::new();
    let target = DisplayTargetId(0);
    let wallpaper_id = WallpaperId::new();
    let wallpaper_type = WallpaperType::Video;

    let event = DiagnosticEvent::new(
        instance_id,
        session_id,
        target,
        wallpaper_id,
        wallpaper_type,
        DiagnosticCategory::PlaybackStarted,
        150,
    )
    .with_detail("Media opened successfully")
    .with_duration_ms(45);

    assert_eq!(event.instance_id, instance_id);
    assert_eq!(event.session_id, session_id);
    assert_eq!(event.target_id, target);
    assert_eq!(event.wallpaper_id, wallpaper_id);
    assert_eq!(event.wallpaper_type, wallpaper_type);
    assert_eq!(event.category, DiagnosticCategory::PlaybackStarted);
    assert_eq!(event.duration_ms, Some(45));
    assert_eq!(event.detail.as_deref(), Some("Media opened successfully"));
    assert_eq!(event.timestamp_millis, 150);
}

#[test]
fn test_lifecycle_timings_measurement() {
    let t0 = Instant::now();
    let mut timings = LifecycleTimings::new(t0);

    let t1 = t0 + Duration::from_millis(50);
    timings.record_init_started(t1);

    let t2 = t0 + Duration::from_millis(120);
    timings.record_ready(t2);

    let t3 = t0 + Duration::from_millis(200);
    timings.record_playing(t3);

    assert_eq!(timings.time_to_ready(), Some(Duration::from_millis(120)));
    assert_eq!(timings.time_to_playing(), Some(Duration::from_millis(200)));

    let t4 = t0 + Duration::from_millis(1200);
    timings.record_stop_requested(t4);

    let t5 = t0 + Duration::from_millis(1250);
    timings.record_stopped(t5);

    assert_eq!(
        timings.active_duration(t5),
        Some(Duration::from_millis(1050))
    );
    assert_eq!(timings.stop_duration(), Some(Duration::from_millis(50)));

    let summary = timings.summary(t5);
    assert_eq!(summary.time_to_ready_ms, Some(120));
    assert_eq!(summary.time_to_playing_ms, Some(200));
    assert_eq!(summary.active_duration_ms, Some(1050));
    assert_eq!(summary.stop_duration_ms, Some(50));
    assert!(!summary.has_failed);
}

#[test]
fn test_lifecycle_timings_failure_handling() {
    let t0 = Instant::now();
    let mut timings = LifecycleTimings::new(t0);

    let t1 = t0 + Duration::from_millis(60);
    timings.record_failed(t1);

    assert_eq!(timings.time_to_ready(), None);
    assert_eq!(timings.time_to_playing(), None);
    assert_eq!(timings.active_duration(t1), None);

    let summary = timings.summary(t1);
    assert!(summary.has_failed);
    assert_eq!(summary.time_to_playing_ms, None);
}

#[test]
fn test_bounded_diagnostics_aggregator_fifo_eviction() {
    let mut aggregator = DiagnosticsAggregator::new();
    let instance_id = PlayerInstanceId::new();
    let session_id = PlaybackSessionId::new();
    let target = DisplayTargetId(0);
    let wallpaper_id = WallpaperId::new();

    // Push 150 events (limit is 100)
    for i in 0..150 {
        let event = DiagnosticEvent::new(
            instance_id,
            session_id,
            target,
            wallpaper_id,
            WallpaperType::Video,
            DiagnosticCategory::PlaybackSuspended,
            i as u64,
        )
        .with_detail(format!("event_{}", i));
        aggregator.record_event(event);
    }

    assert_eq!(aggregator.events().len(), MAX_RETAINED_EVENTS);
    assert_eq!(aggregator.events().len(), 100);

    // Oldest 50 events should have been dropped; first retained is event_50
    assert_eq!(aggregator.events()[0].detail.as_deref(), Some("event_50"));
    assert_eq!(aggregator.events()[99].detail.as_deref(), Some("event_149"));

    // Push 100 resource samples (limit is 60)
    for i in 0u64..100 {
        let mut sample = ProcessResourceSample::new(i * 1000, 1000);
        sample.cpu_percent = Some(i as f32);
        sample.working_set_bytes = Some(10_000_000 + i * 1000);
        sample.private_memory_bytes = Some(5_000_000 + i * 1000);
        sample.thread_count = Some(4);
        aggregator.record_sample(sample);
    }

    assert_eq!(aggregator.samples().len(), MAX_RETAINED_SAMPLES);
    assert_eq!(aggregator.samples().len(), 60);

    // First retained sample should have timestamp_millis = 40_000
    assert_eq!(aggregator.samples()[0].timestamp_millis, 40_000);
    assert_eq!(aggregator.samples()[59].timestamp_millis, 99_000);
}

#[test]
fn test_resource_summary_statistics() {
    let mut aggregator = DiagnosticsAggregator::new();

    // Empty summary
    let empty_summary = aggregator.summarize_resources();
    assert_eq!(empty_summary.sample_count, 0);
    assert_eq!(empty_summary.average_cpu_percent, None);
    assert_eq!(empty_summary.peak_working_set_bytes, None);
    assert_eq!(empty_summary.latest_working_set_bytes, None);

    // Add samples with varying values
    let mut s1 = ProcessResourceSample::new(1000, 2000);
    s1.cpu_percent = Some(10.0);
    s1.working_set_bytes = Some(50_000_000);
    s1.private_memory_bytes = Some(30_000_000);
    s1.thread_count = Some(5);
    aggregator.record_sample(s1);

    let mut s2 = ProcessResourceSample::new(2000, 2000);
    s2.cpu_percent = None; // Missing CPU measurement should be ignored in avg
    s2.working_set_bytes = Some(120_000_000);
    s2.private_memory_bytes = Some(80_000_000);
    s2.thread_count = Some(8);
    aggregator.record_sample(s2);

    let mut s3 = ProcessResourceSample::new(3000, 2000);
    s3.cpu_percent = Some(20.0);
    s3.working_set_bytes = Some(80_000_000);
    s3.private_memory_bytes = Some(40_000_000);
    s3.thread_count = Some(6);
    aggregator.record_sample(s3);

    let summary = aggregator.summarize_resources();
    assert_eq!(summary.sample_count, 3);
    // (10.0 + 20.0) / 2 = 15.0
    assert!((summary.average_cpu_percent.unwrap() - 15.0).abs() < 0.01);
    assert_eq!(summary.peak_working_set_bytes, Some(120_000_000));
    assert_eq!(summary.peak_private_memory_bytes, Some(80_000_000));
    assert_eq!(summary.latest_working_set_bytes, Some(80_000_000));
}

#[test]
fn test_supervisor_emits_startup_and_stop_diagnostics() {
    let wallpaper_id = WallpaperId::new();
    let lookup = MockLookup {
        wallpaper_id,
        wallpaper_type: WallpaperType::Video,
    };

    let mut orchestrator = PlaybackOrchestrator::new();
    let adapter = ControllableDiagnosticsAdapter::new(WallpaperType::Video);
    orchestrator
        .register_adapter(Box::new(adapter))
        .expect("register");

    let mut supervisor = PlayerSupervisor::new(orchestrator);

    let target = DisplayTargetId(0);
    let req = PlaybackRequest::new(wallpaper_id, WallpaperType::Video, target);

    // 1. Start playback
    supervisor
        .play_wallpaper(&lookup, req)
        .expect("start should succeed");

    let snapshot = supervisor
        .diagnostics_snapshot(target)
        .expect("snapshot exists");
    assert_eq!(snapshot.target_id, target);
    assert_eq!(snapshot.wallpaper_id, wallpaper_id);
    assert_eq!(snapshot.wallpaper_type, WallpaperType::Video);

    // Verify startup events were recorded
    let categories: Vec<DiagnosticCategory> =
        snapshot.recent_events.iter().map(|e| e.category).collect();
    assert!(categories.contains(&DiagnosticCategory::StartupRequested));
    assert!(categories.contains(&DiagnosticCategory::BackendInitStarted));

    // Poll to register ready/playing
    supervisor.poll_and_supervise(&lookup);

    let playing_snapshot = supervisor
        .diagnostics_snapshot(target)
        .expect("snapshot exists");
    let playing_categories: Vec<DiagnosticCategory> = playing_snapshot
        .recent_events
        .iter()
        .map(|e| e.category)
        .collect();
    assert!(playing_categories.contains(&DiagnosticCategory::PlaybackStarted));

    // Verify timings
    assert!(playing_snapshot.timings.time_to_ready_ms.is_some());
    assert!(playing_snapshot.timings.time_to_playing_ms.is_some());
    assert_eq!(playing_snapshot.timings.stop_duration_ms, None);

    // 2. Stop target
    supervisor.stop_target(target).expect("stop should succeed");

    let stop_snapshot = supervisor
        .diagnostics_snapshot(target)
        .expect("snapshot exists after stop");
    let stop_categories: Vec<DiagnosticCategory> = stop_snapshot
        .recent_events
        .iter()
        .map(|e| e.category)
        .collect();
    assert!(stop_categories.contains(&DiagnosticCategory::PlaybackStopped));
    assert!(stop_snapshot.timings.stop_duration_ms.is_some());
}

#[test]
fn test_supervisor_records_process_crash_and_recovery_diagnostics() {
    let wallpaper_id = WallpaperId::new();
    let lookup = MockLookup {
        wallpaper_id,
        wallpaper_type: WallpaperType::Video,
    };

    let adapter = ControllableDiagnosticsAdapter::new(WallpaperType::Video);
    let is_alive_flag = Arc::clone(&adapter.is_alive);

    let mut orchestrator = PlaybackOrchestrator::new();
    orchestrator
        .register_adapter(Box::new(adapter))
        .expect("register");

    let mut supervisor = PlayerSupervisor::new(orchestrator);
    supervisor.set_recovery_policy(RecoveryPolicy {
        max_retries: 2,
        window_duration: Duration::from_secs(30),
        initial_backoff: Duration::from_millis(10),
        max_backoff: Duration::from_millis(50),
    });

    let target = DisplayTargetId(0);
    supervisor
        .play_wallpaper(
            &lookup,
            PlaybackRequest::new(wallpaper_id, WallpaperType::Video, target),
        )
        .expect("start should succeed");

    // Poll once to reach Playing
    supervisor.poll_and_supervise(&lookup);

    // Simulate unexpected process termination
    is_alive_flag.store(false, Ordering::SeqCst);

    // Poll supervisor to detect crash and execute automatic recovery
    supervisor.poll_and_supervise(&lookup);

    let snapshot = supervisor
        .diagnostics_snapshot(target)
        .expect("snapshot exists");
    let categories: Vec<DiagnosticCategory> =
        snapshot.recent_events.iter().map(|e| e.category).collect();
    assert!(categories.contains(&DiagnosticCategory::ProcessExitedUnexpectedly));
    assert!(categories.contains(&DiagnosticCategory::RecoveryAttemptStarted));
    assert!(categories.contains(&DiagnosticCategory::RecoveryCompleted));
    assert_eq!(snapshot.recovery_attempts, 1);
}

#[test]
fn test_supervisor_records_startup_timeout_diagnostics() {
    let wallpaper_id = WallpaperId::new();
    let lookup = MockLookup {
        wallpaper_id,
        wallpaper_type: WallpaperType::Video,
    };

    let mut adapter = ControllableDiagnosticsAdapter::new(WallpaperType::Video);
    adapter.auto_confirm_playing = false; // Never emits playing

    let mut orchestrator = PlaybackOrchestrator::new();
    orchestrator
        .register_adapter(Box::new(adapter))
        .expect("register");

    // Fast 10ms startup timeout for test
    let mut supervisor = PlayerSupervisor::new(orchestrator);
    supervisor.set_startup_timeout(Duration::from_millis(10));
    supervisor.set_recovery_policy(RecoveryPolicy {
        max_retries: 0, // No auto recovery
        ..RecoveryPolicy::default()
    });

    let target = DisplayTargetId(0);
    supervisor
        .play_wallpaper(
            &lookup,
            PlaybackRequest::new(wallpaper_id, WallpaperType::Video, target),
        )
        .expect("start request dispatched");

    // Sleep past timeout
    std::thread::sleep(Duration::from_millis(25));
    supervisor.poll_and_supervise(&lookup);

    let timeout_snapshot = supervisor
        .diagnostics_snapshot(target)
        .expect("snapshot exists");
    let categories: Vec<DiagnosticCategory> = timeout_snapshot
        .recent_events
        .iter()
        .map(|e| e.category)
        .collect();
    assert!(categories.contains(&DiagnosticCategory::InitTimedOut));
    assert!(timeout_snapshot.timings.has_failed);
}

#[test]
fn test_adapter_metrics_collection_and_snapshot() {
    let wallpaper_id = WallpaperId::new();
    let lookup = MockLookup {
        wallpaper_id,
        wallpaper_type: WallpaperType::Video,
    };

    let adapter = ControllableDiagnosticsAdapter::new(WallpaperType::Video);
    let sample_slot = Arc::clone(&adapter.simulated_sample);
    let metrics_slot = Arc::clone(&adapter.simulated_backend_metrics);

    let mut orchestrator = PlaybackOrchestrator::new();
    orchestrator
        .register_adapter(Box::new(adapter))
        .expect("register");

    let mut supervisor = PlayerSupervisor::new(orchestrator);

    let target = DisplayTargetId(1);
    supervisor
        .play_wallpaper(
            &lookup,
            PlaybackRequest::new(wallpaper_id, WallpaperType::Video, target),
        )
        .expect("start should succeed");

    // Reach playing
    supervisor.poll_and_supervise(&lookup);

    // Update simulated metrics
    let mut sample = ProcessResourceSample::new(1000, 4242);
    sample.cpu_percent = Some(4.8);
    sample.working_set_bytes = Some(45_000_000);
    sample.private_memory_bytes = Some(28_000_000);
    sample.thread_count = Some(7);
    *sample_slot.lock().unwrap() = Some(sample);

    *metrics_slot.lock().unwrap() = BackendPerformanceMetrics {
        hwdec_current: Some("d3d11va".to_string()),
        media_load_ms: Some(85),
        web_navigation_ms: None,
        audio_frames_delivered: Some(0),
    };

    // Poll to trigger metric sampling
    supervisor.poll_and_supervise(&lookup);

    let snapshot = supervisor
        .diagnostics_snapshot(target)
        .expect("snapshot exists");
    assert!(snapshot.latest_sample.is_some());
    let latest = snapshot.latest_sample.as_ref().unwrap();
    assert_eq!(latest.pid, 4242);
    assert_eq!(latest.working_set_bytes, Some(45_000_000));
    assert_eq!(
        snapshot.backend_metrics.hwdec_current.as_deref(),
        Some("d3d11va")
    );
    assert_eq!(snapshot.backend_metrics.media_load_ms, Some(85));
    assert_eq!(
        snapshot.resource_summary.latest_working_set_bytes,
        Some(45_000_000)
    );
}

#[test]
fn test_diagnostics_privacy_and_json_export() {
    let wallpaper_id = WallpaperId::new();
    let lookup = MockLookup {
        wallpaper_id,
        wallpaper_type: WallpaperType::Video,
    };

    let adapter = ControllableDiagnosticsAdapter::new(WallpaperType::Video);
    let mut orchestrator = PlaybackOrchestrator::new();
    orchestrator
        .register_adapter(Box::new(adapter))
        .expect("register");

    let mut supervisor = PlayerSupervisor::new(orchestrator);

    let target = DisplayTargetId(0);
    supervisor
        .play_wallpaper(
            &lookup,
            PlaybackRequest::new(wallpaper_id, WallpaperType::Video, target),
        )
        .expect("start should succeed");

    let json = supervisor
        .export_diagnostics_json(target)
        .expect("export should succeed");

    // Verify it is valid JSON
    let val: serde_json::Value = serde_json::from_str(&json).expect("valid JSON expected");
    assert_eq!(val["target_id"], 0);
    assert!(val["instance_id"].is_string());
    assert!(val["session_id"].is_string());
    assert!(val["wallpaper_id"].is_string());
    assert_eq!(val["wallpaper_type"], "video");
    assert!(val["recent_events"].is_array());
    assert!(val["timings"].is_object());
    assert!(val["resource_summary"].is_object());

    // Privacy check: Verify no file paths or drive letters are leaked
    assert!(!json.contains("C:\\"));
    assert!(!json.contains("E:\\"));
    assert!(!json.contains("/Users/"));
    assert!(!json.contains(".mp4"));
}

#[test]
fn test_unrecovered_process_crash_diagnostics() {
    let wallpaper_id = WallpaperId::new();
    let lookup = MockLookup {
        wallpaper_id,
        wallpaper_type: WallpaperType::Video,
    };

    let adapter = ControllableDiagnosticsAdapter::new(WallpaperType::Video);
    let is_alive_flag = Arc::clone(&adapter.is_alive);

    let mut orchestrator = PlaybackOrchestrator::new();
    orchestrator
        .register_adapter(Box::new(adapter))
        .expect("register");

    let mut supervisor = PlayerSupervisor::new(orchestrator);
    supervisor.set_recovery_policy(RecoveryPolicy {
        max_retries: 0, // No auto recovery
        ..RecoveryPolicy::default()
    });

    let target = DisplayTargetId(0);
    supervisor
        .play_wallpaper(
            &lookup,
            PlaybackRequest::new(wallpaper_id, WallpaperType::Video, target),
        )
        .expect("start should succeed");

    supervisor.poll_and_supervise(&lookup);

    // Terminate process unexpectedly
    is_alive_flag.store(false, Ordering::SeqCst);
    supervisor.poll_and_supervise(&lookup);

    let snapshot = supervisor
        .diagnostics_snapshot(target)
        .expect("snapshot exists");
    assert!(snapshot.timings.has_failed);
    let categories: Vec<DiagnosticCategory> =
        snapshot.recent_events.iter().map(|e| e.category).collect();
    assert!(categories.contains(&DiagnosticCategory::ProcessExitedUnexpectedly));
    assert_eq!(snapshot.recovery_attempts, 0);
}

#[test]
fn test_session_switch_diagnostics_isolation() {
    let wp1 = WallpaperId::new();
    let wp2 = WallpaperId::new();

    struct MultiLookup {
        wp1: WallpaperId,
        wp2: WallpaperId,
    }
    impl WallpaperPlaybackLookup for MultiLookup {
        fn playback_info(&self, id: WallpaperId) -> Option<PlaybackAssetInfo> {
            if id == self.wp1 || id == self.wp2 {
                Some(PlaybackAssetInfo {
                    wallpaper_type: WallpaperType::Video,
                    availability: AvailabilityState::Available,
                })
            } else {
                None
            }
        }
    }

    let lookup = MultiLookup { wp1, wp2 };
    let adapter = ControllableDiagnosticsAdapter::new(WallpaperType::Video);
    let mut orchestrator = PlaybackOrchestrator::new();
    orchestrator
        .register_adapter(Box::new(adapter))
        .expect("register");

    let mut supervisor = PlayerSupervisor::new(orchestrator);
    let target = DisplayTargetId(0);

    // 1. Play first wallpaper
    let inst1 = supervisor
        .play_wallpaper(
            &lookup,
            PlaybackRequest::new(wp1, WallpaperType::Video, target),
        )
        .expect("play wp1");
    supervisor.poll_and_supervise(&lookup);

    let snap1 = supervisor.diagnostics_snapshot(target).expect("snap1");
    assert_eq!(snap1.instance_id, inst1);
    assert_eq!(snap1.wallpaper_id, wp1);

    // 2. Switch to second wallpaper on same target
    let inst2 = supervisor
        .play_wallpaper(
            &lookup,
            PlaybackRequest::new(wp2, WallpaperType::Video, target),
        )
        .expect("play wp2");
    supervisor.poll_and_supervise(&lookup);

    let snap2 = supervisor.diagnostics_snapshot(target).expect("snap2");
    assert_ne!(snap2.instance_id, snap1.instance_id);
    assert_eq!(snap2.instance_id, inst2);
    assert_eq!(snap2.wallpaper_id, wp2);
    // Timings must be refreshed for the new session
    assert!(snap2.timings.time_to_ready_ms.is_some());
}
