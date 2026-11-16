//! Comprehensive test suite for Unified Policy Arbitration, Conflict Resolution,
//! and Deterministic Playback Decisions (Step 56).

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use kinetiwall_catalog::{
    AssetReference, AvailabilityState, CatalogError, CatalogLifecycle, WallpaperCatalogRepository,
    WallpaperId, WallpaperMetadata, WallpaperRecord, WallpaperSourceKind, WallpaperType,
};
use kinetiwall_desktop::attachment::constants;
use kinetiwall_desktop::{
    AssignmentService, AttachmentCoordinator, CandidateRole, DesktopCandidateWindow,
    DesiredSessionState, DisplayId, DisplaySessionCoordinator, DisplaySessionId,
    DisplaySessionState, DisplaySnapshotManager, FullscreenPlaybackSettings,
    MockAssignmentRepository, MockDesktopDisplayProvider, MockDesktopWindowProvider,
    MockWindowRecord, MultiMonitorLayoutCoordinator, MultiMonitorLayoutMode,
    NativeWindowHandle, PauseReason, PolicyArbitrator, PolicyExplanationCode,
    SessionArbitrationInput, SessionPlaybackSettings, SmartPlaybackPolicyEvaluator, WindowRect,
    WindowsSessionSnapshot, WorkstationLockState,
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

    fn add_wallpaper(&self, id: WallpaperId, title: &str) {
        let record = WallpaperRecord {
            id,
            schema_version: 1,
            wallpaper_type: WallpaperType::Video,
            asset: AssetReference {
                source_kind: WallpaperSourceKind::ManagedPackage,
                entrypoint: "video.mp4".to_string(),
            },
            metadata: WallpaperMetadata::new(title),
            lifecycle: CatalogLifecycle::now(false),
            availability: AvailabilityState::Available,
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
// Pure Arbitration Logic Tests
// -------------------------------------------------------------------------------------------------

#[test]
fn test_pure_arbitration_unconstrained_active() {
    let display_id = DisplayId::new("DISPLAY_1");
    let session_id = DisplaySessionId::new();

    let input = SessionArbitrationInput {
        display_id: display_id.clone(),
        session_id: Some(session_id.clone()),
        canonical_target_fps: 60,
        active_pause_reasons: HashSet::new(),
        adaptive_fps_cap: None,
        budget_fps_cap: None,
        is_session_terminated: false,
        termination_reason: None,
        allows_static_fallback: true,
        default_fallback_source: Some("E:\\wallpapers\\fallback.png".to_string()),
        current_actual_state: DisplaySessionState::Active,
    };

    let decision = PolicyArbitrator::arbitrate(&input);

    assert_eq!(decision.display_id, display_id);
    assert_eq!(decision.session_id, Some(session_id));
    assert_eq!(decision.desired_state, DesiredSessionState::Active);
    assert_eq!(decision.effective_fps_limit, None);
    assert!(decision.active_constraints.is_empty());
    assert_eq!(
        decision.explanation_codes,
        vec![PolicyExplanationCode::UnconstrainedActive]
    );
    assert!(!decision.requires_transition); // already active, deduplication passes
}

#[test]
fn test_pure_arbitration_manual_pause_precedence() {
    let display_id = DisplayId::new("DISPLAY_1");
    let session_id = DisplaySessionId::new();

    let mut pause_reasons = HashSet::new();
    pause_reasons.insert(PauseReason::UserManual);
    pause_reasons.insert(PauseReason::Fullscreen {
        display_id: display_id.clone(),
    });

    let input = SessionArbitrationInput {
        display_id: display_id.clone(),
        session_id: Some(session_id.clone()),
        canonical_target_fps: 60,
        active_pause_reasons: pause_reasons,
        adaptive_fps_cap: Some(30),
        budget_fps_cap: Some(15),
        is_session_terminated: false,
        termination_reason: None,
        allows_static_fallback: false,
        default_fallback_source: None,
        current_actual_state: DisplaySessionState::Active,
    };

    let decision = PolicyArbitrator::arbitrate(&input);

    assert_eq!(decision.desired_state, DesiredSessionState::Suspended);
    assert!(decision.requires_transition); // from Active to Suspended
    assert!(decision
        .explanation_codes
        .contains(&PolicyExplanationCode::ManualPauseActive));
    assert!(decision
        .explanation_codes
        .contains(&PolicyExplanationCode::FullscreenActive));

    // Even if fullscreen were to clear, UserManual keeps it suspended:
    let mut cleared_reasons = HashSet::new();
    cleared_reasons.insert(PauseReason::UserManual);
    let mut input_after_clear = input.clone();
    input_after_clear.active_pause_reasons = cleared_reasons;

    let decision_after_clear = PolicyArbitrator::arbitrate(&input_after_clear);
    assert_eq!(
        decision_after_clear.desired_state,
        DesiredSessionState::Suspended
    );
    assert!(decision_after_clear
        .explanation_codes
        .contains(&PolicyExplanationCode::ManualPauseActive));
    assert!(!decision_after_clear
        .explanation_codes
        .contains(&PolicyExplanationCode::FullscreenActive));
}

#[test]
fn test_pure_arbitration_fallback_presentation() {
    let display_id = DisplayId::new("DISPLAY_1");
    let session_id = DisplaySessionId::new();

    let mut pause_reasons = HashSet::new();
    pause_reasons.insert(PauseReason::WorkstationLocked);

    let input = SessionArbitrationInput {
        display_id: display_id.clone(),
        session_id: Some(session_id.clone()),
        canonical_target_fps: 60,
        active_pause_reasons: pause_reasons,
        adaptive_fps_cap: None,
        budget_fps_cap: None,
        is_session_terminated: false,
        termination_reason: None,
        allows_static_fallback: true,
        default_fallback_source: Some("E:\\wallpapers\\saved_frame.png".to_string()),
        current_actual_state: DisplaySessionState::Active,
    };

    let decision = PolicyArbitrator::arbitrate(&input);

    match &decision.desired_state {
        DesiredSessionState::StaticFallback { fallback_source } => {
            assert_eq!(fallback_source, "E:\\wallpapers\\saved_frame.png");
        }
        _ => panic!("Expected StaticFallback state"),
    }
    assert!(decision.requires_transition);
    assert!(decision
        .explanation_codes
        .contains(&PolicyExplanationCode::WorkstationLocked));
    assert!(decision
        .explanation_codes
        .contains(&PolicyExplanationCode::StaticFallbackPresented));
}

#[test]
fn test_pure_arbitration_quality_cap_composition() {
    let display_id = DisplayId::new("DISPLAY_1");
    let session_id = DisplaySessionId::new();

    // Canonical is 60, adaptive wants 30, budget arbiter wants 20.
    // Arbiter should compose to min(30, 20, 60) = 20.
    let input = SessionArbitrationInput {
        display_id: display_id.clone(),
        session_id: Some(session_id.clone()),
        canonical_target_fps: 60,
        active_pause_reasons: HashSet::new(),
        adaptive_fps_cap: Some(30),
        budget_fps_cap: Some(20),
        is_session_terminated: false,
        termination_reason: None,
        allows_static_fallback: false,
        default_fallback_source: None,
        current_actual_state: DisplaySessionState::Active,
    };

    let decision = PolicyArbitrator::arbitrate(&input);

    assert_eq!(decision.desired_state, DesiredSessionState::Active);
    assert_eq!(decision.effective_fps_limit, Some(20));
    assert!(decision
        .explanation_codes
        .contains(&PolicyExplanationCode::AdaptiveQualityPacing));
    assert!(decision
        .explanation_codes
        .contains(&PolicyExplanationCode::ResourceBudgetPressure));
    assert!(decision
        .explanation_codes
        .contains(&PolicyExplanationCode::UnconstrainedActive));
}

#[test]
fn test_pure_arbitration_termination_precedence() {
    let display_id = DisplayId::new("DISPLAY_1");
    let session_id = DisplaySessionId::new();

    let input = SessionArbitrationInput {
        display_id: display_id.clone(),
        session_id: Some(session_id.clone()),
        canonical_target_fps: 60,
        active_pause_reasons: HashSet::new(), // no pause reasons
        adaptive_fps_cap: None,
        budget_fps_cap: None,
        is_session_terminated: true, // terminated!
        termination_reason: Some("Terminated by user".to_string()),
        allows_static_fallback: true,
        default_fallback_source: Some("E:\\wallpapers\\fallback.png".to_string()),
        current_actual_state: DisplaySessionState::Active,
    };

    let decision = PolicyArbitrator::arbitrate(&input);

    match decision.desired_state {
        DesiredSessionState::Terminated { .. } => {}
        _ => panic!("Expected Terminated state"),
    }
    assert!(decision.requires_transition);
    assert_eq!(
        decision.explanation_codes,
        vec![PolicyExplanationCode::SessionTerminated]
    );
}

#[test]
fn test_transition_deduplication() {
    let display_id = DisplayId::new("DISPLAY_1");
    let session_id = DisplaySessionId::new();

    let mut pause_reasons = HashSet::new();
    pause_reasons.insert(PauseReason::DisplayPowerOff);

    // Case 1: Backend is already Suspended, desired is Suspended => no transition
    let input1 = SessionArbitrationInput {
        display_id: display_id.clone(),
        session_id: Some(session_id.clone()),
        canonical_target_fps: 60,
        active_pause_reasons: pause_reasons.clone(),
        adaptive_fps_cap: None,
        budget_fps_cap: None,
        is_session_terminated: false,
        termination_reason: None,
        allows_static_fallback: false,
        default_fallback_source: None,
        current_actual_state: DisplaySessionState::Suspended,
    };
    let decision1 = PolicyArbitrator::arbitrate(&input1);
    assert_eq!(decision1.desired_state, DesiredSessionState::Suspended);
    assert!(!decision1.requires_transition);

    // Case 2: Backend is Active, desired is Suspended => requires transition
    let mut input2 = input1.clone();
    input2.current_actual_state = DisplaySessionState::Active;
    let decision2 = PolicyArbitrator::arbitrate(&input2);
    assert!(decision2.requires_transition);

    // Case 3: Backend has same StaticFallback source => no transition
    let input3 = SessionArbitrationInput {
        display_id: display_id.clone(),
        session_id: Some(session_id.clone()),
        canonical_target_fps: 60,
        active_pause_reasons: pause_reasons,
        adaptive_fps_cap: None,
        budget_fps_cap: None,
        is_session_terminated: false,
        termination_reason: None,
        allows_static_fallback: true,
        default_fallback_source: Some("E:\\img.png".to_string()),
        current_actual_state: DisplaySessionState::StaticFallback {
            fallback_source: "E:\\img.png".to_string(),
        },
    };
    let decision3 = PolicyArbitrator::arbitrate(&input3);
    assert!(!decision3.requires_transition);

    // Case 4: Backend has different StaticFallback source => requires transition
    let mut input4 = input3.clone();
    input4.current_actual_state = DisplaySessionState::StaticFallback {
        fallback_source: "E:\\old_img.png".to_string(),
    };
    let decision4 = PolicyArbitrator::arbitrate(&input4);
    assert!(decision4.requires_transition);
}

// -------------------------------------------------------------------------------------------------
// Test Policy Harness for Integration
// -------------------------------------------------------------------------------------------------

struct TestArbitrationHarness {
    _window_provider: Arc<MockDesktopWindowProvider>,
    snapshot: Arc<kinetiwall_desktop::DisplayTopologySnapshot>,
    session_coordinator: Arc<Mutex<DisplaySessionCoordinator>>,
    evaluator: SmartPlaybackPolicyEvaluator,
    display1_id: DisplayId,
    _display2_id: DisplayId,
}

impl TestArbitrationHarness {
    fn new_dual() -> Self {
        let window_provider = Arc::new(MockDesktopWindowProvider::new());
        let display_provider = Arc::new(MockDesktopDisplayProvider::new());

        let mon1 = WindowRect::new(0, 0, 1920, 1080);
        let mon2 = WindowRect::new(1920, 0, 3840, 1080);
        display_provider.setup_dual_extended(mon1, mon2);

        let mut display_manager = DisplaySnapshotManager::new(display_provider.clone());
        let (snapshot, _) = display_manager.refresh().expect("Snapshot refresh");
        let display1_id = snapshot.displays[0].id.clone();
        let display2_id = snapshot.displays[1].id.clone();

        let owning_pid = 4000;
        let workerw = NativeWindowHandle(100);

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

        let catalog = TestCatalog::new();
        let wp1 = WallpaperId::new();
        let wp2 = WallpaperId::new();
        catalog.add_wallpaper(wp1, "Wallpaper One");
        catalog.add_wallpaper(wp2, "Wallpaper Two");

        let repo = Arc::new(MockAssignmentRepository::new());
        let assignment_service =
            Arc::new(AssignmentService::new(repo).expect("AssignmentService creation"));
        assignment_service
            .assign_wallpaper(&display1_id, wp1)
            .expect("Assign D1");
        assignment_service
            .assign_wallpaper(&display2_id, wp2)
            .expect("Assign D2");

        let session_coord = DisplaySessionCoordinator::with_default_factory(
            window_provider.clone(),
            attachment_coordinator,
        );
        let session_coordinator = Arc::new(Mutex::new(session_coord));

        let mut layout_coordinator = MultiMonitorLayoutCoordinator::new(
            session_coordinator.clone(),
            assignment_service,
        );
        layout_coordinator
            .apply_layout(&snapshot, &catalog, &host_candidate)
            .expect("Apply layout");

        let mut settings = FullscreenPlaybackSettings::default();
        settings.debounce_duration_millis = 0; // Immediate for deterministic testing

        let evaluator = SmartPlaybackPolicyEvaluator::new(
            settings,
            session_coordinator.clone(),
            window_provider.clone(),
        );

        Self {
            _window_provider: window_provider,
            snapshot,
            session_coordinator,
            evaluator,
            display1_id,
            _display2_id: display2_id,
        }
    }
}

#[test]
fn test_evaluator_arbitration_reconciliation_multi_constraint() {
    let mut harness = TestArbitrationHarness::new_dual();
    let snapshot = harness.snapshot.clone();
    let did1 = &harness.display1_id;

    // Verify session 1 is active initially
    {
        let sc = harness.session_coordinator.lock().unwrap();
        let s1 = sc.get_session(did1).unwrap();
        assert_eq!(s1.state, DisplaySessionState::Active);
    }

    // Initial arbitration when unconstrained
    let initial_decision = harness.evaluator.arbitrate_display_session(did1);
    assert_eq!(initial_decision.desired_state, DesiredSessionState::Active);
    assert!(!initial_decision.requires_transition); // already active

    // 1. Apply Fullscreen pause
    let obs = kinetiwall_desktop::policy::ForegroundObservation {
        handle: NativeWindowHandle(501),
        process_id: 8888,
        thread_id: 8889,
        class_name: "GameWindow".to_string(),
        bounds: WindowRect::new(0, 0, 1920, 1080),
        is_visible: true,
        is_minimized: false,
        is_cloaked: false,
        timestamp_millis: 1000,
        observation_generation: 1,
    };
    harness.evaluator.evaluate_observation(
        Some(&obs),
        &snapshot,
        MultiMonitorLayoutMode::Independent,
        1000,
    );

    // D1 is now paused by fullscreen
    assert!(harness.evaluator.is_display_fullscreen_paused(did1));
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(
            sc.get_session(did1).unwrap().state,
            DisplaySessionState::Suspended
        );
    }

    // Check arbitration decision directly
    let fs_decision = harness.evaluator.arbitrate_display_session(did1);
    assert_eq!(fs_decision.desired_state, DesiredSessionState::Suspended);
    assert!(!fs_decision.requires_transition); // already suspended in coordinator
    assert!(fs_decision
        .explanation_codes
        .contains(&PolicyExplanationCode::FullscreenActive));

    // 2. Add second constraint: Workstation Locked
    let mut session_settings = SessionPlaybackSettings::default();
    session_settings.enabled = true;
    session_settings.pause_on_lock = true;
    harness
        .evaluator
        .update_session_settings(session_settings, &snapshot);

    let mut session_snap = WindowsSessionSnapshot::default();
    session_snap.lock_state = WorkstationLockState::Locked;
    harness
        .evaluator
        .evaluate_session_state(&session_snap, &snapshot);

    assert!(harness.evaluator.is_display_workstation_locked(did1));

    // 3. Clear Fullscreen constraint (window closed)
    harness.evaluator.evaluate_observation(
        None,
        &snapshot,
        MultiMonitorLayoutMode::Independent,
        2000,
    );
    assert!(!harness.evaluator.is_display_fullscreen_paused(did1));

    // CRITICAL: Session MUST STILL be suspended because workstation is locked!
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(
            sc.get_session(did1).unwrap().state,
            DisplaySessionState::Suspended
        );
    }

    let arb_locked = harness.evaluator.arbitrate_display_session(did1);
    assert_eq!(arb_locked.desired_state, DesiredSessionState::Suspended);
    assert!(arb_locked
        .explanation_codes
        .contains(&PolicyExplanationCode::WorkstationLocked));
    assert!(!arb_locked
        .explanation_codes
        .contains(&PolicyExplanationCode::FullscreenActive));

    // 4. Unlock workstation -> Now all constraints are cleared, session should resume
    session_snap.lock_state = WorkstationLockState::Unlocked;
    harness
        .evaluator
        .evaluate_session_state(&session_snap, &snapshot);
    assert!(!harness.evaluator.is_display_workstation_locked(did1));

    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(
            sc.get_session(did1).unwrap().state,
            DisplaySessionState::Active
        );
    }

    let final_decision = harness.evaluator.arbitrate_display_session(did1);
    assert_eq!(final_decision.desired_state, DesiredSessionState::Active);
    assert!(!final_decision.requires_transition);
}

#[test]
fn test_manual_pause_survives_automatic_policy_events() {
    let mut harness = TestArbitrationHarness::new_dual();
    let snapshot = harness.snapshot.clone();
    let did1 = &harness.display1_id;

    // User explicitly pauses D1
    harness.evaluator.register_user_pause(did1).unwrap();
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(
            sc.get_session(did1).unwrap().state,
            DisplaySessionState::Suspended
        );
    }

    // Automatic fullscreen event occurs
    let obs = kinetiwall_desktop::policy::ForegroundObservation {
        handle: NativeWindowHandle(501),
        process_id: 8888,
        thread_id: 8889,
        class_name: "GameWindow".to_string(),
        bounds: WindowRect::new(0, 0, 1920, 1080),
        is_visible: true,
        is_minimized: false,
        is_cloaked: false,
        timestamp_millis: 1000,
        observation_generation: 1,
    };
    harness.evaluator.evaluate_observation(
        Some(&obs),
        &snapshot,
        MultiMonitorLayoutMode::Independent,
        1000,
    );

    // Fullscreen clears
    harness.evaluator.evaluate_observation(
        None,
        &snapshot,
        MultiMonitorLayoutMode::Independent,
        2000,
    );

    // Session MUST remain suspended because manual pause is active
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(
            sc.get_session(did1).unwrap().state,
            DisplaySessionState::Suspended
        );
    }

    let decision = harness.evaluator.arbitrate_display_session(did1);
    assert_eq!(decision.desired_state, DesiredSessionState::Suspended);
    assert!(decision
        .explanation_codes
        .contains(&PolicyExplanationCode::ManualPauseActive));

    // Clear user pause -> Now session resumes
    harness.evaluator.clear_user_pause(did1).unwrap();
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(
            sc.get_session(did1).unwrap().state,
            DisplaySessionState::Active
        );
    }
}

#[test]
fn test_multi_monitor_per_display_arbitration_isolation() {
    let mut harness = TestArbitrationHarness::new_dual();
    let snapshot = harness.snapshot.clone();
    let did1 = harness.display1_id.clone();
    let did2 = harness.snapshot.displays[1].id.clone();

    // Verify both are Active initially
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(&did1).unwrap().state, DisplaySessionState::Active);
        assert_eq!(sc.get_session(&did2).unwrap().state, DisplaySessionState::Active);
    }

    // Trigger fullscreen only on display 2 (left: 1920, top: 0, right: 3840, bottom: 1080)
    let obs = kinetiwall_desktop::policy::ForegroundObservation {
        handle: NativeWindowHandle(601),
        process_id: 9999,
        thread_id: 9998,
        class_name: "GameWindowSecondary".to_string(),
        bounds: WindowRect::new(1920, 0, 3840, 1080),
        is_visible: true,
        is_minimized: false,
        is_cloaked: false,
        timestamp_millis: 1000,
        observation_generation: 1,
    };
    harness.evaluator.evaluate_observation(
        Some(&obs),
        &snapshot,
        MultiMonitorLayoutMode::Independent,
        1000,
    );

    // D1 should remain active, D2 should be paused
    assert!(!harness.evaluator.is_display_fullscreen_paused(&did1));
    assert!(harness.evaluator.is_display_fullscreen_paused(&did2));

    let dec1 = harness.evaluator.arbitrate_display_session(&did1);
    let dec2 = harness.evaluator.arbitrate_display_session(&did2);

    assert_eq!(dec1.desired_state, DesiredSessionState::Active);
    assert_eq!(dec2.desired_state, DesiredSessionState::Suspended);
    assert!(dec2.explanation_codes.contains(&PolicyExplanationCode::FullscreenActive));

    // Reconcile and verify coordinator state
    let reconciled = harness.evaluator.reconcile_arbitration(&snapshot);
    assert_eq!(reconciled.len(), 2);

    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(&did1).unwrap().state, DisplaySessionState::Active);
        assert_eq!(sc.get_session(&did2).unwrap().state, DisplaySessionState::Suspended);
    }
}

// -------------------------------------------------------------------------------------------------
// Step 57 Diagnostics & Explainability Tests
// -------------------------------------------------------------------------------------------------

#[test]
fn test_diagnostics_explanation_and_reason_code_mapping() {
    assert_eq!(
        PolicyExplanationCode::ManualPauseActive.description(),
        "Playback manually paused by user"
    );
    assert_eq!(
        PolicyExplanationCode::ManualPauseActive.reason_code(),
        "MANUAL_PAUSE_ACTIVE"
    );

    assert_eq!(
        PolicyExplanationCode::FullscreenActive.description(),
        "Suspended due to active foreground fullscreen application"
    );
    assert_eq!(
        PolicyExplanationCode::FullscreenActive.reason_code(),
        "FULLSCREEN_ACTIVE"
    );

    assert_eq!(
        PolicyExplanationCode::WorkstationLocked.description(),
        "Suspended because Windows workstation is locked"
    );
    assert_eq!(
        PolicyExplanationCode::WorkstationLocked.reason_code(),
        "WORKSTATION_LOCKED"
    );

    assert_eq!(
        PolicyExplanationCode::DisplayPowerOff.description(),
        "Suspended because display panel is powered off"
    );
    assert_eq!(
        PolicyExplanationCode::DisplayPowerOff.reason_code(),
        "DISPLAY_POWER_OFF"
    );

    assert_eq!(
        PolicyExplanationCode::UserInactivityIdle.description(),
        "Suspended due to user inactivity (session idle)"
    );
    assert_eq!(
        PolicyExplanationCode::UserInactivityIdle.reason_code(),
        "USER_INACTIVITY_IDLE"
    );

    assert_eq!(
        PolicyExplanationCode::StaticFallbackPresented.description(),
        "Displaying preserved static wallpaper fallback"
    );
    assert_eq!(
        PolicyExplanationCode::StaticFallbackPresented.reason_code(),
        "STATIC_FALLBACK_PRESENTED"
    );

    assert_eq!(
        PolicyExplanationCode::UnconstrainedActive.description(),
        "All constraints cleared; playback active"
    );
    assert_eq!(
        PolicyExplanationCode::UnconstrainedActive.reason_code(),
        "UNCONSTRAINED_ACTIVE"
    );
}

#[test]
fn test_diagnostics_actual_vs_desired_state_divergence() {
    let display_id = DisplayId::new("DISPLAY_1");
    let session_id = DisplaySessionId::new();

    let mut pause_reasons = HashSet::new();
    pause_reasons.insert(PauseReason::WorkstationLocked);

    // Backend is currently Active, but arbitrator requires Suspended
    let input = SessionArbitrationInput {
        display_id: display_id.clone(),
        session_id: Some(session_id.clone()),
        canonical_target_fps: 60,
        active_pause_reasons: pause_reasons,
        adaptive_fps_cap: None,
        budget_fps_cap: None,
        is_session_terminated: false,
        termination_reason: None,
        allows_static_fallback: false,
        default_fallback_source: None,
        current_actual_state: DisplaySessionState::Active,
    };

    let decision = PolicyArbitrator::arbitrate(&input);

    assert_eq!(decision.desired_state, DesiredSessionState::Suspended);
    assert_eq!(decision.actual_state, DisplaySessionState::Active);
    assert!(decision.is_state_divergent);
    assert!(decision.requires_transition);
    assert_eq!(decision.primary_reason_code, "WORKSTATION_LOCKED");
    assert_eq!(
        decision.primary_explanation,
        "Suspended because Windows workstation is locked"
    );
}

#[test]
fn test_evaluator_bounded_arbitration_diagnostic_ring_buffer() {
    let mut harness = TestArbitrationHarness::new_dual();
    let snapshot = harness.snapshot.clone();
    let did1 = &harness.display1_id;

    assert!(harness.evaluator.arbitration_diagnostics().is_empty());

    // Inject pause reason directly to test arbitration reconciliation transition
    harness.evaluator.register_user_pause(did1).unwrap();
    // Reconcile arbitration
    let _ = harness.evaluator.reconcile_arbitration(&snapshot);

    // Diagnostic event should now be recorded because a transition occurred or was reconciled
    // Now clear pause and reconcile: Suspended -> Active transition
    harness.evaluator.clear_user_pause(did1).unwrap();
    let decisions = harness.evaluator.reconcile_arbitration(&snapshot);
    assert!(!decisions.is_empty());

    let diags = harness.evaluator.arbitration_diagnostics();
    assert!(!diags.is_empty());
    let d1_diag = diags.iter().rev().find(|d| d.display_id == *did1).unwrap();
    assert_eq!(d1_diag.desired_state, DesiredSessionState::Active);
    assert_eq!(d1_diag.primary_reason_code, "UNCONSTRAINED_ACTIVE");
    assert_eq!(d1_diag.primary_explanation, "All constraints cleared; playback active");
}
