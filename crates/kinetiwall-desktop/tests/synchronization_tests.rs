//! Unit tests for desktop surface synchronization, display-change handling, and safe reattachment.

use std::sync::Arc;
use uuid::Uuid;

use kinetiwall_desktop::attachment::constants;
use kinetiwall_desktop::{
    AttachmentCoordinator, AttachmentRequest, CandidateRole, DesktopCandidateWindow,
    DesktopWindowProvider, MockDesktopWindowProvider, MockWindowRecord, NativeWindowHandle,
    SurfaceChangeEvent, SurfaceSyncError, SurfaceSyncState, WindowRect, MAX_SYNC_DIAGNOSTIC_EVENTS,
};

fn setup_test_environment() -> (
    Arc<MockDesktopWindowProvider>,
    AttachmentCoordinator,
    NativeWindowHandle,
    NativeWindowHandle,
    Uuid,
) {
    let provider = Arc::new(MockDesktopWindowProvider::new());
    let mut coordinator = AttachmentCoordinator::new(provider.clone());

    let owning_pid = 1234;
    coordinator.register_allowed_process_id(owning_pid);

    let progman = NativeWindowHandle(10);
    let shelldll = NativeWindowHandle(20);
    let workerw = NativeWindowHandle(30);
    let player_hwnd = NativeWindowHandle(100);

    provider.add_window(MockWindowRecord {
        handle: progman,
        class_name: "Progman".to_string(),
        window_title: "Program Manager".to_string(),
        process_id: 1000,
        thread_id: 1001,
        parent: None,
        is_visible: true,
        bounds: WindowRect::new(0, 0, 1920, 1080),
        children: vec![shelldll],
        style: constants::WS_POPUP | constants::WS_VISIBLE,
        ex_style: 0,
        dpi_context: Some(2),
    });

    provider.add_window(MockWindowRecord {
        handle: shelldll,
        class_name: "SHELLDLL_DefView".to_string(),
        window_title: String::new(),
        process_id: 1000,
        thread_id: 1002,
        parent: Some(progman),
        is_visible: true,
        bounds: WindowRect::new(0, 0, 1920, 1080),
        children: Vec::new(),
        style: constants::WS_CHILD | constants::WS_VISIBLE,
        ex_style: 0,
        dpi_context: Some(2),
    });

    provider.add_window(MockWindowRecord {
        handle: workerw,
        class_name: "WorkerW".to_string(),
        window_title: String::new(),
        process_id: 1000,
        thread_id: 1003,
        parent: None,
        is_visible: true,
        bounds: WindowRect::new(0, 0, 1920, 1080),
        children: Vec::new(),
        style: constants::WS_POPUP | constants::WS_VISIBLE,
        ex_style: 0,
        dpi_context: Some(2),
    });

    provider.add_window(MockWindowRecord {
        handle: player_hwnd,
        class_name: "KinetiWallPlayerSurface".to_string(),
        window_title: "KinetiWall Player".to_string(),
        process_id: owning_pid,
        thread_id: 1235,
        parent: None,
        is_visible: true,
        bounds: WindowRect::new(100, 100, 900, 700),
        children: Vec::new(),
        style: constants::WS_POPUP | constants::WS_VISIBLE,
        ex_style: 0,
        dpi_context: Some(2),
    });

    let instance_id = Uuid::new_v4();
    let candidate = DesktopCandidateWindow {
        handle: workerw,
        class_name: "WorkerW".to_string(),
        window_title: String::new(),
        process_id: 1000,
        thread_id: 1003,
        role: CandidateRole::WallpaperHostCandidate,
        parent: None,
        bounds: Some(WindowRect::new(0, 0, 1920, 1080)),
        is_visible: true,
        has_shelldll_defview_child: false,
        validated_at_millis: 1000,
    };

    let request = AttachmentRequest {
        source_handle: player_hwnd,
        target_candidate: candidate,
        owning_process_id: owning_pid,
        player_instance_id: instance_id,
        target_display_bounds: Some(WindowRect::new(0, 0, 1920, 1080)),
    };

    let record = coordinator
        .attach(&request)
        .expect("Attachment must succeed");
    assert_eq!(record.source_handle, player_hwnd);
    assert_eq!(record.target_handle, workerw);

    (provider, coordinator, player_hwnd, workerw, instance_id)
}

#[test]
fn test_sync_no_op_when_geometry_unchanged() {
    let (_provider, mut coordinator, player_hwnd, workerw, instance_id) = setup_test_environment();

    assert_eq!(
        coordinator.get_sync_state(player_hwnd),
        SurfaceSyncState::Synchronized
    );

    // Request synchronization to the exact same 1920x1080 bounds
    let initial_bounds = WindowRect::new(0, 0, 1920, 1080);
    let report = coordinator
        .synchronize_surface_geometry(player_hwnd, initial_bounds)
        .expect("Sync should succeed");

    // Must be coalesced: no native mutation applied
    assert!(!report.mutation_applied);
    assert_eq!(report.effective_bounds, initial_bounds);
    assert_eq!(report.source_handle, player_hwnd);
    assert_eq!(report.host_handle, workerw);
    assert_eq!(report.player_instance_id, instance_id);
    assert_eq!(report.state, SurfaceSyncState::Synchronized);
}

#[test]
fn test_sync_applies_single_mutation_on_changed_bounds() {
    let (provider, mut coordinator, player_hwnd, workerw, instance_id) = setup_test_environment();

    let new_resolution = WindowRect::new(0, 0, 2560, 1440);
    let report = coordinator
        .synchronize_surface_geometry(player_hwnd, new_resolution)
        .expect("Sync should succeed");

    assert!(report.mutation_applied);
    assert_eq!(report.effective_bounds, new_resolution);
    assert_eq!(report.sync_generation, 2);
    assert_eq!(report.source_handle, player_hwnd);
    assert_eq!(report.host_handle, workerw);
    assert_eq!(report.player_instance_id, instance_id);

    // Verify window bounds in the provider
    let provider_bounds = provider
        .get_window_rect(player_hwnd)
        .expect("Provider rect");
    assert_eq!(provider_bounds.width(), 2560);
    assert_eq!(provider_bounds.height(), 1440);
}

#[test]
fn test_sync_rejects_zero_or_negative_geometry() {
    let (_provider, mut coordinator, player_hwnd, _workerw, _instance_id) =
        setup_test_environment();

    // Zero width
    let invalid_rect = WindowRect::new(0, 0, 0, 1080);
    let err = coordinator
        .synchronize_surface_geometry(player_hwnd, invalid_rect)
        .expect_err("Zero dimension must be rejected");

    match err {
        SurfaceSyncError::InvalidGeometry { bounds, .. } => {
            assert_eq!(bounds, invalid_rect);
        }
        other => panic!("Expected InvalidGeometry error, got: {:?}", other),
    }

    assert_eq!(
        coordinator.get_sync_state(player_hwnd),
        SurfaceSyncState::SynchronizationFailed
    );
}

#[test]
fn test_sync_coalesces_rapid_resolution_events() {
    let (_provider, mut coordinator, player_hwnd, _workerw, _instance_id) =
        setup_test_environment();

    let target_resolution = WindowRect::new(0, 0, 3840, 2160);

    // First call: mutates
    let report1 = coordinator
        .synchronize_surface_geometry(player_hwnd, target_resolution)
        .expect("First sync should succeed");
    assert!(report1.mutation_applied);
    assert_eq!(report1.sync_generation, 2);

    // Subsequent calls with same bounds: coalesced no-ops
    for _ in 0..5 {
        let report_coalesced = coordinator
            .synchronize_surface_geometry(player_hwnd, target_resolution)
            .expect("Coalesced sync should succeed");
        assert!(!report_coalesced.mutation_applied);
        assert_eq!(report_coalesced.sync_generation, 2);
    }
}

#[test]
fn test_sync_reentrancy_anti_loop_guard() {
    let (_provider, mut coordinator, player_hwnd, _workerw, _instance_id) =
        setup_test_environment();

    // Manually simulate that a synchronization is already in progress
    coordinator.set_synchronizing_flag_for_testing(player_hwnd, true);

    let report = coordinator
        .synchronize_surface_geometry(player_hwnd, WindowRect::new(0, 0, 2560, 1440))
        .expect("Re-entrant sync should safely return without panicking or mutating");

    assert!(!report.mutation_applied);
}

#[test]
fn test_sync_display_change_event_dispatch() {
    let (_provider, mut coordinator, player_hwnd, _workerw, _instance_id) =
        setup_test_environment();

    // Dispatch DisplayResolutionChanged event
    let report = coordinator
        .handle_surface_event(
            player_hwnd,
            SurfaceChangeEvent::DisplayResolutionChanged {
                width: 1920,
                height: 1200,
                bits_per_pixel: 32,
            },
        )
        .expect("Resolution change event handling must succeed");

    assert!(report.mutation_applied);
    assert_eq!(report.effective_bounds, WindowRect::new(0, 0, 1920, 1200));

    // Dispatch DpiChanged event
    let report_dpi = coordinator
        .handle_surface_event(
            player_hwnd,
            SurfaceChangeEvent::DpiChanged {
                new_dpi: 144,
                suggested_rect: Some(WindowRect::new(0, 0, 2880, 1800)),
            },
        )
        .expect("DPI change event handling must succeed");

    assert!(report_dpi.mutation_applied);
    assert_eq!(
        report_dpi.effective_bounds,
        WindowRect::new(0, 0, 2880, 1800)
    );
}

#[test]
fn test_sync_host_invalidation_and_safe_recovery() {
    let (provider, mut coordinator, player_hwnd, workerw, _instance_id) = setup_test_environment();

    // Invalidate the current host by destroying it
    let _ = provider.destroy_window(workerw);

    // Create a new valid WorkerW host to be discovered by Step 39 discovery
    let new_workerw = NativeWindowHandle(40);
    provider.add_window(MockWindowRecord {
        handle: new_workerw,
        class_name: "WorkerW".to_string(),
        window_title: String::new(),
        process_id: 1000,
        thread_id: 1004,
        parent: None,
        is_visible: true,
        bounds: WindowRect::new(0, 0, 1920, 1080),
        children: Vec::new(),
        style: constants::WS_POPUP | constants::WS_VISIBLE,
        ex_style: 0,
        dpi_context: Some(2),
    });

    // Trigger host invalidation handling
    let report = coordinator
        .handle_surface_event(
            player_hwnd,
            SurfaceChangeEvent::HostWindowDestroyed(workerw),
        )
        .expect("Safe reattachment recovery should succeed");

    assert_eq!(report.host_handle, new_workerw);
    assert_eq!(report.state, SurfaceSyncState::Synchronized);
    assert!(report.mutation_applied);

    // Verify parent of player_hwnd is now new_workerw
    assert_eq!(provider.get_parent(player_hwnd), Some(new_workerw));
}

#[test]
fn test_sync_recovery_budget_exhaustion() {
    let (provider, mut coordinator, player_hwnd, workerw, _instance_id) = setup_test_environment();

    // Invalidate host and ensure no valid candidate exists
    let _ = provider.destroy_window(workerw);

    // Attempts 1, 2, and 3 fail with HostLostAndRecoveryFailed
    for _ in 0..3 {
        let err = coordinator
            .handle_host_invalidation(player_hwnd)
            .expect_err("Attempt must fail when no candidate is available");
        assert!(matches!(
            err,
            SurfaceSyncError::HostLostAndRecoveryFailed(_)
        ));
    }

    // 4th attempt exceeds max budget (3) and returns RecoveryBudgetExhausted
    let err = coordinator
        .handle_host_invalidation(player_hwnd)
        .expect_err("Must fail when budget is exhausted");

    match err {
        SurfaceSyncError::RecoveryBudgetExhausted { attempts, max } => {
            assert_eq!(attempts, 3);
            assert_eq!(max, 3);
        }
        other => panic!("Expected RecoveryBudgetExhausted error, got: {:?}", other),
    }

    assert_eq!(
        coordinator.get_sync_state(player_hwnd),
        SurfaceSyncState::SynchronizationFailed
    );
}

#[test]
fn test_sync_shutdown_cancels_and_rejects_new_sync() {
    let (_provider, mut coordinator, player_hwnd, _workerw, _instance_id) =
        setup_test_environment();

    let detach_results = coordinator.detach_all_shutdown();
    assert_eq!(detach_results.len(), 1);

    // Now attempt to synchronize
    let err = coordinator
        .synchronize_surface_geometry(player_hwnd, WindowRect::new(0, 0, 1920, 1080))
        .expect_err("Sync must fail when coordinator is shut down");

    assert_eq!(err, SurfaceSyncError::CoordinatorShutDown);
}

#[test]
fn test_sync_event_history_bounded() {
    let (_provider, mut coordinator, player_hwnd, workerw, instance_id) = setup_test_environment();

    for i in 0..150 {
        coordinator.record_sync_diagnostic(
            player_hwnd,
            workerw,
            instance_id,
            i as u64,
            format!("Diagnostic event {}", i),
            None,
            None,
            SurfaceSyncState::Synchronized,
        );
    }

    let events = coordinator.get_sync_diagnostic_events();
    assert_eq!(events.len(), MAX_SYNC_DIAGNOSTIC_EVENTS);
    assert_eq!(events.last().unwrap().generation, 149);
}
