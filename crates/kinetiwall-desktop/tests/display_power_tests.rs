//! Comprehensive test suite for Windows display-power awareness, screen-off rendering suspension,
//! and safe wake recovery policies (Step 54).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use kinetiwall_catalog::{
    AssetReference, AvailabilityState, CatalogError, CatalogLifecycle, WallpaperCatalogRepository,
    WallpaperId, WallpaperMetadata, WallpaperRecord, WallpaperSourceKind, WallpaperType,
};
use kinetiwall_desktop::attachment::constants;
use kinetiwall_desktop::{
    decode_display_power_value, AssignmentService, AttachmentCoordinator, CandidateRole,
    DesktopCandidateWindow, DesktopDisplayPowerProvider, DisplayDimmedAction, DisplayId,
    DisplayPowerError, DisplayPowerPlaybackSettings, DisplayPowerPolicyEvaluation,
    DisplayPowerScope, DisplayPowerSnapshot, DisplayPowerState, DisplaySessionCoordinator,
    DisplaySessionState, DisplaySnapshotManager, FullscreenPlaybackSettings,
    MockAssignmentRepository, MockDesktopDisplayProvider, MockDesktopDisplayPowerProvider,
    MockDesktopWindowProvider, MockWindowRecord, MultiMonitorLayoutCoordinator,
    NativeWindowHandle, SmartPlaybackPolicyEvaluator, WindowRect,
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

struct TestDisplayPowerHarness {
    _window_provider: Arc<MockDesktopWindowProvider>,
    snapshot: Arc<kinetiwall_desktop::DisplayTopologySnapshot>,
    session_coordinator: Arc<Mutex<DisplaySessionCoordinator>>,
    evaluator: SmartPlaybackPolicyEvaluator,
    display1_id: DisplayId,
    display2_id: DisplayId,
}

impl TestDisplayPowerHarness {
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
fn test_decode_display_power_values() {
    assert_eq!(decode_display_power_value(0), DisplayPowerState::Off);
    assert_eq!(decode_display_power_value(1), DisplayPowerState::On);
    assert_eq!(decode_display_power_value(2), DisplayPowerState::Dimmed);
    assert_eq!(decode_display_power_value(3), DisplayPowerState::Unknown);
    assert_eq!(decode_display_power_value(99), DisplayPowerState::Unknown);

    assert!(DisplayPowerState::Off.is_off());
    assert!(!DisplayPowerState::Off.is_on());
    assert!(DisplayPowerState::On.is_on());
    assert!(DisplayPowerState::Dimmed.is_dimmed());
    assert!(!DisplayPowerState::Unknown.is_known());
}

#[test]
fn test_display_power_snapshot_defaults_and_construction() {
    let default_snap = DisplayPowerSnapshot::default();
    assert_eq!(default_snap.state, DisplayPowerState::Unknown);
    assert_eq!(default_snap.scope, DisplayPowerScope::SessionDisplayStatus);
    assert_eq!(default_snap.sampled_at_millis, 0);
    assert!(!default_snap.is_stale);

    let verified = DisplayPowerSnapshot::new(
        DisplayPowerState::On,
        DisplayPowerScope::SessionDisplayStatus,
        12345,
    );
    assert_eq!(verified.state, DisplayPowerState::On);
    assert_eq!(verified.sampled_at_millis, 12345);
}

#[test]
fn test_display_power_policy_evaluation_disabled_by_default() {
    let settings = DisplayPowerPlaybackSettings::default();
    assert!(!settings.enabled, "Policy must be disabled by default");

    let off_snapshot = DisplayPowerSnapshot::new(
        DisplayPowerState::Off,
        DisplayPowerScope::SessionDisplayStatus,
        1000,
    );

    let eval = DisplayPowerPolicyEvaluation::evaluate(&settings, &off_snapshot);
    assert!(!eval.should_pause, "Disabled policy must never trigger pause");
    assert!(eval.trigger_reason.is_none());
}

#[test]
fn test_mock_provider_registration_and_unregistration() {
    let mut provider = MockDesktopDisplayPowerProvider::new(DisplayPowerScope::SessionDisplayStatus);
    assert!(!provider.is_registered());
    assert_eq!(provider.registered_handles_count(), 0);

    // Invalid HWND fails
    let err = provider.register_display_power_notification(0, DisplayPowerScope::SessionDisplayStatus);
    assert!(matches!(err, Err(DisplayPowerError::InvalidWindowHandle)));

    // Valid HWND succeeds
    let res = provider.register_display_power_notification(0x1234, DisplayPowerScope::SessionDisplayStatus);
    assert!(res.is_ok());
    assert!(provider.is_registered());
    assert_eq!(provider.registered_handles_count(), 1);

    // Unregister succeeds
    let unres = provider.unregister_display_power_notification(0x1234);
    assert!(unres.is_ok());
    assert!(!provider.is_registered());
    assert_eq!(provider.registered_handles_count(), 0);

    // Registration failure simulation
    provider.set_registration_failure(true);
    let fail_res = provider.register_display_power_notification(0x5678, DisplayPowerScope::SessionDisplayStatus);
    assert!(matches!(fail_res, Err(DisplayPowerError::RegistrationFailed { .. })));
}

#[test]
fn test_display_power_off_suspends_sessions_when_enabled() {
    let mut harness = TestDisplayPowerHarness::new_dual();

    // Verify initial state: both sessions active
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(&harness.display1_id).unwrap().state, DisplaySessionState::Active);
        assert_eq!(sc.get_session(&harness.display2_id).unwrap().state, DisplaySessionState::Active);
    }

    // Enable display-power policy
    let mut settings = DisplayPowerPlaybackSettings::default();
    settings.enabled = true;
    settings.pause_on_display_off = true;
    harness.evaluator.update_display_power_settings(settings, &harness.snapshot);

    // Trigger display off
    let off_snapshot = DisplayPowerSnapshot::new(
        DisplayPowerState::Off,
        DisplayPowerScope::SessionDisplayStatus,
        2000,
    );
    harness.evaluator.evaluate_display_power_state(&off_snapshot, &harness.snapshot);

    // Verify both sessions are suspended with DisplayPowerOff pause reason
    assert!(harness.evaluator.is_display_power_off_paused(&harness.display1_id));
    assert!(harness.evaluator.is_display_power_off_paused(&harness.display2_id));

    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(&harness.display1_id).unwrap().state, DisplaySessionState::Suspended);
        assert_eq!(sc.get_session(&harness.display2_id).unwrap().state, DisplaySessionState::Suspended);
    }
}

#[test]
fn test_display_power_on_resumes_sessions() {
    let mut harness = TestDisplayPowerHarness::new_dual();

    let mut settings = DisplayPowerPlaybackSettings::default();
    settings.enabled = true;
    harness.evaluator.update_display_power_settings(settings, &harness.snapshot);

    // 1. Off -> Suspended
    harness.evaluator.handle_display_off(&harness.snapshot);
    assert!(harness.evaluator.is_display_power_off_paused(&harness.display1_id));

    // 2. On -> Resumed
    harness.evaluator.handle_display_on(&harness.snapshot);
    assert!(!harness.evaluator.is_display_power_off_paused(&harness.display1_id));
    assert!(!harness.evaluator.is_display_power_off_paused(&harness.display2_id));

    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(&harness.display1_id).unwrap().state, DisplaySessionState::Active);
        assert_eq!(sc.get_session(&harness.display2_id).unwrap().state, DisplaySessionState::Active);
    }
}

#[test]
fn test_pause_reason_independence_user_manual_and_display_power() {
    let mut harness = TestDisplayPowerHarness::new_dual();

    let mut settings = DisplayPowerPlaybackSettings::default();
    settings.enabled = true;
    harness.evaluator.update_display_power_settings(settings, &harness.snapshot);

    // User manually pauses Display 1
    let _ = harness.evaluator.set_user_manual_pause(&harness.display1_id, true);
    assert!(harness.evaluator.is_display_manually_paused(&harness.display1_id));
    assert!(!harness.evaluator.is_display_manually_paused(&harness.display2_id));

    // Display powers off: both displays receive display-power-off pause
    harness.evaluator.handle_display_off(&harness.snapshot);
    assert!(harness.evaluator.is_display_power_off_paused(&harness.display1_id));
    assert!(harness.evaluator.is_display_power_off_paused(&harness.display2_id));

    // Display returns to On:
    // Display 2 has NO surviving pause reasons -> resumes to Active
    // Display 1 STILL has UserManual pause reason -> stays Suspended!
    harness.evaluator.handle_display_on(&harness.snapshot);
    assert!(!harness.evaluator.is_display_power_off_paused(&harness.display1_id));
    assert!(!harness.evaluator.is_display_power_off_paused(&harness.display2_id));

    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(
            sc.get_session(&harness.display1_id).unwrap().state,
            DisplaySessionState::Suspended,
            "Display 1 must remain suspended due to manual pause"
        );
        assert_eq!(
            sc.get_session(&harness.display2_id).unwrap().state,
            DisplaySessionState::Active,
            "Display 2 has no other pause reasons and must resume"
        );
    }

    // Now user resumes Display 1 -> returns to Active
    let _ = harness.evaluator.set_user_manual_pause(&harness.display1_id, false);
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(
            sc.get_session(&harness.display1_id).unwrap().state,
            DisplaySessionState::Active
        );
    }
}

#[test]
fn test_display_dimmed_policy_configurable_action() {
    let mut harness = TestDisplayPowerHarness::new_dual();

    // 1. Dimmed with default DoNothing action -> does NOT pause
    let mut settings = DisplayPowerPlaybackSettings::default();
    settings.enabled = true;
    settings.dimmed_action = DisplayDimmedAction::DoNothing;
    harness.evaluator.update_display_power_settings(settings, &harness.snapshot);

    let dimmed_snap = DisplayPowerSnapshot::new(
        DisplayPowerState::Dimmed,
        DisplayPowerScope::SessionDisplayStatus,
        3000,
    );
    harness.evaluator.evaluate_display_power_state(&dimmed_snap, &harness.snapshot);

    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(&harness.display1_id).unwrap().state, DisplaySessionState::Active);
    }

    // 2. Dimmed with Pause action -> suspends playback
    let mut settings_pause = DisplayPowerPlaybackSettings::default();
    settings_pause.enabled = true;
    settings_pause.dimmed_action = DisplayDimmedAction::Pause;
    harness.evaluator.update_display_power_settings(settings_pause, &harness.snapshot);

    harness.evaluator.evaluate_display_power_state(&dimmed_snap, &harness.snapshot);

    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(&harness.display1_id).unwrap().state, DisplaySessionState::Suspended);
    }
}

#[test]
fn test_display_power_policy_toggle_reconciles_active_pauses() {
    let mut harness = TestDisplayPowerHarness::new_dual();

    // Enable policy and trigger display off
    let mut settings = DisplayPowerPlaybackSettings::default();
    settings.enabled = true;
    harness.evaluator.update_display_power_settings(settings, &harness.snapshot);
    harness.evaluator.handle_display_off(&harness.snapshot);

    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(&harness.display1_id).unwrap().state, DisplaySessionState::Suspended);
    }

    // User disables the policy while display is still reported off
    let mut disabled_settings = DisplayPowerPlaybackSettings::default();
    disabled_settings.enabled = false;
    harness.evaluator.update_display_power_settings(disabled_settings, &harness.snapshot);

    // Active pause must be reconciled immediately and sessions resumed!
    assert!(!harness.evaluator.is_display_power_off_paused(&harness.display1_id));
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(&harness.display1_id).unwrap().state, DisplaySessionState::Active);
    }
}
