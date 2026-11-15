//! Comprehensive unit tests for wallpaper window attachment, window styles, and transactional rollback.

use std::sync::Arc;
use uuid::Uuid;

use kinetiwall_desktop::constants;
use kinetiwall_desktop::{
    AttachmentCoordinator, AttachmentError, AttachmentRequest, AttachmentState, CandidateRole,
    DesktopCandidateWindow, DesktopWindowProvider, MockDesktopWindowProvider, MockWindowRecord,
    NativeWindowHandle, RollbackOutcome, WindowRect,
};

fn create_mock_candidate(handle: usize, role: CandidateRole) -> DesktopCandidateWindow {
    DesktopCandidateWindow {
        handle: NativeWindowHandle(handle),
        class_name: "WorkerW".to_string(),
        window_title: String::new(),
        process_id: 2000,
        thread_id: 2001,
        role,
        parent: None,
        bounds: Some(WindowRect::new(0, 0, 1920, 1080)),
        is_visible: true,
        has_shelldll_defview_child: false,
        validated_at_millis: 1000,
    }
}

fn create_source_window(
    mock: &Arc<MockDesktopWindowProvider>,
    handle: usize,
    pid: u32,
    style: isize,
    ex_style: isize,
) -> NativeWindowHandle {
    let h = NativeWindowHandle(handle);
    mock.add_window(MockWindowRecord {
        handle: h,
        class_name: "KinetiWallPlayerTest".to_string(),
        window_title: "KinetiWall Isolated Surface".to_string(),
        process_id: pid,
        thread_id: 500,
        parent: None,
        is_visible: true,
        bounds: WindowRect::new(100, 100, 900, 700),
        children: Vec::new(),
        style,
        ex_style,
        dpi_context: Some(2),
    });
    h
}

fn create_target_window(
    mock: &Arc<MockDesktopWindowProvider>,
    handle: usize,
    pid: u32,
) -> DesktopCandidateWindow {
    let h = NativeWindowHandle(handle);
    mock.add_window(MockWindowRecord {
        handle: h,
        class_name: "WorkerW".to_string(),
        window_title: String::new(),
        process_id: pid,
        thread_id: 600,
        parent: None,
        is_visible: true,
        bounds: WindowRect::new(0, 0, 1920, 1080),
        children: Vec::new(),
        style: constants::WS_VISIBLE | constants::WS_CLIPCHILDREN,
        ex_style: 0,
        dpi_context: Some(2),
    });
    create_mock_candidate(handle, CandidateRole::WallpaperHostCandidate)
}

#[test]
fn test_successful_attachment_transaction_and_verification() {
    let mock = Arc::new(MockDesktopWindowProvider::new());
    let mut coordinator = AttachmentCoordinator::new(mock.clone());

    let source = create_source_window(
        &mock,
        0x1000,
        5000,
        constants::WS_POPUP | constants::WS_VISIBLE,
        0,
    );
    let target_candidate = create_target_window(&mock, 0x2000, 2000);
    let instance_id = Uuid::new_v4();

    let request = AttachmentRequest {
        source_handle: source,
        player_instance_id: instance_id,
        owning_process_id: 5000,
        target_candidate,
        target_display_bounds: Some(WindowRect::new(0, 0, 1920, 1080)),
    };

    let result = coordinator
        .attach(&request)
        .expect("Attachment must succeed");

    assert_eq!(result.source_handle, source);
    assert_eq!(result.target_handle, NativeWindowHandle(0x2000));
    assert!(result.is_verified);
    assert_eq!(coordinator.get_state(source), AttachmentState::Attached);

    // Verify parentage in provider
    assert_eq!(mock.get_parent(source), Some(NativeWindowHandle(0x2000)));

    // Verify styles in provider: WS_CHILD must be set, WS_POPUP stripped, WS_EX_TOOLWINDOW and WS_EX_NOACTIVATE set
    let style = mock.get_window_long(source, constants::GWL_STYLE).unwrap();
    assert_ne!(style & constants::WS_CHILD, 0, "WS_CHILD must be set");
    assert_eq!(style & constants::WS_POPUP, 0, "WS_POPUP must be cleared");

    let ex_style = mock
        .get_window_long(source, constants::GWL_EXSTYLE)
        .unwrap();
    assert_ne!(
        ex_style & constants::WS_EX_TOOLWINDOW,
        0,
        "WS_EX_TOOLWINDOW must be set"
    );
    assert_ne!(
        ex_style & constants::WS_EX_NOACTIVATE,
        0,
        "WS_EX_NOACTIVATE must be set"
    );

    // Verify bounds applied
    let bounds = mock.get_window_rect(source).unwrap();
    assert_eq!(bounds, WindowRect::new(0, 0, 1920, 1080));
}

#[test]
fn test_rejection_of_prohibited_management_ui_handle() {
    let mock = Arc::new(MockDesktopWindowProvider::new());
    let mut coordinator = AttachmentCoordinator::new(mock.clone());

    let mgmt_ui = create_source_window(&mock, 0x9999, 5000, constants::WS_VISIBLE, 0);
    coordinator.register_prohibited_handle(mgmt_ui);

    let target = create_target_window(&mock, 0x2000, 2000);
    let request = AttachmentRequest {
        source_handle: mgmt_ui,
        player_instance_id: Uuid::new_v4(),
        owning_process_id: 5000,
        target_candidate: target,
        target_display_bounds: None,
    };

    let err = coordinator.attach(&request).unwrap_err();
    assert_eq!(err, AttachmentError::SourceIsManagementUi(mgmt_ui));
    assert_eq!(coordinator.get_state(mgmt_ui), AttachmentState::Failed);
}

#[test]
fn test_rejection_of_desktop_root_and_icon_carrier_as_source() {
    let mock = Arc::new(MockDesktopWindowProvider::new());
    let mut coordinator = AttachmentCoordinator::new(mock.clone());

    // 1. Progman as source
    let progman = NativeWindowHandle(0x0001);
    mock.add_window(MockWindowRecord {
        handle: progman,
        class_name: "Progman".to_string(),
        window_title: "Program Manager".to_string(),
        process_id: 1000,
        thread_id: 1001,
        parent: None,
        is_visible: true,
        bounds: WindowRect::new(0, 0, 1920, 1080),
        children: Vec::new(),
        style: 0,
        ex_style: 0,
        dpi_context: None,
    });

    let target = create_target_window(&mock, 0x2000, 2000);
    let req1 = AttachmentRequest {
        source_handle: progman,
        player_instance_id: Uuid::new_v4(),
        owning_process_id: 1000,
        target_candidate: target.clone(),
        target_display_bounds: None,
    };
    assert_eq!(
        coordinator.attach(&req1).unwrap_err(),
        AttachmentError::SourceIsDesktopRoot(progman)
    );

    // 2. SHELLDLL_DefView as source
    let defview = NativeWindowHandle(0x0002);
    mock.add_window(MockWindowRecord {
        handle: defview,
        class_name: "SHELLDLL_DefView".to_string(),
        window_title: String::new(),
        process_id: 1000,
        thread_id: 1001,
        parent: None,
        is_visible: true,
        bounds: WindowRect::new(0, 0, 1920, 1080),
        children: Vec::new(),
        style: 0,
        ex_style: 0,
        dpi_context: None,
    });
    let req2 = AttachmentRequest {
        source_handle: defview,
        player_instance_id: Uuid::new_v4(),
        owning_process_id: 1000,
        target_candidate: target,
        target_display_bounds: None,
    };
    assert_eq!(
        coordinator.attach(&req2).unwrap_err(),
        AttachmentError::SourceIsIconHost(defview)
    );
}

#[test]
fn test_rejection_of_unrelated_process_ownership() {
    let mock = Arc::new(MockDesktopWindowProvider::new());
    let mut coordinator = AttachmentCoordinator::new(mock.clone());

    // Window owned by PID 7777, but request claims 5000
    let source = create_source_window(
        &mock,
        0x1000,
        7777,
        constants::WS_POPUP | constants::WS_VISIBLE,
        0,
    );
    let target = create_target_window(&mock, 0x2000, 2000);

    let request = AttachmentRequest {
        source_handle: source,
        player_instance_id: Uuid::new_v4(),
        owning_process_id: 5000,
        target_candidate: target,
        target_display_bounds: None,
    };

    let err = coordinator.attach(&request).unwrap_err();
    match err {
        AttachmentError::SourceProcessMismatch { expected, actual } => {
            assert_eq!(expected, 5000);
            assert_eq!(actual, 7777);
        }
        other => panic!("Expected SourceProcessMismatch, got {:?}", other),
    }
}

#[test]
fn test_rejection_of_ineligible_target_carrier_window() {
    let mock = Arc::new(MockDesktopWindowProvider::new());
    let mut coordinator = AttachmentCoordinator::new(mock.clone());

    let source = create_source_window(
        &mock,
        0x1000,
        5000,
        constants::WS_POPUP | constants::WS_VISIBLE,
        0,
    );

    // Target marked as IconHostCarrier
    let mut bad_target = create_target_window(&mock, 0x2000, 2000);
    bad_target.role = CandidateRole::IconHostCarrier;

    let request = AttachmentRequest {
        source_handle: source,
        player_instance_id: Uuid::new_v4(),
        owning_process_id: 5000,
        target_candidate: bad_target,
        target_display_bounds: None,
    };

    let err = coordinator.attach(&request).unwrap_err();
    match err {
        AttachmentError::TargetIncompatible(msg) => {
            assert!(msg.contains("not a valid WallpaperHostCandidate"));
        }
        other => panic!("Expected TargetIncompatible, got {:?}", other),
    }
}

#[test]
fn test_rejection_of_dpi_incompatible_windows() {
    let mock = Arc::new(MockDesktopWindowProvider::new());
    let mut coordinator = AttachmentCoordinator::new(mock.clone());

    let source = create_source_window(
        &mock,
        0x1000,
        5000,
        constants::WS_POPUP | constants::WS_VISIBLE,
        0,
    );
    let mut target = create_target_window(&mock, 0x2000, 2000);

    // Configure target with DPI awareness context 1 vs source 2
    mock.add_window(MockWindowRecord {
        handle: NativeWindowHandle(0x2000),
        class_name: "WorkerW".to_string(),
        window_title: String::new(),
        process_id: 2000,
        thread_id: 600,
        parent: None,
        is_visible: true,
        bounds: WindowRect::new(0, 0, 1920, 1080),
        children: Vec::new(),
        style: constants::WS_VISIBLE,
        ex_style: 0,
        dpi_context: Some(1), // Mismatch!
    });
    target.handle = NativeWindowHandle(0x2000);

    let request = AttachmentRequest {
        source_handle: source,
        player_instance_id: Uuid::new_v4(),
        owning_process_id: 5000,
        target_candidate: target,
        target_display_bounds: None,
    };

    let err = coordinator.attach(&request).unwrap_err();
    match err {
        AttachmentError::DpiIncompatible {
            source_ctx,
            target_ctx,
        } => {
            assert_eq!(source_ctx, Some(2));
            assert_eq!(target_ctx, Some(1));
        }
        other => panic!("Expected DpiIncompatible, got {:?}", other),
    }
}

#[test]
fn test_rollback_on_set_parent_failure() {
    let mock = Arc::new(MockDesktopWindowProvider::new());
    let mut coordinator = AttachmentCoordinator::new(mock.clone());

    let orig_style = constants::WS_POPUP | constants::WS_VISIBLE;
    let source = create_source_window(&mock, 0x1000, 5000, orig_style, 0);
    let target = create_target_window(&mock, 0x2000, 2000);

    // Configure mock to fail during SetParent
    mock.set_fail_set_parent(true);

    let request = AttachmentRequest {
        source_handle: source,
        player_instance_id: Uuid::new_v4(),
        owning_process_id: 5000,
        target_candidate: target,
        target_display_bounds: None,
    };

    let err = coordinator.attach(&request).unwrap_err();
    match err {
        AttachmentError::RollbackTriggered { cause, outcome } => {
            match *cause {
                AttachmentError::ParentMutationFailed(_) => {}
                other => panic!("Expected ParentMutationFailed cause, got {:?}", other),
            }
            assert_eq!(
                outcome,
                RollbackOutcome::RollbackCompleted { verified: true }
            );
        }
        other => panic!("Expected RollbackTriggered, got {:?}", other),
    }

    assert_eq!(coordinator.get_state(source), AttachmentState::Failed);

    // Assert that original styles were restored by rollback
    let current_style = mock.get_window_long(source, constants::GWL_STYLE).unwrap();
    assert_eq!(
        current_style, orig_style,
        "Original style must be restored by rollback"
    );
    assert_eq!(mock.get_parent(source), None, "Parent must remain None");
}

#[test]
fn test_rollback_on_bounds_positioning_failure() {
    let mock = Arc::new(MockDesktopWindowProvider::new());
    let mut coordinator = AttachmentCoordinator::new(mock.clone());

    let orig_style = constants::WS_POPUP | constants::WS_VISIBLE;
    let source = create_source_window(&mock, 0x1000, 5000, orig_style, 0);
    let target = create_target_window(&mock, 0x2000, 2000);

    // SetWindowPos will fail during stage 4 (sizing)
    mock.set_fail_set_window_pos(true);

    let request = AttachmentRequest {
        source_handle: source,
        player_instance_id: Uuid::new_v4(),
        owning_process_id: 5000,
        target_candidate: target,
        target_display_bounds: None,
    };

    let err = coordinator.attach(&request).unwrap_err();
    match err {
        AttachmentError::RollbackTriggered { cause, outcome } => {
            match *cause {
                AttachmentError::BoundsMutationFailed(_) => {}
                other => panic!("Expected BoundsMutationFailed, got {:?}", other),
            }
            assert_eq!(
                outcome,
                RollbackOutcome::RollbackCompleted { verified: true }
            );
        }
        other => panic!("Expected RollbackTriggered, got {:?}", other),
    }

    // Provider state must be restored
    assert_eq!(
        mock.get_parent(source),
        None,
        "Parent must be rolled back to None"
    );
    assert_eq!(
        mock.get_window_long(source, constants::GWL_STYLE).unwrap(),
        orig_style
    );
}

#[test]
fn test_controlled_detachment_and_restoration_lifecycle() {
    let mock = Arc::new(MockDesktopWindowProvider::new());
    let mut coordinator = AttachmentCoordinator::new(mock.clone());

    let orig_style = constants::WS_POPUP | constants::WS_VISIBLE;
    let orig_bounds = WindowRect::new(100, 100, 900, 700);
    let source = create_source_window(&mock, 0x1000, 5000, orig_style, 0);
    let target = create_target_window(&mock, 0x2000, 2000);

    let request = AttachmentRequest {
        source_handle: source,
        player_instance_id: Uuid::new_v4(),
        owning_process_id: 5000,
        target_candidate: target,
        target_display_bounds: Some(WindowRect::new(0, 0, 1920, 1080)),
    };

    // Attach
    let _attach_rec = coordinator.attach(&request).expect("Attach must succeed");
    assert_eq!(coordinator.get_state(source), AttachmentState::Attached);

    // Revalidation should be healthy
    assert!(coordinator.revalidate_attached(source));

    // Detach
    let detach_rec = coordinator.detach(source).expect("Detach must succeed");
    assert_eq!(detach_rec.source_handle, source);
    assert!(detach_rec.is_verified);
    assert_eq!(detach_rec.restored_parent, None);
    assert_eq!(detach_rec.restored_style, orig_style);
    assert_eq!(detach_rec.restored_bounds, orig_bounds);

    assert_eq!(coordinator.get_state(source), AttachmentState::Detached);

    // In provider, parent is restored to None, style is restored to orig_style
    assert_eq!(mock.get_parent(source), None);
    assert_eq!(
        mock.get_window_long(source, constants::GWL_STYLE).unwrap(),
        orig_style
    );

    // Idempotency: Detaching again when already detached must succeed cleanly with no-op
    let second_detach = coordinator
        .detach(source)
        .expect("Second detach must succeed as no-op");
    assert!(second_detach.is_verified);
}

#[test]
fn test_shutdown_detaches_all_active_windows() {
    let mock = Arc::new(MockDesktopWindowProvider::new());
    let mut coordinator = AttachmentCoordinator::new(mock.clone());

    let s1 = create_source_window(
        &mock,
        0x1001,
        5000,
        constants::WS_POPUP | constants::WS_VISIBLE,
        0,
    );
    let s2 = create_source_window(
        &mock,
        0x1002,
        5000,
        constants::WS_POPUP | constants::WS_VISIBLE,
        0,
    );
    let target = create_target_window(&mock, 0x2000, 2000);

    coordinator
        .attach(&AttachmentRequest {
            source_handle: s1,
            player_instance_id: Uuid::new_v4(),
            owning_process_id: 5000,
            target_candidate: target.clone(),
            target_display_bounds: None,
        })
        .unwrap();

    coordinator
        .attach(&AttachmentRequest {
            source_handle: s2,
            player_instance_id: Uuid::new_v4(),
            owning_process_id: 5000,
            target_candidate: target,
            target_display_bounds: None,
        })
        .unwrap();

    assert_eq!(coordinator.get_state(s1), AttachmentState::Attached);
    assert_eq!(coordinator.get_state(s2), AttachmentState::Attached);

    let shutdown_reports = coordinator.detach_all_shutdown();
    assert_eq!(shutdown_reports.len(), 2);
    for rep in shutdown_reports {
        assert!(rep.is_ok());
    }

    assert_eq!(coordinator.get_state(s1), AttachmentState::Detached);
    assert_eq!(coordinator.get_state(s2), AttachmentState::Detached);

    // New requests after shutdown are rejected
    let s3 = create_source_window(
        &mock,
        0x1003,
        5000,
        constants::WS_POPUP | constants::WS_VISIBLE,
        0,
    );
    let target3 = create_target_window(&mock, 0x2000, 2000);
    let err = coordinator
        .attach(&AttachmentRequest {
            source_handle: s3,
            player_instance_id: Uuid::new_v4(),
            owning_process_id: 5000,
            target_candidate: target3,
            target_display_bounds: None,
        })
        .unwrap_err();
    assert_eq!(err, AttachmentError::CoordinatorShutDown);
}
