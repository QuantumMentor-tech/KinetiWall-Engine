//! Unit and persistence tests for per-monitor wallpaper assignment, validation, and resolution (Step 43).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use kinetiwall_catalog::{
    AssetReference, AvailabilityState, CatalogError, CatalogLifecycle, WallpaperCatalogRepository,
    WallpaperId, WallpaperMetadata, WallpaperRecord, WallpaperSourceKind, WallpaperType,
};
use kinetiwall_desktop::{
    AssignmentKind, AssignmentResolutionOutcome, AssignmentService, DisplayId,
    DisplayPresenceStatus, FileAssignmentRepository, MockAssignmentRepository,
    MockDesktopDisplayProvider, MAX_ASSIGNMENT_DIAGNOSTIC_EVENTS,
};
use uuid::Uuid;

// -------------------------------------------------------------------------------------------------
// Test Catalog Repository
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
// Unit Tests
// -------------------------------------------------------------------------------------------------

#[test]
fn test_assignment_creation_and_retrieval() {
    let repo = Arc::new(MockAssignmentRepository::new());
    let service = AssignmentService::new(repo).expect("Service init");

    let disp_id = DisplayId::new("disp_DISPLAY1_MONITOR_DEL4182_1");
    let wall_id = WallpaperId::new();

    let assignment = service
        .assign_wallpaper(&disp_id, wall_id)
        .expect("Assign should succeed");

    assert_eq!(assignment.display_id, disp_id);
    assert_eq!(assignment.wallpaper_id, wall_id);
    assert_eq!(assignment.kind, AssignmentKind::Explicit);

    let retrieved = service
        .get_assignment(&disp_id)
        .expect("Should find assignment");
    assert_eq!(retrieved.wallpaper_id, wall_id);
    assert_eq!(service.list_assignments().len(), 1);
}

#[test]
fn test_assignment_replacement_and_idempotency() {
    let repo = Arc::new(MockAssignmentRepository::new());
    let service = AssignmentService::new(repo).expect("Service init");

    let disp_id = DisplayId::new("disp_DISPLAY1");
    let wall_id1 = WallpaperId::new();
    let wall_id2 = WallpaperId::new();

    // First assignment
    let assign1 = service
        .assign_wallpaper(&disp_id, wall_id1)
        .expect("Assign 1");
    assert_eq!(assign1.wallpaper_id, wall_id1);

    // Idempotent assignment: same display and wallpaper
    let assign_repeat = service
        .assign_wallpaper(&disp_id, wall_id1)
        .expect("Assign repeat");
    assert_eq!(assign_repeat.assignment_id, assign1.assignment_id);
    assert_eq!(service.list_assignments().len(), 1);

    // Atomically replace with different wallpaper
    let assign2 = service
        .assign_wallpaper(&disp_id, wall_id2)
        .expect("Assign 2");
    assert_eq!(assign2.wallpaper_id, wall_id2);

    // Must enforce uniqueness: still exactly 1 assignment for this display!
    let all = service.list_assignments();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].wallpaper_id, wall_id2);
}

#[test]
fn test_assignment_removal() {
    let repo = Arc::new(MockAssignmentRepository::new());
    let service = AssignmentService::new(repo).expect("Service init");

    let disp_id = DisplayId::new("disp_DISPLAY1");
    let wall_id = WallpaperId::new();

    service.assign_wallpaper(&disp_id, wall_id).expect("Assign");
    assert!(service.get_assignment(&disp_id).is_some());

    // Removal of existing assignment
    assert!(service.remove_assignment(&disp_id).expect("Remove"));
    assert!(service.get_assignment(&disp_id).is_none());

    // Removal of absent assignment returns false
    assert!(!service.remove_assignment(&disp_id).expect("Remove absent"));
}

#[test]
fn test_assignment_global_fallback() {
    let repo = Arc::new(MockAssignmentRepository::new());
    let service = AssignmentService::new(repo).expect("Service init");

    assert!(service.get_global_wallpaper().is_none());

    let global_id = WallpaperId::new();
    service
        .set_global_wallpaper(Some(global_id))
        .expect("Set global");
    assert_eq!(service.get_global_wallpaper(), Some(global_id));

    // Clear global fallback
    service.set_global_wallpaper(None).expect("Clear global");
    assert!(service.get_global_wallpaper().is_none());
}

#[test]
fn test_assignment_resolution_valid_explicit() {
    let repo = Arc::new(MockAssignmentRepository::new());
    let service = AssignmentService::new(repo).expect("Service init");

    // Setup mock display provider and snapshot
    let display_provider = Arc::new(MockDesktopDisplayProvider::new());
    display_provider.setup_single_display(1920, 1080);
    let mut snap_mgr = kinetiwall_desktop::DisplaySnapshotManager::new(display_provider);
    let (snapshot, _) = snap_mgr.refresh().expect("Snapshot refresh");
    let disp_id = snapshot.displays[0].id.clone();

    let wall_id = WallpaperId::new();
    service.assign_wallpaper(&disp_id, wall_id).expect("Assign");

    // Setup catalog with available wallpaper
    let catalog = TestCatalog::new();
    catalog.add_wallpaper(
        wall_id,
        "Cosmic Nebula",
        WallpaperType::Video,
        AvailabilityState::Available,
    );

    let resolved = service.resolve_display(&disp_id, &snapshot, &catalog);

    assert_eq!(resolved.display_id, disp_id);
    assert_eq!(resolved.display_presence, DisplayPresenceStatus::Connected);
    assert_eq!(resolved.outcome, AssignmentResolutionOutcome::ValidExplicit);
    assert!(resolved.is_playable);
    assert_eq!(resolved.wallpaper_id, Some(wall_id));
    assert_eq!(resolved.effective_title, Some("Cosmic Nebula".to_string()));
    assert_eq!(resolved.effective_type, Some(WallpaperType::Video));
}

#[test]
fn test_assignment_resolution_disconnected_display() {
    let repo = Arc::new(MockAssignmentRepository::new());
    let service = AssignmentService::new(repo).expect("Service init");

    let disconnected_disp = DisplayId::new("disp_DISCONNECTED_MONITOR");
    let wall_id = WallpaperId::new();
    service
        .assign_wallpaper(&disconnected_disp, wall_id)
        .expect("Assign");

    // Snapshot has only DISPLAY1
    let display_provider = Arc::new(MockDesktopDisplayProvider::new());
    display_provider.setup_single_display(1920, 1080);
    let mut snap_mgr = kinetiwall_desktop::DisplaySnapshotManager::new(display_provider);
    let (snapshot, _) = snap_mgr.refresh().expect("Snapshot refresh");

    let catalog = TestCatalog::new();
    catalog.add_wallpaper(
        wall_id,
        "Aurora Borealis",
        WallpaperType::Video,
        AvailabilityState::Available,
    );

    let resolved = service.resolve_display(&disconnected_disp, &snapshot, &catalog);

    // Disconnected target preserves saved intent without attempting playback
    assert_eq!(
        resolved.display_presence,
        DisplayPresenceStatus::Disconnected
    );
    assert_eq!(
        resolved.outcome,
        AssignmentResolutionOutcome::DisplayDisconnected
    );
    assert!(!resolved.is_playable);
    assert_eq!(resolved.wallpaper_id, Some(wall_id));
}

#[test]
fn test_assignment_resolution_missing_wallpaper() {
    let repo = Arc::new(MockAssignmentRepository::new());
    let service = AssignmentService::new(repo).expect("Service init");

    let display_provider = Arc::new(MockDesktopDisplayProvider::new());
    display_provider.setup_single_display(1920, 1080);
    let mut snap_mgr = kinetiwall_desktop::DisplaySnapshotManager::new(display_provider);
    let (snapshot, _) = snap_mgr.refresh().expect("Snapshot refresh");
    let disp_id = snapshot.displays[0].id.clone();

    let missing_wall_id = WallpaperId::new();
    service
        .assign_wallpaper(&disp_id, missing_wall_id)
        .expect("Assign");

    let empty_catalog = TestCatalog::new(); // does not contain missing_wall_id

    let resolved = service.resolve_display(&disp_id, &snapshot, &empty_catalog);

    assert_eq!(resolved.display_presence, DisplayPresenceStatus::Connected);
    assert_eq!(
        resolved.outcome,
        AssignmentResolutionOutcome::WallpaperMissing
    );
    assert!(!resolved.is_playable);
}

#[test]
fn test_assignment_resolution_unavailable_asset() {
    let repo = Arc::new(MockAssignmentRepository::new());
    let service = AssignmentService::new(repo).expect("Service init");

    let display_provider = Arc::new(MockDesktopDisplayProvider::new());
    display_provider.setup_single_display(1920, 1080);
    let mut snap_mgr = kinetiwall_desktop::DisplaySnapshotManager::new(display_provider);
    let (snapshot, _) = snap_mgr.refresh().expect("Snapshot refresh");
    let disp_id = snapshot.displays[0].id.clone();

    let wall_id = WallpaperId::new();
    service.assign_wallpaper(&disp_id, wall_id).expect("Assign");

    let catalog = TestCatalog::new();
    catalog.add_wallpaper(
        wall_id,
        "Broken Asset",
        WallpaperType::Web,
        AvailabilityState::Missing,
    );

    let resolved = service.resolve_display(&disp_id, &snapshot, &catalog);

    assert_eq!(resolved.display_presence, DisplayPresenceStatus::Connected);
    assert_eq!(
        resolved.outcome,
        AssignmentResolutionOutcome::WallpaperUnavailable
    );
    assert!(!resolved.is_playable);
}

#[test]
fn test_assignment_file_persistence_atomic_roundtrip() {
    let temp_dir = std::env::temp_dir().join(format!("kw_test_assign_{}", Uuid::new_v4()));
    let store_path = temp_dir.join("assignments.json");

    let repo1 = Arc::new(FileAssignmentRepository::new(store_path.clone()));
    let service1 = AssignmentService::new(repo1).expect("Service 1 init");

    let disp1 = DisplayId::new("disp_MONITOR_1");
    let disp2 = DisplayId::new("disp_MONITOR_2");
    let wall1 = WallpaperId::new();
    let wall2 = WallpaperId::new();
    let global_wall = WallpaperId::new();

    service1.assign_wallpaper(&disp1, wall1).expect("Assign 1");
    service1.assign_wallpaper(&disp2, wall2).expect("Assign 2");
    service1
        .set_global_wallpaper(Some(global_wall))
        .expect("Set global");

    // Reopen using a completely new repository instance targeting the same file
    let repo2 = Arc::new(FileAssignmentRepository::new(store_path.clone()));
    let service2 = AssignmentService::new(repo2).expect("Service 2 init");

    assert_eq!(service2.get_global_wallpaper(), Some(global_wall));
    assert_eq!(service2.get_assignment(&disp1).unwrap().wallpaper_id, wall1);
    assert_eq!(service2.get_assignment(&disp2).unwrap().wallpaper_id, wall2);
    assert_eq!(service2.list_assignments().len(), 2);

    // Clean up
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_assignment_resolve_all_displays_includes_connected_and_disconnected() {
    let repo = Arc::new(MockAssignmentRepository::new());
    let service = AssignmentService::new(repo).expect("Service init");

    let display_provider = Arc::new(MockDesktopDisplayProvider::new());
    display_provider.setup_single_display(1920, 1080);
    let mut snap_mgr = kinetiwall_desktop::DisplaySnapshotManager::new(display_provider);
    let (snapshot, _) = snap_mgr.refresh().expect("Snapshot refresh");
    let active_disp = snapshot.displays[0].id.clone();

    let disconnected_disp = DisplayId::new("disp_DISCONNECTED_PROJECTOR");
    let wall1 = WallpaperId::new();
    let wall2 = WallpaperId::new();

    service
        .assign_wallpaper(&active_disp, wall1)
        .expect("Assign 1");
    service
        .assign_wallpaper(&disconnected_disp, wall2)
        .expect("Assign 2");

    let catalog = TestCatalog::new();
    catalog.add_wallpaper(
        wall1,
        "Active Wallpaper",
        WallpaperType::Video,
        AvailabilityState::Available,
    );
    catalog.add_wallpaper(
        wall2,
        "Disconnected Wallpaper",
        WallpaperType::Web,
        AvailabilityState::Available,
    );

    let all_resolved = service.resolve_all_displays(&snapshot, &catalog);

    // Must resolve 2 displays: the connected one from the snapshot, AND the disconnected saved one!
    assert_eq!(all_resolved.len(), 2);

    let res_active = all_resolved
        .iter()
        .find(|r| r.display_id == active_disp)
        .expect("Active found");
    assert_eq!(
        res_active.display_presence,
        DisplayPresenceStatus::Connected
    );
    assert!(res_active.is_playable);

    let res_disconn = all_resolved
        .iter()
        .find(|r| r.display_id == disconnected_disp)
        .expect("Disconnected found");
    assert_eq!(
        res_disconn.display_presence,
        DisplayPresenceStatus::Disconnected
    );
    assert!(!res_disconn.is_playable);
}

#[test]
fn test_assignment_diagnostic_history_bounded() {
    let repo = Arc::new(MockAssignmentRepository::new());
    let service = AssignmentService::new(repo).expect("Service init");

    for i in 0..150 {
        let disp = DisplayId::new(format!("disp_{}", i));
        let wall = WallpaperId::new();
        service.assign_wallpaper(&disp, wall).expect("Assign");
    }

    let history = service.get_diagnostic_history();
    assert_eq!(history.len(), MAX_ASSIGNMENT_DIAGNOSTIC_EVENTS);

    service.clear_diagnostics();
    assert_eq!(service.get_diagnostic_history().len(), 0);
}
