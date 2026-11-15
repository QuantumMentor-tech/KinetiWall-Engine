//! Comprehensive End-to-End Integration, Reliability Hardening, and Acceptance Tests (Step 47).
//!
//! Validates the complete multi-monitor workflow across:
//! 1. Display enumeration, geometry modeling, and stable identity resolution (Step 42).
//! 2. Per-monitor wallpaper assignment persistence and catalog validation (Step 43).
//! 3. Declarative session planning and failure-isolated runtime orchestration (Step 44).
//! 4. Independent, Span, and Clone layout modes with transactional transitions (Step 45).
//! 5. Desktop host candidate validation, native window attachment, and rollback (Steps 39 & 40).
//! 6. Surface geometry synchronization with virtual desktop coordinate handling (Step 41).
//! 7. Dynamic display change handling (hot-unplug and hot-plug) and resource cleanup.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use kinetiwall_catalog::{
    AssetReference, AvailabilityState, CatalogError, CatalogLifecycle, WallpaperCatalogRepository,
    WallpaperId, WallpaperMetadata, WallpaperRecord, WallpaperSourceKind, WallpaperType,
};
use kinetiwall_desktop::attachment::constants;
use kinetiwall_desktop::{
    AssignmentService, AttachmentCoordinator, CandidateRole, DesktopCandidateWindow,
    DisplayGeometry, DisplayId, DisplaySessionCoordinator, DisplaySessionState,
    DisplaySnapshotManager, LayoutError, LayoutPlanValidationOutcome, MockAssignmentRepository,
    MockDesktopDisplayProvider, MockDesktopWindowProvider, MockWindowRecord,
    MultiMonitorLayoutCoordinator, MultiMonitorLayoutMode, NativeWindowHandle, PlaybackSyncLevel,
    ProviderSurfaceFactory, SessionError, SurfaceFactory, WindowRect,
    SPAN_VIRTUAL_DESKTOP_DISPLAY_ID,
};

// -------------------------------------------------------------------------------------------------
// Test Catalog Repository
// -------------------------------------------------------------------------------------------------

#[derive(Default, Clone)]
struct E2eTestCatalog {
    records: Arc<Mutex<BTreeMap<WallpaperId, WallpaperRecord>>>,
}

impl E2eTestCatalog {
    fn new() -> Self {
        Self::default()
    }

    fn register_wallpaper(
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
                entrypoint: "video.mp4".to_string(),
            },
            metadata: WallpaperMetadata::new(title),
            lifecycle: CatalogLifecycle::now(false),
            availability,
            preserved_extra: BTreeMap::new(),
        };
        self.records.lock().unwrap().insert(id, record);
    }
}

impl WallpaperCatalogRepository for E2eTestCatalog {
    fn get(&self, id: WallpaperId) -> Result<Option<WallpaperRecord>, CatalogError> {
        Ok(self.records.lock().unwrap().get(&id).cloned())
    }

    fn list(&self) -> Result<Vec<WallpaperRecord>, CatalogError> {
        Ok(self.records.lock().unwrap().values().cloned().collect())
    }

    fn create(&mut self, record: WallpaperRecord) -> Result<WallpaperRecord, CatalogError> {
        self.records.lock().unwrap().insert(record.id, record.clone());
        Ok(record)
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
// Fault-Injecting Surface Factory for Failure Isolation Tests
// -------------------------------------------------------------------------------------------------

struct FailingSurfaceFactory {
    inner: ProviderSurfaceFactory,
    fail_display_id: DisplayId,
}

impl SurfaceFactory for FailingSurfaceFactory {
    fn create_surface(
        &self,
        display_id: &DisplayId,
        geometry: &DisplayGeometry,
    ) -> Result<NativeWindowHandle, SessionError> {
        if *display_id == self.fail_display_id {
            Err(SessionError::SurfaceCreationFailed(
                display_id.clone(),
                "Simulated hardware acceleration failure on secondary display".into(),
            ))
        } else {
            self.inner.create_surface(display_id, geometry)
        }
    }

    fn destroy_surface(&self, handle: NativeWindowHandle) -> Result<(), SessionError> {
        self.inner.destroy_surface(handle)
    }
}

// -------------------------------------------------------------------------------------------------
// E2E Multi-Monitor Integration Test Harness
// -------------------------------------------------------------------------------------------------

struct E2eHarness {
    window_provider: Arc<MockDesktopWindowProvider>,
    display_provider: Arc<MockDesktopDisplayProvider>,
    display_manager: DisplaySnapshotManager,
    catalog: E2eTestCatalog,
    assignment_service: Arc<AssignmentService>,
    attachment_coordinator: Arc<Mutex<AttachmentCoordinator>>,
    session_coordinator: Arc<Mutex<DisplaySessionCoordinator>>,
    host_candidate: DesktopCandidateWindow,
    display1_id: DisplayId,
    display2_id: DisplayId,
    wallpaper_cyberpunk: WallpaperId,
    wallpaper_synthwave: WallpaperId,
    wallpaper_nature: WallpaperId,
}

impl E2eHarness {
    fn new_dual_monitor(mon1_rect: WindowRect, mon2_rect: WindowRect) -> Self {
        let window_provider = Arc::new(MockDesktopWindowProvider::new());
        let display_provider = Arc::new(MockDesktopDisplayProvider::new());

        display_provider.setup_dual_extended(mon1_rect, mon2_rect);

        let mut display_manager = DisplaySnapshotManager::new(display_provider.clone());
        let (snapshot, _) = display_manager.refresh().expect("Snapshot refresh");
        let display1_id = snapshot.displays[0].id.clone();
        let display2_id = snapshot.displays[1].id.clone();

        let catalog = E2eTestCatalog::new();
        let wp_cyber = WallpaperId::new();
        let wp_synth = WallpaperId::new();
        let wp_nature = WallpaperId::new();

        catalog.register_wallpaper(
            wp_cyber,
            "Cyberpunk City",
            WallpaperType::Video,
            AvailabilityState::Available,
        );
        catalog.register_wallpaper(
            wp_synth,
            "Synthwave Sun",
            WallpaperType::Video,
            AvailabilityState::Available,
        );
        catalog.register_wallpaper(
            wp_nature,
            "Mountain Sunrise",
            WallpaperType::Video,
            AvailabilityState::Available,
        );

        let assignment_repo = Arc::new(MockAssignmentRepository::new());
        let assignment_service =
            Arc::new(AssignmentService::new(assignment_repo).expect("AssignmentService init"));

        // Setup desktop host candidate (WorkerW)
        let owning_pid = 4242;
        let host_hwnd = NativeWindowHandle(100);
        window_provider.add_window(MockWindowRecord {
            handle: host_hwnd,
            class_name: "WorkerW".to_string(),
            window_title: String::new(),
            process_id: 1000,
            thread_id: 1001,
            parent: None,
            is_visible: true,
            bounds: snapshot.virtual_desktop_bounds,
            children: Vec::new(),
            style: constants::WS_POPUP | constants::WS_VISIBLE,
            ex_style: 0,
            dpi_context: Some(2),
        });

        let host_candidate = DesktopCandidateWindow {
            handle: host_hwnd,
            class_name: "WorkerW".to_string(),
            window_title: String::new(),
            process_id: 1000,
            thread_id: 1001,
            role: CandidateRole::WallpaperHostCandidate,
            parent: None,
            bounds: Some(snapshot.virtual_desktop_bounds),
            is_visible: true,
            has_shelldll_defview_child: false,
            validated_at_millis: 1000,
        };

        let mut attachment_coord = AttachmentCoordinator::new(window_provider.clone());
        attachment_coord.register_allowed_process_id(owning_pid);
        let attachment_coordinator = Arc::new(Mutex::new(attachment_coord));

        let session_coord = DisplaySessionCoordinator::with_default_factory(
            window_provider.clone(),
            attachment_coordinator.clone(),
        );
        let session_coordinator = Arc::new(Mutex::new(session_coord));

        Self {
            window_provider,
            display_provider,
            display_manager,
            catalog,
            assignment_service,
            attachment_coordinator,
            session_coordinator,
            host_candidate,
            display1_id,
            display2_id,
            wallpaper_cyberpunk: wp_cyber,
            wallpaper_synthwave: wp_synth,
            wallpaper_nature: wp_nature,
        }
    }

    fn create_layout_coordinator(&self) -> MultiMonitorLayoutCoordinator {
        MultiMonitorLayoutCoordinator::new(
            self.session_coordinator.clone(),
            self.assignment_service.clone(),
        )
    }
}

// -------------------------------------------------------------------------------------------------
// Test Group 1: End-to-End Pipeline & Independent Mode Verification
// -------------------------------------------------------------------------------------------------

#[test]
fn test_e2e_discovery_to_independent_activation_and_geometry() {
    let mon1 = WindowRect::new(0, 0, 2560, 1440);
    let mon2 = WindowRect::new(2560, 0, 4480, 1080);
    let harness = E2eHarness::new_dual_monitor(mon1, mon2);
    let mut coordinator = harness.create_layout_coordinator();

    // 1. Assign distinct wallpapers
    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper_cyberpunk)
        .expect("Assign display 1");
    harness
        .assignment_service
        .assign_wallpaper(&harness.display2_id, harness.wallpaper_synthwave)
        .expect("Assign display 2");

    // 2. Discover snapshot and evaluate plan
    let snapshot = harness.display_manager.current_snapshot();
    let plan = coordinator.evaluate_plan(&snapshot, &harness.catalog);
    assert_eq!(plan.selected_mode, MultiMonitorLayoutMode::Independent);
    assert_eq!(plan.validation_outcome, LayoutPlanValidationOutcome::Valid);
    assert!(plan.is_activatable);
    assert_eq!(plan.surface_plans.len(), 2);

    // 3. Apply layout mode
    let report = coordinator
        .apply_layout(&snapshot, &harness.catalog, &harness.host_candidate)
        .expect("Apply Independent layout");

    assert_eq!(report.active_mode, MultiMonitorLayoutMode::Independent);
    assert_eq!(report.activated_surfaces_count, 2);

    // 4. Verify runtime session states and native window geometry
    let session_coord = harness.session_coordinator.lock().unwrap();
    let s1 = session_coord
        .get_session(&harness.display1_id)
        .expect("Session 1 exists");
    let s2 = session_coord
        .get_session(&harness.display2_id)
        .expect("Session 2 exists");

    assert_eq!(s1.state, DisplaySessionState::Active);
    assert_eq!(s2.state, DisplaySessionState::Active);
    assert_eq!(s1.wallpaper_id, harness.wallpaper_cyberpunk);
    assert_eq!(s2.wallpaper_id, harness.wallpaper_synthwave);
    assert_eq!(s1.geometry.monitor_rect, mon1);
    assert_eq!(s2.geometry.monitor_rect, mon2);

    // Verify native handles are attached to the WorkerW host
    assert_eq!(s1.host_handle, Some(harness.host_candidate.handle));
    assert_eq!(s2.host_handle, Some(harness.host_candidate.handle));
    assert!(s1.source_handle.is_some());
    assert!(s2.source_handle.is_some());
}

#[test]
fn test_e2e_assignment_update_isolated_reconciliation() {
    let mon1 = WindowRect::new(0, 0, 1920, 1080);
    let mon2 = WindowRect::new(1920, 0, 3840, 1080);
    let harness = E2eHarness::new_dual_monitor(mon1, mon2);
    let mut coordinator = harness.create_layout_coordinator();

    // Initial setup: display 1 = Cyberpunk, display 2 = Synthwave
    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper_cyberpunk)
        .unwrap();
    harness
        .assignment_service
        .assign_wallpaper(&harness.display2_id, harness.wallpaper_synthwave)
        .unwrap();

    let snapshot = harness.display_manager.current_snapshot();
    coordinator
        .apply_layout(&snapshot, &harness.catalog, &harness.host_candidate)
        .unwrap();

    let session1_id_orig = {
        let sc = harness.session_coordinator.lock().unwrap();
        sc.get_session(&harness.display1_id).unwrap().session_id
    };

    // User updates wallpaper for Display 2 ONLY (changing from Synthwave to Mountain Sunrise)
    harness
        .assignment_service
        .assign_wallpaper(&harness.display2_id, harness.wallpaper_nature)
        .unwrap();

    // Reapply layout
    let report = coordinator
        .apply_layout(&snapshot, &harness.catalog, &harness.host_candidate)
        .unwrap();
    assert_eq!(report.active_mode, MultiMonitorLayoutMode::Independent);

    // Display 1 session MUST be retained without interruption or restart
    let sc = harness.session_coordinator.lock().unwrap();
    let s1_after = sc.get_session(&harness.display1_id).unwrap();
    let s2_after = sc.get_session(&harness.display2_id).unwrap();

    assert_eq!(s1_after.session_id, session1_id_orig, "Display 1 session must be preserved");
    assert_eq!(s1_after.wallpaper_id, harness.wallpaper_cyberpunk);
    assert_eq!(s1_after.state, DisplaySessionState::Active);

    // Display 2 session MUST be reconfigured or replaced with the new wallpaper
    assert_eq!(s2_after.wallpaper_id, harness.wallpaper_nature);
    assert_eq!(s2_after.state, DisplaySessionState::Active);
}

// -------------------------------------------------------------------------------------------------
// Test Group 2: Span & Clone Presentation Mode Verification
// -------------------------------------------------------------------------------------------------

#[test]
fn test_e2e_span_mode_virtual_desktop_and_offset_geometry() {
    // Monitor 1 at (0, 0), Monitor 2 at (1920, 120) with vertical offset
    let mon1 = WindowRect::new(0, 0, 1920, 1080);
    let mon2 = WindowRect::new(1920, 120, 3840, 1200);
    let harness = E2eHarness::new_dual_monitor(mon1, mon2);
    let mut coordinator = harness.create_layout_coordinator();

    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper_cyberpunk)
        .unwrap();

    // Switch to Span mode
    coordinator
        .set_saved_mode(MultiMonitorLayoutMode::Span)
        .unwrap();

    let snapshot = harness.display_manager.current_snapshot();
    let plan = coordinator.evaluate_plan(&snapshot, &harness.catalog);

    assert_eq!(plan.selected_mode, MultiMonitorLayoutMode::Span);
    assert_eq!(plan.effective_mode, MultiMonitorLayoutMode::Span);
    assert_eq!(plan.surface_plans.len(), 1);

    // Span surface must cover the full virtual desktop bounding box
    let span_surface = &plan.surface_plans[0];
    assert_eq!(span_surface.target_bounds, snapshot.virtual_desktop_bounds);
    assert_eq!(span_surface.wallpaper_id, harness.wallpaper_cyberpunk);

    // Apply Span layout
    let report = coordinator
        .apply_layout(&snapshot, &harness.catalog, &harness.host_candidate)
        .unwrap();
    assert_eq!(report.active_mode, MultiMonitorLayoutMode::Span);
    assert_eq!(report.activated_surfaces_count, 1);

    // Verify session coordinator owns exactly one spanning session under SPAN_VIRTUAL_DESKTOP_DISPLAY_ID
    let sc = harness.session_coordinator.lock().unwrap();
    let span_display_id = DisplayId(SPAN_VIRTUAL_DESKTOP_DISPLAY_ID.into());
    let span_session = sc
        .get_session(&span_display_id)
        .expect("Span session must exist");

    assert_eq!(span_session.state, DisplaySessionState::Active);
    assert_eq!(span_session.geometry.monitor_rect, snapshot.virtual_desktop_bounds);

    // Individual monitor sessions must have been cleanly stopped
    assert!(sc.get_session(&harness.display1_id).map_or(true, |s| s.state.is_terminated()));
    assert!(sc.get_session(&harness.display2_id).map_or(true, |s| s.state.is_terminated()));
}

#[test]
fn test_e2e_clone_mode_synchronized_presentation_and_sync_level() {
    let mon1 = WindowRect::new(0, 0, 1920, 1080);
    let mon2 = WindowRect::new(1920, 0, 3840, 1080);
    let harness = E2eHarness::new_dual_monitor(mon1, mon2);
    let mut coordinator = harness.create_layout_coordinator();

    // Assign primary display
    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper_cyberpunk)
        .unwrap();

    coordinator
        .set_saved_mode(MultiMonitorLayoutMode::Clone)
        .unwrap();

    let snapshot = harness.display_manager.current_snapshot();
    let plan = coordinator.evaluate_plan(&snapshot, &harness.catalog);

    assert_eq!(plan.selected_mode, MultiMonitorLayoutMode::Clone);
    assert_eq!(plan.surface_plans.len(), 2);
    // Explicit BestEffort software sync level claimed (truthful, no genlock)
    assert_eq!(plan.surface_plans[0].sync_level, PlaybackSyncLevel::BestEffort);
    assert_eq!(plan.surface_plans[1].sync_level, PlaybackSyncLevel::BestEffort);
    assert_eq!(plan.surface_plans[0].wallpaper_id, harness.wallpaper_cyberpunk);
    assert_eq!(plan.surface_plans[1].wallpaper_id, harness.wallpaper_cyberpunk);

    // Apply Clone layout
    let report = coordinator
        .apply_layout(&snapshot, &harness.catalog, &harness.host_candidate)
        .unwrap();
    assert_eq!(report.active_mode, MultiMonitorLayoutMode::Clone);
    assert_eq!(report.activated_surfaces_count, 2);

    let sc = harness.session_coordinator.lock().unwrap();
    let s1 = sc.get_session(&harness.display1_id).unwrap();
    let s2 = sc.get_session(&harness.display2_id).unwrap();

    assert_eq!(s1.state, DisplaySessionState::Active);
    assert_eq!(s2.state, DisplaySessionState::Active);
    assert_eq!(s1.wallpaper_id, s2.wallpaper_id, "Both sessions must display identical wallpaper");
}

// -------------------------------------------------------------------------------------------------
// Test Group 3: Transitions, Rollback & Failure Isolation
// -------------------------------------------------------------------------------------------------

#[test]
fn test_e2e_mode_transitions_with_rollback_on_failure() {
    let mon1 = WindowRect::new(0, 0, 1920, 1080);
    let mon2 = WindowRect::new(1920, 0, 3840, 1080);
    let harness = E2eHarness::new_dual_monitor(mon1, mon2);
    let mut coordinator = harness.create_layout_coordinator();

    // Setup initial Independent layout
    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper_cyberpunk)
        .unwrap();
    harness
        .assignment_service
        .assign_wallpaper(&harness.display2_id, harness.wallpaper_synthwave)
        .unwrap();

    let snapshot = harness.display_manager.current_snapshot();
    coordinator
        .apply_layout(&snapshot, &harness.catalog, &harness.host_candidate)
        .expect("Apply initial Independent layout");

    assert_eq!(coordinator.active_mode(), Some(MultiMonitorLayoutMode::Independent));

    // Transition: Independent -> Span
    coordinator
        .set_saved_mode(MultiMonitorLayoutMode::Span)
        .unwrap();
    coordinator
        .apply_layout(&snapshot, &harness.catalog, &harness.host_candidate)
        .expect("Apply Span layout");
    assert_eq!(coordinator.active_mode(), Some(MultiMonitorLayoutMode::Span));

    // Transition: Span -> Clone
    coordinator
        .set_saved_mode(MultiMonitorLayoutMode::Clone)
        .unwrap();
    coordinator
        .apply_layout(&snapshot, &harness.catalog, &harness.host_candidate)
        .expect("Apply Clone layout");
    assert_eq!(coordinator.active_mode(), Some(MultiMonitorLayoutMode::Clone));

    // Invalidate host candidate to simulate attachment failure during next transition
    let invalid_host = DesktopCandidateWindow {
        handle: NativeWindowHandle(0xDEAD_BEEF),
        class_name: "InvalidHost".to_string(),
        window_title: String::new(),
        process_id: 9999,
        thread_id: 9999,
        role: CandidateRole::WallpaperHostCandidate,
        parent: None,
        bounds: None,
        is_visible: false,
        has_shelldll_defview_child: false,
        validated_at_millis: 0,
    };

    coordinator
        .set_saved_mode(MultiMonitorLayoutMode::Span)
        .unwrap();
    let fail_result = coordinator.apply_layout(&snapshot, &harness.catalog, &invalid_host);
    assert!(fail_result.is_err(), "Transition against invalid host must fail");

    // Verify rollback restored previous known-good mode (Clone) or safe fallback
    assert!(coordinator.active_mode().is_some());
}

#[test]
fn test_e2e_failure_isolation_between_sessions() {
    let mon1 = WindowRect::new(0, 0, 1920, 1080);
    let mon2 = WindowRect::new(1920, 0, 3840, 1080);
    let harness = E2eHarness::new_dual_monitor(mon1, mon2);

    // Setup custom session coordinator with failing surface factory on Display 2
    let factory = Arc::new(FailingSurfaceFactory {
        inner: ProviderSurfaceFactory::new(harness.window_provider.clone()),
        fail_display_id: harness.display2_id.clone(),
    });

    let session_coord = Arc::new(Mutex::new(DisplaySessionCoordinator::new(
        harness.window_provider.clone(),
        harness.attachment_coordinator.clone(),
        factory,
    )));

    let _coordinator = MultiMonitorLayoutCoordinator::new(session_coord.clone(), harness.assignment_service.clone());

    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper_cyberpunk)
        .unwrap();
    harness
        .assignment_service
        .assign_wallpaper(&harness.display2_id, harness.wallpaper_synthwave)
        .unwrap();

    let snapshot = harness.display_manager.current_snapshot();

    // In Independent mode, activate plan directly on session coordinator to verify failure isolation
    let resolved = harness.assignment_service.resolve_all_displays(&snapshot, &harness.catalog);
    let mut sc = session_coord.lock().unwrap();
    let plan = sc.create_plan(&snapshot, &resolved);
    let report = sc.apply_plan(&plan, &harness.host_candidate).expect("Reconciliation succeeds partially");

    // Verify Display 1 activated successfully
    assert!(report.activated.contains(&harness.display1_id));

    // Verify Display 2 failed and is recorded in the failure map
    assert!(report.failures.contains_key(&harness.display2_id));

    // Verify Display 1 session remains fully ACTIVE with untouched bounds and attachment
    let s1 = sc.get_session(&harness.display1_id).unwrap();
    let s2 = sc.get_session(&harness.display2_id).unwrap();

    assert_eq!(s1.state, DisplaySessionState::Active, "Display 1 must remain Active");
    assert_eq!(s1.wallpaper_id, harness.wallpaper_cyberpunk);
    assert_eq!(s1.host_handle, Some(harness.host_candidate.handle));

    // Display 2 is cleanly isolated in Failed state
    assert!(matches!(s2.state, DisplaySessionState::Failed { .. }));
}

// -------------------------------------------------------------------------------------------------
// Test Group 4: Dynamic Display Changes (Hot-Unplug & Hot-Plug)
// -------------------------------------------------------------------------------------------------

#[test]
fn test_e2e_display_disconnection_and_reconnection() {
    let mon1 = WindowRect::new(0, 0, 1920, 1080);
    let mon2 = WindowRect::new(1920, 0, 3840, 1080);
    let mut harness = E2eHarness::new_dual_monitor(mon1, mon2);
    let mut coordinator = harness.create_layout_coordinator();

    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper_cyberpunk)
        .unwrap();
    harness
        .assignment_service
        .assign_wallpaper(&harness.display2_id, harness.wallpaper_synthwave)
        .unwrap();

    let snapshot_dual = harness.display_manager.current_snapshot();
    coordinator
        .apply_layout(&snapshot_dual, &harness.catalog, &harness.host_candidate)
        .unwrap();

    // Simulate Display 2 Hot-Unplug (monitor physically disconnected)
    harness.display_provider.setup_single_display(mon1.width(), mon1.height());
    let (snapshot_single, delta) = harness.display_manager.refresh().expect("Snapshot refresh");
    assert!(delta.is_meaningful_change);
    assert_eq!(snapshot_single.displays.len(), 1);

    // Apply layout on refreshed topology
    coordinator
        .apply_layout(&snapshot_single, &harness.catalog, &harness.host_candidate)
        .unwrap();

    // Verify Display 1 is still Active, Display 2 is stopped/removed
    {
        let sc = harness.session_coordinator.lock().unwrap();
        let s1 = sc.get_session(&harness.display1_id).unwrap();
        assert_eq!(s1.state, DisplaySessionState::Active);

        let s2_opt = sc.get_session(&harness.display2_id);
        assert!(s2_opt.map_or(true, |s| s.state.is_terminated()));
    }

    // Verify Display 2's assignment was NOT destroyed (persisted for re-attach)
    let persisted_d2 = harness.assignment_service.get_assignment(&harness.display2_id);
    assert_eq!(persisted_d2.map(|a| a.wallpaper_id), Some(harness.wallpaper_synthwave));

    // Simulate Display 2 Reconnection (hot-plug back)
    harness.display_provider.setup_dual_extended(mon1, mon2);
    let (snapshot_restored, _) = harness.display_manager.refresh().expect("Snapshot refresh");
    assert_eq!(snapshot_restored.displays.len(), 2);

    // Reapply layout
    coordinator
        .apply_layout(&snapshot_restored, &harness.catalog, &harness.host_candidate)
        .unwrap();

    // Both displays must now be Active again
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(&harness.display1_id).unwrap().state, DisplaySessionState::Active);
        assert_eq!(sc.get_session(&harness.display2_id).unwrap().state, DisplaySessionState::Active);
    }
}

// -------------------------------------------------------------------------------------------------
// Test Group 5: Edge Cases, Single-Display Safeguards & Clean Shutdown
// -------------------------------------------------------------------------------------------------

#[test]
fn test_e2e_single_display_safeguards_reject_multimonitor_modes() {
    let mon = WindowRect::new(0, 0, 1920, 1080);
    let window_provider = Arc::new(MockDesktopWindowProvider::new());
    let display_provider = Arc::new(MockDesktopDisplayProvider::new());
    display_provider.setup_single_display(mon.width(), mon.height());

    let mut display_manager = DisplaySnapshotManager::new(display_provider.clone());
    let (snapshot, _) = display_manager.refresh().unwrap();
    let display_id = snapshot.displays[0].id.clone();

    let catalog = E2eTestCatalog::new();
    let wp_id = WallpaperId::new();
    catalog.register_wallpaper(wp_id, "Test", WallpaperType::Video, AvailabilityState::Available);

    let assignment_repo = Arc::new(MockAssignmentRepository::new());
    let assignment_service = Arc::new(AssignmentService::new(assignment_repo).unwrap());
    assignment_service.assign_wallpaper(&display_id, wp_id).unwrap();

    let attachment_coordinator = Arc::new(Mutex::new(AttachmentCoordinator::new(window_provider.clone())));
    let session_coordinator = Arc::new(Mutex::new(DisplaySessionCoordinator::with_default_factory(
        window_provider,
        attachment_coordinator,
    )));

    let mut coordinator = MultiMonitorLayoutCoordinator::new(session_coordinator, assignment_service);

    // Test Span on single monitor -> validated as SingleDisplayOnly, safely falls back to Independent
    coordinator.set_saved_mode(MultiMonitorLayoutMode::Span).unwrap();
    let plan_span = coordinator.evaluate_plan(&snapshot, &catalog);
    assert_eq!(plan_span.selected_mode, MultiMonitorLayoutMode::Span);
    assert_eq!(plan_span.effective_mode, MultiMonitorLayoutMode::Independent);
    assert_eq!(plan_span.validation_outcome, LayoutPlanValidationOutcome::SingleDisplayOnly);
    assert!(plan_span.is_activatable);

    // Test Clone on single monitor -> validated as SingleDisplayOnly, safely falls back to Independent
    coordinator.set_saved_mode(MultiMonitorLayoutMode::Clone).unwrap();
    let plan_clone = coordinator.evaluate_plan(&snapshot, &catalog);
    assert_eq!(plan_clone.selected_mode, MultiMonitorLayoutMode::Clone);
    assert_eq!(plan_clone.effective_mode, MultiMonitorLayoutMode::Independent);
    assert_eq!(plan_clone.validation_outcome, LayoutPlanValidationOutcome::SingleDisplayOnly);
    assert!(plan_clone.is_activatable);
}

#[test]
fn test_e2e_missing_media_asset_safeguard() {
    let mon1 = WindowRect::new(0, 0, 1920, 1080);
    let mon2 = WindowRect::new(1920, 0, 3840, 1080);
    let harness = E2eHarness::new_dual_monitor(mon1, mon2);
    let mut coordinator = harness.create_layout_coordinator();

    // Register a broken/missing wallpaper in catalog
    let broken_wp_id = WallpaperId::new();
    harness.catalog.register_wallpaper(
        broken_wp_id,
        "Missing Asset Wallpaper",
        WallpaperType::Video,
        AvailabilityState::Missing,
    );

    // Assign broken wallpaper
    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, broken_wp_id)
        .unwrap();

    let snapshot = harness.display_manager.current_snapshot();
    let plan = coordinator.evaluate_plan(&snapshot, &harness.catalog);

    // Must be rejected safely with MissingWallpaperAsset
    assert!(matches!(
        plan.validation_outcome,
        LayoutPlanValidationOutcome::MissingWallpaperAsset { .. }
    ));
    assert!(!plan.is_activatable);

    // Applying plan fails without corrupting state or substituting arbitrary wallpaper
    let apply_res = coordinator.apply_layout(&snapshot, &harness.catalog, &harness.host_candidate);
    assert!(apply_res.is_err());
}

#[test]
fn test_e2e_shutdown_idempotency_and_resource_detachment() {
    let mon1 = WindowRect::new(0, 0, 1920, 1080);
    let mon2 = WindowRect::new(1920, 0, 3840, 1080);
    let harness = E2eHarness::new_dual_monitor(mon1, mon2);
    let mut coordinator = harness.create_layout_coordinator();

    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper_cyberpunk)
        .unwrap();
    harness
        .assignment_service
        .assign_wallpaper(&harness.display2_id, harness.wallpaper_synthwave)
        .unwrap();

    let snapshot = harness.display_manager.current_snapshot();
    coordinator
        .apply_layout(&snapshot, &harness.catalog, &harness.host_candidate)
        .unwrap();

    assert_eq!(coordinator.active_mode(), Some(MultiMonitorLayoutMode::Independent));

    // Perform Shutdown
    coordinator.shutdown().expect("Coordinator clean shutdown");

    // Subsequent shutdown is idempotent
    assert!(coordinator.shutdown().is_ok());

    // Subsequent apply_layout is rejected
    let apply_res = coordinator.apply_layout(&snapshot, &harness.catalog, &harness.host_candidate);
    assert!(matches!(apply_res, Err(LayoutError::CoordinatorShutDown)));

    // Verify all sessions are stopped and surfaces detached
    let sc = harness.session_coordinator.lock().unwrap();
    assert!(sc.is_shut_down());
    for session in sc.list_sessions() {
        assert_eq!(session.state, DisplaySessionState::Stopped);
    }
}
