//! Unit and integration tests for foreground application awareness, fullscreen detection,
//! and smart pause/resume (Step 48).

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use kinetiwall_catalog::{
    AssetReference, AvailabilityState, CatalogError, CatalogLifecycle, WallpaperCatalogRepository,
    WallpaperId, WallpaperMetadata, WallpaperRecord, WallpaperSourceKind, WallpaperType,
};
use kinetiwall_desktop::attachment::constants;
use kinetiwall_desktop::{
    AssignmentService, AttachmentCoordinator, CandidateRole, DesktopCandidateWindow, DisplayId,
    DisplaySessionCoordinator, DisplaySessionState, DisplaySnapshotManager,
    FullscreenClassification, FullscreenClassifier, FullscreenPauseMode, FullscreenPlaybackSettings,
    MockAssignmentRepository, MockDesktopDisplayProvider, MockDesktopWindowProvider,
    MockWindowRecord, MultiMonitorLayoutCoordinator, MultiMonitorLayoutMode, NativeWindowHandle,
    SmartPlaybackPolicyEvaluator, WindowRect, MAX_POLICY_DIAGNOSTIC_EVENTS,
};

// -------------------------------------------------------------------------------------------------
// Test Catalog
// -------------------------------------------------------------------------------------------------

#[derive(Default, Clone)]
struct TestCatalog {
    records: Arc<Mutex<BTreeMap<WallpaperId, WallpaperRecord>>>,
}

impl TestCatalog {
    fn new() -> Self {
        Self::default()
    }

    fn add_wallpaper(&self, id: WallpaperId, title: &str) {
        let record = WallpaperRecord {
            id,
            schema_version: 1,
            wallpaper_type: WallpaperType::Video,
            asset: AssetReference {
                source_kind: WallpaperSourceKind::ManagedPackage,
                entrypoint: "video.mp4".to_string(),
            },
            metadata: WallpaperMetadata::new(title),
            lifecycle: CatalogLifecycle::now(false),
            availability: AvailabilityState::Available,
            preserved_extra: BTreeMap::new(),
        };
        self.records.lock().unwrap().insert(id, record);
    }
}

impl WallpaperCatalogRepository for TestCatalog {
    fn create(&mut self, record: WallpaperRecord) -> Result<WallpaperRecord, CatalogError> {
        let id = record.id;
        self.records.lock().unwrap().insert(id, record.clone());
        Ok(record)
    }

    fn get(&self, id: WallpaperId) -> Result<Option<WallpaperRecord>, CatalogError> {
        Ok(self.records.lock().unwrap().get(&id).cloned())
    }

    fn list(&self) -> Result<Vec<WallpaperRecord>, CatalogError> {
        Ok(self.records.lock().unwrap().values().cloned().collect())
    }

    fn update_metadata(
        &mut self,
        _id: WallpaperId,
        _metadata: WallpaperMetadata,
        _updated_at: chrono::DateTime<Utc>,
    ) -> Result<WallpaperRecord, CatalogError> {
        unimplemented!()
    }

    fn relink_asset(
        &mut self,
        _id: WallpaperId,
        _new_asset: AssetReference,
        _availability: AvailabilityState,
        _updated_at: chrono::DateTime<Utc>,
    ) -> Result<WallpaperRecord, CatalogError> {
        unimplemented!()
    }

    fn remove_record(&mut self, id: WallpaperId) -> Result<bool, CatalogError> {
        Ok(self.records.lock().unwrap().remove(&id).is_some())
    }

    fn check_availability(&mut self, id: WallpaperId) -> Result<AvailabilityState, CatalogError> {
        if let Some(r) = self.records.lock().unwrap().get(&id) {
            Ok(r.availability.clone())
        } else {
            Err(CatalogError::NotFound(id))
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Test Harness
// -------------------------------------------------------------------------------------------------

struct TestPolicyHarness {
    window_provider: Arc<MockDesktopWindowProvider>,
    _display_provider: Arc<MockDesktopDisplayProvider>,
    _display_manager: DisplaySnapshotManager,
    snapshot: Arc<kinetiwall_desktop::DisplayTopologySnapshot>,
    session_coordinator: Arc<Mutex<DisplaySessionCoordinator>>,
    _layout_coordinator: MultiMonitorLayoutCoordinator,
    display1_id: DisplayId,
    display2_id: DisplayId,
    owning_pid: u32,
}

impl TestPolicyHarness {
    fn new_dual() -> Self {
        let window_provider = Arc::new(MockDesktopWindowProvider::new());
        let display_provider = Arc::new(MockDesktopDisplayProvider::new());

        // Mon1: 0..1920, 0..1080 (work area 0..1920, 0..1040)
        let mon1 = WindowRect::new(0, 0, 1920, 1080);
        // Mon2: 1920..3840, 0..1080 (work area 1920..3840, 0..1040)
        let mon2 = WindowRect::new(1920, 0, 3840, 1080);
        display_provider.setup_dual_extended(mon1, mon2);

        let mut display_manager = DisplaySnapshotManager::new(display_provider.clone());
        let (snapshot, _) = display_manager.refresh().expect("Snapshot refresh");
        let display1_id = snapshot.displays[0].id.clone();
        let display2_id = snapshot.displays[1].id.clone();

        let owning_pid = 4000;
        let workerw = NativeWindowHandle(100);

        window_provider.add_window(MockWindowRecord {
            handle: workerw,
            class_name: "WorkerW".to_string(),
            window_title: String::new(),
            process_id: 1000,
            thread_id: 1001,
            parent: None,
            is_visible: true,
            bounds: WindowRect::new(0, 0, 3840, 1080),
            children: Vec::new(),
            style: constants::WS_POPUP | constants::WS_VISIBLE,
            ex_style: 0,
            dpi_context: Some(2),
        });

        let mut attachment_coord = AttachmentCoordinator::new(window_provider.clone());
        attachment_coord.register_allowed_process_id(owning_pid);
        let attachment_coordinator = Arc::new(Mutex::new(attachment_coord));

        let host_candidate = DesktopCandidateWindow {
            handle: workerw,
            class_name: "WorkerW".to_string(),
            window_title: String::new(),
            process_id: 1000,
            thread_id: 1001,
            role: CandidateRole::WallpaperHostCandidate,
            parent: None,
            bounds: Some(WindowRect::new(0, 0, 3840, 1080)),
            is_visible: true,
            has_shelldll_defview_child: false,
            validated_at_millis: 1000,
        };

        let catalog = TestCatalog::new();
        let wp1 = WallpaperId::new();
        let wp2 = WallpaperId::new();
        catalog.add_wallpaper(wp1, "Wallpaper One");
        catalog.add_wallpaper(wp2, "Wallpaper Two");

        let repo = Arc::new(MockAssignmentRepository::new());
        let assignment_service =
            Arc::new(AssignmentService::new(repo).expect("AssignmentService creation"));
        assignment_service
            .assign_wallpaper(&display1_id, wp1)
            .expect("Assign D1");
        assignment_service
            .assign_wallpaper(&display2_id, wp2)
            .expect("Assign D2");

        let session_coord = DisplaySessionCoordinator::with_default_factory(
            window_provider.clone(),
            attachment_coordinator,
        );
        let session_coordinator = Arc::new(Mutex::new(session_coord));

        let mut layout_coordinator = MultiMonitorLayoutCoordinator::new(
            session_coordinator.clone(),
            assignment_service,
        );
        layout_coordinator
            .apply_layout(&snapshot, &catalog, &host_candidate)
            .expect("Apply layout");

        Self {
            window_provider,
            _display_provider: display_provider,
            _display_manager: display_manager,
            snapshot,
            session_coordinator,
            _layout_coordinator: layout_coordinator,
            display1_id,
            display2_id,
            owning_pid,
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------------------------------

#[test]
fn test_fullscreen_classifier_primary_monitor() {
    let harness = TestPolicyHarness::new_dual();
    let snapshot = &harness.snapshot;

    let mut pids = HashSet::new();
    pids.insert(harness.owning_pid);

    // Test game window covering display 1 completely (0, 0, 1920, 1080)
    let game_obs = kinetiwall_desktop::policy::ForegroundObservation {
        handle: NativeWindowHandle(501),
        process_id: 8888,
        thread_id: 8889,
        class_name: "UnrealWindow".to_string(),
        bounds: WindowRect::new(0, 0, 1920, 1080),
        is_visible: true,
        is_minimized: false,
        is_cloaked: false,
        timestamp_millis: 1000,
        observation_generation: 1,
    };

    let classification = FullscreenClassifier::classify(Some(&game_obs), &snapshot, &pids);
    match classification {
        FullscreenClassification::ProbableFullscreen { display_id, coverage_percent } => {
            assert_eq!(display_id, harness.display1_id);
            assert!(coverage_percent >= 99.0, "Expected coverage >= 99%, got {}", coverage_percent);
        }
        other => panic!("Expected ProbableFullscreen, got {:?}", other),
    }
}

#[test]
fn test_fullscreen_classifier_secondary_monitor_offset() {
    let harness = TestPolicyHarness::new_dual();
    let snapshot = &harness.snapshot;

    let mut pids = HashSet::new();
    pids.insert(harness.owning_pid);

    // Game covering display 2 completely (1920, 0, 3840, 1080)
    let game_obs = kinetiwall_desktop::policy::ForegroundObservation {
        handle: NativeWindowHandle(502),
        process_id: 9999,
        thread_id: 9990,
        class_name: "Valve001".to_string(),
        bounds: WindowRect::new(1920, 0, 3840, 1080),
        is_visible: true,
        is_minimized: false,
        is_cloaked: false,
        timestamp_millis: 1000,
        observation_generation: 1,
    };

    let classification = FullscreenClassifier::classify(Some(&game_obs), &snapshot, &pids);
    match classification {
        FullscreenClassification::ProbableFullscreen { display_id, coverage_percent } => {
            assert_eq!(display_id, harness.display2_id);
            assert!(coverage_percent >= 99.0);
        }
        other => panic!("Expected ProbableFullscreen for D2, got {:?}", other),
    }
}

#[test]
fn test_fullscreen_classifier_distinguishes_maximized_from_fullscreen() {
    let harness = TestPolicyHarness::new_dual();
    let snapshot = &harness.snapshot;

    let mut pids = HashSet::new();
    pids.insert(harness.owning_pid);

    // Ordinary maximized browser conforming to work area (0, 0, 1920, 1040) (taskbar at bottom 40px)
    let browser_obs = kinetiwall_desktop::policy::ForegroundObservation {
        handle: NativeWindowHandle(503),
        process_id: 7777,
        thread_id: 7778,
        class_name: "Chrome_WidgetWin_1".to_string(),
        bounds: WindowRect::new(0, 0, 1920, 1040),
        is_visible: true,
        is_minimized: false,
        is_cloaked: false,
        timestamp_millis: 1000,
        observation_generation: 1,
    };

    let classification = FullscreenClassifier::classify(Some(&browser_obs), &snapshot, &pids);
    match classification {
        FullscreenClassification::MaximizedWorkArea { display_id } => {
            assert_eq!(display_id, harness.display1_id);
        }
        other => panic!("Expected MaximizedWorkArea, got {:?}", other),
    }
}

#[test]
fn test_fullscreen_classifier_excludes_kinetiwall_and_shell() {
    let harness = TestPolicyHarness::new_dual();
    let snapshot = &harness.snapshot;

    let mut pids = HashSet::new();
    pids.insert(harness.owning_pid);

    // 1. KinetiWall window
    let own_obs = kinetiwall_desktop::policy::ForegroundObservation {
        handle: NativeWindowHandle(101),
        process_id: harness.owning_pid,
        thread_id: 1002,
        class_name: "KinetiWallPlayerWindow".to_string(),
        bounds: WindowRect::new(0, 0, 1920, 1080),
        is_visible: true,
        is_minimized: false,
        is_cloaked: false,
        timestamp_millis: 1000,
        observation_generation: 1,
    };
    match FullscreenClassifier::classify(Some(&own_obs), &snapshot, &pids) {
        FullscreenClassification::ExcludedWindow { .. } => {}
        other => panic!("Expected ExcludedWindow for self, got {:?}", other),
    }

    // 2. Taskbar window
    let taskbar_obs = kinetiwall_desktop::policy::ForegroundObservation {
        handle: NativeWindowHandle(102),
        process_id: 1000,
        thread_id: 1003,
        class_name: "Shell_TrayWnd".to_string(),
        bounds: WindowRect::new(0, 1040, 1920, 1080),
        is_visible: true,
        is_minimized: false,
        is_cloaked: false,
        timestamp_millis: 1000,
        observation_generation: 1,
    };
    match FullscreenClassifier::classify(Some(&taskbar_obs), &snapshot, &pids) {
        FullscreenClassification::ExcludedWindow { .. } => {}
        other => panic!("Expected ExcludedWindow for taskbar, got {:?}", other),
    }

    // 3. Desktop shell window
    let shell_obs = kinetiwall_desktop::policy::ForegroundObservation {
        handle: NativeWindowHandle(103),
        process_id: 1000,
        thread_id: 1004,
        class_name: "Progman".to_string(),
        bounds: WindowRect::new(0, 0, 3840, 1080),
        is_visible: true,
        is_minimized: false,
        is_cloaked: false,
        timestamp_millis: 1000,
        observation_generation: 1,
    };
    match FullscreenClassifier::classify(Some(&shell_obs), &snapshot, &pids) {
        FullscreenClassification::ExcludedWindow { .. } => {}
        other => panic!("Expected ExcludedWindow for Progman, got {:?}", other),
    }
}

#[test]
fn test_fullscreen_classifier_minimized_and_cloaked() {
    let harness = TestPolicyHarness::new_dual();
    let snapshot = &harness.snapshot;

    let mut pids = HashSet::new();
    pids.insert(harness.owning_pid);

    // Minimized window
    let min_obs = kinetiwall_desktop::policy::ForegroundObservation {
        handle: NativeWindowHandle(601),
        process_id: 8888,
        thread_id: 8889,
        class_name: "GameWindow".to_string(),
        bounds: WindowRect::new(0, 0, 1920, 1080),
        is_visible: true,
        is_minimized: true,
        is_cloaked: false,
        timestamp_millis: 1000,
        observation_generation: 1,
    };
    assert_eq!(
        FullscreenClassifier::classify(Some(&min_obs), &snapshot, &pids),
        FullscreenClassification::NotFullscreen
    );

    // Cloaked window (e.g. on virtual desktop or background UWP)
    let cloaked_obs = kinetiwall_desktop::policy::ForegroundObservation {
        handle: NativeWindowHandle(602),
        process_id: 8888,
        thread_id: 8889,
        class_name: "GameWindow".to_string(),
        bounds: WindowRect::new(0, 0, 1920, 1080),
        is_visible: true,
        is_minimized: false,
        is_cloaked: true,
        timestamp_millis: 1000,
        observation_generation: 1,
    };
    assert_eq!(
        FullscreenClassifier::classify(Some(&cloaked_obs), &snapshot, &pids),
        FullscreenClassification::NotFullscreen
    );
}

#[test]
fn test_policy_evaluator_debouncing() {
    let harness = TestPolicyHarness::new_dual();
    let snapshot = &harness.snapshot;

    let mut settings = FullscreenPlaybackSettings::default();
    settings.debounce_duration_millis = 200; // 200ms debounce

    let mut evaluator = SmartPlaybackPolicyEvaluator::new(
        settings,
        harness.session_coordinator.clone(),
        harness.window_provider.clone(),
    );
    evaluator.register_kinetiwall_pid(harness.owning_pid);

    let game_obs = kinetiwall_desktop::policy::ForegroundObservation {
        handle: NativeWindowHandle(501),
        process_id: 8888,
        thread_id: 8889,
        class_name: "UnrealWindow".to_string(),
        bounds: WindowRect::new(0, 0, 1920, 1080),
        is_visible: true,
        is_minimized: false,
        is_cloaked: false,
        timestamp_millis: 100,
        observation_generation: 1,
    };

    // Evaluation 1 at t=100ms: Debounce candidate initialized, session should STILL be active
    evaluator.evaluate_observation(
        Some(&game_obs),
        &snapshot,
        MultiMonitorLayoutMode::Independent,
        100,
    );
    {
        let sc = harness.session_coordinator.lock().unwrap();
        let session = sc.get_session(&harness.display1_id).unwrap();
        assert_eq!(session.state, DisplaySessionState::Active, "Session should not pause immediately");
    }

    // Evaluation 2 at t=250ms (elapsed 150ms < 200ms threshold): Still debouncing
    evaluator.evaluate_observation(
        Some(&game_obs),
        &snapshot,
        MultiMonitorLayoutMode::Independent,
        250,
    );
    {
        let sc = harness.session_coordinator.lock().unwrap();
        let session = sc.get_session(&harness.display1_id).unwrap();
        assert_eq!(session.state, DisplaySessionState::Active, "Session should still be active");
    }

    // Evaluation 3 at t=350ms (elapsed 250ms >= 200ms threshold): Debounce met, session paused!
    evaluator.evaluate_observation(
        Some(&game_obs),
        &snapshot,
        MultiMonitorLayoutMode::Independent,
        350,
    );
    {
        let sc = harness.session_coordinator.lock().unwrap();
        let session = sc.get_session(&harness.display1_id).unwrap();
        assert_eq!(session.state, DisplaySessionState::Suspended, "Session should now be suspended");
    }
}

#[test]
fn test_per_display_pause_mode_isolates_unaffected_display() {
    let harness = TestPolicyHarness::new_dual();
    let snapshot = &harness.snapshot;

    let mut settings = FullscreenPlaybackSettings::default();
    settings.pause_mode = FullscreenPauseMode::PerDisplay;
    settings.debounce_duration_millis = 0; // immediate for testing

    let mut evaluator = SmartPlaybackPolicyEvaluator::new(
        settings,
        harness.session_coordinator.clone(),
        harness.window_provider.clone(),
    );
    evaluator.register_kinetiwall_pid(harness.owning_pid);

    // Fullscreen app on Display 1 ONLY
    let game_obs = kinetiwall_desktop::policy::ForegroundObservation {
        handle: NativeWindowHandle(501),
        process_id: 8888,
        thread_id: 8889,
        class_name: "GameWindow".to_string(),
        bounds: WindowRect::new(0, 0, 1920, 1080),
        is_visible: true,
        is_minimized: false,
        is_cloaked: false,
        timestamp_millis: 100,
        observation_generation: 1,
    };

    evaluator.evaluate_observation(
        Some(&game_obs),
        &snapshot,
        MultiMonitorLayoutMode::Independent,
        100,
    );

    let sc = harness.session_coordinator.lock().unwrap();
    let s1 = sc.get_session(&harness.display1_id).unwrap();
    let s2 = sc.get_session(&harness.display2_id).unwrap();

    assert_eq!(s1.state, DisplaySessionState::Suspended, "Display 1 must be suspended");
    assert_eq!(s2.state, DisplaySessionState::Active, "Display 2 must remain Active");
}

#[test]
fn test_all_displays_pause_mode_and_clone_mode() {
    let harness = TestPolicyHarness::new_dual();
    let snapshot = &harness.snapshot;

    // 1. AllDisplays mode in Independent layout
    let mut settings = FullscreenPlaybackSettings::default();
    settings.pause_mode = FullscreenPauseMode::AllDisplays;
    settings.debounce_duration_millis = 0;

    let mut evaluator = SmartPlaybackPolicyEvaluator::new(
        settings,
        harness.session_coordinator.clone(),
        harness.window_provider.clone(),
    );
    evaluator.register_kinetiwall_pid(harness.owning_pid);

    let game_obs = kinetiwall_desktop::policy::ForegroundObservation {
        handle: NativeWindowHandle(501),
        process_id: 8888,
        thread_id: 8889,
        class_name: "GameWindow".to_string(),
        bounds: WindowRect::new(0, 0, 1920, 1080),
        is_visible: true,
        is_minimized: false,
        is_cloaked: false,
        timestamp_millis: 100,
        observation_generation: 1,
    };

    evaluator.evaluate_observation(
        Some(&game_obs),
        &snapshot,
        MultiMonitorLayoutMode::Independent,
        100,
    );

    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(&harness.display1_id).unwrap().state, DisplaySessionState::Suspended);
        assert_eq!(sc.get_session(&harness.display2_id).unwrap().state, DisplaySessionState::Suspended);
    }

    // Exiting fullscreen restores both
    evaluator.evaluate_observation(None, &snapshot, MultiMonitorLayoutMode::Independent, 200);
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(&harness.display1_id).unwrap().state, DisplaySessionState::Active);
        assert_eq!(sc.get_session(&harness.display2_id).unwrap().state, DisplaySessionState::Active);
    }

    // 2. Clone mode with PerDisplay setting: Clone layout requires synchronized playback, so all pause
    let mut per_display_settings = FullscreenPlaybackSettings::default();
    per_display_settings.pause_mode = FullscreenPauseMode::PerDisplay;
    per_display_settings.debounce_duration_millis = 0;

    let mut clone_evaluator = SmartPlaybackPolicyEvaluator::new(
        per_display_settings,
        harness.session_coordinator.clone(),
        harness.window_provider.clone(),
    );
    clone_evaluator.register_kinetiwall_pid(harness.owning_pid);

    clone_evaluator.evaluate_observation(
        Some(&game_obs),
        &snapshot,
        MultiMonitorLayoutMode::Clone,
        300,
    );

    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(&harness.display1_id).unwrap().state, DisplaySessionState::Suspended);
        assert_eq!(sc.get_session(&harness.display2_id).unwrap().state, DisplaySessionState::Suspended);
    }
}

#[test]
fn test_independent_reason_tracking_respects_manual_pause() {
    let harness = TestPolicyHarness::new_dual();
    let snapshot = &harness.snapshot;

    let mut settings = FullscreenPlaybackSettings::default();
    settings.debounce_duration_millis = 0;

    let mut evaluator = SmartPlaybackPolicyEvaluator::new(
        settings,
        harness.session_coordinator.clone(),
        harness.window_provider.clone(),
    );
    evaluator.register_kinetiwall_pid(harness.owning_pid);

    // User manually pauses wallpaper on Display 1
    evaluator.register_user_pause(&harness.display1_id).unwrap();

    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(
            sc.get_session(&harness.display1_id).unwrap().state,
            DisplaySessionState::Suspended,
            "Display 1 should be suspended due to user manual pause"
        );
    }

    // Fullscreen app starts on Display 1
    let game_obs = kinetiwall_desktop::policy::ForegroundObservation {
        handle: NativeWindowHandle(501),
        process_id: 8888,
        thread_id: 8889,
        class_name: "GameWindow".to_string(),
        bounds: WindowRect::new(0, 0, 1920, 1080),
        is_visible: true,
        is_minimized: false,
        is_cloaked: false,
        timestamp_millis: 100,
        observation_generation: 1,
    };
    evaluator.evaluate_observation(
        Some(&game_obs),
        &snapshot,
        MultiMonitorLayoutMode::Independent,
        100,
    );

    // Fullscreen app terminates / exits
    evaluator.evaluate_observation(None, &snapshot, MultiMonitorLayoutMode::Independent, 200);

    // CRITICAL: Display 1 must NOT resume because UserManual pause reason is still active!
    {
        let sc = harness.session_coordinator.lock().unwrap();
        let s1 = sc.get_session(&harness.display1_id).unwrap();
        assert_eq!(
            s1.state,
            DisplaySessionState::Suspended,
            "Display 1 must remain suspended because UserManual reason is active"
        );
    }

    // Now user explicitly resumes
    evaluator.clear_user_pause(&harness.display1_id).unwrap();
    {
        let sc = harness.session_coordinator.lock().unwrap();
        let s1 = sc.get_session(&harness.display1_id).unwrap();
        assert_eq!(
            s1.state,
            DisplaySessionState::Active,
            "Display 1 resumes after clearing manual pause"
        );
    }
}

#[test]
fn test_disabling_policy_clears_fullscreen_pauses() {
    let harness = TestPolicyHarness::new_dual();
    let snapshot = &harness.snapshot;

    let mut settings = FullscreenPlaybackSettings::default();
    settings.debounce_duration_millis = 0;

    let mut evaluator = SmartPlaybackPolicyEvaluator::new(
        settings.clone(),
        harness.session_coordinator.clone(),
        harness.window_provider.clone(),
    );
    evaluator.register_kinetiwall_pid(harness.owning_pid);

    let game_obs = kinetiwall_desktop::policy::ForegroundObservation {
        handle: NativeWindowHandle(501),
        process_id: 8888,
        thread_id: 8889,
        class_name: "GameWindow".to_string(),
        bounds: WindowRect::new(0, 0, 1920, 1080),
        is_visible: true,
        is_minimized: false,
        is_cloaked: false,
        timestamp_millis: 100,
        observation_generation: 1,
    };

    evaluator.evaluate_observation(
        Some(&game_obs),
        &snapshot,
        MultiMonitorLayoutMode::Independent,
        100,
    );

    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(&harness.display1_id).unwrap().state, DisplaySessionState::Suspended);
    }

    // User toggles off "Smart Pause on Fullscreen"
    settings.enabled = false;
    evaluator.update_settings(settings);

    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(
            sc.get_session(&harness.display1_id).unwrap().state,
            DisplaySessionState::Active,
            "Session should resume when policy is turned off"
        );
    }
}

#[test]
fn test_bounded_diagnostic_event_history() {
    let harness = TestPolicyHarness::new_dual();
    let snapshot = &harness.snapshot;

    let mut settings = FullscreenPlaybackSettings::default();
    settings.debounce_duration_millis = 0;

    let mut evaluator = SmartPlaybackPolicyEvaluator::new(
        settings,
        harness.session_coordinator.clone(),
        harness.window_provider.clone(),
    );
    evaluator.register_kinetiwall_pid(harness.owning_pid);

    let game_obs = kinetiwall_desktop::policy::ForegroundObservation {
        handle: NativeWindowHandle(501),
        process_id: 8888,
        thread_id: 8889,
        class_name: "GameWindow".to_string(),
        bounds: WindowRect::new(0, 0, 1920, 1080),
        is_visible: true,
        is_minimized: false,
        is_cloaked: false,
        timestamp_millis: 100,
        observation_generation: 1,
    };

    // Emit 300 state changes
    for i in 0..150 {
        evaluator.evaluate_observation(
            Some(&game_obs),
            &snapshot,
            MultiMonitorLayoutMode::Independent,
            100 + i * 2,
        );
        evaluator.evaluate_observation(
            None,
            &snapshot,
            MultiMonitorLayoutMode::Independent,
            101 + i * 2,
        );
    }

    let diagnostics = evaluator.diagnostics();
    assert!(
        diagnostics.len() <= MAX_POLICY_DIAGNOSTIC_EVENTS,
        "Diagnostic log length {} exceeded bound {}",
        diagnostics.len(),
        MAX_POLICY_DIAGNOSTIC_EVENTS
    );
    assert_eq!(diagnostics.len(), MAX_POLICY_DIAGNOSTIC_EVENTS);
}
