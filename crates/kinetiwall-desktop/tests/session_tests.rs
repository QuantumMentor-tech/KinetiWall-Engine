//! Comprehensive unit tests for per-monitor playback sessions, assignment activation,
//! delta reconciliation, failure isolation, and lifecycle management (Step 44).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use kinetiwall_catalog::{
    AssetReference, AvailabilityState, CatalogError, CatalogLifecycle, WallpaperCatalogRepository,
    WallpaperId, WallpaperMetadata, WallpaperRecord, WallpaperSourceKind, WallpaperType,
};
use kinetiwall_desktop::attachment::constants;
use kinetiwall_desktop::{
    AssignmentResolutionOutcome, AssignmentService, AttachmentCoordinator, CandidateRole,
    DesktopCandidateWindow, DisplayGeometry, DisplayId, DisplaySessionCoordinator,
    DisplaySessionState, DisplaySnapshotManager, MockAssignmentRepository,
    MockDesktopDisplayProvider, MockDesktopWindowProvider, MockWindowRecord, NativeWindowHandle,
    ProviderSurfaceFactory, SessionError, SessionPlan, SessionPlanAction, SessionStopReason,
    SurfaceFactory, WindowRect, MAX_SESSION_DIAGNOSTIC_EVENTS,
};

// -------------------------------------------------------------------------------------------------
// Test In-Memory Catalog Repository
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

struct TestSessionHarness {
    window_provider: Arc<MockDesktopWindowProvider>,
    display_provider: Arc<MockDesktopDisplayProvider>,
    display_manager: DisplaySnapshotManager,
    catalog: TestCatalog,
    assignment_service: Arc<AssignmentService>,
    attachment_coordinator: Arc<Mutex<AttachmentCoordinator>>,
    host_candidate: DesktopCandidateWindow,
    display1_id: DisplayId,
    display2_id: DisplayId,
    wallpaper1_id: WallpaperId,
    wallpaper2_id: WallpaperId,
}

impl TestSessionHarness {
    fn new() -> Self {
        let window_provider = Arc::new(MockDesktopWindowProvider::new());
        let display_provider = Arc::new(MockDesktopDisplayProvider::new());

        // Setup dual displays: Primary (0, 0, 1920x1080), Secondary (1920, 0, 1920x1080)
        let mon1 = WindowRect::new(0, 0, 1920, 1080);
        let mon2 = WindowRect::new(1920, 0, 3840, 1080);
        display_provider.setup_dual_extended(mon1, mon2);

        let mut display_manager = DisplaySnapshotManager::new(display_provider.clone());
        let (snapshot, _) = display_manager
            .refresh()
            .expect("Snapshot refresh must succeed");
        let display1_id = snapshot.displays[0].id.clone();
        let display2_id = snapshot.displays[1].id.clone();

        // Setup desktop host hierarchy: Progman -> SHELLDLL_DefView, WorkerW
        let owning_pid = 1234;
        let progman = NativeWindowHandle(10);
        let shelldll = NativeWindowHandle(20);
        let workerw = NativeWindowHandle(30);

        window_provider.add_window(MockWindowRecord {
            handle: progman,
            class_name: "Progman".to_string(),
            window_title: "Program Manager".to_string(),
            process_id: 1000,
            thread_id: 1001,
            parent: None,
            is_visible: true,
            bounds: WindowRect::new(0, 0, 3840, 1080),
            children: vec![shelldll],
            style: constants::WS_POPUP | constants::WS_VISIBLE,
            ex_style: 0,
            dpi_context: Some(2),
        });

        window_provider.add_window(MockWindowRecord {
            handle: shelldll,
            class_name: "SHELLDLL_DefView".to_string(),
            window_title: String::new(),
            process_id: 1000,
            thread_id: 1002,
            parent: Some(progman),
            is_visible: true,
            bounds: WindowRect::new(0, 0, 3840, 1080),
            children: Vec::new(),
            style: constants::WS_CHILD | constants::WS_VISIBLE,
            ex_style: 0,
            dpi_context: Some(2),
        });

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

        // Setup catalog with 2 available wallpapers
        let catalog = TestCatalog::new();
        let wallpaper1_id = WallpaperId::new();
        let wallpaper2_id = WallpaperId::new();

        catalog.add_wallpaper(
            wallpaper1_id,
            "Cyberpunk City Skyline",
            WallpaperType::Video,
            AvailabilityState::Available,
        );

        catalog.add_wallpaper(
            wallpaper2_id,
            "Deep Space Nebula",
            WallpaperType::Web,
            AvailabilityState::Available,
        );

        let repo = Arc::new(MockAssignmentRepository::new());
        let assignment_service =
            Arc::new(AssignmentService::new(repo).expect("AssignmentService creation"));

        Self {
            window_provider,
            display_provider,
            display_manager,
            catalog,
            assignment_service,
            attachment_coordinator,
            host_candidate,
            display1_id,
            display2_id,
            wallpaper1_id,
            wallpaper2_id,
        }
    }

    fn create_coordinator(&self) -> DisplaySessionCoordinator {
        DisplaySessionCoordinator::with_default_factory(
            self.window_provider.clone(),
            self.attachment_coordinator.clone(),
        )
    }
}

// -------------------------------------------------------------------------------------------------
// Unit Tests
// -------------------------------------------------------------------------------------------------

#[test]
fn test_single_display_session_activation_and_state() {
    let harness = TestSessionHarness::new();
    let mut coordinator = harness.create_coordinator();

    // Assign wallpaper 1 to display 1
    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper1_id)
        .expect("Assignment must succeed");

    let snapshot = harness.display_manager.current_snapshot();
    let resolved = harness
        .assignment_service
        .resolve_all_displays(&snapshot, &harness.catalog);

    assert_eq!(resolved.len(), 2);
    let resolved_d1 = resolved
        .iter()
        .find(|r| r.display_id == harness.display1_id)
        .unwrap();
    assert!(resolved_d1.is_playable);

    // Create session plan
    let plan = coordinator.create_plan(&snapshot, &resolved);
    assert_eq!(plan.mutation_count(), 1);

    // Apply plan
    let report = coordinator
        .apply_plan(&plan, &harness.host_candidate)
        .expect("Apply plan must succeed");

    assert_eq!(report.activated.len(), 1);
    assert_eq!(report.activated[0], harness.display1_id);
    assert!(report.failures.is_empty());
    assert_eq!(coordinator.active_session_count(), 1);

    // Verify session descriptor
    let session = coordinator
        .get_session(&harness.display1_id)
        .expect("Session must exist");
    assert_eq!(session.state, DisplaySessionState::Active);
    assert_eq!(session.wallpaper_id, harness.wallpaper1_id);
    assert!(session.source_handle.is_some());
    assert_eq!(session.host_handle, Some(harness.host_candidate.handle));
    assert_eq!(session.generation, 1);
}

#[test]
fn test_dual_monitor_independent_sessions_distinct_wallpapers() {
    let harness = TestSessionHarness::new();
    let mut coordinator = harness.create_coordinator();

    // Assign distinct wallpapers
    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper1_id)
        .expect("Assign D1");
    harness
        .assignment_service
        .assign_wallpaper(&harness.display2_id, harness.wallpaper2_id)
        .expect("Assign D2");

    let snapshot = harness.display_manager.current_snapshot();
    let resolved = harness
        .assignment_service
        .resolve_all_displays(&snapshot, &harness.catalog);

    let plan = coordinator.create_plan(&snapshot, &resolved);
    assert_eq!(plan.actions.len(), 2);

    let report = coordinator
        .apply_plan(&plan, &harness.host_candidate)
        .expect("Apply plan");

    assert_eq!(report.activated.len(), 2);
    assert_eq!(coordinator.active_session_count(), 2);

    let s1 = coordinator.get_session(&harness.display1_id).unwrap();
    let s2 = coordinator.get_session(&harness.display2_id).unwrap();

    assert_ne!(s1.session_id, s2.session_id);
    assert_ne!(s1.source_handle, s2.source_handle);
    assert_eq!(s1.wallpaper_id, harness.wallpaper1_id);
    assert_eq!(s2.wallpaper_id, harness.wallpaper2_id);
    assert_eq!(s1.geometry.monitor_rect, WindowRect::new(0, 0, 1920, 1080));
    assert_eq!(
        s2.geometry.monitor_rect,
        WindowRect::new(1920, 0, 3840, 1080)
    );
}

#[test]
fn test_same_wallpaper_assigned_to_multiple_displays() {
    let harness = TestSessionHarness::new();
    let mut coordinator = harness.create_coordinator();

    // Both displays assigned wallpaper 1
    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper1_id)
        .expect("Assign D1");
    harness
        .assignment_service
        .assign_wallpaper(&harness.display2_id, harness.wallpaper1_id)
        .expect("Assign D2");

    let snapshot = harness.display_manager.current_snapshot();
    let resolved = harness
        .assignment_service
        .resolve_all_displays(&snapshot, &harness.catalog);

    let plan = coordinator.create_plan(&snapshot, &resolved);
    let report = coordinator
        .apply_plan(&plan, &harness.host_candidate)
        .expect("Apply plan");

    assert_eq!(report.activated.len(), 2);
    let s1 = coordinator.get_session(&harness.display1_id).unwrap();
    let s2 = coordinator.get_session(&harness.display2_id).unwrap();

    // Wallpapers match, but sessions and surfaces MUST be distinct!
    assert_eq!(s1.wallpaper_id, s2.wallpaper_id);
    assert_ne!(s1.session_id, s2.session_id);
    assert_ne!(s1.source_handle, s2.source_handle);
    assert_ne!(s1.geometry.monitor_rect, s2.geometry.monitor_rect);
}

#[test]
fn test_assignment_change_on_display_a_does_not_affect_display_b() {
    let harness = TestSessionHarness::new();
    let mut coordinator = harness.create_coordinator();

    // Initial dual setup
    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper1_id)
        .unwrap();
    harness
        .assignment_service
        .assign_wallpaper(&harness.display2_id, harness.wallpaper2_id)
        .unwrap();

    let snapshot = harness.display_manager.current_snapshot();
    let resolved = harness
        .assignment_service
        .resolve_all_displays(&snapshot, &harness.catalog);
    let plan = coordinator.create_plan(&snapshot, &resolved);
    coordinator
        .apply_plan(&plan, &harness.host_candidate)
        .unwrap();

    let s2_initial = coordinator
        .get_session(&harness.display2_id)
        .unwrap()
        .clone();

    // Now change Display 1's assignment to wallpaper 2
    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper2_id)
        .unwrap();

    let resolved2 = harness
        .assignment_service
        .resolve_all_displays(&snapshot, &harness.catalog);
    let plan2 = coordinator.create_plan(&snapshot, &resolved2);

    // Verify delta: D2 is retained, D1 is reconfigured
    assert!(plan2.actions.iter().any(|a| matches!(
        a,
        SessionPlanAction::RetainSession { display_id, .. } if *display_id == harness.display2_id
    )));
    assert!(plan2.actions.iter().any(|a| matches!(
        a,
        SessionPlanAction::ReconfigureSession { display_id, .. } if *display_id == harness.display1_id
    )));

    let report2 = coordinator
        .apply_plan(&plan2, &harness.host_candidate)
        .unwrap();
    assert_eq!(report2.retained_count, 1);
    assert_eq!(report2.reconfigured.len(), 1);
    assert_eq!(report2.reconfigured[0], harness.display1_id);

    // Verify Display 2 was completely undisturbed
    let s2_after = coordinator.get_session(&harness.display2_id).unwrap();
    assert_eq!(s2_after.session_id, s2_initial.session_id);
    assert_eq!(s2_after.source_handle, s2_initial.source_handle);
    assert_eq!(s2_after.generation, s2_initial.generation);
    assert_eq!(s2_after.state, DisplaySessionState::Active);

    // Verify Display 1 has new wallpaper
    let s1_after = coordinator.get_session(&harness.display1_id).unwrap();
    assert_eq!(s1_after.wallpaper_id, harness.wallpaper2_id);
    assert_eq!(s1_after.state, DisplaySessionState::Active);
}

#[test]
fn test_display_disconnect_cleans_up_session_preserves_intent() {
    let mut harness = TestSessionHarness::new();
    let mut coordinator = harness.create_coordinator();

    // Initial dual setup
    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper1_id)
        .unwrap();
    harness
        .assignment_service
        .assign_wallpaper(&harness.display2_id, harness.wallpaper2_id)
        .unwrap();

    let snapshot1 = harness.display_manager.current_snapshot();
    let resolved1 = harness
        .assignment_service
        .resolve_all_displays(&snapshot1, &harness.catalog);
    let plan1 = coordinator.create_plan(&snapshot1, &resolved1);
    coordinator
        .apply_plan(&plan1, &harness.host_candidate)
        .unwrap();
    assert_eq!(coordinator.active_session_count(), 2);

    // Disconnect Display 2: single display remaining
    harness.display_provider.setup_single_display(1920, 1080);
    let (snapshot2, _) = harness.display_manager.refresh().expect("Snapshot refresh");
    assert_eq!(snapshot2.displays.len(), 1);

    let resolved2 = harness
        .assignment_service
        .resolve_all_displays(&snapshot2, &harness.catalog);

    // Verify D2 resolution preserves intent with DisplayDisconnected outcome
    let r2 = resolved2
        .iter()
        .find(|r| r.display_id == harness.display2_id)
        .unwrap();
    assert_eq!(r2.outcome, AssignmentResolutionOutcome::DisplayDisconnected);
    assert!(!r2.is_playable);

    // Reconcile
    let plan2 = coordinator.create_plan(&snapshot2, &resolved2);
    assert!(plan2.actions.iter().any(|a| matches!(
        a,
        SessionPlanAction::StopSession { display_id, reason: SessionStopReason::DisplayDisconnected, .. }
        if *display_id == harness.display2_id
    )));
    assert!(plan2.actions.iter().any(|a| matches!(
        a,
        SessionPlanAction::RetainSession { display_id, .. } if *display_id == harness.display1_id
    )));

    let report2 = coordinator
        .apply_plan(&plan2, &harness.host_candidate)
        .unwrap();
    assert_eq!(report2.stopped.len(), 1);
    assert_eq!(report2.stopped[0], harness.display2_id);
    assert_eq!(coordinator.active_session_count(), 1);

    // Verify Display 2 session is stopped and surface released
    let s2 = coordinator.get_session(&harness.display2_id).unwrap();
    assert_eq!(s2.state, DisplaySessionState::Stopped);
    assert!(s2.source_handle.is_none());

    // Verify Display 1 is still active
    let s1 = coordinator.get_session(&harness.display1_id).unwrap();
    assert_eq!(s1.state, DisplaySessionState::Active);
}

#[test]
fn test_display_reconnect_reactivates_session() {
    let mut harness = TestSessionHarness::new();
    let mut coordinator = harness.create_coordinator();

    // Initial dual setup
    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper1_id)
        .unwrap();
    harness
        .assignment_service
        .assign_wallpaper(&harness.display2_id, harness.wallpaper2_id)
        .unwrap();

    let snapshot1 = harness.display_manager.current_snapshot();
    let resolved1 = harness
        .assignment_service
        .resolve_all_displays(&snapshot1, &harness.catalog);
    let plan1 = coordinator.create_plan(&snapshot1, &resolved1);
    coordinator
        .apply_plan(&plan1, &harness.host_candidate)
        .unwrap();

    // Disconnect Display 2
    harness.display_provider.setup_single_display(1920, 1080);
    let (snapshot2, _) = harness.display_manager.refresh().unwrap();
    let resolved2 = harness
        .assignment_service
        .resolve_all_displays(&snapshot2, &harness.catalog);
    let plan2 = coordinator.create_plan(&snapshot2, &resolved2);
    coordinator
        .apply_plan(&plan2, &harness.host_candidate)
        .unwrap();
    assert_eq!(coordinator.active_session_count(), 1);

    // Now re-connect Display 2 with identical hardware geometry
    let mon1 = WindowRect::new(0, 0, 1920, 1080);
    let mon2 = WindowRect::new(1920, 0, 3840, 1080);
    harness.display_provider.setup_dual_extended(mon1, mon2);

    let (snapshot3, _) = harness.display_manager.refresh().unwrap();
    let resolved3 = harness
        .assignment_service
        .resolve_all_displays(&snapshot3, &harness.catalog);

    let plan3 = coordinator.create_plan(&snapshot3, &resolved3);
    assert!(plan3.actions.iter().any(|a| matches!(
        a,
        SessionPlanAction::CreateSession { display_id, .. } if *display_id == harness.display2_id
    )));

    let report3 = coordinator
        .apply_plan(&plan3, &harness.host_candidate)
        .unwrap();
    assert_eq!(report3.activated.len(), 1);
    assert_eq!(report3.activated[0], harness.display2_id);
    assert_eq!(coordinator.active_session_count(), 2);

    let s2_reconnected = coordinator.get_session(&harness.display2_id).unwrap();
    assert_eq!(s2_reconnected.state, DisplaySessionState::Active);
    assert_eq!(s2_reconnected.wallpaper_id, harness.wallpaper2_id);
}

/// Custom surface factory to test failure isolation
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
                "Simulated hardware acceleration failure".into(),
            ))
        } else {
            self.inner.create_surface(display_id, geometry)
        }
    }

    fn destroy_surface(&self, handle: NativeWindowHandle) -> Result<(), SessionError> {
        self.inner.destroy_surface(handle)
    }
}

#[test]
fn test_failure_isolation_between_displays() {
    let harness = TestSessionHarness::new();

    // Use a factory that fails ONLY on Display 2
    let factory = Arc::new(FailingSurfaceFactory {
        inner: ProviderSurfaceFactory::new(harness.window_provider.clone()),
        fail_display_id: harness.display2_id.clone(),
    });

    let mut coordinator = DisplaySessionCoordinator::new(
        harness.window_provider.clone(),
        harness.attachment_coordinator.clone(),
        factory,
    );

    // Assign wallpapers to both displays
    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper1_id)
        .unwrap();
    harness
        .assignment_service
        .assign_wallpaper(&harness.display2_id, harness.wallpaper2_id)
        .unwrap();

    let snapshot = harness.display_manager.current_snapshot();
    let resolved = harness
        .assignment_service
        .resolve_all_displays(&snapshot, &harness.catalog);

    let plan = coordinator.create_plan(&snapshot, &resolved);
    let report = coordinator
        .apply_plan(&plan, &harness.host_candidate)
        .unwrap();

    // Display 1 succeeded!
    assert_eq!(report.activated.len(), 1);
    assert_eq!(report.activated[0], harness.display1_id);

    // Display 2 failed, recorded in failure map
    assert_eq!(report.failures.len(), 1);
    assert!(report.failures.contains_key(&harness.display2_id));

    // Verify Display 1 is healthy and Active
    let s1 = coordinator.get_session(&harness.display1_id).unwrap();
    assert_eq!(s1.state, DisplaySessionState::Active);

    // Verify Display 2 is marked Failed with error message
    let s2 = coordinator.get_session(&harness.display2_id).unwrap();
    assert!(matches!(s2.state, DisplaySessionState::Failed { .. }));
}

#[test]
fn test_stale_generation_rejection() {
    let harness = TestSessionHarness::new();
    let mut coordinator = harness.create_coordinator();

    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper1_id)
        .unwrap();

    let snapshot = harness.display_manager.current_snapshot();
    let resolved = harness
        .assignment_service
        .resolve_all_displays(&snapshot, &harness.catalog);
    let plan = coordinator.create_plan(&snapshot, &resolved);
    coordinator
        .apply_plan(&plan, &harness.host_candidate)
        .unwrap();

    // Generation 1 validation passes
    assert!(coordinator
        .validate_generation(&harness.display1_id, 1)
        .is_ok());

    // Stale generation 0 fails
    assert!(matches!(
        coordinator.validate_generation(&harness.display1_id, 0),
        Err(SessionError::StaleGeneration {
            expected: 1,
            actual: 0,
            ..
        })
    ));

    // Update geometry advances generation to 2
    let new_geom = DisplayGeometry {
        monitor_rect: WindowRect::new(0, 0, 2560, 1440),
        work_area_rect: WindowRect::new(0, 0, 2560, 1400),
        width: 2560,
        height: 1440,
        scale_factor_percent: Some(100),
    };

    let plan_update = SessionPlan {
        actions: vec![SessionPlanAction::UpdateGeometry {
            session_id: coordinator
                .get_session(&harness.display1_id)
                .unwrap()
                .session_id,
            display_id: harness.display1_id.clone(),
            new_geometry: new_geom,
        }],
        snapshot_generation: 2,
        plan_timestamp_millis: 1000,
    };

    coordinator
        .apply_plan(&plan_update, &harness.host_candidate)
        .unwrap();

    // Now generation 1 is stale!
    assert!(matches!(
        coordinator.validate_generation(&harness.display1_id, 1),
        Err(SessionError::StaleGeneration {
            expected: 2,
            actual: 1,
            ..
        })
    ));
    assert!(coordinator
        .validate_generation(&harness.display1_id, 2)
        .is_ok());
}

#[test]
fn test_session_suspend_and_resume_lifecycle() {
    let harness = TestSessionHarness::new();
    let mut coordinator = harness.create_coordinator();

    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper1_id)
        .unwrap();

    let snapshot = harness.display_manager.current_snapshot();
    let resolved = harness
        .assignment_service
        .resolve_all_displays(&snapshot, &harness.catalog);
    let plan = coordinator.create_plan(&snapshot, &resolved);
    coordinator
        .apply_plan(&plan, &harness.host_candidate)
        .unwrap();

    // Suspend
    coordinator.suspend_session(&harness.display1_id).unwrap();
    let s = coordinator.get_session(&harness.display1_id).unwrap();
    assert_eq!(s.state, DisplaySessionState::Suspended);

    // Resume
    coordinator.resume_session(&harness.display1_id).unwrap();
    let s = coordinator.get_session(&harness.display1_id).unwrap();
    assert_eq!(s.state, DisplaySessionState::Active);
}

#[test]
fn test_session_coordinator_orderly_shutdown() {
    let harness = TestSessionHarness::new();
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
    let resolved = harness
        .assignment_service
        .resolve_all_displays(&snapshot, &harness.catalog);
    let plan = coordinator.create_plan(&snapshot, &resolved);
    coordinator
        .apply_plan(&plan, &harness.host_candidate)
        .unwrap();
    assert_eq!(coordinator.active_session_count(), 2);

    // Orderly shutdown
    coordinator.shutdown().unwrap();
    assert!(coordinator.is_shut_down());
    assert_eq!(coordinator.active_session_count(), 0);

    for session in coordinator.list_sessions() {
        assert_eq!(session.state, DisplaySessionState::Stopped);
        assert!(session.source_handle.is_none());
    }

    // Subsequent actions rejected
    assert!(matches!(
        coordinator.apply_plan(&plan, &harness.host_candidate),
        Err(SessionError::CoordinatorShutDown)
    ));
}

#[test]
fn test_session_diagnostic_history_bounded() {
    let harness = TestSessionHarness::new();
    let mut coordinator = harness.create_coordinator();

    harness
        .assignment_service
        .assign_wallpaper(&harness.display1_id, harness.wallpaper1_id)
        .unwrap();

    let snapshot = harness.display_manager.current_snapshot();
    let resolved = harness
        .assignment_service
        .resolve_all_displays(&snapshot, &harness.catalog);
    let plan = coordinator.create_plan(&snapshot, &resolved);
    coordinator
        .apply_plan(&plan, &harness.host_candidate)
        .unwrap();

    // Trigger many suspend/resume cycles to exceed 100 events
    for _ in 0..60 {
        coordinator.suspend_session(&harness.display1_id).unwrap();
        coordinator.resume_session(&harness.display1_id).unwrap();
    }

    let diagnostics = coordinator.get_diagnostics();
    assert_eq!(diagnostics.len(), MAX_SESSION_DIAGNOSTIC_EVENTS);
}
