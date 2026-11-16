//! Comprehensive test suite for Windows session-lifecycle awareness, lock/unlock,
//! and remote-session playback policies (Step 53).

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
    DesktopSessionNotificationProvider, DisplayId, DisplaySessionCoordinator, DisplaySessionState,
    DisplaySnapshotManager, FullscreenPlaybackSettings, MockAssignmentRepository,
    MockDesktopDisplayProvider, MockDesktopSessionNotificationProvider, MockDesktopWindowProvider,
    MockWindowRecord, MultiMonitorLayoutCoordinator, NativeWindowHandle,
    SessionConnectionState, SessionNotificationEventKind, SessionNotificationScope,
    SessionPlaybackSettings, SessionPolicyEvaluation, SmartPlaybackPolicyEvaluator,
    WinSessionError, WindowsSessionSnapshot, WindowRect, WorkstationLockState,
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
// Test Policy Harness
// -------------------------------------------------------------------------------------------------

struct TestSessionPolicyHarness {
    _window_provider: Arc<MockDesktopWindowProvider>,
    snapshot: Arc<kinetiwall_desktop::DisplayTopologySnapshot>,
    session_coordinator: Arc<Mutex<DisplaySessionCoordinator>>,
    evaluator: SmartPlaybackPolicyEvaluator,
    display1_id: DisplayId,
    display2_id: DisplayId,
}

impl TestSessionPolicyHarness {
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

        let evaluator = SmartPlaybackPolicyEvaluator::new(
            FullscreenPlaybackSettings::default(),
            session_coordinator.clone(),
            window_provider.clone(),
        );

        Self {
            _window_provider: window_provider,
            snapshot,
            session_coordinator,
            evaluator,
            display1_id,
            display2_id,
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------------------------------

#[test]
fn test_wts_event_decoding() {
    let provider = MockDesktopSessionNotificationProvider::new();
    let now = 50_000;

    // 0x1: Console Connect
    let ev = provider.decode_session_message(0x1, 10, now).unwrap();
    assert_eq!(ev.kind, SessionNotificationEventKind::ConsoleConnect);
    assert_eq!(ev.session_id, 10);
    assert_eq!(ev.timestamp_millis, now);

    // 0x2: Console Disconnect
    let ev = provider.decode_session_message(0x2, 10, now).unwrap();
    assert_eq!(ev.kind, SessionNotificationEventKind::ConsoleDisconnect);

    // 0x3: Remote Connect
    let ev = provider.decode_session_message(0x3, 11, now).unwrap();
    assert_eq!(ev.kind, SessionNotificationEventKind::RemoteConnect);
    assert_eq!(ev.session_id, 11);

    // 0x4: Remote Disconnect
    let ev = provider.decode_session_message(0x4, 11, now).unwrap();
    assert_eq!(ev.kind, SessionNotificationEventKind::RemoteDisconnect);

    // 0x7: Session Lock
    let ev = provider.decode_session_message(0x7, 10, now).unwrap();
    assert_eq!(ev.kind, SessionNotificationEventKind::SessionLock);

    // 0x8: Session Unlock
    let ev = provider.decode_session_message(0x8, 10, now).unwrap();
    assert_eq!(ev.kind, SessionNotificationEventKind::SessionUnlock);

    // Unknown code 0x99
    let ev = provider.decode_session_message(0x99, 10, now).unwrap();
    assert_eq!(ev.kind, SessionNotificationEventKind::Unknown(0x99));
}

#[test]
fn test_session_state_model_initial_and_transitions() {
    let mut snapshot = WindowsSessionSnapshot::default();

    // Initial state: LockState must be Unknown (never guess at startup!)
    assert_eq!(snapshot.lock_state, WorkstationLockState::Unknown);
    assert_eq!(snapshot.connection_state, SessionConnectionState::Unknown);
    assert_eq!(snapshot.session_id, None);
    assert_eq!(snapshot.last_event, None);

    let provider = MockDesktopSessionNotificationProvider::new();

    // Event: Session Lock
    let lock_ev = provider.decode_session_message(0x7, 1, 10_000).unwrap();
    snapshot.apply_event(&lock_ev);
    assert_eq!(snapshot.lock_state, WorkstationLockState::Locked);
    assert_eq!(snapshot.session_id, Some(1));
    assert_eq!(snapshot.last_event, Some(SessionNotificationEventKind::SessionLock));
    assert_eq!(snapshot.last_event_timestamp_millis, 10_000);

    // Event: Remote Connect (must update connection state without altering lock state!)
    let rdp_ev = provider.decode_session_message(0x3, 1, 15_000).unwrap();
    snapshot.apply_event(&rdp_ev);
    assert_eq!(snapshot.lock_state, WorkstationLockState::Locked);
    assert_eq!(snapshot.connection_state, SessionConnectionState::RemoteConnected);

    // Event: Session Unlock
    let unlock_ev = provider.decode_session_message(0x8, 1, 20_000).unwrap();
    snapshot.apply_event(&unlock_ev);
    assert_eq!(snapshot.lock_state, WorkstationLockState::Unlocked);
    assert_eq!(snapshot.connection_state, SessionConnectionState::RemoteConnected);

    // Event: Console Connect
    let con_ev = provider.decode_session_message(0x1, 1, 25_000).unwrap();
    snapshot.apply_event(&con_ev);
    assert_eq!(snapshot.connection_state, SessionConnectionState::ConsoleConnected);
}

#[test]
fn test_session_policy_evaluation_pure() {
    let mut settings = SessionPlaybackSettings::default();
    assert!(!settings.enabled, "Session policy must be disabled by default!");

    let mut snapshot = WindowsSessionSnapshot::default();
    snapshot.lock_state = WorkstationLockState::Locked;

    // Disabled: never pauses
    let eval = SessionPolicyEvaluation::evaluate(&settings, &snapshot);
    assert!(!eval.should_pause);

    // Enabled with pause_on_lock: pauses on Locked
    settings.enabled = true;
    settings.pause_on_lock = true;
    let eval = SessionPolicyEvaluation::evaluate(&settings, &snapshot);
    assert!(eval.should_pause);
    assert!(eval.trigger_reason.unwrap().contains("locked"));

    // Unlocked: does not pause
    snapshot.lock_state = WorkstationLockState::Unlocked;
    let eval = SessionPolicyEvaluation::evaluate(&settings, &snapshot);
    assert!(!eval.should_pause);

    // Unknown lock state: does not pause
    snapshot.lock_state = WorkstationLockState::Unknown;
    let eval = SessionPolicyEvaluation::evaluate(&settings, &snapshot);
    assert!(!eval.should_pause);

    // Remote session optimization
    settings.optimize_remote_session = true;
    snapshot.connection_state = SessionConnectionState::RemoteConnected;
    let eval = SessionPolicyEvaluation::evaluate(&settings, &snapshot);
    assert!(eval.should_pause);
    assert!(eval.trigger_reason.unwrap().contains("Remote Desktop"));
}

#[test]
fn test_lock_and_unlock_lifecycle_with_coordinator() {
    let mut harness = TestSessionPolicyHarness::new_dual();
    let snapshot = &harness.snapshot;

    let mut settings = SessionPlaybackSettings::default();
    settings.enabled = true;
    settings.pause_on_lock = true;
    harness.evaluator.update_session_settings(settings, snapshot);

    let did1 = &harness.display1_id;
    let did2 = &harness.display2_id;

    // Verify initial active state
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(did1).unwrap().state, DisplaySessionState::Active);
        assert_eq!(sc.get_session(did2).unwrap().state, DisplaySessionState::Active);
    }

    // 1. Workstation Lock occurs
    harness.evaluator.handle_workstation_lock(snapshot);

    // Both displays must be paused with PauseReason::WorkstationLocked
    assert!(harness.evaluator.is_display_workstation_locked(did1));
    assert!(harness.evaluator.is_display_workstation_locked(did2));

    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(did1).unwrap().state, DisplaySessionState::Suspended);
        assert_eq!(sc.get_session(did2).unwrap().state, DisplaySessionState::Suspended);
    }

    // 2. Workstation Unlock occurs
    harness.evaluator.handle_workstation_unlock(snapshot);

    // Both displays must be resumed
    assert!(!harness.evaluator.is_display_workstation_locked(did1));
    assert!(!harness.evaluator.is_display_workstation_locked(did2));

    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(did1).unwrap().state, DisplaySessionState::Active);
        assert_eq!(sc.get_session(did2).unwrap().state, DisplaySessionState::Active);
    }
}

#[test]
fn test_manual_pause_interlock_preserved_on_unlock() {
    let mut harness = TestSessionPolicyHarness::new_dual();
    let snapshot = &harness.snapshot;

    let mut settings = SessionPlaybackSettings::default();
    settings.enabled = true;
    settings.pause_on_lock = true;
    harness.evaluator.update_session_settings(settings, snapshot);

    let did1 = &harness.display1_id;
    let did2 = &harness.display2_id;

    // User manually pauses Display 1
    harness.evaluator.set_user_manual_pause(did1, true).unwrap();
    assert!(harness.evaluator.is_display_manually_paused(did1));

    // Workstation Lock occurs
    harness.evaluator.handle_workstation_lock(snapshot);
    assert!(harness.evaluator.is_display_workstation_locked(did1));
    assert!(harness.evaluator.is_display_workstation_locked(did2));

    // Workstation Unlock occurs
    harness.evaluator.handle_workstation_unlock(snapshot);

    // WorkstationLocked reason is cleared for both
    assert!(!harness.evaluator.is_display_workstation_locked(did1));
    assert!(!harness.evaluator.is_display_workstation_locked(did2));

    // DISPLAY 2 resumes to Active because no other pause reasons exist
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(did2).unwrap().state, DisplaySessionState::Active);
        // DISPLAY 1 MUST REMAIN SUSPENDED because UserManual pause is still active!
        assert_eq!(sc.get_session(did1).unwrap().state, DisplaySessionState::Suspended);
    }
    assert!(harness.evaluator.is_display_manually_paused(did1));

    // When user explicitly resumes Display 1, it finally becomes Active
    harness.evaluator.set_user_manual_pause(did1, false).unwrap();
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(did1).unwrap().state, DisplaySessionState::Active);
    }
}

#[test]
fn test_disabling_session_policy_while_locked_clears_pause() {
    let mut harness = TestSessionPolicyHarness::new_dual();
    let snapshot = &harness.snapshot;

    let did1 = &harness.display1_id;

    let mut settings = SessionPlaybackSettings::default();
    settings.enabled = true;
    settings.pause_on_lock = true;
    harness.evaluator.update_session_settings(settings, snapshot);

    harness.evaluator.handle_workstation_lock(snapshot);
    assert!(harness.evaluator.is_display_workstation_locked(did1));

    // Disable session policy while still locked
    let mut disabled_settings = SessionPlaybackSettings::default();
    disabled_settings.enabled = false;
    harness.evaluator.update_session_settings(disabled_settings, snapshot);

    // Policy constraint is lifted immediately and session resumes
    assert!(!harness.evaluator.is_display_workstation_locked(did1));
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(did1).unwrap().state, DisplaySessionState::Active);
    }
}

#[test]
fn test_remote_session_optimization_lifecycle() {
    let mut harness = TestSessionPolicyHarness::new_dual();
    let snapshot = &harness.snapshot;

    let mut settings = SessionPlaybackSettings::default();
    settings.enabled = true;
    settings.optimize_remote_session = true;
    harness.evaluator.update_session_settings(settings, snapshot);

    let did1 = &harness.display1_id;

    let mut session_snap = WindowsSessionSnapshot::default();
    session_snap.connection_state = SessionConnectionState::RemoteConnected;

    // Remote session connected -> pauses
    harness.evaluator.evaluate_session_state(&session_snap, snapshot);
    assert!(harness.evaluator.is_display_remote_session_paused(did1));
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(did1).unwrap().state, DisplaySessionState::Suspended);
    }

    // Remote session disconnects -> resumes
    session_snap.connection_state = SessionConnectionState::ConsoleConnected;
    harness.evaluator.evaluate_session_state(&session_snap, snapshot);
    assert!(!harness.evaluator.is_display_remote_session_paused(did1));
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(did1).unwrap().state, DisplaySessionState::Active);
    }
}

#[test]
fn test_registration_lifecycle() {
    let provider = MockDesktopSessionNotificationProvider::new();
    let hwnd = NativeWindowHandle::from_raw(0xABC1 as _);

    // Initial state: 0 registrations
    assert_eq!(provider.registered_handles_count(), 0);

    // Register
    let mut reg = provider
        .register_session_notifications(hwnd, SessionNotificationScope::ThisSession)
        .expect("Registration must succeed");
    assert!(reg.is_registered);
    assert_eq!(reg.hwnd, Some(hwnd));
    assert_eq!(reg.scope, SessionNotificationScope::ThisSession);
    assert_eq!(provider.registered_handles_count(), 1);

    // Duplicate registration for same hwnd must be rejected
    let dup_err = provider.register_session_notifications(hwnd, SessionNotificationScope::ThisSession);
    assert!(matches!(dup_err, Err(WinSessionError::AlreadyRegistered(_))));

    // Unregister
    provider
        .unregister_session_notifications(&mut reg)
        .expect("Unregistration must succeed");
    assert!(!reg.is_registered);
    assert_eq!(reg.hwnd, None);
    assert_eq!(provider.registered_handles_count(), 0);

    // Redundant unregistration is safe no-op
    assert!(provider.unregister_session_notifications(&mut reg).is_ok());
}

#[test]
fn test_registration_failure_handling() {
    let provider = MockDesktopSessionNotificationProvider::new();
    provider.set_fail_registration(true);
    let hwnd = NativeWindowHandle::from_raw(0xABC2 as _);

    let err = provider
        .register_session_notifications(hwnd, SessionNotificationScope::ThisSession)
        .unwrap_err();

    assert!(matches!(err, WinSessionError::RegistrationFailed { code: 5 }));
}
