//! Unit tests for multi-monitor display enumeration, stable display identity,
//! and display topology modeling (Step 42).

use std::sync::Arc;

use kinetiwall_desktop::{
    DisplayDiscoveryError, DisplayGeometry, DisplayId, DisplaySnapshotManager, DisplayTopologyType,
    MockDesktopDisplayProvider, MockDisplayRecord, NativeMonitorHandle, WindowRect,
};

#[test]
fn test_display_single_monitor_discovery_and_geometry() {
    let mock = Arc::new(MockDesktopDisplayProvider::new());
    mock.setup_single_display(1920, 1080);

    let mut manager = DisplaySnapshotManager::new(mock);
    let (snapshot, delta) = manager.refresh().expect("Refresh must succeed");

    assert_eq!(snapshot.generation, 1);
    assert!(snapshot.is_valid);
    assert_eq!(snapshot.displays.len(), 1);
    assert_eq!(snapshot.topology_type, DisplayTopologyType::SingleDisplay);
    assert_eq!(
        snapshot.virtual_desktop_bounds,
        WindowRect::new(0, 0, 1920, 1080)
    );

    let primary = snapshot.get_primary().expect("Primary display must exist");
    assert!(primary.is_primary);
    assert_eq!(primary.geometry.width, 1920);
    assert_eq!(primary.geometry.height, 1080);
    assert_eq!(
        primary.geometry.work_area_rect,
        WindowRect::new(0, 0, 1920, 1040)
    );

    assert_eq!(delta.current_generation, 1);
    assert_eq!(delta.displays_added.len(), 1);
    assert!(delta.is_meaningful_change);
}

#[test]
fn test_display_dual_extended_geometry_and_negative_coordinates() {
    let mock = Arc::new(MockDesktopDisplayProvider::new());

    // Monitor 1 (Primary): 0, 0, 1920, 1080
    // Monitor 2 (Left of Primary): -1920, 0, 0, 1080
    let mon1 = WindowRect::new(0, 0, 1920, 1080);
    let mon2 = WindowRect::new(-1920, 0, 0, 1080);
    mock.setup_dual_extended(mon1, mon2);

    let mut manager = DisplaySnapshotManager::new(mock);
    let (snapshot, _) = manager.refresh().expect("Refresh must succeed");

    assert_eq!(snapshot.displays.len(), 2);
    assert_eq!(snapshot.topology_type, DisplayTopologyType::ExtendedDesktop);

    // Negative coordinate must be preserved in virtual desktop bounds: [-1920..1920] x [0..1080]
    assert_eq!(
        snapshot.virtual_desktop_bounds,
        WindowRect::new(-1920, 0, 1920, 1080)
    );
    assert_eq!(snapshot.virtual_desktop_bounds.width(), 3840);
    assert_eq!(snapshot.virtual_desktop_bounds.height(), 1080);

    let secondary = snapshot
        .displays
        .iter()
        .find(|d| !d.is_primary)
        .expect("Secondary display exists");
    assert_eq!(secondary.geometry.monitor_rect.left, -1920);
    assert_eq!(secondary.geometry.monitor_rect.right, 0);
    assert_eq!(secondary.geometry.width, 1920);
}

#[test]
fn test_display_cloned_mirrored_topology_classification() {
    let mock = Arc::new(MockDesktopDisplayProvider::new());
    mock.setup_cloned_displays(1920, 1080);

    let mut manager = DisplaySnapshotManager::new(mock);
    let (snapshot, _) = manager.refresh().expect("Refresh must succeed");

    assert_eq!(snapshot.displays.len(), 2);
    assert_eq!(snapshot.topology_type, DisplayTopologyType::CloneOrMirrored);
    assert_eq!(
        snapshot.virtual_desktop_bounds,
        WindowRect::new(0, 0, 1920, 1080)
    );
}

#[test]
fn test_display_stable_identity_preserved_across_refresh() {
    let mock = Arc::new(MockDesktopDisplayProvider::new());
    mock.setup_single_display(2560, 1440);

    let mut manager = DisplaySnapshotManager::new(mock.clone());

    // First refresh
    let (snap1, _) = manager.refresh().expect("First refresh");
    let id1 = snap1.displays[0].id.clone();
    assert_eq!(snap1.generation, 1);

    // Second refresh (geometry or state may re-evaluate, but hardware/device remains identical)
    let (snap2, delta) = manager.refresh().expect("Second refresh");
    let id2 = snap2.displays[0].id.clone();
    assert_eq!(snap2.generation, 2);

    // Stable identity is strictly identical!
    assert_eq!(id1, id2);
    assert!(!delta.is_meaningful_change);
}

#[test]
fn test_display_change_delta_detection() {
    let mock = Arc::new(MockDesktopDisplayProvider::new());
    mock.setup_single_display(1920, 1080);

    let mut manager = DisplaySnapshotManager::new(mock.clone());
    let (snap1, _) = manager.refresh().expect("First refresh");

    // Add a second display
    mock.add_display(MockDisplayRecord {
        handle: NativeMonitorHandle(0x3000),
        device_name: "\\\\.\\DISPLAY2".to_string(),
        friendly_name: Some("Second Screen".to_string()),
        hardware_id: Some("MONITOR\\SEC003\\3".to_string()),
        monitor_rect: WindowRect::new(1920, 0, 3840, 1080),
        work_area_rect: WindowRect::new(1920, 0, 3840, 1040),
        is_primary: false,
        is_attached: true,
        is_mirroring: false,
    });

    let (snap2, delta) = manager.refresh().expect("Second refresh");

    assert_eq!(snap1.displays.len(), 1);
    assert_eq!(snap2.displays.len(), 2);
    assert_eq!(delta.displays_added.len(), 1);
    assert_eq!(delta.displays_removed.len(), 0);
    assert!(delta.geometry_changed);
    assert!(delta.topology_changed);
    assert!(delta.is_meaningful_change);
}

#[test]
fn test_display_heterogeneous_resolutions_and_arrangements() {
    let mock = Arc::new(MockDesktopDisplayProvider::new());

    // Monitor 1: 4K (3840x2160) at (0, 0)
    mock.add_display(MockDisplayRecord {
        handle: NativeMonitorHandle(0x1000),
        device_name: "\\\\.\\DISPLAY1".to_string(),
        friendly_name: Some("4K Monitor".to_string()),
        hardware_id: Some("MONITOR\\4K001\\1".to_string()),
        monitor_rect: WindowRect::new(0, 0, 3840, 2160),
        work_area_rect: WindowRect::new(0, 0, 3840, 2100),
        is_primary: true,
        is_attached: true,
        is_mirroring: false,
    });

    // Monitor 2: 1440p (2560x1440) placed above primary at (-200, -1440, 2360, 0)
    mock.add_display(MockDisplayRecord {
        handle: NativeMonitorHandle(0x2000),
        device_name: "\\\\.\\DISPLAY2".to_string(),
        friendly_name: Some("1440p Monitor".to_string()),
        hardware_id: Some("MONITOR\\2K002\\2".to_string()),
        monitor_rect: WindowRect::new(-200, -1440, 2360, 0),
        work_area_rect: WindowRect::new(-200, -1440, 2360, -40),
        is_primary: false,
        is_attached: true,
        is_mirroring: false,
    });

    let mut manager = DisplaySnapshotManager::new(mock);
    let (snapshot, _) = manager.refresh().expect("Refresh must succeed");

    assert_eq!(snapshot.displays.len(), 2);
    assert_eq!(snapshot.topology_type, DisplayTopologyType::ExtendedDesktop);

    // Virtual desktop bounds should be union: left=-200, top=-1440, right=3840, bottom=2160
    assert_eq!(
        snapshot.virtual_desktop_bounds,
        WindowRect::new(-200, -1440, 3840, 2160)
    );
    assert_eq!(snapshot.virtual_desktop_bounds.width(), 4040);
    assert_eq!(snapshot.virtual_desktop_bounds.height(), 3600);
}

#[test]
fn test_display_invalid_zero_geometry_rejected() {
    let mock = Arc::new(MockDesktopDisplayProvider::new());

    // Degenerate zero-width monitor
    mock.add_display(MockDisplayRecord {
        handle: NativeMonitorHandle(0x1000),
        device_name: "\\\\.\\DISPLAY1".to_string(),
        friendly_name: None,
        hardware_id: None,
        monitor_rect: WindowRect::new(0, 0, 0, 1080),
        work_area_rect: WindowRect::new(0, 0, 0, 1080),
        is_primary: true,
        is_attached: true,
        is_mirroring: false,
    });

    let mut manager = DisplaySnapshotManager::new(mock);
    let err = manager.refresh().expect_err("Must reject zero width");

    match err {
        DisplayDiscoveryError::InvalidGeometry { bounds, .. } => {
            assert_eq!(bounds.width(), 0);
        }
        other => panic!("Expected InvalidGeometry error, got: {:?}", other),
    }
}

#[test]
fn test_display_empty_inventory_rejected() {
    let mock = Arc::new(MockDesktopDisplayProvider::new());
    // No displays added

    let mut manager = DisplaySnapshotManager::new(mock);
    let err = manager.refresh().expect_err("Must reject empty inventory");

    assert_eq!(err, DisplayDiscoveryError::NoActiveDisplays);
}

#[test]
fn test_display_lookup_by_id_and_handle() {
    let mock = Arc::new(MockDesktopDisplayProvider::new());
    mock.setup_single_display(1920, 1080);

    let mut manager = DisplaySnapshotManager::new(mock);
    let (snapshot, _) = manager.refresh().expect("Refresh");

    let handle = NativeMonitorHandle(0x1000);
    let by_handle = snapshot
        .find_by_handle(handle)
        .expect("Must find by handle");
    assert_eq!(by_handle.device_name, "\\\\.\\DISPLAY1");

    let by_id = snapshot
        .find_by_id(&by_handle.id)
        .expect("Must find by display ID");
    assert_eq!(by_id.native_handle, handle);

    assert!(snapshot
        .find_by_handle(NativeMonitorHandle(0x9999))
        .is_none());
    assert!(snapshot
        .find_by_id(&DisplayId::new("non_existent"))
        .is_none());
}

#[test]
fn test_display_geometry_validation_logic() {
    let valid_geom = DisplayGeometry {
        monitor_rect: WindowRect::new(0, 0, 1920, 1080),
        work_area_rect: WindowRect::new(0, 0, 1920, 1040),
        width: 1920,
        height: 1080,
        scale_factor_percent: Some(100),
    };
    assert!(valid_geom.is_valid());

    // Work area leaking outside monitor bounds
    let invalid_leak = DisplayGeometry {
        monitor_rect: WindowRect::new(0, 0, 1920, 1080),
        work_area_rect: WindowRect::new(0, 0, 1920, 1200),
        width: 1920,
        height: 1080,
        scale_factor_percent: None,
    };
    assert!(!invalid_leak.is_valid());
}
