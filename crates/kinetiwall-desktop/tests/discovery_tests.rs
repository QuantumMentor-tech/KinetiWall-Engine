//! Unit tests and synthetic topology classification tests for `kinetiwall-desktop`.

use std::sync::Arc;

use kinetiwall_desktop::{
    CandidateRole, CompatibilityPolicy, CompatibilityRejectionReason, DesktopDiscoveryOptions,
    DesktopHostDiscovery, DesktopShellTopology, DesktopWindowProvider, DiscoveryError,
    MockDesktopWindowProvider, MockWindowRecord, NativeWindowHandle, WindowRect,
};

fn create_window(
    handle: usize,
    class_name: &str,
    title: &str,
    pid: u32,
    parent: Option<usize>,
    bounds: WindowRect,
) -> MockWindowRecord {
    MockWindowRecord {
        handle: NativeWindowHandle(handle),
        class_name: class_name.to_string(),
        window_title: title.to_string(),
        process_id: pid,
        thread_id: 100,
        parent: parent.map(NativeWindowHandle),
        is_visible: true,
        bounds,
        children: Vec::new(),
        style: 0,
        ex_style: 0,
        dpi_context: None,
    }
}

/// Tests the classic unsplit desktop shell where `Progman` owns `SHELLDLL_DefView`,
/// and no background `WorkerW` exists. Passive discovery must NOT send active probe messages,
/// must return `ClassicUnsplit`, and must NOT recommend any candidate for wallpaper attachment.
#[test]
fn test_topology_classic_unsplit_passive_discovery() {
    let mock = Arc::new(MockDesktopWindowProvider::new());

    // 1. Progman
    mock.add_window(create_window(
        0x1000,
        "Progman",
        "Program Manager",
        4000,
        None,
        WindowRect::new(0, 0, 1920, 1080),
    ));
    // 2. SHELLDLL_DefView under Progman
    mock.add_window(create_window(
        0x1004,
        "SHELLDLL_DefView",
        "",
        4000,
        Some(0x1000),
        WindowRect::new(0, 0, 1920, 1080),
    ));
    // 3. SysListView32 under SHELLDLL_DefView
    mock.add_window(create_window(
        0x1008,
        "SysListView32",
        "FolderView",
        4000,
        Some(0x1004),
        WindowRect::new(0, 0, 1920, 1080),
    ));

    let discovery = DesktopHostDiscovery::new(mock);
    let options = DesktopDiscoveryOptions {
        allow_active_probe: false,
        ..Default::default()
    };

    let result = discovery
        .discover_topology(&options)
        .expect("discovery should succeed");
    assert_eq!(result.topology, DesktopShellTopology::ClassicUnsplit);
    assert!(result.recommended_candidate.is_none());
    assert!(!result.is_safe_for_attachment_review);
    assert!(result.icon_view.is_some());
    assert_eq!(
        result.icon_view.as_ref().unwrap().handle,
        NativeWindowHandle(0x1004)
    );

    // Compatibility policy must reject attachment
    let suit = CompatibilityPolicy::evaluate_attachment_suitability(&result);
    assert!(matches!(
        suit,
        Err(CompatibilityRejectionReason::TopologyNotSupported(
            DesktopShellTopology::ClassicUnsplit,
            _
        ))
    ));
}

/// Tests that when active probing is explicitly permitted, discovery sends the bounded 0x052C
/// probe to Progman, and upon WorkerW creation, correctly detects the newly split topology.
#[test]
fn test_topology_active_probe_split_transition() {
    let mock = Arc::new(MockDesktopWindowProvider::new());

    mock.add_window(create_window(
        0x1000,
        "Progman",
        "Program Manager",
        4000,
        None,
        WindowRect::new(0, 0, 1920, 1080),
    ));
    mock.add_window(create_window(
        0x1004,
        "SHELLDLL_DefView",
        "",
        4000,
        Some(0x1000),
        WindowRect::new(0, 0, 1920, 1080),
    ));

    // Configure probe handler: when 0x052C is received by Progman, spawn WorkerW (0x2000)
    let mock_clone = Arc::clone(&mock);
    mock.set_probe_handler(move |target, msg| {
        if target == NativeWindowHandle(0x1000) && msg == 0x052C {
            // Spawn WorkerW background window
            mock_clone.add_window(create_window(
                0x2000,
                "WorkerW",
                "",
                4000,
                None,
                WindowRect::new(0, 0, 1920, 1080),
            ));
            Ok(0)
        } else {
            Err(DiscoveryError::InvalidHandle(target))
        }
    });

    let discovery = DesktopHostDiscovery::new(mock);
    let options = DesktopDiscoveryOptions {
        allow_active_probe: true,
        probe_timeout_ms: 500,
        ..Default::default()
    };

    let result = discovery
        .discover_topology(&options)
        .expect("discovery should succeed");
    assert_eq!(result.topology, DesktopShellTopology::StandardWorkerWSplit);
    assert!(result.is_safe_for_attachment_review);
    assert!(result.recommended_candidate.is_some());

    let candidate = result.recommended_candidate.as_ref().unwrap();
    assert_eq!(candidate.handle, NativeWindowHandle(0x2000));
    assert_eq!(candidate.class_name, "WorkerW");
    assert_eq!(candidate.role, CandidateRole::WallpaperHostCandidate);

    // Compatibility policy must approve attachment suitability
    let approved =
        CompatibilityPolicy::evaluate_attachment_suitability(&result).expect("must be approved");
    assert_eq!(approved.handle, NativeWindowHandle(0x2000));
}

/// Tests standard split WorkerW desktop topology where SHELLDLL_DefView resides inside
/// an icon carrier WorkerW (0x2000), and a secondary WorkerW (0x2010) sits directly behind it.
#[test]
fn test_topology_standard_workerw_split() {
    let mock = Arc::new(MockDesktopWindowProvider::new());

    // 1. Icon Carrier WorkerW
    mock.add_window(create_window(
        0x2000,
        "WorkerW",
        "",
        4000,
        None,
        WindowRect::new(0, 0, 1920, 1080),
    ));
    // 2. SHELLDLL_DefView inside Icon Carrier WorkerW
    mock.add_window(create_window(
        0x2004,
        "SHELLDLL_DefView",
        "",
        4000,
        Some(0x2000),
        WindowRect::new(0, 0, 1920, 1080),
    ));
    // 3. SysListView32 inside SHELLDLL_DefView
    mock.add_window(create_window(
        0x2008,
        "SysListView32",
        "FolderView",
        4000,
        Some(0x2004),
        WindowRect::new(0, 0, 1920, 1080),
    ));
    // 4. Wallpaper Host Candidate WorkerW (sibling without SHELLDLL_DefView)
    mock.add_window(create_window(
        0x2010,
        "WorkerW",
        "",
        4000,
        None,
        WindowRect::new(0, 0, 1920, 1080),
    ));

    let discovery = DesktopHostDiscovery::new(mock);
    let result = discovery
        .discover_topology(&DesktopDiscoveryOptions::default())
        .expect("discovery should succeed");

    assert_eq!(result.topology, DesktopShellTopology::StandardWorkerWSplit);
    assert!(result.is_safe_for_attachment_review);

    let rec = result.recommended_candidate.expect("candidate exists");
    assert_eq!(rec.handle, NativeWindowHandle(0x2010));
    assert_eq!(rec.role, CandidateRole::WallpaperHostCandidate);
    assert!(!rec.has_shelldll_defview_child);

    // Verify icon carrier is identified
    let carrier = result.icon_carrier.expect("carrier exists");
    assert_eq!(carrier.handle, NativeWindowHandle(0x2000));
    assert!(carrier.has_shelldll_defview_child);
}

/// Tests modern multi-WorkerW shell topology (e.g. Windows 11 with multiple WorkerWs for
/// taskbar previews / Spotlight). Verifies that the discovery algorithm does NOT pick
/// the first WorkerW blindly, but correctly matches the one positioned directly behind
/// the icon carrier in Z-order.
#[test]
fn test_topology_modern_multi_workerw_resolution() {
    let mock = Arc::new(MockDesktopWindowProvider::new());

    // Top-level Z-order:
    // 1. WorkerW 0x1000 (Shell flyout / preview)
    mock.add_window(create_window(
        0x1000,
        "WorkerW",
        "Flyout",
        4000,
        None,
        WindowRect::new(100, 100, 300, 300),
    ));
    // 2. WorkerW 0x2000 (Icon carrier with SHELLDLL_DefView)
    mock.add_window(create_window(
        0x2000,
        "WorkerW",
        "",
        4000,
        None,
        WindowRect::new(0, 0, 1920, 1080),
    ));
    mock.add_window(create_window(
        0x2004,
        "SHELLDLL_DefView",
        "",
        4000,
        Some(0x2000),
        WindowRect::new(0, 0, 1920, 1080),
    ));
    // 3. WorkerW 0x3000 (Wallpaper host sitting directly behind icon carrier in Z-order)
    mock.add_window(create_window(
        0x3000,
        "WorkerW",
        "",
        4000,
        None,
        WindowRect::new(0, 0, 1920, 1080),
    ));
    // 4. WorkerW 0x4000 (Another ancillary WorkerW)
    mock.add_window(create_window(
        0x4000,
        "WorkerW",
        "DWM",
        4000,
        None,
        WindowRect::new(0, 0, 1920, 1080),
    ));

    let discovery = DesktopHostDiscovery::new(mock);
    let result = discovery
        .discover_topology(&DesktopDiscoveryOptions::default())
        .expect("discovery should succeed");

    assert_eq!(result.topology, DesktopShellTopology::ModernMultiWorkerW);
    assert!(result.is_safe_for_attachment_review);

    let rec = result.recommended_candidate.expect("candidate exists");
    // Must NOT be 0x1000 (first WorkerW), but exactly 0x3000 (behind icon carrier)
    assert_eq!(rec.handle, NativeWindowHandle(0x3000));
    assert_eq!(rec.role, CandidateRole::WallpaperHostCandidate);
}

/// Tests ambiguous desktop topology where multiple WorkerWs exist with no clear Z-order
/// association to the icon carrier. Must return Ambiguous and NOT recommend any candidate.
#[test]
fn test_topology_ambiguous_workerw_candidates() {
    let mock = Arc::new(MockDesktopWindowProvider::new());

    // 1. WorkerW 0x1000 (Candidate A)
    mock.add_window(create_window(
        0x1000,
        "WorkerW",
        "",
        4000,
        None,
        WindowRect::new(0, 0, 1920, 1080),
    ));
    // 2. Another top-level window in between
    mock.add_window(create_window(
        0x1500,
        "Notepad",
        "Untitled",
        5000,
        None,
        WindowRect::new(0, 0, 800, 600),
    ));
    // 3. WorkerW 0x2000 (Icon carrier with SHELLDLL_DefView)
    mock.add_window(create_window(
        0x2000,
        "WorkerW",
        "",
        4000,
        None,
        WindowRect::new(0, 0, 1920, 1080),
    ));
    mock.add_window(create_window(
        0x2004,
        "SHELLDLL_DefView",
        "",
        4000,
        Some(0x2000),
        WindowRect::new(0, 0, 1920, 1080),
    ));
    // 4. Another unrelated window in between
    mock.add_window(create_window(
        0x2500,
        "Calc",
        "Calculator",
        6000,
        None,
        WindowRect::new(0, 0, 400, 400),
    ));
    // 5. WorkerW 0x3000 (Candidate B, not immediately following icon carrier)
    mock.add_window(create_window(
        0x3000,
        "WorkerW",
        "",
        4000,
        None,
        WindowRect::new(0, 0, 1920, 1080),
    ));

    let discovery = DesktopHostDiscovery::new(mock);
    let result = discovery
        .discover_topology(&DesktopDiscoveryOptions::default())
        .expect("discovery should succeed");

    assert_eq!(result.topology, DesktopShellTopology::Ambiguous);
    assert!(!result.is_safe_for_attachment_review);
    assert!(result.recommended_candidate.is_none());

    // Compatibility policy must reject ambiguous topology
    let suit = CompatibilityPolicy::evaluate_attachment_suitability(&result);
    assert!(matches!(
        suit,
        Err(CompatibilityRejectionReason::AmbiguousTopology(2))
    ));
}

/// Tests that when neither Progman nor any WorkerW desktop windows exist, discovery returns
/// DesktopRootUnavailable safely without panicking.
#[test]
fn test_topology_desktop_root_unavailable() {
    let mock = Arc::new(MockDesktopWindowProvider::new());
    // Only arbitrary application windows
    mock.add_window(create_window(
        0x9000,
        "Chrome_WidgetWin_1",
        "Browser",
        7000,
        None,
        WindowRect::new(0, 0, 1920, 1080),
    ));

    let discovery = DesktopHostDiscovery::new(mock);
    let result = discovery
        .discover_topology(&DesktopDiscoveryOptions::default())
        .expect("discovery should return classification");

    assert_eq!(
        result.topology,
        DesktopShellTopology::DesktopRootUnavailable
    );
    assert!(result.recommended_candidate.is_none());
    assert!(!result.is_safe_for_attachment_review);

    let suit = CompatibilityPolicy::evaluate_attachment_suitability(&result);
    assert_eq!(
        suit,
        Err(CompatibilityRejectionReason::DesktopRootUnavailable)
    );
}

/// Tests handle revalidation: detects when a window is destroyed or replaced with another class.
#[test]
fn test_candidate_revalidation_and_stale_handle_detection() {
    let mock = Arc::new(MockDesktopWindowProvider::new());
    mock.add_window(create_window(
        0x2000,
        "WorkerW",
        "",
        4000,
        None,
        WindowRect::new(0, 0, 1920, 1080),
    ));

    let provider: Arc<dyn kinetiwall_desktop::DesktopWindowProvider> = Arc::clone(&mock) as _;
    let discovery = DesktopHostDiscovery::new(provider);

    let candidate = kinetiwall_desktop::DesktopCandidateWindow {
        handle: NativeWindowHandle(0x2000),
        class_name: "WorkerW".to_string(),
        window_title: String::new(),
        process_id: 4000,
        thread_id: 100,
        role: CandidateRole::WallpaperHostCandidate,
        parent: None,
        bounds: Some(WindowRect::new(0, 0, 1920, 1080)),
        is_visible: true,
        has_shelldll_defview_child: false,
        validated_at_millis: 1000,
    };

    // 1. Initial revalidation succeeds
    assert!(discovery.revalidate_candidate(&candidate));

    // 2. Window is destroyed
    let _ = mock.destroy_window(NativeWindowHandle(0x2000));
    assert!(
        !discovery.revalidate_candidate(&candidate),
        "Destroyed window must fail revalidation"
    );

    // 3. Handle is reused with a different class name (e.g. Notepad)
    mock.add_window(create_window(
        0x2000,
        "Notepad",
        "Reused Handle",
        4000,
        None,
        WindowRect::new(0, 0, 800, 600),
    ));
    assert!(
        !discovery.revalidate_candidate(&candidate),
        "Handle reused with different class must fail revalidation"
    );
}

/// Tests that CompatibilityPolicy strictly rejects candidate windows that carry
/// SHELLDLL_DefView, ensuring the wallpaper is never attached inside the icon carrier.
#[test]
fn test_compatibility_policy_rejects_icon_carrier_as_host() {
    let icon_carrier_cand = kinetiwall_desktop::DesktopCandidateWindow {
        handle: NativeWindowHandle(0x2000),
        class_name: "WorkerW".to_string(),
        window_title: String::new(),
        process_id: 4000,
        thread_id: 100,
        role: CandidateRole::IconHostCarrier,
        parent: None,
        bounds: Some(WindowRect::new(0, 0, 1920, 1080)),
        is_visible: true,
        has_shelldll_defview_child: true,
        validated_at_millis: 1000,
    };

    let classification = kinetiwall_desktop::TopologyClassification {
        topology: DesktopShellTopology::StandardWorkerWSplit,
        recommended_candidate: Some(icon_carrier_cand),
        all_candidates: Vec::new(),
        icon_carrier: None,
        icon_view: None,
        observed_at_millis: 1000,
        is_safe_for_attachment_review: true,
        reason: "Test".to_string(),
    };

    let res = CompatibilityPolicy::evaluate_attachment_suitability(&classification);
    assert_eq!(
        res,
        Err(CompatibilityRejectionReason::WrongCandidateRole(
            CandidateRole::IconHostCarrier
        ))
    );
}
