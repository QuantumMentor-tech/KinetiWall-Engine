//! Comprehensive test suite for static wallpaper fallback, last-frame preservation,
//! and reversible low-power transitions (Step 50).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use kinetiwall_catalog::{WallpaperId, WallpaperMetadata, WallpaperType};
use kinetiwall_desktop::{
    attachment::constants, AttachmentCoordinator, CandidateRole, DesktopCandidateWindow,
    DisplayGeometry, DisplayId, DisplaySessionCoordinator, DisplaySessionState, FallbackAsset,
    FallbackAssetCache, FallbackCapability, FallbackDiagnosticEventType, FallbackError,
    FallbackSourceKind, FallbackTransitionStage, MockDesktopWindowProvider,
    MockFrameCaptureProvider, MockWindowRecord, MultiMonitorLayoutMode, NativeWindowHandle,
    PauseReason, StaticFallbackCoordinator, WindowRect, MAX_FALLBACK_DIAGNOSTICS,
    MAX_FALLBACK_DIMENSION, MAX_FALLBACK_FILE_SIZE_BYTES, MAX_RETAINED_FALLBACK_ASSETS,
};

/// Test harness setting up mock desktop host, display sessions, and fallback coordinator.
struct TestFallbackHarness {
    pub temp_dir: PathBuf,
    pub session_coordinator: Arc<Mutex<DisplaySessionCoordinator>>,
    pub host_candidate: DesktopCandidateWindow,
    pub display1_id: DisplayId,
    pub display2_id: DisplayId,
    pub display1_geom: DisplayGeometry,
    pub display2_geom: DisplayGeometry,
}

impl TestFallbackHarness {
    pub fn new() -> Self {
        let temp_dir = std::env::temp_dir().join(format!("kinetiwall_fb_{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&temp_dir);

        let window_provider = Arc::new(MockDesktopWindowProvider::new());
        let owning_pid = 5000;
        let workerw = NativeWindowHandle(200);

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

        let session_coord = DisplaySessionCoordinator::with_default_factory(
            window_provider,
            attachment_coordinator,
        );
        let session_coordinator = Arc::new(Mutex::new(session_coord));

        let display1_id = DisplayId::new("DISPLAY_PRIMARY");
        let display2_id = DisplayId::new("DISPLAY_SECONDARY");

        let display1_geom = DisplayGeometry {
            monitor_rect: WindowRect::new(0, 0, 1920, 1080),
            work_area_rect: WindowRect::new(0, 0, 1920, 1040),
            width: 1920,
            height: 1080,
            scale_factor_percent: Some(100),
        };

        let display2_geom = DisplayGeometry {
            monitor_rect: WindowRect::new(1920, 0, 3840, 1080),
            work_area_rect: WindowRect::new(1920, 0, 3840, 1040),
            width: 1920,
            height: 1080,
            scale_factor_percent: Some(100),
        };

        Self {
            temp_dir,
            session_coordinator,
            host_candidate,
            display1_id,
            display2_id,
            display1_geom,
            display2_geom,
        }
    }

    pub fn activate_session(&self, display_id: &DisplayId, geom: DisplayGeometry) -> WallpaperId {
        let wallpaper_id = WallpaperId::new();
        let mut sc = self.session_coordinator.lock().unwrap();
        sc.activate_session(display_id, wallpaper_id, geom, &self.host_candidate)
            .expect("Failed to activate session");
        wallpaper_id
    }
}

impl Drop for TestFallbackHarness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.temp_dir);
    }
}

// -------------------------------------------------------------------------------------------------
// Unit and Integration Tests
// -------------------------------------------------------------------------------------------------

#[test]
fn test_fallback_capability_matrix_video_vs_web() {
    // 1. Video backend capabilities
    let video_cap = FallbackCapability::for_video();
    assert!(video_cap.supports_static_image);
    assert!(video_cap.supports_catalog_poster);
    assert!(video_cap.supports_live_frame_capture);
    assert!(video_cap.supports_media_position_restore);
    assert!(!video_cap.supports_js_state_restore);

    // Derived from WallpaperType
    assert_eq!(
        FallbackCapability::from_wallpaper_type(WallpaperType::Video),
        video_cap
    );

    // Derived from backend name
    assert_eq!(FallbackCapability::from_backend_name("libmpv"), video_cap);
    assert_eq!(FallbackCapability::from_backend_name("video"), video_cap);

    // 2. Web backend capabilities
    let web_cap = FallbackCapability::for_web();
    assert!(web_cap.supports_static_image);
    assert!(web_cap.supports_catalog_poster);
    // Strictly FALSE for Web:
    assert!(!web_cap.supports_live_frame_capture);
    assert!(!web_cap.supports_media_position_restore);
    assert!(!web_cap.supports_js_state_restore);

    // Derived from WallpaperType
    assert_eq!(
        FallbackCapability::from_wallpaper_type(WallpaperType::Web),
        web_cap
    );

    // Derived from backend name
    assert_eq!(FallbackCapability::from_backend_name("WebView2"), web_cap);
}

#[test]
fn test_fallback_source_resolution_priority() {
    let harness = TestFallbackHarness::new();
    let cache_dir = harness.temp_dir.join("cache");
    let capture_provider = Arc::new(MockFrameCaptureProvider::default());
    let mut coordinator = StaticFallbackCoordinator::new(&cache_dir, capture_provider);

    let wallpaper_id = WallpaperId::new();
    let dummy_hwnd = NativeWindowHandle(1234);

    // Create real files for testing
    let explicit_file = harness.temp_dir.join("explicit.png");
    std::fs::write(&explicit_file, b"EXPLICIT_IMAGE").unwrap();

    let poster_file = harness.temp_dir.join("poster.jpg");
    std::fs::write(&poster_file, b"CATALOG_POSTER").unwrap();

    let mut metadata = WallpaperMetadata::new("Cyberpunk City");
    metadata.thumbnail_ref = Some(poster_file.to_string_lossy().to_string());

    // Priority 1: Explicit fallback provided and file exists
    let src1 = coordinator.resolve_fallback_source(
        wallpaper_id,
        WallpaperType::Video,
        Some(&explicit_file),
        Some(&metadata),
        Some(dummy_hwnd),
    );
    assert_eq!(src1, FallbackSourceKind::ExplicitFallback(explicit_file.clone()));

    // Priority 2: No explicit fallback; catalog poster exists
    let src2 = coordinator.resolve_fallback_source(
        wallpaper_id,
        WallpaperType::Video,
        None,
        Some(&metadata),
        Some(dummy_hwnd),
    );
    assert_eq!(src2, FallbackSourceKind::CatalogPoster(poster_file));

    // Priority 3: No explicit, no poster; Video backend with live frame capture
    let mut meta_no_poster = WallpaperMetadata::new("Ocean Waves");
    meta_no_poster.thumbnail_ref = None;

    let src3 = coordinator.resolve_fallback_source(
        wallpaper_id,
        WallpaperType::Video,
        None,
        Some(&meta_no_poster),
        Some(dummy_hwnd),
    );
    assert!(matches!(src3, FallbackSourceKind::LiveFrameCapture(_)));

    // Priority 4: Web backend with no poster -> strictly skips capture and returns NoneAvailable
    let src4 = coordinator.resolve_fallback_source(
        wallpaper_id,
        WallpaperType::Web,
        None,
        Some(&meta_no_poster),
        Some(dummy_hwnd),
    );
    assert_eq!(src4, FallbackSourceKind::NoneAvailable);
}

#[test]
fn test_static_fallback_transition_and_session_state() {
    let harness = TestFallbackHarness::new();
    let wallpaper_id = harness.activate_session(&harness.display1_id, harness.display1_geom);

    let cache_dir = harness.temp_dir.join("cache");
    let capture_provider = Arc::new(MockFrameCaptureProvider::default());
    let mut fallback_coord = StaticFallbackCoordinator::new(&cache_dir, capture_provider);

    let poster_file = harness.temp_dir.join("poster.jpg");
    std::fs::write(&poster_file, b"POSTER_DATA").unwrap();

    let mut metadata = WallpaperMetadata::new("Space Nebulae");
    metadata.thumbnail_ref = Some(poster_file.to_string_lossy().to_string());

    // Execute transition with media position preservation (e.g. 42000 ms = 42s)
    let outcome = fallback_coord
        .transition_to_fallback(
            &harness.display1_id,
            WallpaperType::Video,
            None,
            Some(&metadata),
            &harness.session_coordinator,
            Some(42000),
        )
        .expect("Transition to fallback failed");

    assert_eq!(outcome, FallbackSourceKind::CatalogPoster(poster_file));

    // Verify session coordinator state is now StaticFallback
    {
        let sc = harness.session_coordinator.lock().unwrap();
        let session = sc.get_session(&harness.display1_id).unwrap();
        assert!(session.state.is_fallback());
        assert!(!session.state.is_active());
        assert!(!session.state.is_terminated());
    }

    // Verify preserved session state
    let preserved = fallback_coord.get_preserved_state(&harness.display1_id).unwrap();
    assert_eq!(preserved.display_id, harness.display1_id);
    assert_eq!(preserved.wallpaper_id, wallpaper_id);
    assert_eq!(preserved.media_position_ms, Some(42000));
    assert_eq!(preserved.original_session_state, DisplaySessionState::Active);

    // Verify transition stage is DisplayingFallback
    assert_eq!(
        fallback_coord.get_transition_stage(&harness.display1_id),
        FallbackTransitionStage::DisplayingFallback
    );
}

#[test]
fn test_fallback_duplicate_transition_is_idempotent() {
    let harness = TestFallbackHarness::new();
    let _ = harness.activate_session(&harness.display1_id, harness.display1_geom);

    let cache_dir = harness.temp_dir.join("cache");
    let capture_provider = Arc::new(MockFrameCaptureProvider::default());
    let mut fallback_coord = StaticFallbackCoordinator::new(&cache_dir, capture_provider);

    let poster_file = harness.temp_dir.join("poster.jpg");
    std::fs::write(&poster_file, b"POSTER_DATA").unwrap();

    let mut metadata = WallpaperMetadata::new("Aurora");
    metadata.thumbnail_ref = Some(poster_file.to_string_lossy().to_string());

    // First transition
    let res1 = fallback_coord
        .transition_to_fallback(
            &harness.display1_id,
            WallpaperType::Video,
            None,
            Some(&metadata),
            &harness.session_coordinator,
            Some(10000),
        )
        .expect("First transition failed");
    assert!(matches!(res1, FallbackSourceKind::CatalogPoster(_)));

    // Second transition must be a safe idempotent no-op
    let res2 = fallback_coord
        .transition_to_fallback(
            &harness.display1_id,
            WallpaperType::Video,
            None,
            Some(&metadata),
            &harness.session_coordinator,
            Some(20000),
        )
        .expect("Second transition failed");
    assert_eq!(res2, FallbackSourceKind::NoneAvailable);

    // Initial preserved position (10000 ms) must remain intact
    let preserved = fallback_coord.get_preserved_state(&harness.display1_id).unwrap();
    assert_eq!(preserved.media_position_ms, Some(10000));
}

#[test]
fn test_safe_restoration_to_active_when_power_cleared() {
    let harness = TestFallbackHarness::new();
    let wallpaper_id = harness.activate_session(&harness.display1_id, harness.display1_geom);

    let cache_dir = harness.temp_dir.join("cache");
    let capture_provider = Arc::new(MockFrameCaptureProvider::default());
    let mut fallback_coord = StaticFallbackCoordinator::new(&cache_dir, capture_provider);

    let poster_file = harness.temp_dir.join("poster.jpg");
    std::fs::write(&poster_file, b"POSTER_DATA").unwrap();

    let mut metadata = WallpaperMetadata::new("Matrix Rain");
    metadata.thumbnail_ref = Some(poster_file.to_string_lossy().to_string());

    fallback_coord
        .transition_to_fallback(
            &harness.display1_id,
            WallpaperType::Video,
            None,
            Some(&metadata),
            &harness.session_coordinator,
            Some(55000),
        )
        .unwrap();

    // No other pause reasons active (only PowerSaving was active and is now cleared)
    let active_reasons: HashMap<DisplayId, HashSet<PauseReason>> = HashMap::new();

    let restored = fallback_coord
        .restore_from_fallback(
            &harness.display1_id,
            &active_reasons,
            &harness.session_coordinator,
        )
        .expect("Restoration failed")
        .expect("Preserved state missing");

    assert_eq!(restored.wallpaper_id, wallpaper_id);
    assert_eq!(restored.media_position_ms, Some(55000));

    // Verify session coordinator is back to Active
    {
        let sc = harness.session_coordinator.lock().unwrap();
        let session = sc.get_session(&harness.display1_id).unwrap();
        assert_eq!(session.state, DisplaySessionState::Active);
    }

    assert_eq!(
        fallback_coord.get_transition_stage(&harness.display1_id),
        FallbackTransitionStage::Idle
    );
}

#[test]
fn test_reversibility_multi_reason_pause_interlock() {
    let harness = TestFallbackHarness::new();
    let _ = harness.activate_session(&harness.display1_id, harness.display1_geom);

    let cache_dir = harness.temp_dir.join("cache");
    let capture_provider = Arc::new(MockFrameCaptureProvider::default());
    let mut fallback_coord = StaticFallbackCoordinator::new(&cache_dir, capture_provider);

    let poster_file = harness.temp_dir.join("poster.jpg");
    std::fs::write(&poster_file, b"POSTER_DATA").unwrap();

    let mut metadata = WallpaperMetadata::new("Landscape");
    metadata.thumbnail_ref = Some(poster_file.to_string_lossy().to_string());

    fallback_coord
        .transition_to_fallback(
            &harness.display1_id,
            WallpaperType::Video,
            None,
            Some(&metadata),
            &harness.session_coordinator,
            None,
        )
        .unwrap();

    // Display has a surviving UserManual pause reason!
    let mut active_reasons = HashMap::new();
    let mut reasons_set = HashSet::new();
    reasons_set.insert(PauseReason::UserManual);
    active_reasons.insert(harness.display1_id.clone(), reasons_set);

    let _ = fallback_coord
        .restore_from_fallback(
            &harness.display1_id,
            &active_reasons,
            &harness.session_coordinator,
        )
        .unwrap();

    // Session coordinator must NOT be active because UserManual pause is still in effect!
    {
        let sc = harness.session_coordinator.lock().unwrap();
        let session = sc.get_session(&harness.display1_id).unwrap();
        assert!(!session.state.is_active());
    }

    // Diagnostic event confirms manual override was respected
    let diagnostics = fallback_coord.get_diagnostics();
    assert!(diagnostics.iter().any(|d| d.event_type == FallbackDiagnosticEventType::ManualOverrideRespected));
}

#[test]
fn test_fallback_cache_budget_and_dimension_validation() {
    let temp_dir = std::env::temp_dir().join(format!("kinetiwall_fb_budget_{}", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let mut cache = FallbackAssetCache::new(&temp_dir);

    let wallpaper_id = WallpaperId::new();
    let valid_path = temp_dir.join("valid.png");
    std::fs::write(&valid_path, b"IMAGE").unwrap();

    // 1. Valid asset within bounds
    let valid_asset = FallbackAsset {
        asset_id: uuid::Uuid::new_v4(),
        wallpaper_id,
        source_kind: FallbackSourceKind::ExplicitFallback(valid_path.clone()),
        file_path: valid_path.clone(),
        width: 1920,
        height: 1080,
        file_size_bytes: 1024 * 1024,
        is_transient: false,
        created_at_millis: 1000,
        last_accessed_millis: 1000,
    };
    assert!(cache.store(valid_asset).is_ok());
    assert_eq!(cache.len(), 1);

    // 2. Reject dimensions exceeding MAX_FALLBACK_DIMENSION (4096)
    let oversized_dim_asset = FallbackAsset {
        asset_id: uuid::Uuid::new_v4(),
        wallpaper_id,
        source_kind: FallbackSourceKind::ExplicitFallback(valid_path.clone()),
        file_path: valid_path.clone(),
        width: MAX_FALLBACK_DIMENSION + 1,
        height: 1080,
        file_size_bytes: 1024,
        is_transient: false,
        created_at_millis: 1000,
        last_accessed_millis: 1000,
    };
    let err_dim = cache.store(oversized_dim_asset).unwrap_err();
    assert!(matches!(err_dim, FallbackError::AssetExceedsBounds { .. }));

    // 3. Reject file size exceeding MAX_FALLBACK_FILE_SIZE_BYTES (32MB)
    let oversized_size_asset = FallbackAsset {
        asset_id: uuid::Uuid::new_v4(),
        wallpaper_id,
        source_kind: FallbackSourceKind::ExplicitFallback(valid_path.clone()),
        file_path: valid_path,
        width: 1920,
        height: 1080,
        file_size_bytes: MAX_FALLBACK_FILE_SIZE_BYTES + 1,
        is_transient: false,
        created_at_millis: 1000,
        last_accessed_millis: 1000,
    };
    let err_size = cache.store(oversized_size_asset).unwrap_err();
    assert!(matches!(err_size, FallbackError::AssetExceedsFileSize { .. }));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_cache_eviction_strictly_protects_non_transient_assets() {
    let temp_dir = std::env::temp_dir().join(format!("kinetiwall_fb_evict_{}", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let mut cache = FallbackAssetCache::new(&temp_dir);

    // Create 1 permanent user file
    let user_file = temp_dir.join("user_wallpaper.jpg");
    std::fs::write(&user_file, b"PRECIOUS_USER_DATA").unwrap();

    let permanent_asset = FallbackAsset {
        asset_id: uuid::Uuid::new_v4(),
        wallpaper_id: WallpaperId::new(),
        source_kind: FallbackSourceKind::ExplicitFallback(user_file.clone()),
        file_path: user_file.clone(),
        width: 1920,
        height: 1080,
        file_size_bytes: 100,
        is_transient: false, // Protected permanent asset!
        created_at_millis: 1,
        last_accessed_millis: 1,
    };
    cache.store(permanent_asset).unwrap();

    // Now insert transient assets up to MAX_RETAINED_FALLBACK_ASSETS + 5
    let mut transient_paths = Vec::new();
    for i in 0..(MAX_RETAINED_FALLBACK_ASSETS + 5) {
        let capture_path = temp_dir.join(format!("capture_{}.png", i));
        std::fs::write(&capture_path, b"TRANSIENT_FRAME_CAPTURE").unwrap();
        transient_paths.push(capture_path.clone());

        let transient_asset = FallbackAsset {
            asset_id: uuid::Uuid::new_v4(),
            wallpaper_id: WallpaperId::new(),
            source_kind: FallbackSourceKind::LiveFrameCapture(capture_path.clone()),
            file_path: capture_path,
            width: 1920,
            height: 1080,
            file_size_bytes: 100,
            is_transient: true, // Safe to evict and delete on disk!
            created_at_millis: 10 + i as u64,
            last_accessed_millis: 10 + i as u64,
        };
        cache.store(transient_asset).unwrap();
    }

    // Cache must remain bounded at MAX_RETAINED_FALLBACK_ASSETS
    assert!(cache.len() <= MAX_RETAINED_FALLBACK_ASSETS);

    // CRITICAL: The user's precious permanent file MUST NEVER BE DELETED on disk!
    assert!(
        user_file.exists(),
        "User permanent file was deleted during cache eviction!"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_multi_monitor_independent_fallback() {
    let harness = TestFallbackHarness::new();
    let _wp1 = harness.activate_session(&harness.display1_id, harness.display1_geom);
    let _wp2 = harness.activate_session(&harness.display2_id, harness.display2_geom);

    let cache_dir = harness.temp_dir.join("cache");
    let capture_provider = Arc::new(MockFrameCaptureProvider::default());
    let mut fallback_coord = StaticFallbackCoordinator::new(&cache_dir, capture_provider);

    let poster_file = harness.temp_dir.join("poster.jpg");
    std::fs::write(&poster_file, b"POSTER").unwrap();
    let mut metadata = WallpaperMetadata::new("Forest");
    metadata.thumbnail_ref = Some(poster_file.to_string_lossy().to_string());

    let displays = vec![harness.display1_id.clone(), harness.display2_id.clone()];
    let results = fallback_coord
        .transition_layout_group(
            &displays,
            MultiMonitorLayoutMode::Independent,
            WallpaperType::Video,
            None,
            Some(&metadata),
            &harness.session_coordinator,
        )
        .expect("Multi-monitor independent fallback failed");

    assert_eq!(results.len(), 2);
    assert_eq!(
        results.get(&harness.display1_id).unwrap(),
        &FallbackSourceKind::CatalogPoster(poster_file.clone())
    );
    assert_eq!(
        results.get(&harness.display2_id).unwrap(),
        &FallbackSourceKind::CatalogPoster(poster_file)
    );

    // Both sessions in coordinator are in StaticFallback state
    let sc = harness.session_coordinator.lock().unwrap();
    assert!(sc.get_session(&harness.display1_id).unwrap().state.is_fallback());
    assert!(sc.get_session(&harness.display2_id).unwrap().state.is_fallback());
}

#[test]
fn test_multi_monitor_clone_mode_shared_fallback() {
    let harness = TestFallbackHarness::new();
    let _wp1 = harness.activate_session(&harness.display1_id, harness.display1_geom);
    let _wp2 = harness.activate_session(&harness.display2_id, harness.display2_geom);

    let cache_dir = harness.temp_dir.join("cache");
    let capture_provider = Arc::new(MockFrameCaptureProvider::default());
    let mut fallback_coord = StaticFallbackCoordinator::new(&cache_dir, capture_provider);

    let poster_file = harness.temp_dir.join("poster_clone.jpg");
    std::fs::write(&poster_file, b"CLONE_POSTER").unwrap();
    let mut metadata = WallpaperMetadata::new("Clone Presentation");
    metadata.thumbnail_ref = Some(poster_file.to_string_lossy().to_string());

    let displays = vec![harness.display1_id.clone(), harness.display2_id.clone()];
    let results = fallback_coord
        .transition_layout_group(
            &displays,
            MultiMonitorLayoutMode::Clone,
            WallpaperType::Video,
            None,
            Some(&metadata),
            &harness.session_coordinator,
        )
        .expect("Clone fallback transition failed");

    assert_eq!(results.len(), 2);
    // Both displays share the exact same primary fallback source presentation
    assert_eq!(
        results.get(&harness.display1_id).unwrap(),
        results.get(&harness.display2_id).unwrap()
    );

    let sc = harness.session_coordinator.lock().unwrap();
    assert!(sc.get_session(&harness.display1_id).unwrap().state.is_fallback());
    assert!(sc.get_session(&harness.display2_id).unwrap().state.is_fallback());
}

#[test]
fn test_frame_capture_failure_gracefully_degrades_to_standard_suspension() {
    let harness = TestFallbackHarness::new();
    let _ = harness.activate_session(&harness.display1_id, harness.display1_geom);

    let cache_dir = harness.temp_dir.join("cache");
    // Mock provider that fails capture
    let capture_provider = Arc::new(
        MockFrameCaptureProvider::new(true, (1920, 1080))
            .with_failure("DWM compositor surface locked"),
    );
    let mut fallback_coord = StaticFallbackCoordinator::new(&cache_dir, capture_provider);

    // No explicit fallback, no catalog poster
    let meta_no_poster = WallpaperMetadata::new("No Poster Video");

    let outcome = fallback_coord
        .transition_to_fallback(
            &harness.display1_id,
            WallpaperType::Video,
            None,
            Some(&meta_no_poster),
            &harness.session_coordinator,
            None,
        )
        .expect("Transition should not panic on capture failure");

    // Gracefully resolved to NoneAvailable
    assert_eq!(outcome, FallbackSourceKind::NoneAvailable);

    // Coordinator suspended safely without crashing
    let sc = harness.session_coordinator.lock().unwrap();
    assert_eq!(
        sc.get_session(&harness.display1_id).unwrap().state,
        DisplaySessionState::Suspended
    );
}

#[test]
fn test_bounded_diagnostic_event_history() {
    let temp_dir = std::env::temp_dir().join(format!("kinetiwall_fb_diag_{}", uuid::Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let capture_provider = Arc::new(MockFrameCaptureProvider::default());
    let mut coord = StaticFallbackCoordinator::new(&temp_dir, capture_provider);

    for i in 0..(MAX_FALLBACK_DIAGNOSTICS + 20) {
        coord.record_diagnostic(
            None,
            FallbackDiagnosticEventType::CapabilityEvaluated,
            format!("Diagnostic event {}", i),
        );
    }

    assert!(coord.get_diagnostics().len() <= MAX_FALLBACK_DIAGNOSTICS);

    let _ = std::fs::remove_dir_all(&temp_dir);
}
