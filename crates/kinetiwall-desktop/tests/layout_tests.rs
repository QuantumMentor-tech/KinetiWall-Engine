//! Comprehensive unit tests for multi-monitor layout modes, wallpaper spanning,
//! and synchronized clone playback (Step 45).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use kinetiwall_catalog::{
    AssetReference, AvailabilityState, CatalogError, CatalogLifecycle, WallpaperCatalogRepository,
    WallpaperId, WallpaperMetadata, WallpaperRecord, WallpaperSourceKind, WallpaperType,
};
use kinetiwall_desktop::attachment::constants;
use kinetiwall_desktop::{
    AssignmentService, AttachmentCoordinator, CandidateRole, CloneSourcePolicy,
    DesktopCandidateWindow, DisplayId, DisplaySessionCoordinator, DisplaySessionState,
    DisplaySnapshotManager, LayoutError, LayoutPlanValidationOutcome, MockAssignmentRepository,
    MockDesktopDisplayProvider, MockDesktopWindowProvider, MockWindowRecord,
    MultiMonitorLayoutCoordinator, MultiMonitorLayoutMode, NativeWindowHandle, PlaybackSyncLevel,
    WindowRect, SPAN_VIRTUAL_DESKTOP_DISPLAY_ID,
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

    fn add_wallpaper(
        &self,
        id: WallpaperId,
        title: &str,
        wallpaper_type: WallpaperType,
        availability: AvailabilityState,
    ) {
        let record = WallpaperRecord {
            id,
            schema_version: 1,
            wallpaper_type,
            asset: AssetReference {
                source_kind: WallpaperSourceKind::ManagedPackage,
                entrypoint: "test_entry.mp4".to_string(),
            },
            metadata: WallpaperMetadata::new(title),
            lifecycle: CatalogLifecycle::now(false),
            availability,
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

struct TestLayoutHarness {
    _window_provider: Arc<MockDesktopWindowProvider>,
    display_provider: Arc<MockDesktopDisplayProvider>,
    display_manager: DisplaySnapshotManager,
    catalog: TestCatalog,
    assignment_service: Arc<AssignmentService>,
    session_coordinator: Arc<Mutex<DisplaySessionCoordinator>>,
    host_candidate: DesktopCandidateWindow,
    display1_id: DisplayId,
    display2_id: DisplayId,
    wallpaper1_id: WallpaperId,
    wallpaper2_id: WallpaperId,
}

impl TestLayoutHarness {
    fn new() -> Self {
        let window_provider = Arc::new(MockDesktopWindowProvider::new());
        let display_provider = Arc::new(MockDesktopDisplayProvider::new());

        let mon1 = WindowRect::new(0, 0, 1920, 1080);
        let mon2 = WindowRect::new(1920, 0, 3840, 1080);
        display_provider.setup_dual_extended(mon1, mon2);

        let mut display_manager = DisplaySnapshotManager::new(display_provider.clone());
        let (snapshot, _) = display_manager.refresh().expect("Snapshot refresh");
        let display1_id = snapshot.displays[0].id.clone();
        let display2_id = snapshot.displays[1].id.clone();

        let owning_pid = 1234;
        let workerw = NativeWindowHandle(30);

        window_provider.add_window(MockWindowRecord {
            handle: workerw,
            class_name: "WorkerW".to_string(),
            window_title: String::new(),
            process_id: 1000,
            thread_id: 1003,
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
            thread_id: 1003,
            role: CandidateRole::WallpaperHostCandidate,
            parent: None,
            bounds: Some(WindowRect::new(0, 0, 3840, 1080)),
            is_visible: true,
            has_shelldll_defview_child: false,
            validated_at_millis: 1000,
        };

        let catalog = TestCatalog::new();
        let wallpaper1_id = WallpaperId::new();
        let wallpaper2_id = WallpaperId::new();

        catalog.add_wallpaper(
            wallpaper1_id,
            "Cyberpunk City",
            WallpaperType::Video,
            AvailabilityState::Available,
        );

        catalog.add_wallpaper(
            wallpaper2_id,
            "Deep Space",
            WallpaperType::Web,
            AvailabilityState::Available,
        );

        let repo = Arc::new(MockAssignmentRepository::new());
        let assignment_service =
            Arc::new(AssignmentService::new(repo).expect("AssignmentService creation"));

        let session_coord = DisplaySessionCoordinator::with_default_factory(
            window_provider.clone(),
            attachment_coordinator,
        );
        let session_coordinator = Arc::new(Mutex::new(session_coord));

        Self {
            _window_provider: window_provider,
            display_provider,
            display_manager,
            catalog,
            assignment_service,
            session_coordinator,
            host_candidate,
            display1_id,
            display2_id,
            wallpaper1_id,
            wallpaper2_id,
        }
    }

    fn create_coordinator(&self) -> MultiMonitorLayoutCoordinator {
        MultiMonitorLayoutCoordinator::new(
            self.session_coordinator.clone(),
            self.assignment_service.clone(),
        )
    }
}

// -------------------------------------------------------------------------------------------------
// Layout Mode Plan Resolution Tests
// -------------------------------------------------------------------------------------------------

#[test]
fn test_independent_mode_resolution_and_planning() {
    let harness = TestLayoutHarness::new();
    let coordinator = harness.create_coordinator();

    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper1_id)
        .unwrap();
    harness
        .assignment_service
        .assign_wallpaper(&harness.display2_id, harness.wallpaper2_id)
        .unwrap();

    let snapshot = harness.display_manager.current_snapshot();
    let plan = coordinator.evaluate_plan(&snapshot, &harness.catalog);

    assert_eq!(plan.selected_mode, MultiMonitorLayoutMode::Independent);
    assert_eq!(plan.effective_mode, MultiMonitorLayoutMode::Independent);
    assert_eq!(plan.validation_outcome, LayoutPlanValidationOutcome::Valid);
    assert!(plan.is_activatable);
    assert_eq!(plan.surface_plans.len(), 2);

    let sp1 = &plan.surface_plans[0];
    let sp2 = &plan.surface_plans[1];
    assert_eq!(sp1.wallpaper_id, harness.wallpaper1_id);
    assert_eq!(sp2.wallpaper_id, harness.wallpaper2_id);
    assert_eq!(sp1.target_bounds, WindowRect::new(0, 0, 1920, 1080));
    assert_eq!(sp2.target_bounds, WindowRect::new(1920, 0, 3840, 1080));
    assert_eq!(sp1.sync_level, PlaybackSyncLevel::None);
}

#[test]
fn test_span_mode_adjacent_equal_dimensions_geometry() {
    let harness = TestLayoutHarness::new();
    let mut coordinator = harness.create_coordinator();
    coordinator
        .set_saved_mode(MultiMonitorLayoutMode::Span)
        .unwrap();

    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper1_id)
        .unwrap();

    let snapshot = harness.display_manager.current_snapshot();
    let plan = coordinator.evaluate_plan(&snapshot, &harness.catalog);

    assert_eq!(plan.selected_mode, MultiMonitorLayoutMode::Span);
    assert_eq!(plan.effective_mode, MultiMonitorLayoutMode::Span);
    assert_eq!(plan.validation_outcome, LayoutPlanValidationOutcome::Valid);
    assert!(plan.is_activatable);

    // Virtual desktop bounds must be the union: (0, 0, 3840, 1080)
    assert_eq!(
        plan.virtual_desktop_bounds,
        WindowRect::new(0, 0, 3840, 1080)
    );
    assert_eq!(plan.surface_plans.len(), 1);
    assert_eq!(
        plan.surface_plans[0].target_bounds,
        WindowRect::new(0, 0, 3840, 1080)
    );
    assert_eq!(plan.surface_plans[0].wallpaper_id, harness.wallpaper1_id);

    // Viewport mappings
    assert_eq!(plan.viewport_mappings.len(), 2);
    assert_eq!(
        plan.viewport_mappings[0].composition_rect,
        WindowRect::new(0, 0, 1920, 1080)
    );
    assert_eq!(
        plan.viewport_mappings[1].composition_rect,
        WindowRect::new(1920, 0, 3840, 1080)
    );
}

#[test]
fn test_span_mode_heterogeneous_resolutions_and_aspect_ratios() {
    let mut harness = TestLayoutHarness::new();
    // Display 1 is 4K (3840x2160), Display 2 is 1080p (1920x1080)
    harness.display_provider.setup_dual_extended(
        WindowRect::new(0, 0, 3840, 2160),
        WindowRect::new(3840, 0, 5760, 1080),
    );
    let (snapshot, _) = harness.display_manager.refresh().unwrap();

    let mut coordinator = harness.create_coordinator();
    coordinator
        .set_saved_mode(MultiMonitorLayoutMode::Span)
        .unwrap();

    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper1_id)
        .unwrap();

    let plan = coordinator.evaluate_plan(&snapshot, &harness.catalog);
    assert_eq!(
        plan.virtual_desktop_bounds,
        WindowRect::new(0, 0, 5760, 2160)
    );
    assert_eq!(plan.surface_plans[0].target_bounds.width(), 5760);
    assert_eq!(plan.surface_plans[0].target_bounds.height(), 2160);
}

#[test]
fn test_span_mode_negative_coordinates_preservation() {
    let mut harness = TestLayoutHarness::new();
    // Display 2 placed to the left of Display 1: Display 2 at (-1920, 0, 0, 1080), Display 1 at (0, 0, 1920, 1080)
    harness.display_provider.setup_dual_extended(
        WindowRect::new(0, 0, 1920, 1080),
        WindowRect::new(-1920, 0, 0, 1080),
    );
    let (snapshot, _) = harness.display_manager.refresh().unwrap();

    let mut coordinator = harness.create_coordinator();
    coordinator
        .set_saved_mode(MultiMonitorLayoutMode::Span)
        .unwrap();

    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper1_id)
        .unwrap();

    let plan = coordinator.evaluate_plan(&snapshot, &harness.catalog);

    // Negative origin preserved: bounds are (-1920, 0, 1920, 1080)
    assert_eq!(
        plan.virtual_desktop_bounds,
        WindowRect::new(-1920, 0, 1920, 1080)
    );
    assert_eq!(plan.virtual_desktop_bounds.width(), 3840);
    assert_eq!(plan.virtual_desktop_bounds.height(), 1080);

    // Normalized composition rects: Display 2 (left) starts at 0, Display 1 starts at 1920
    let vm_d2 = plan
        .viewport_mappings
        .iter()
        .find(|vm| vm.display_id == harness.display2_id)
        .unwrap();
    let vm_d1 = plan
        .viewport_mappings
        .iter()
        .find(|vm| vm.display_id == harness.display1_id)
        .unwrap();

    assert_eq!(vm_d2.composition_rect, WindowRect::new(0, 0, 1920, 1080));
    assert_eq!(vm_d1.composition_rect, WindowRect::new(1920, 0, 3840, 1080));
}

#[test]
fn test_span_mode_vertically_offset_and_gaps() {
    let mut harness = TestLayoutHarness::new();
    // Display 1 at (0, 0, 1920, 1080), Display 2 with 580px horizontal gap and 200px vertical offset: (2500, 200, 4420, 1280)
    harness.display_provider.setup_dual_extended(
        WindowRect::new(0, 0, 1920, 1080),
        WindowRect::new(2500, 200, 4420, 1280),
    );
    let (snapshot, _) = harness.display_manager.refresh().unwrap();

    let mut coordinator = harness.create_coordinator();
    coordinator
        .set_saved_mode(MultiMonitorLayoutMode::Span)
        .unwrap();

    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper1_id)
        .unwrap();

    let plan = coordinator.evaluate_plan(&snapshot, &harness.catalog);
    assert_eq!(
        plan.virtual_desktop_bounds,
        WindowRect::new(0, 0, 4420, 1280)
    );
    assert_eq!(plan.virtual_desktop_bounds.width(), 4420);
    assert_eq!(plan.virtual_desktop_bounds.height(), 1280);
    assert!(plan.is_activatable);
}

#[test]
fn test_clone_mode_matching_assignments() {
    let harness = TestLayoutHarness::new();
    let mut coordinator = harness.create_coordinator();
    coordinator
        .set_saved_mode(MultiMonitorLayoutMode::Clone)
        .unwrap();

    // Both displays assigned wallpaper 1
    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper1_id)
        .unwrap();
    harness
        .assignment_service
        .assign_wallpaper(&harness.display2_id, harness.wallpaper1_id)
        .unwrap();

    let snapshot = harness.display_manager.current_snapshot();
    let plan = coordinator.evaluate_plan(&snapshot, &harness.catalog);

    assert_eq!(plan.selected_mode, MultiMonitorLayoutMode::Clone);
    assert_eq!(plan.effective_mode, MultiMonitorLayoutMode::Clone);
    assert_eq!(plan.validation_outcome, LayoutPlanValidationOutcome::Valid);
    assert_eq!(plan.authoritative_wallpaper_id, Some(harness.wallpaper1_id));
    assert_eq!(plan.surface_plans.len(), 2);

    for sp in &plan.surface_plans {
        assert_eq!(sp.wallpaper_id, harness.wallpaper1_id);
        assert_eq!(sp.sync_level, PlaybackSyncLevel::BestEffort);
    }
}

#[test]
fn test_clone_mode_conflicting_assignments_with_strict_match() {
    let harness = TestLayoutHarness::new();
    let mut coordinator = harness.create_coordinator();
    coordinator
        .set_saved_mode(MultiMonitorLayoutMode::Clone)
        .unwrap();
    coordinator.set_clone_policy(CloneSourcePolicy::StrictMatch);

    // Displays assigned conflicting wallpapers
    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper1_id)
        .unwrap();
    harness
        .assignment_service
        .assign_wallpaper(&harness.display2_id, harness.wallpaper2_id)
        .unwrap();

    let snapshot = harness.display_manager.current_snapshot();
    let plan = coordinator.evaluate_plan(&snapshot, &harness.catalog);

    assert!(!plan.is_activatable);
    assert!(matches!(
        plan.validation_outcome,
        LayoutPlanValidationOutcome::ConflictingCloneAssignments { .. }
    ));
}

#[test]
fn test_clone_mode_primary_display_authoritative_resolution() {
    let harness = TestLayoutHarness::new();
    let mut coordinator = harness.create_coordinator();
    coordinator
        .set_saved_mode(MultiMonitorLayoutMode::Clone)
        .unwrap();
    coordinator.set_clone_policy(CloneSourcePolicy::PrimaryDisplay);

    // Primary has wallpaper 1, secondary has wallpaper 2
    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper1_id)
        .unwrap();
    harness
        .assignment_service
        .assign_wallpaper(&harness.display2_id, harness.wallpaper2_id)
        .unwrap();

    let snapshot = harness.display_manager.current_snapshot();
    let plan = coordinator.evaluate_plan(&snapshot, &harness.catalog);

    assert!(plan.is_activatable);
    assert_eq!(plan.authoritative_wallpaper_id, Some(harness.wallpaper1_id));
    // Both surfaces will show primary wallpaper 1
    assert_eq!(plan.surface_plans[0].wallpaper_id, harness.wallpaper1_id);
    assert_eq!(plan.surface_plans[1].wallpaper_id, harness.wallpaper1_id);
}

#[test]
fn test_single_monitor_simplifies_to_single_display() {
    let mut harness = TestLayoutHarness::new();
    harness.display_provider.setup_single_display(1920, 1080);
    let (snapshot, _) = harness.display_manager.refresh().unwrap();

    let mut coordinator = harness.create_coordinator();
    // User requested Span or Clone on a single display setup
    coordinator
        .set_saved_mode(MultiMonitorLayoutMode::Span)
        .unwrap();

    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper1_id)
        .unwrap();

    let plan = coordinator.evaluate_plan(&snapshot, &harness.catalog);

    assert_eq!(plan.selected_mode, MultiMonitorLayoutMode::Span);
    // Effective mode simplifies to Independent
    assert_eq!(plan.effective_mode, MultiMonitorLayoutMode::Independent);
    assert_eq!(
        plan.validation_outcome,
        LayoutPlanValidationOutcome::SingleDisplayOnly
    );
    assert!(plan.is_activatable);
    assert_eq!(plan.surface_plans.len(), 1);
}

#[test]
fn test_missing_wallpaper_rejected_safely() {
    let harness = TestLayoutHarness::new();
    let coordinator = harness.create_coordinator();

    let missing_id = WallpaperId::new();
    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, missing_id)
        .unwrap();

    let snapshot = harness.display_manager.current_snapshot();
    let plan = coordinator.evaluate_plan(&snapshot, &harness.catalog);

    assert!(!plan.is_activatable);
}

// -------------------------------------------------------------------------------------------------
// Layout Mode Transitions & Reversibility Tests
// -------------------------------------------------------------------------------------------------

#[test]
fn test_mode_transition_independent_to_span() {
    let harness = TestLayoutHarness::new();
    let mut coordinator = harness.create_coordinator();

    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper1_id)
        .unwrap();
    harness
        .assignment_service
        .assign_wallpaper(&harness.display2_id, harness.wallpaper2_id)
        .unwrap();

    let snapshot = harness.display_manager.current_snapshot();

    // 1. Activate Independent mode
    let report1 = coordinator
        .apply_layout(&snapshot, &harness.catalog, &harness.host_candidate)
        .unwrap();
    assert_eq!(report1.active_mode, MultiMonitorLayoutMode::Independent);
    assert_eq!(report1.activated_surfaces_count, 2);

    // 2. Transition to Span mode
    coordinator
        .set_saved_mode(MultiMonitorLayoutMode::Span)
        .unwrap();
    let report2 = coordinator
        .apply_layout(&snapshot, &harness.catalog, &harness.host_candidate)
        .unwrap();
    assert_eq!(
        report2.previous_mode,
        Some(MultiMonitorLayoutMode::Independent)
    );
    assert_eq!(report2.active_mode, MultiMonitorLayoutMode::Span);
    assert_eq!(report2.activated_surfaces_count, 1);

    // Verify virtual span session is active
    let session_coord = harness.session_coordinator.lock().unwrap();
    let span_id = DisplayId(SPAN_VIRTUAL_DESKTOP_DISPLAY_ID.into());
    let span_session = session_coord.get_session(&span_id).unwrap();
    assert_eq!(span_session.state, DisplaySessionState::Active);
    assert_eq!(
        span_session.geometry.monitor_rect,
        WindowRect::new(0, 0, 3840, 1080)
    );
}

#[test]
fn test_mode_transition_span_to_clone() {
    let harness = TestLayoutHarness::new();
    let mut coordinator = harness.create_coordinator();

    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper1_id)
        .unwrap();
    let snapshot = harness.display_manager.current_snapshot();

    // Start in Span
    coordinator
        .set_saved_mode(MultiMonitorLayoutMode::Span)
        .unwrap();
    coordinator
        .apply_layout(&snapshot, &harness.catalog, &harness.host_candidate)
        .unwrap();

    // Transition to Clone
    coordinator
        .set_saved_mode(MultiMonitorLayoutMode::Clone)
        .unwrap();
    let report = coordinator
        .apply_layout(&snapshot, &harness.catalog, &harness.host_candidate)
        .unwrap();

    assert_eq!(report.active_mode, MultiMonitorLayoutMode::Clone);
    assert_eq!(report.activated_surfaces_count, 2);

    let session_coord = harness.session_coordinator.lock().unwrap();
    let s1 = session_coord.get_session(&harness.display1_id).unwrap();
    let s2 = session_coord.get_session(&harness.display2_id).unwrap();

    assert_eq!(s1.wallpaper_id, harness.wallpaper1_id);
    assert_eq!(s2.wallpaper_id, harness.wallpaper1_id);
    assert_eq!(s1.state, DisplaySessionState::Active);
    assert_eq!(s2.state, DisplaySessionState::Active);
}

#[test]
fn test_mode_transition_clone_to_independent_restores_saved_assignments() {
    let harness = TestLayoutHarness::new();
    let mut coordinator = harness.create_coordinator();

    // User assigns distinct wallpapers to each display
    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper1_id)
        .unwrap();
    harness
        .assignment_service
        .assign_wallpaper(&harness.display2_id, harness.wallpaper2_id)
        .unwrap();

    let snapshot = harness.display_manager.current_snapshot();

    // 1. Enter Clone mode (both will play wallpaper 1)
    coordinator
        .set_saved_mode(MultiMonitorLayoutMode::Clone)
        .unwrap();
    coordinator
        .apply_layout(&snapshot, &harness.catalog, &harness.host_candidate)
        .unwrap();

    // 2. Return to Independent mode
    coordinator
        .set_saved_mode(MultiMonitorLayoutMode::Independent)
        .unwrap();
    let report = coordinator
        .apply_layout(&snapshot, &harness.catalog, &harness.host_candidate)
        .unwrap();

    assert_eq!(report.active_mode, MultiMonitorLayoutMode::Independent);

    // Verify per-display saved assignments were preserved and restored!
    let session_coord = harness.session_coordinator.lock().unwrap();
    let s1 = session_coord.get_session(&harness.display1_id).unwrap();
    let s2 = session_coord.get_session(&harness.display2_id).unwrap();

    assert_eq!(s1.wallpaper_id, harness.wallpaper1_id);
    assert_eq!(s2.wallpaper_id, harness.wallpaper2_id);
}

#[test]
fn test_layout_mode_persistence_atomic_roundtrip() {
    let harness = TestLayoutHarness::new();
    let mut coordinator = harness.create_coordinator();

    assert_eq!(
        coordinator.saved_mode(),
        MultiMonitorLayoutMode::Independent
    );

    coordinator
        .set_saved_mode(MultiMonitorLayoutMode::Span)
        .unwrap();
    assert_eq!(coordinator.saved_mode(), MultiMonitorLayoutMode::Span);
    assert_eq!(
        harness.assignment_service.get_layout_mode(),
        MultiMonitorLayoutMode::Span
    );

    coordinator
        .set_saved_mode(MultiMonitorLayoutMode::Clone)
        .unwrap();
    assert_eq!(coordinator.saved_mode(), MultiMonitorLayoutMode::Clone);
    assert_eq!(
        harness.assignment_service.get_layout_mode(),
        MultiMonitorLayoutMode::Clone
    );
}

#[test]
fn test_layout_coordinator_diagnostics_and_shutdown() {
    let harness = TestLayoutHarness::new();
    let mut coordinator = harness.create_coordinator();

    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper1_id)
        .unwrap();
    let snapshot = harness.display_manager.current_snapshot();

    coordinator
        .apply_layout(&snapshot, &harness.catalog, &harness.host_candidate)
        .unwrap();

    let diag = coordinator.get_diagnostics();
    assert!(!diag.is_empty());

    coordinator.shutdown().unwrap();
    assert!(matches!(
        coordinator.apply_layout(&snapshot, &harness.catalog, &harness.host_candidate),
        Err(LayoutError::CoordinatorShutDown)
    ));
}
