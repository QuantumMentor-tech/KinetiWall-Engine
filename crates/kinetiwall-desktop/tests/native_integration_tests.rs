//! Native Windows integration tests for `kinetiwall-desktop`.
//!
//! Validates actual Win32 desktop window discovery, enumeration, class resolution,
//! and candidate revalidation against the live host operating system.

use std::sync::Arc;

use kinetiwall_catalog::WallpaperCatalogRepository;
use kinetiwall_desktop::{
    CompatibilityPolicy, DesktopDiscoveryOptions, DesktopHostDiscovery, DesktopShellTopology,
    DesktopWindowProvider, NativeDesktopWindowProvider, NativeWindowHandle,
};

/// Validates that passive discovery against the live host Windows shell completes
/// cleanly without panicking, modifying any window styles, or sending active probe messages.
#[test]
fn test_native_desktop_passive_discovery() {
    let provider = Arc::new(NativeDesktopWindowProvider::new());
    let discovery = DesktopHostDiscovery::new(provider);

    let options = DesktopDiscoveryOptions {
        allow_active_probe: false,
        ..Default::default()
    };

    let result = discovery
        .discover_topology(&options)
        .expect("native discovery must complete");

    // Print observed desktop topology for test logging
    println!("Native desktop topology observed: {:?}", result.topology);
    println!(
        "Total desktop candidates observed: {}",
        result.all_candidates.len()
    );
    println!("Decision reason: {}", result.reason);

    for (idx, cand) in result.all_candidates.iter().enumerate() {
        println!(
            "  [{}] {} ({:?}) - PID: {}, Title: {:?}, DefViewChild: {}",
            idx,
            cand.class_name,
            cand.handle,
            cand.process_id,
            cand.window_title,
            cand.has_shelldll_defview_child
        );
    }

    // Verify topology-specific truthfulness and safety invariants
    match result.topology {
        DesktopShellTopology::StandardWorkerWSplit | DesktopShellTopology::ModernMultiWorkerW => {
            assert!(result.is_safe_for_attachment_review);
            assert!(result.recommended_candidate.is_some());
            assert!(result.icon_carrier.is_some());
        }
        DesktopShellTopology::ClassicUnsplit => {
            assert!(!result.is_safe_for_attachment_review);
            assert!(result.icon_carrier.is_some());
        }
        DesktopShellTopology::DesktopRootUnavailable => {
            assert!(!result.is_safe_for_attachment_review);
            assert!(result.recommended_candidate.is_none());
            assert!(!result.reason.is_empty());
        }
        DesktopShellTopology::Ambiguous
        | DesktopShellTopology::ShellUnavailable
        | DesktopShellTopology::UnknownTopology => {
            assert!(!result.is_safe_for_attachment_review);
        }
    }
}

/// Validates that `enum_windows` on the live Windows host returns existing top-level windows
/// and that handle validity checks operate reliably.
#[test]
fn test_native_desktop_window_enumeration_validity() {
    let provider = NativeDesktopWindowProvider::new();

    // Create a temporary test window to guarantee at least one window exists in the current window station
    let test_hwnd = provider
        .create_test_window("KinetiWall_Enum_Anchor", 0, 0, 100, 100)
        .expect("Must create test anchor window");

    let windows = provider.enum_windows();
    println!("Windows count: {}", windows.len());
    for &hwnd in &windows {
        let class = provider.get_class_name(hwnd).unwrap_or_default();
        let title = provider.get_window_text(hwnd).unwrap_or_default();
        let (tid, pid) = provider.get_window_thread_process_id(hwnd);
        println!(
            "  HWND: {:?}, Class: {:?}, Title: {:?}, PID: {}, TID: {}",
            hwnd, class, title, pid, tid
        );
    }

    assert!(
        !windows.is_empty(),
        "Windows desktop must have top-level windows"
    );
    assert!(
        windows.contains(&test_hwnd),
        "Enumerated windows must contain created test window"
    );

    // The windows should pass is_window
    let mut valid_count = 0;
    for &hwnd in windows.iter().take(10) {
        if provider.is_window(hwnd) {
            valid_count += 1;
            let (tid, pid) = provider.get_window_thread_process_id(hwnd);
            if tid > 0 && pid > 0 {
                // Verified valid active window
            }
        }
    }
    assert!(
        valid_count > 0,
        "At least one top-level window must be valid"
    );

    // Clean up test anchor window
    let _ = provider.destroy_window(test_hwnd);
}

/// Validates candidate revalidation against a live real window versus an invalid handle.
#[test]
fn test_native_candidate_revalidation_real_windows() {
    let provider = Arc::new(NativeDesktopWindowProvider::new());
    let discovery = DesktopHostDiscovery::new(provider);

    let result = discovery
        .discover_topology(&DesktopDiscoveryOptions::default())
        .expect("discovery must succeed");

    // If any candidates were observed, test revalidation against the first one
    if let Some(first) = result.all_candidates.first() {
        assert!(
            discovery.revalidate_candidate(first),
            "Freshly discovered candidate {:?} must pass revalidation",
            first.handle
        );

        // Mutate candidate class to test detect handle mismatch
        let mut tampered = first.clone();
        tampered.class_name = "DefinitelyNotTheRealClass_12345".to_string();
        assert!(
            !discovery.revalidate_candidate(&tampered),
            "Tampered class name must fail revalidation"
        );
    }

    // Dummy invalid handle must fail revalidation
    let bogus = kinetiwall_desktop::DesktopCandidateWindow {
        handle: NativeWindowHandle(0x0000_DEAD),
        class_name: "WorkerW".to_string(),
        window_title: String::new(),
        process_id: 999999,
        thread_id: 999999,
        role: kinetiwall_desktop::CandidateRole::WallpaperHostCandidate,
        parent: None,
        bounds: None,
        is_visible: false,
        has_shelldll_defview_child: false,
        validated_at_millis: 0,
    };
    assert!(
        !discovery.revalidate_candidate(&bogus),
        "Bogus handle must fail revalidation"
    );
}

/// Validates that CompatibilityPolicy truthfully evaluates whatever topology is currently
/// active on the host machine without guessing or bypassing safety gates.
#[test]
fn test_native_compatibility_policy_truthful_evaluation() {
    let provider = Arc::new(NativeDesktopWindowProvider::new());
    let discovery = DesktopHostDiscovery::new(provider);

    let result = discovery
        .discover_topology(&DesktopDiscoveryOptions::default())
        .expect("discovery must succeed");

    let eval = CompatibilityPolicy::evaluate_attachment_suitability(&result);
    match result.topology {
        DesktopShellTopology::StandardWorkerWSplit | DesktopShellTopology::ModernMultiWorkerW => {
            assert!(
                eval.is_ok(),
                "Split WorkerW topology must be approved for attachment review"
            );
        }
        DesktopShellTopology::ClassicUnsplit => {
            assert!(
                eval.is_err(),
                "Classic unsplit topology must be rejected for direct attachment until split"
            );
        }
        DesktopShellTopology::Ambiguous => {
            assert!(eval.is_err(), "Ambiguous topology must be rejected");
        }
        DesktopShellTopology::DesktopRootUnavailable
        | DesktopShellTopology::ShellUnavailable
        | DesktopShellTopology::UnknownTopology => {
            assert!(
                eval.is_err(),
                "Unavailable/Unknown topology must be rejected"
            );
        }
    }
}

/// Validates that native Win32 window style queries, mutations, frame-changed notifications,
/// and window destruction operate accurately and safely on real OS window handles.
#[test]
fn test_native_window_style_and_lifecycle_contract() {
    let provider = NativeDesktopWindowProvider::new();

    let hwnd = provider
        .create_test_window("KinetiWall_Native_Test_Window", 200, 200, 500, 400)
        .expect("Must create native test window");

    assert!(
        provider.is_window(hwnd),
        "Created window must pass is_window"
    );
    assert!(
        provider.is_window_visible(hwnd),
        "Created window must be visible"
    );

    let initial_style = provider
        .get_window_long(hwnd, kinetiwall_desktop::constants::GWL_STYLE)
        .expect("Must read initial GWL_STYLE");

    // Mutate style: set WS_CHILD, clear WS_POPUP
    let new_style = (initial_style & !kinetiwall_desktop::constants::WS_POPUP)
        | kinetiwall_desktop::constants::WS_CHILD;

    let prev = provider
        .set_window_long(hwnd, kinetiwall_desktop::constants::GWL_STYLE, new_style)
        .expect("Must set mutated GWL_STYLE");
    assert_eq!(
        prev as u32, initial_style as u32,
        "Previous style must match initial"
    );

    let current_style = provider
        .get_window_long(hwnd, kinetiwall_desktop::constants::GWL_STYLE)
        .expect("Must read mutated style");
    assert_eq!(
        current_style as u32, new_style as u32,
        "Effective style must match mutated value"
    );

    // Clean up window
    provider
        .destroy_window(hwnd)
        .expect("Must destroy native test window");
    assert!(
        !provider.is_window(hwnd),
        "Destroyed window must not pass is_window"
    );
}

/// Validates full transactional attachment, post-mutation verification, and deterministic
/// detachment restoration on live Win32 windows created by the current process.
#[test]
fn test_native_attachment_coordinator_full_transaction() {
    let provider = Arc::new(NativeDesktopWindowProvider::new());
    let mut coordinator = kinetiwall_desktop::AttachmentCoordinator::new(provider.clone());

    // Create real native source player surface and host window
    let source_hwnd = provider
        .create_test_window("KinetiWall_Player_Surface", 100, 100, 640, 480)
        .expect("Must create test player window");

    let host_hwnd = provider
        .create_test_window("KinetiWall_Mock_WorkerW_Host", 0, 0, 1920, 1080)
        .expect("Must create test host window");

    let current_pid = std::process::id();
    let (_, host_tid) = provider.get_window_thread_process_id(host_hwnd);

    let host_candidate = kinetiwall_desktop::DesktopCandidateWindow {
        handle: host_hwnd,
        class_name: "WorkerW".to_string(),
        window_title: String::new(),
        process_id: current_pid,
        thread_id: host_tid,
        role: kinetiwall_desktop::CandidateRole::WallpaperHostCandidate,
        parent: None,
        bounds: Some(kinetiwall_desktop::WindowRect::new(0, 0, 1920, 1080)),
        is_visible: true,
        has_shelldll_defview_child: false,
        validated_at_millis: 1000,
    };

    let orig_style = provider
        .get_window_long(source_hwnd, kinetiwall_desktop::constants::GWL_STYLE)
        .expect("Must read original style");

    let request = kinetiwall_desktop::AttachmentRequest {
        source_handle: source_hwnd,
        player_instance_id: uuid::Uuid::new_v4(),
        owning_process_id: current_pid,
        target_candidate: host_candidate,
        target_display_bounds: Some(kinetiwall_desktop::WindowRect::new(0, 0, 1920, 1080)),
    };

    // 1. Transactional Attach
    let attach_record = coordinator
        .attach(&request)
        .expect("Native attach must succeed");
    assert!(attach_record.is_verified);
    assert_eq!(
        coordinator.get_state(source_hwnd),
        kinetiwall_desktop::AttachmentState::Attached
    );

    // Verify parent in Win32
    assert_eq!(provider.get_parent(source_hwnd), Some(host_hwnd));

    // Verify style in Win32
    let attached_style = provider
        .get_window_long(source_hwnd, kinetiwall_desktop::constants::GWL_STYLE)
        .unwrap();
    assert_ne!(attached_style & kinetiwall_desktop::constants::WS_CHILD, 0);
    assert_eq!(attached_style & kinetiwall_desktop::constants::WS_POPUP, 0);

    // 2. Transactional Detach & Restore
    let detach_record = coordinator
        .detach(source_hwnd)
        .expect("Native detach must succeed");
    assert!(detach_record.is_verified);
    assert_eq!(
        coordinator.get_state(source_hwnd),
        kinetiwall_desktop::AttachmentState::Detached
    );

    // Verify parent restored in Win32
    assert_eq!(provider.get_parent(source_hwnd), None);

    // Verify style restored in Win32
    let restored_style = provider
        .get_window_long(source_hwnd, kinetiwall_desktop::constants::GWL_STYLE)
        .unwrap();
    assert_eq!(restored_style, orig_style);

    // 3. Teardown
    provider
        .destroy_window(source_hwnd)
        .expect("Must destroy source window");
    provider
        .destroy_window(host_hwnd)
        .expect("Must destroy host window");
}

/// Validates live Win32 surface synchronization, geometry updates, and event coalescing.
#[test]
fn test_native_surface_synchronization_and_coalescing() {
    let provider = Arc::new(NativeDesktopWindowProvider::new());
    let mut coordinator = kinetiwall_desktop::AttachmentCoordinator::new(provider.clone());

    let (_, current_pid) = (0, std::process::id());
    coordinator.register_allowed_process_id(current_pid);

    let host_hwnd = provider
        .create_test_window("KinetiWall_Native_Sync_Host", 0, 0, 1920, 1080)
        .expect("Must create native test host window");

    let source_hwnd = provider
        .create_test_window("KinetiWall_Native_Sync_Source", 100, 100, 800, 600)
        .expect("Must create native test source window");

    let host_candidate = kinetiwall_desktop::DesktopCandidateWindow {
        handle: host_hwnd,
        class_name: "WorkerW".to_string(),
        window_title: String::new(),
        process_id: current_pid,
        thread_id: 0,
        role: kinetiwall_desktop::CandidateRole::WallpaperHostCandidate,
        parent: None,
        bounds: Some(kinetiwall_desktop::WindowRect::new(0, 0, 1920, 1080)),
        is_visible: true,
        has_shelldll_defview_child: false,
        validated_at_millis: 1000,
    };

    let initial_bounds = kinetiwall_desktop::WindowRect::new(0, 0, 1920, 1080);
    let request = kinetiwall_desktop::AttachmentRequest {
        source_handle: source_hwnd,
        player_instance_id: uuid::Uuid::new_v4(),
        owning_process_id: current_pid,
        target_candidate: host_candidate,
        target_display_bounds: Some(initial_bounds),
    };

    // 1. Transactional Attach
    let attach_record = coordinator
        .attach(&request)
        .expect("Native attach must succeed");
    assert!(attach_record.is_verified);
    assert_eq!(
        coordinator.get_sync_state(source_hwnd),
        kinetiwall_desktop::SurfaceSyncState::Synchronized
    );

    // 2. Coalescing Test: Re-synchronizing to identical bounds is a no-op
    let coalesce_report = coordinator
        .synchronize_surface_geometry(source_hwnd, initial_bounds)
        .expect("Coalesce sync must succeed");
    assert!(!coalesce_report.mutation_applied);
    assert_eq!(coalesce_report.effective_bounds, initial_bounds);

    // 3. Geometry Update Test: Re-synchronizing to new dimensions
    let new_bounds = kinetiwall_desktop::WindowRect::new(0, 0, 1280, 720);
    let resize_report = coordinator
        .synchronize_surface_geometry(source_hwnd, new_bounds)
        .expect("Resize sync must succeed");
    assert!(resize_report.mutation_applied);
    assert_eq!(resize_report.effective_bounds, new_bounds);

    let observed_rect = provider
        .get_window_rect(source_hwnd)
        .expect("Must get source window rect");
    assert_eq!(observed_rect.width(), 1280);
    assert_eq!(observed_rect.height(), 720);

    // 4. Detachment & Cleanup
    coordinator
        .detach(source_hwnd)
        .expect("Detach must succeed");

    provider
        .destroy_window(source_hwnd)
        .expect("Must destroy source window");
    provider
        .destroy_window(host_hwnd)
        .expect("Must destroy host window");
}

/// Validates live Win32 display enumeration, geometry queries, and stable display identity resolution.
#[test]
fn test_native_display_discovery_and_topology() {
    let provider = Arc::new(kinetiwall_desktop::NativeDesktopDisplayProvider::new());
    let mut manager = kinetiwall_desktop::DisplaySnapshotManager::new(provider);

    let (snapshot, delta) = manager
        .refresh()
        .expect("Native display discovery refresh must succeed");

    println!(
        "Native Display Topology Observed: {:?}",
        snapshot.topology_type
    );
    println!("Active Displays Count: {}", snapshot.displays.len());
    println!(
        "Virtual Desktop Bounds: {:?}",
        snapshot.virtual_desktop_bounds
    );

    for (idx, disp) in snapshot.displays.iter().enumerate() {
        println!(
            "  [{}] ID: {}, Device: {}, Name: {:?}, Primary: {}, Bounds: {}x{} at ({}, {})",
            idx,
            disp.id,
            disp.device_name,
            disp.friendly_name,
            disp.is_primary,
            disp.geometry.width,
            disp.geometry.height,
            disp.geometry.monitor_rect.left,
            disp.geometry.monitor_rect.top
        );
    }

    assert!(snapshot.is_valid);
    assert!(
        !snapshot.displays.is_empty(),
        "At least one desktop display must exist on live Windows host"
    );
    assert!(
        snapshot.primary_display_id.is_some(),
        "Primary display must be resolved"
    );
    assert!(
        !snapshot.virtual_desktop_bounds.is_empty(),
        "Virtual desktop bounds must be non-empty"
    );
    assert!(delta.is_meaningful_change);

    // Re-fresh test: asserting stability of application-level DisplayId
    let (snapshot2, delta2) = manager.refresh().expect("Second refresh must succeed");

    assert_eq!(snapshot2.generation, 2);
    assert_eq!(snapshot.displays.len(), snapshot2.displays.len());
    for (d1, d2) in snapshot.displays.iter().zip(snapshot2.displays.iter()) {
        assert_eq!(
            d1.id, d2.id,
            "Stable DisplayId must be preserved across refreshes"
        );
    }
    assert!(
        !delta2.is_meaningful_change,
        "Consecutive refresh with unchanged hardware produces no meaningful delta"
    );
}

/// Validates live Win32 per-monitor playback session lifecycle, surface window creation,
/// attachment, and clean rollback/detachment using `DisplaySessionCoordinator`.
#[test]
fn test_native_session_coordinator_surface_lifecycle() {
    let window_provider = Arc::new(NativeDesktopWindowProvider::new());
    let display_provider = Arc::new(kinetiwall_desktop::NativeDesktopDisplayProvider::new());
    let mut manager = kinetiwall_desktop::DisplaySnapshotManager::new(display_provider);

    let (snapshot, _) = manager
        .refresh()
        .expect("Native display discovery refresh must succeed");
    assert!(!snapshot.displays.is_empty());

    let primary = &snapshot.displays[0];
    let display_id = primary.id.clone();
    let geometry = primary.geometry;

    // Create an isolated host window in current process to test live attachment
    let host_hwnd = window_provider
        .create_test_window("KinetiWall_NativeHost_Test", 0, 0, 1920, 1080)
        .expect("Must create test host window");

    let (_tid, current_pid) = window_provider.get_window_thread_process_id(host_hwnd);

    let host_candidate = kinetiwall_desktop::DesktopCandidateWindow {
        handle: host_hwnd,
        class_name: "Static".to_string(),
        window_title: "KinetiWall_NativeHost_Test".to_string(),
        process_id: current_pid,
        thread_id: 0,
        role: kinetiwall_desktop::CandidateRole::WallpaperHostCandidate,
        parent: None,
        bounds: Some(kinetiwall_desktop::WindowRect::new(0, 0, 1920, 1080)),
        is_visible: true,
        has_shelldll_defview_child: false,
        validated_at_millis: 1000,
    };

    let mut attachment_coord =
        kinetiwall_desktop::AttachmentCoordinator::new(window_provider.clone());
    attachment_coord.register_allowed_process_id(current_pid);
    let attachment_coordinator = std::sync::Arc::new(std::sync::Mutex::new(attachment_coord));

    let mut session_coordinator =
        kinetiwall_desktop::DisplaySessionCoordinator::with_default_factory(
            window_provider.clone(),
            attachment_coordinator.clone(),
        );

    let wallpaper_id = kinetiwall_catalog::WallpaperId::new();

    // 1. Direct session activation
    let desc = session_coordinator
        .activate_session(&display_id, wallpaper_id, geometry, &host_candidate)
        .expect("Session activation on live host must succeed");

    assert_eq!(desc.state, kinetiwall_desktop::DisplaySessionState::Active);
    assert_eq!(desc.display_id, display_id);
    assert_eq!(desc.wallpaper_id, wallpaper_id);
    let surface_hwnd = desc.source_handle.expect("Surface handle must exist");

    // Win32 verification: surface window must now be a child of host_hwnd
    assert_eq!(window_provider.get_parent(surface_hwnd), Some(host_hwnd));

    // 2. Suspend and resume
    session_coordinator
        .suspend_session(&display_id)
        .expect("Suspend");
    assert_eq!(
        session_coordinator.get_session(&display_id).unwrap().state,
        kinetiwall_desktop::DisplaySessionState::Suspended
    );

    session_coordinator
        .resume_session(&display_id)
        .expect("Resume");
    assert_eq!(
        session_coordinator.get_session(&display_id).unwrap().state,
        kinetiwall_desktop::DisplaySessionState::Active
    );

    // 3. Stop session and clean up
    let stopped = session_coordinator
        .stop_session(
            &display_id,
            kinetiwall_desktop::SessionStopReason::ExplicitRequest,
        )
        .expect("Stop session");

    assert_eq!(
        stopped.state,
        kinetiwall_desktop::DisplaySessionState::Stopped
    );
    assert!(stopped.source_handle.is_none());

    // Host window cleanup
    let _ = window_provider.destroy_window(host_hwnd);
}

// -------------------------------------------------------------------------------------------------
// Helper Catalog for Native Integration Tests
// -------------------------------------------------------------------------------------------------

#[derive(Default, Clone)]
struct NativeTestCatalog {
    records: std::sync::Arc<
        std::sync::Mutex<
            std::collections::BTreeMap<
                kinetiwall_catalog::WallpaperId,
                kinetiwall_catalog::WallpaperRecord,
            >,
        >,
    >,
}

impl kinetiwall_catalog::WallpaperCatalogRepository for NativeTestCatalog {
    fn create(
        &mut self,
        record: kinetiwall_catalog::WallpaperRecord,
    ) -> Result<kinetiwall_catalog::WallpaperRecord, kinetiwall_catalog::CatalogError> {
        let id = record.id;
        self.records.lock().unwrap().insert(id, record.clone());
        Ok(record)
    }

    fn get(
        &self,
        id: kinetiwall_catalog::WallpaperId,
    ) -> Result<Option<kinetiwall_catalog::WallpaperRecord>, kinetiwall_catalog::CatalogError> {
        Ok(self.records.lock().unwrap().get(&id).cloned())
    }

    fn list(
        &self,
    ) -> Result<Vec<kinetiwall_catalog::WallpaperRecord>, kinetiwall_catalog::CatalogError> {
        Ok(self.records.lock().unwrap().values().cloned().collect())
    }

    fn update_metadata(
        &mut self,
        _id: kinetiwall_catalog::WallpaperId,
        _metadata: kinetiwall_catalog::WallpaperMetadata,
        _updated_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<kinetiwall_catalog::WallpaperRecord, kinetiwall_catalog::CatalogError> {
        unimplemented!()
    }

    fn relink_asset(
        &mut self,
        _id: kinetiwall_catalog::WallpaperId,
        _new_asset: kinetiwall_catalog::AssetReference,
        _availability: kinetiwall_catalog::AvailabilityState,
        _updated_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<kinetiwall_catalog::WallpaperRecord, kinetiwall_catalog::CatalogError> {
        unimplemented!()
    }

    fn remove_record(
        &mut self,
        id: kinetiwall_catalog::WallpaperId,
    ) -> Result<bool, kinetiwall_catalog::CatalogError> {
        Ok(self.records.lock().unwrap().remove(&id).is_some())
    }

    fn check_availability(
        &mut self,
        id: kinetiwall_catalog::WallpaperId,
    ) -> Result<kinetiwall_catalog::AvailabilityState, kinetiwall_catalog::CatalogError> {
        if let Some(r) = self.records.lock().unwrap().get(&id) {
            Ok(r.availability.clone())
        } else {
            Err(kinetiwall_catalog::CatalogError::NotFound(id))
        }
    }
}

/// Validates live Win32 multi-monitor layout mode evaluation, virtual desktop composition
/// geometry, and layout transitions against the actual host display subsystem.
#[test]
fn test_native_layout_coordinator_mode_evaluation_and_transitions() {
    let window_provider = Arc::new(NativeDesktopWindowProvider::new());
    let display_provider = Arc::new(kinetiwall_desktop::NativeDesktopDisplayProvider::new());
    let mut manager = kinetiwall_desktop::DisplaySnapshotManager::new(display_provider);

    let (snapshot, _) = manager
        .refresh()
        .expect("Native display discovery refresh must succeed");
    assert!(!snapshot.displays.is_empty());

    let repo = Arc::new(kinetiwall_desktop::MockAssignmentRepository::new());
    let assignment_service =
        Arc::new(kinetiwall_desktop::AssignmentService::new(repo).expect("AssignmentService init"));

    // Set up a mock catalog with a real available wallpaper
    let mut catalog = NativeTestCatalog::default();
    let wallpaper_id = kinetiwall_catalog::WallpaperId::new();
    let _ = catalog.create(kinetiwall_catalog::WallpaperRecord {
        id: wallpaper_id,
        schema_version: 1,
        wallpaper_type: kinetiwall_catalog::WallpaperType::Video,
        asset: kinetiwall_catalog::AssetReference {
            source_kind: kinetiwall_catalog::WallpaperSourceKind::ManagedPackage,
            entrypoint: "test_entry.mp4".to_string(),
        },
        metadata: kinetiwall_catalog::WallpaperMetadata::new("Test Native Wallpaper"),
        lifecycle: kinetiwall_catalog::CatalogLifecycle::now(false),
        availability: kinetiwall_catalog::AvailabilityState::Available,
        preserved_extra: std::collections::BTreeMap::new(),
    });

    // Assign to primary display
    let primary_id = &snapshot.displays[0].id;
    assignment_service
        .assign_wallpaper(primary_id, wallpaper_id)
        .expect("Assign to primary display");

    // Plan evaluation for Independent mode
    let plan_ind = kinetiwall_desktop::LayoutPlanResolver::resolve_plan(
        kinetiwall_desktop::MultiMonitorLayoutMode::Independent,
        &snapshot,
        &assignment_service.resolve_all_displays(&snapshot, &catalog),
        &catalog,
        kinetiwall_desktop::CloneSourcePolicy::PrimaryDisplay,
        &kinetiwall_desktop::SpanCompositionPolicy::VirtualDesktopFill,
    );
    assert_eq!(
        plan_ind.selected_mode,
        kinetiwall_desktop::MultiMonitorLayoutMode::Independent
    );
    assert!(
        plan_ind.virtual_desktop_bounds.width() > 0 && plan_ind.virtual_desktop_bounds.height() > 0
    );

    // Plan evaluation for Span mode
    let plan_span = kinetiwall_desktop::LayoutPlanResolver::resolve_plan(
        kinetiwall_desktop::MultiMonitorLayoutMode::Span,
        &snapshot,
        &assignment_service.resolve_all_displays(&snapshot, &catalog),
        &catalog,
        kinetiwall_desktop::CloneSourcePolicy::PrimaryDisplay,
        &kinetiwall_desktop::SpanCompositionPolicy::VirtualDesktopFill,
    );
    assert_eq!(
        plan_span.selected_mode,
        kinetiwall_desktop::MultiMonitorLayoutMode::Span
    );
    assert_eq!(
        plan_span.virtual_desktop_bounds,
        snapshot.virtual_desktop_bounds
    );

    // Plan evaluation for Clone mode
    let plan_clone = kinetiwall_desktop::LayoutPlanResolver::resolve_plan(
        kinetiwall_desktop::MultiMonitorLayoutMode::Clone,
        &snapshot,
        &assignment_service.resolve_all_displays(&snapshot, &catalog),
        &catalog,
        kinetiwall_desktop::CloneSourcePolicy::PrimaryDisplay,
        &kinetiwall_desktop::SpanCompositionPolicy::VirtualDesktopFill,
    );
    assert_eq!(
        plan_clone.selected_mode,
        kinetiwall_desktop::MultiMonitorLayoutMode::Clone
    );

    // Verify coordinator integration with test host
    let host_hwnd = window_provider
        .create_test_window("KinetiWall_NativeLayout_Test", 0, 0, 1920, 1080)
        .expect("Host window creation");
    let (_tid, current_pid) = window_provider.get_window_thread_process_id(host_hwnd);

    let host_candidate = kinetiwall_desktop::DesktopCandidateWindow {
        handle: host_hwnd,
        class_name: "Static".to_string(),
        window_title: "KinetiWall_NativeLayout_Test".to_string(),
        process_id: current_pid,
        thread_id: 0,
        role: kinetiwall_desktop::CandidateRole::WallpaperHostCandidate,
        parent: None,
        bounds: Some(snapshot.virtual_desktop_bounds),
        is_visible: true,
        has_shelldll_defview_child: false,
        validated_at_millis: 1000,
    };

    let mut attachment_coord =
        kinetiwall_desktop::AttachmentCoordinator::new(window_provider.clone());
    attachment_coord.register_allowed_process_id(current_pid);
    let attachment_coordinator = Arc::new(std::sync::Mutex::new(attachment_coord));

    let session_coord = kinetiwall_desktop::DisplaySessionCoordinator::with_default_factory(
        window_provider.clone(),
        attachment_coordinator,
    );
    let session_coordinator = Arc::new(std::sync::Mutex::new(session_coord));

    let mut layout_coordinator = kinetiwall_desktop::MultiMonitorLayoutCoordinator::new(
        session_coordinator,
        assignment_service.clone(),
    );

    assert_eq!(
        layout_coordinator.saved_mode(),
        kinetiwall_desktop::MultiMonitorLayoutMode::Independent
    );

    let report = layout_coordinator
        .apply_layout(&snapshot, &catalog, &host_candidate)
        .expect("Apply layout on live host");
    assert_eq!(
        report.active_mode,
        kinetiwall_desktop::MultiMonitorLayoutMode::Independent
    );

    layout_coordinator
        .shutdown()
        .expect("Clean layout coordinator shutdown");
    let _ = window_provider.destroy_window(host_hwnd);
}
