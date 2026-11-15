//! Unit and integration tests for power-state awareness, battery-sensitive playback,
//! and safe suspension policies (Step 49).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use kinetiwall_catalog::{
    AssetReference, AvailabilityState, CatalogError, CatalogLifecycle, WallpaperCatalogRepository,
    WallpaperId, WallpaperMetadata, WallpaperRecord, WallpaperSourceKind, WallpaperType,
};
use kinetiwall_desktop::attachment::constants;
use kinetiwall_desktop::{
    AssignmentService, AttachmentCoordinator, BatteryAvailability, BatteryChargingState,
    BatterySaverState, CandidateRole, DesktopCandidateWindow, DesktopPowerProvider, DisplayId,
    DisplaySessionCoordinator, DisplaySessionState, DisplaySnapshotManager,
    FullscreenPlaybackSettings, MockAssignmentRepository, MockDesktopDisplayProvider,
    MockDesktopPowerProvider, MockDesktopWindowProvider, MockWindowRecord,
    MultiMonitorLayoutCoordinator, NativeWindowHandle, PowerError,
    PowerPlaybackSettings, PowerPolicyEvaluation, PowerPolicyMode, PowerSource,
    PowerStatusQuality, SmartPlaybackPolicyEvaluator, SystemPowerSnapshot, WindowRect,
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
// Test Harness
// -------------------------------------------------------------------------------------------------

struct TestPowerHarness {
    window_provider: Arc<MockDesktopWindowProvider>,
    _display_provider: Arc<MockDesktopDisplayProvider>,
    snapshot: Arc<kinetiwall_desktop::DisplayTopologySnapshot>,
    session_coordinator: Arc<Mutex<DisplaySessionCoordinator>>,
    display1_id: DisplayId,
    display2_id: DisplayId,
    owning_pid: u32,
    _power_provider: MockDesktopPowerProvider,
}

impl TestPowerHarness {
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

        let power_provider = MockDesktopPowerProvider::new();

        Self {
            window_provider,
            _display_provider: display_provider,
            snapshot,
            session_coordinator,
            display1_id,
            display2_id,
            owning_pid,
            _power_provider: power_provider,
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------------------------------

#[test]
fn test_power_snapshot_desktop_ac() {
    let snap = SystemPowerSnapshot::desktop_ac(1000);
    assert_eq!(snap.power_source, PowerSource::Ac);
    assert_eq!(snap.battery_availability, BatteryAvailability::NoBattery);
    assert_eq!(snap.battery_percent, None);
    assert_eq!(snap.charging_state, BatteryChargingState::NoBattery);
    assert_eq!(snap.battery_saver, BatterySaverState::Disabled);
    assert!(!snap.is_on_battery());
    assert!(!snap.is_battery_saver_active());

    // Evaluated with SaveBattery mode: must NOT pause desktop AC
    let settings = PowerPlaybackSettings {
        mode: PowerPolicyMode::SaveBattery,
        pause_on_battery: true,
        pause_on_battery_saver: true,
        low_battery_threshold_percent: Some(20),
        pause_on_system_suspend: true,
        enable_static_fallback: true,
    };
    let eval = PowerPolicyEvaluation::evaluate(&settings, &snap);
    assert!(!eval.should_pause);
}

#[test]
fn test_power_snapshot_laptop_battery_and_charging() {
    // 1. Laptop on battery (80%, no saver)
    let battery_snap = SystemPowerSnapshot::laptop_battery(80, false, 2000);
    assert_eq!(battery_snap.power_source, PowerSource::Battery);
    assert_eq!(battery_snap.battery_availability, BatteryAvailability::Present);
    assert_eq!(battery_snap.battery_percent, Some(80));
    assert_eq!(battery_snap.charging_state, BatteryChargingState::Discharging);
    assert_eq!(battery_snap.battery_saver, BatterySaverState::Disabled);
    assert!(battery_snap.is_on_battery());
    assert!(!battery_snap.is_battery_saver_active());

    // 2. Laptop charging on AC (80%)
    let charging_snap = SystemPowerSnapshot::laptop_charging(80, 3000);
    assert_eq!(charging_snap.power_source, PowerSource::Ac);
    assert_eq!(charging_snap.battery_availability, BatteryAvailability::Present);
    assert_eq!(charging_snap.battery_percent, Some(80));
    assert_eq!(charging_snap.charging_state, BatteryChargingState::Charging);
    assert!(!charging_snap.is_on_battery());
}

#[test]
fn test_power_snapshot_unknown_handling() {
    let snap = SystemPowerSnapshot::default();
    assert_eq!(snap.power_source, PowerSource::Unknown);
    assert_eq!(snap.battery_availability, BatteryAvailability::Unknown);
    assert_eq!(snap.battery_percent, None);
    assert_eq!(snap.charging_state, BatteryChargingState::Unknown);
    assert_eq!(snap.battery_saver, BatterySaverState::Unknown);
    assert_eq!(snap.status_quality, PowerStatusQuality::Uninitialized);

    // Policy evaluation with unknown values must not cause false-positive pauses
    let settings = PowerPlaybackSettings {
        mode: PowerPolicyMode::SaveBattery,
        pause_on_battery: true,
        pause_on_battery_saver: true,
        low_battery_threshold_percent: Some(20),
        pause_on_system_suspend: true,
        enable_static_fallback: true,
    };
    let eval = PowerPolicyEvaluation::evaluate(&settings, &snap);
    assert!(!eval.should_pause);
}

#[test]
fn test_power_policy_evaluation_off_mode() {
    // Even if device is at 5% battery with Battery Saver enabled, Off mode NEVER pauses
    let low_battery_snap = SystemPowerSnapshot::laptop_battery(5, true, 4000);
    let off_settings = PowerPlaybackSettings {
        mode: PowerPolicyMode::Off,
        pause_on_battery: true,
        pause_on_battery_saver: true,
        low_battery_threshold_percent: Some(20),
        pause_on_system_suspend: true,
        enable_static_fallback: true,
    };

    let eval = PowerPolicyEvaluation::evaluate(&off_settings, &low_battery_snap);
    assert!(!eval.should_pause);
    assert_eq!(eval.trigger_reason, None);
}

#[test]
fn test_power_policy_evaluation_balanced_mode() {
    let balanced_settings = PowerPlaybackSettings {
        mode: PowerPolicyMode::Balanced,
        pause_on_battery: true, // In Balanced mode, mere battery does not pause!
        pause_on_battery_saver: true,
        low_battery_threshold_percent: Some(20),
        pause_on_system_suspend: true,
        enable_static_fallback: true,
    };

    // Case 1: On battery at 50%, no saver -> does NOT pause
    let normal_battery = SystemPowerSnapshot::laptop_battery(50, false, 1000);
    let eval1 = PowerPolicyEvaluation::evaluate(&balanced_settings, &normal_battery);
    assert!(!eval1.should_pause);

    // Case 2: On battery at 20% (threshold met) -> PAUSES
    let threshold_battery = SystemPowerSnapshot::laptop_battery(20, false, 2000);
    let eval2 = PowerPolicyEvaluation::evaluate(&balanced_settings, &threshold_battery);
    assert!(eval2.should_pause);
    assert!(eval2.trigger_reason.unwrap().contains("20%"));

    // Case 3: Battery Saver active -> PAUSES
    let saver_battery = SystemPowerSnapshot::laptop_battery(75, true, 3000);
    let eval3 = PowerPolicyEvaluation::evaluate(&balanced_settings, &saver_battery);
    assert!(eval3.should_pause);
    assert!(eval3.trigger_reason.unwrap().contains("Battery Saver"));
}

#[test]
fn test_power_policy_evaluation_save_battery_mode() {
    let save_battery_settings = PowerPlaybackSettings {
        mode: PowerPolicyMode::SaveBattery,
        pause_on_battery: true,
        pause_on_battery_saver: true,
        low_battery_threshold_percent: Some(20),
        pause_on_system_suspend: true,
        enable_static_fallback: true,
    };

    // Case 1: Switching to battery power (e.g. 90%) -> PAUSES
    let battery_snap = SystemPowerSnapshot::laptop_battery(90, false, 1000);
    let eval = PowerPolicyEvaluation::evaluate(&save_battery_settings, &battery_snap);
    assert!(eval.should_pause);
    assert!(eval.trigger_reason.unwrap().contains("battery power"));

    // Case 2: Plugged into AC -> DOES NOT PAUSE
    let ac_snap = SystemPowerSnapshot::laptop_charging(90, 2000);
    let eval_ac = PowerPolicyEvaluation::evaluate(&save_battery_settings, &ac_snap);
    assert!(!eval_ac.should_pause);
}

#[test]
fn test_power_evaluator_live_pause_and_resume() {
    let harness = TestPowerHarness::new_dual();

    let mut evaluator = SmartPlaybackPolicyEvaluator::with_power_settings(
        FullscreenPlaybackSettings::default(),
        PowerPlaybackSettings {
            mode: PowerPolicyMode::SaveBattery,
            pause_on_battery: true,
            pause_on_battery_saver: true,
            low_battery_threshold_percent: Some(20),
            pause_on_system_suspend: true,
            enable_static_fallback: true,
        },
        harness.session_coordinator.clone(),
        harness.window_provider.clone(),
    );
    evaluator.register_kinetiwall_pid(harness.owning_pid);

    // Initial state: AC charging -> sessions Active
    let ac_snap = SystemPowerSnapshot::laptop_charging(85, 1000);
    evaluator.evaluate_power_state(&ac_snap, &harness.snapshot);
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(&harness.display1_id).unwrap().state, DisplaySessionState::Active);
        assert_eq!(sc.get_session(&harness.display2_id).unwrap().state, DisplaySessionState::Active);
    }
    assert!(!evaluator.is_display_power_paused(&harness.display1_id));

    // Power event: Unplugged, running on battery -> sessions Suspended
    let battery_snap = SystemPowerSnapshot::laptop_battery(85, false, 2000);
    evaluator.evaluate_power_state(&battery_snap, &harness.snapshot);
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(&harness.display1_id).unwrap().state, DisplaySessionState::Suspended);
        assert_eq!(sc.get_session(&harness.display2_id).unwrap().state, DisplaySessionState::Suspended);
    }
    assert!(evaluator.is_display_power_paused(&harness.display1_id));

    // Power event: Plugged back in to AC -> sessions Resumed
    evaluator.evaluate_power_state(&ac_snap, &harness.snapshot);
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(&harness.display1_id).unwrap().state, DisplaySessionState::Active);
        assert_eq!(sc.get_session(&harness.display2_id).unwrap().state, DisplaySessionState::Active);
    }
    assert!(!evaluator.is_display_power_paused(&harness.display1_id));
}

#[test]
fn test_power_policy_independent_reason_tracking_respects_manual_pause() {
    let harness = TestPowerHarness::new_dual();

    let mut evaluator = SmartPlaybackPolicyEvaluator::with_power_settings(
        FullscreenPlaybackSettings::default(),
        PowerPlaybackSettings {
            mode: PowerPolicyMode::SaveBattery,
            pause_on_battery: true,
            pause_on_battery_saver: true,
            low_battery_threshold_percent: Some(20),
            pause_on_system_suspend: true,
            enable_static_fallback: true,
        },
        harness.session_coordinator.clone(),
        harness.window_provider.clone(),
    );

    // 1. User manually pauses Display 1
    evaluator.register_user_pause(&harness.display1_id).unwrap();

    // 2. Power switches to battery -> PowerSaving pause added
    let battery_snap = SystemPowerSnapshot::laptop_battery(50, false, 1000);
    evaluator.evaluate_power_state(&battery_snap, &harness.snapshot);

    // 3. Power returns to AC -> PowerSaving cleared
    let ac_snap = SystemPowerSnapshot::laptop_charging(50, 2000);
    evaluator.evaluate_power_state(&ac_snap, &harness.snapshot);

    // CRITICAL: Display 1 must NOT resume because UserManual is still active!
    // Display 2 was not manually paused, so it MUST resume!
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(
            sc.get_session(&harness.display1_id).unwrap().state,
            DisplaySessionState::Suspended,
            "Display 1 must remain suspended due to UserManual pause"
        );
        assert_eq!(
            sc.get_session(&harness.display2_id).unwrap().state,
            DisplaySessionState::Active,
            "Display 2 must resume on AC power"
        );
    }

    // Explicit manual resume of Display 1 restores it
    evaluator.clear_user_pause(&harness.display1_id).unwrap();
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(&harness.display1_id).unwrap().state, DisplaySessionState::Active);
    }
}

#[test]
fn test_power_policy_fullscreen_and_power_interlock() {
    let harness = TestPowerHarness::new_dual();

    let mut evaluator = SmartPlaybackPolicyEvaluator::with_power_settings(
        FullscreenPlaybackSettings {
            enabled: true,
            pause_mode: kinetiwall_desktop::FullscreenPauseMode::PerDisplay,
            debounce_duration_millis: 0,
        },
        PowerPlaybackSettings {
            mode: PowerPolicyMode::SaveBattery,
            pause_on_battery: true,
            pause_on_battery_saver: true,
            low_battery_threshold_percent: Some(20),
            pause_on_system_suspend: true,
            enable_static_fallback: true,
        },
        harness.session_coordinator.clone(),
        harness.window_provider.clone(),
    );
    evaluator.register_kinetiwall_pid(harness.owning_pid);

    // 1. Fullscreen game on Display 1
    let game_obs = kinetiwall_desktop::ForegroundObservation {
        handle: NativeWindowHandle(999),
        process_id: 7777,
        thread_id: 7778,
        class_name: "UnrealEngine".to_string(),
        bounds: WindowRect::new(0, 0, 1920, 1080),
        is_visible: true,
        is_minimized: false,
        is_cloaked: false,
        timestamp_millis: 100,
        observation_generation: 1,
    };
    evaluator.evaluate_observation(
        Some(&game_obs),
        &harness.snapshot,
        kinetiwall_desktop::MultiMonitorLayoutMode::Independent,
        100,
    );
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(&harness.display1_id).unwrap().state, DisplaySessionState::Suspended);
        assert_eq!(sc.get_session(&harness.display2_id).unwrap().state, DisplaySessionState::Active);
    }

    // 2. Battery power trigger: both displays now pause
    let battery_snap = SystemPowerSnapshot::laptop_battery(60, false, 200);
    evaluator.evaluate_power_state(&battery_snap, &harness.snapshot);
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(&harness.display1_id).unwrap().state, DisplaySessionState::Suspended);
        assert_eq!(sc.get_session(&harness.display2_id).unwrap().state, DisplaySessionState::Suspended);
    }

    // 3. AC power connected: PowerSaving cleared.
    // Display 1 must remain suspended because Fullscreen is still active!
    // Display 2 resumes!
    let ac_snap = SystemPowerSnapshot::laptop_charging(60, 300);
    evaluator.evaluate_power_state(&ac_snap, &harness.snapshot);
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(
            sc.get_session(&harness.display1_id).unwrap().state,
            DisplaySessionState::Suspended,
            "Display 1 must remain suspended due to active fullscreen app"
        );
        assert_eq!(
            sc.get_session(&harness.display2_id).unwrap().state,
            DisplaySessionState::Active,
            "Display 2 resumes"
        );
    }

    // 4. Fullscreen game closes: Display 1 now resumes!
    evaluator.evaluate_observation(
        None,
        &harness.snapshot,
        kinetiwall_desktop::MultiMonitorLayoutMode::Independent,
        400,
    );
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(&harness.display1_id).unwrap().state, DisplaySessionState::Active);
        assert_eq!(sc.get_session(&harness.display2_id).unwrap().state, DisplaySessionState::Active);
    }
}

#[test]
fn test_system_suspend_and_resume_lifecycle() {
    let harness = TestPowerHarness::new_dual();

    let mut evaluator = SmartPlaybackPolicyEvaluator::with_power_settings(
        FullscreenPlaybackSettings::default(),
        PowerPlaybackSettings {
            mode: PowerPolicyMode::SaveBattery,
            pause_on_battery: true,
            pause_on_battery_saver: true,
            low_battery_threshold_percent: Some(20),
            pause_on_system_suspend: true,
            enable_static_fallback: true,
        },
        harness.session_coordinator.clone(),
        harness.window_provider.clone(),
    );

    // Initial state: AC charging
    let ac_snap = SystemPowerSnapshot::laptop_charging(90, 1000);
    evaluator.evaluate_power_state(&ac_snap, &harness.snapshot);

    // 1. Operating system enters Sleep / Suspend
    evaluator.handle_system_suspend(&harness.snapshot);
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(&harness.display1_id).unwrap().state, DisplaySessionState::Suspended);
        assert_eq!(sc.get_session(&harness.display2_id).unwrap().state, DisplaySessionState::Suspended);
    }
    assert!(evaluator.is_display_system_suspended(&harness.display1_id));

    // 2. System wakes up on AC: SystemSuspended cleared and sessions resume
    let wake_ac_snap = SystemPowerSnapshot::laptop_charging(90, 5000);
    evaluator.handle_system_resume(&wake_ac_snap, &harness.snapshot);
    {
        let sc = harness.session_coordinator.lock().unwrap();
        assert_eq!(sc.get_session(&harness.display1_id).unwrap().state, DisplaySessionState::Active);
        assert_eq!(sc.get_session(&harness.display2_id).unwrap().state, DisplaySessionState::Active);
    }
    assert!(!evaluator.is_display_system_suspended(&harness.display1_id));

    // 3. Sleep again, but wake on BATTERY with SaveBattery mode enabled:
    evaluator.handle_system_suspend(&harness.snapshot);
    let wake_battery_snap = SystemPowerSnapshot::laptop_battery(85, false, 10000);
    evaluator.handle_system_resume(&wake_battery_snap, &harness.snapshot);
    {
        let sc = harness.session_coordinator.lock().unwrap();
        // SystemSuspended is cleared, but PowerSaving immediately applies -> sessions stay Suspended!
        assert_eq!(
            sc.get_session(&harness.display1_id).unwrap().state,
            DisplaySessionState::Suspended
        );
    }
    assert!(!evaluator.is_display_system_suspended(&harness.display1_id));
    assert!(evaluator.is_display_power_paused(&harness.display1_id));
}

#[test]
fn test_mock_power_provider_registration_and_failures() {
    let mock = MockDesktopPowerProvider::new();

    // 1. Successful query
    let snap = mock.query_power_status().unwrap();
    assert_eq!(snap.power_source, PowerSource::Ac);

    // 2. Query failure simulation
    mock.set_query_failure(Some(1234));
    let err = mock.query_power_status().unwrap_err();
    assert_eq!(err, PowerError::QueryFailed { code: 1234 });
    mock.set_query_failure(None);

    // 3. Power notification registration
    let hwnd = NativeWindowHandle(100);
    let mut handles = mock.register_power_notifications(hwnd).unwrap();
    assert_eq!(mock.registered_handles_count(), 3);
    assert_eq!(handles.len(), 3);

    // 4. Unregistration
    mock.unregister_power_notifications(&mut handles).unwrap();
    assert_eq!(mock.registered_handles_count(), 0);
    assert!(handles.is_empty());

    // 5. Registration failure simulation
    mock.set_registration_failure(true);
    let reg_err = mock.register_power_notifications(hwnd).unwrap_err();
    match reg_err {
        PowerError::RegistrationFailed { .. } => {}
        other => panic!("Expected RegistrationFailed, got {:?}", other),
    }
}
