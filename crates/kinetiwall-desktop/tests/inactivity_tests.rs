//! Comprehensive test suite for Windows user-inactivity awareness, idle detection,
//! and privacy-preserving playback policies (Step 55).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use kinetiwall_catalog::{
    AssetReference, AvailabilityState, CatalogError, CatalogLifecycle, WallpaperCatalogRepository,
    WallpaperId, WallpaperMetadata, WallpaperRecord, WallpaperSourceKind, WallpaperType,
};
use kinetiwall_desktop::attachment::constants;
use kinetiwall_desktop::{
    calculate_elapsed_ticks, AssignmentService, AttachmentCoordinator, CandidateRole,
    DesktopCandidateWindow, DesktopUserInactivityProvider, DisplayId, DisplaySessionCoordinator,
    DisplaySessionState, DisplaySnapshotManager, FullscreenPlaybackSettings,
    InactivityError, InactivityPlaybackSettings, InactivityPolicyEvaluation, InactivitySnapshot,
    InactivityState, MockAssignmentRepository, MockDesktopDisplayProvider,
    MockDesktopUserInactivityProvider, MockDesktopWindowProvider, MockWindowRecord,
    MultiMonitorLayoutCoordinator, NativeWindowHandle, SmartPlaybackPolicyEvaluator, WindowRect,
    DEFAULT_INACTIVITY_THRESHOLD_MILLIS, MAX_INACTIVITY_THRESHOLD_MILLIS,
    MIN_INACTIVITY_THRESHOLD_MILLIS,
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
// Test Policy Harness
// -------------------------------------------------------------------------------------------------

struct TestInactivityHarness {
    _window_provider: Arc<MockDesktopWindowProvider>,
    snapshot: Arc<kinetiwall_desktop::DisplayTopologySnapshot>,
    session_coordinator: Arc<Mutex<DisplaySessionCoordinator>>,
    evaluator: SmartPlaybackPolicyEvaluator,
    display1_id: DisplayId,
    display2_id: DisplayId,
}

impl TestInactivityHarness {
    fn new_dual() -> Self {
        let window_provider = Arc::new(MockDesktopWindowProvider::new());
        let display_provider = Arc::new(MockDesktopDisplayProvider::new());

        let mon1 = WindowRect::new(0, 0, 1920, 1080);
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

        let evaluator = SmartPlaybackPolicyEvaluator::new(
            FullscreenPlaybackSettings::default(),
            session_coordinator.clone(),
            window_provider.clone(),
        );

        Self {
            _window_provider: window_provider,
            snapshot,
            session_coordinator,
            evaluator,
            display1_id,
            display2_id,
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------------------------------

#[test]
fn test_calculate_elapsed_ticks_normal_and_wraparound() {
    // 1. Normal monotonically increasing
    assert_eq!(calculate_elapsed_ticks(1000, 5000), 4000);
    assert_eq!(calculate_elapsed_ticks(1000, 1000), 0);

    // 2. 32-bit tick wraparound: last input before wrap, current tick after wrap
    // Example: last input at u32::MAX - 500, current at 100 -> elapsed = 500 + 100 + 1 = 601
    let last = u32::MAX - 500;
    let current = 100;
    assert_eq!(calculate_elapsed_ticks(last, current), 601);
}

#[test]
fn test_inactivity_snapshot_defaults_and_construction() {
    let default_snap = InactivitySnapshot::default();
    assert_eq!(default_snap.state, InactivityState::Unknown);
    assert_eq!(default_snap.elapsed_inactivity_millis, 0);
    assert_eq!(default_snap.idle_threshold_millis, DEFAULT_INACTIVITY_THRESHOLD_MILLIS);
    assert!(!default_snap.is_fresh);

    let active_snap = InactivitySnapshot::new(InactivityState::Active, 5000, 60_000, 1000);
    assert_eq!(active_snap.state, InactivityState::Active);
    assert_eq!(active_snap.elapsed_inactivity_millis, 5000);
    assert!(active_snap.is_fresh);
    assert!(active_snap.state.is_active());
    assert!(!active_snap.state.is_idle());

    let idle_snap = InactivitySnapshot::new(InactivityState::Idle, 75_000, 60_000, 2000);
    assert!(idle_snap.state.is_idle());
    assert!(idle_snap.state.is_reliable());
}

#[test]
fn test_inactivity_policy_evaluation_disabled_by_default() {
    let settings = InactivityPlaybackSettings::default();
    assert!(!settings.enabled, "Policy must default to disabled (opt-in)");

    let idle_snap = InactivitySnapshot::new(InactivityState::Idle, 700_000, 600_000, 1000);
    let eval = InactivityPolicyEvaluation::evaluate(&settings, &idle_snap);
    assert!(!eval.should_pause, "Disabled policy must never trigger pause");
    assert!(eval.trigger_reason.is_none());
}

#[test]
fn test_mock_provider_tick_updates_and_query_failure() {
    let provider = MockDesktopUserInactivityProvider::new(10_000, 25_000);

    // Initial snapshot: 15s elapsed, threshold 60s -> Active
    let snap = provider.query_inactivity_snapshot(60_000);
    assert_eq!(snap.state, InactivityState::Active);
    assert_eq!(snap.elapsed_inactivity_millis, 15_000);

    // Advance time by 50s -> 65s elapsed -> Idle
    provider.advance_time(50_000);
    let idle_snap = provider.query_inactivity_snapshot(60_000);
    assert_eq!(idle_snap.state, InactivityState::Idle);
    assert_eq!(idle_snap.elapsed_inactivity_millis, 65_000);

    // Fresh user input -> 0s elapsed -> Active
    provider.record_user_input();
    let resumed_snap = provider.query_inactivity_snapshot(60_000);
    assert_eq!(resumed_snap.state, InactivityState::Active);
    assert_eq!(resumed_snap.elapsed_inactivity_millis, 0);

    // Simulated query failure -> Unknown
    provider.set_query_failure(Some(5));
    let fail_snap = provider.query_inactivity_snapshot(60_000);
    assert_eq!(fail_snap.state, InactivityState::Unknown);
    let res = provider.query_last_input_ticks();
    assert!(matches!(res, Err(InactivityError::QueryFailed { code: 5 })));
}

#[test]
fn test_user_inactivity_suspends_sessions_when_enabled() {
    let mut harness = TestInactivityHarness::new_dual();

    // Verify initial active state
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(&harness.display1_id).unwrap().state, DisplaySessionState::Active);
        assert_eq!(sc.get_session(&harness.display2_id).unwrap().state, DisplaySessionState::Active);
    }

    // Enable inactivity policy
    let mut settings = InactivityPlaybackSettings::default();
    settings.enabled = true;
    settings.idle_threshold_millis = 60_000;
    harness.evaluator.update_inactivity_settings(settings, &harness.snapshot);

    // Trigger idle snapshot (elapsed 70,000 ms > 60,000 ms)
    let idle_snap = InactivitySnapshot::new(InactivityState::Idle, 70_000, 60_000, 1000);
    harness.evaluator.evaluate_inactivity_state(&idle_snap, &harness.snapshot);

    // Verify both sessions suspended with UserInactivity pause reason
    assert!(harness.evaluator.is_display_inactivity_paused(&harness.display1_id));
    assert!(harness.evaluator.is_display_inactivity_paused(&harness.display2_id));

    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(&harness.display1_id).unwrap().state, DisplaySessionState::Suspended);
        assert_eq!(sc.get_session(&harness.display2_id).unwrap().state, DisplaySessionState::Suspended);
    }
}

#[test]
fn test_user_activity_resumes_sessions() {
    let mut harness = TestInactivityHarness::new_dual();

    let mut settings = InactivityPlaybackSettings::default();
    settings.enabled = true;
    settings.idle_threshold_millis = 60_000;
    harness.evaluator.update_inactivity_settings(settings, &harness.snapshot);

    // 1. Idle -> Suspended
    harness.evaluator.handle_user_inactivity(70_000, &harness.snapshot);
    assert!(harness.evaluator.is_display_inactivity_paused(&harness.display1_id));

    // 2. Activity -> Resumed
    harness.evaluator.handle_user_activity(&harness.snapshot);
    assert!(!harness.evaluator.is_display_inactivity_paused(&harness.display1_id));
    assert!(!harness.evaluator.is_display_inactivity_paused(&harness.display2_id));

    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(&harness.display1_id).unwrap().state, DisplaySessionState::Active);
        assert_eq!(sc.get_session(&harness.display2_id).unwrap().state, DisplaySessionState::Active);
    }
}

#[test]
fn test_pause_reason_independence_user_manual_and_inactivity() {
    let mut harness = TestInactivityHarness::new_dual();

    let mut settings = InactivityPlaybackSettings::default();
    settings.enabled = true;
    settings.idle_threshold_millis = 60_000;
    harness.evaluator.update_inactivity_settings(settings, &harness.snapshot);

    // User manually pauses Display 1
    let _ = harness.evaluator.set_user_manual_pause(&harness.display1_id, true);
    assert!(harness.evaluator.is_display_manually_paused(&harness.display1_id));
    assert!(!harness.evaluator.is_display_manually_paused(&harness.display2_id));

    // Session becomes idle -> both displays get UserInactivity pause reason
    harness.evaluator.handle_user_inactivity(80_000, &harness.snapshot);
    assert!(harness.evaluator.is_display_inactivity_paused(&harness.display1_id));
    assert!(harness.evaluator.is_display_inactivity_paused(&harness.display2_id));

    // User returns and interacts -> activity clears UserInactivity:
    // Display 2 resumes to Active (no other pause reasons)
    // Display 1 remains Suspended (UserManual reason survives!)
    harness.evaluator.handle_user_activity(&harness.snapshot);
    assert!(!harness.evaluator.is_display_inactivity_paused(&harness.display1_id));
    assert!(!harness.evaluator.is_display_inactivity_paused(&harness.display2_id));

    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(
            sc.get_session(&harness.display1_id).unwrap().state,
            DisplaySessionState::Suspended,
            "Display 1 must remain suspended due to manual pause"
        );
        assert_eq!(
            sc.get_session(&harness.display2_id).unwrap().state,
            DisplaySessionState::Active,
            "Display 2 has no other pause reasons and must resume"
        );
    }

    // Now user resumes Display 1 manually -> returns to Active
    let _ = harness.evaluator.set_user_manual_pause(&harness.display1_id, false);
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(&harness.display1_id).unwrap().state, DisplaySessionState::Active);
    }
}

#[test]
fn test_inactivity_threshold_clamping() {
    let mut settings = InactivityPlaybackSettings::default();

    // Too small -> clamped to MIN (10s)
    settings.idle_threshold_millis = 1000;
    assert_eq!(settings.clamped_threshold(), MIN_INACTIVITY_THRESHOLD_MILLIS);

    // Too large -> clamped to MAX (24h)
    settings.idle_threshold_millis = 200_000_000;
    assert_eq!(settings.clamped_threshold(), MAX_INACTIVITY_THRESHOLD_MILLIS);

    // In range -> preserved
    settings.idle_threshold_millis = 300_000;
    assert_eq!(settings.clamped_threshold(), 300_000);
}

#[test]
fn test_policy_toggle_reconciles_active_inactivity_pause() {
    let mut harness = TestInactivityHarness::new_dual();

    // Enable policy and trigger idle pause
    let mut settings = InactivityPlaybackSettings::default();
    settings.enabled = true;
    settings.idle_threshold_millis = 60_000;
    harness.evaluator.update_inactivity_settings(settings, &harness.snapshot);
    harness.evaluator.handle_user_inactivity(90_000, &harness.snapshot);

    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(&harness.display1_id).unwrap().state, DisplaySessionState::Suspended);
    }

    // User disables the policy while system is still idle
    let mut disabled_settings = InactivityPlaybackSettings::default();
    disabled_settings.enabled = false;
    harness.evaluator.update_inactivity_settings(disabled_settings, &harness.snapshot);

    // Active pause must be reconciled immediately and sessions resumed!
    assert!(!harness.evaluator.is_display_inactivity_paused(&harness.display1_id));
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(&harness.display1_id).unwrap().state, DisplaySessionState::Active);
    }
}

