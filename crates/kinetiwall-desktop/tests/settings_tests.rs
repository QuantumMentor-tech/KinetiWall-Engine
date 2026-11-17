//! Integration tests for Centralized Settings Architecture, Persistent Preferences,
//! Validation, and Safe Migrations (Step 60).

use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

use kinetiwall_desktop::settings::{
    migrate_raw, BatteryAction, FileSettingsRepository, FullscreenAction,
    InMemorySettingsRepository, KinetiWallConfig, MonitorAssignmentConfig,
    RemoteDesktopAction, SettingsError, SettingsRepository, StretchMode,
    ThemePreference, BACKUP_CONFIG_EXTENSION, CURRENT_CONFIG_SCHEMA_VERSION,
};

fn create_temp_store_path(test_name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("kinetiwall_settings_test_{}_{}", test_name, Uuid::new_v4()));
    fs::create_dir_all(&dir).unwrap();
    dir.join("config.json")
}

#[test]
fn test_default_config_is_valid() {
    let config = KinetiWallConfig::default();
    assert_eq!(config.schema_version, CURRENT_CONFIG_SCHEMA_VERSION);
    assert_eq!(config.general.theme, ThemePreference::System);
    assert_eq!(config.general.language, "en-US");
    assert_eq!(config.performance.on_fullscreen_app, FullscreenAction::Pause);
    assert_eq!(config.performance.on_battery_power, BatteryAction::Throttle30fps);
    assert_eq!(config.performance.target_framerate_ac, 60);
    assert_eq!(config.performance.target_framerate_battery, 30);
    assert!(config.performance.enable_adaptive_quality);
    assert!(config.performance.enable_resource_budget);
    assert!(config.performance.enable_display_power_pause);
    assert!(config.performance.enable_inactivity_pause);
    assert_eq!(config.rendering.stretch_mode_default, StretchMode::Fill);
    assert!((config.audio.master_volume - 0.5).abs() < f32::EPSILON);
    assert_eq!(config.diagnostics.log_level, "info");
    assert_eq!(config.diagnostics.max_log_retention_days, 7);

    assert!(config.validate().is_ok());
}

#[test]
fn test_serialization_round_trip() {
    let original = KinetiWallConfig::default();
    let json = serde_json::to_string_pretty(&original).expect("Serialization failed");
    let deserialized: KinetiWallConfig = serde_json::from_str(&json).expect("Deserialization failed");

    assert_eq!(original, deserialized);
}

#[test]
fn test_strict_validation_rules() {
    // 1. Empty language
    let mut config = KinetiWallConfig::default();
    config.general.language = "   ".to_string();
    assert!(matches!(
        config.validate(),
        Err(SettingsError::ValidationFailure { setting, .. }) if setting == "general.language"
    ));

    // 2. Framerate AC out of bounds
    let mut config = KinetiWallConfig::default();
    config.performance.target_framerate_ac = 10; // below 15
    assert!(matches!(
        config.validate(),
        Err(SettingsError::ValidationFailure { setting, .. }) if setting == "performance.target_framerate_ac"
    ));

    config.performance.target_framerate_ac = 300; // above 240
    assert!(matches!(
        config.validate(),
        Err(SettingsError::ValidationFailure { setting, .. }) if setting == "performance.target_framerate_ac"
    ));

    // 3. Framerate Battery out of bounds
    let mut config = KinetiWallConfig::default();
    config.performance.target_framerate_battery = 120; // above 60
    assert!(matches!(
        config.validate(),
        Err(SettingsError::ValidationFailure { setting, .. }) if setting == "performance.target_framerate_battery"
    ));

    // 4. Audio volume out of bounds
    let mut config = KinetiWallConfig::default();
    config.audio.master_volume = 1.5;
    assert!(matches!(
        config.validate(),
        Err(SettingsError::ValidationFailure { setting, .. }) if setting == "audio.master_volume"
    ));

    config.audio.master_volume = -0.1;
    assert!(matches!(
        config.validate(),
        Err(SettingsError::ValidationFailure { setting, .. }) if setting == "audio.master_volume"
    ));

    config.audio.master_volume = f32::NAN;
    assert!(matches!(
        config.validate(),
        Err(SettingsError::ValidationFailure { setting, .. }) if setting == "audio.master_volume"
    ));

    // 5. Monitor volume out of bounds
    let mut config = KinetiWallConfig::default();
    config.monitors.insert(
        "DISPLAY1".to_string(),
        MonitorAssignmentConfig {
            volume: 2.0,
            ..Default::default()
        },
    );
    assert!(matches!(
        config.validate(),
        Err(SettingsError::ValidationFailure { setting, .. }) if setting == "monitors[DISPLAY1].volume"
    ));

    // 6. Invalid log level
    let mut config = KinetiWallConfig::default();
    config.diagnostics.log_level = "verbose_everything".to_string();
    assert!(matches!(
        config.validate(),
        Err(SettingsError::ValidationFailure { setting, .. }) if setting == "diagnostics.log_level"
    ));

    // 7. Retention days out of bounds
    let mut config = KinetiWallConfig::default();
    config.diagnostics.max_log_retention_days = 0;
    assert!(matches!(
        config.validate(),
        Err(SettingsError::ValidationFailure { setting, .. }) if setting == "diagnostics.max_log_retention_days"
    ));
    config.diagnostics.max_log_retention_days = 40;
    assert!(matches!(
        config.validate(),
        Err(SettingsError::ValidationFailure { setting, .. }) if setting == "diagnostics.max_log_retention_days"
    ));

    // 8. Unsupported schema version
    let mut config = KinetiWallConfig::default();
    config.schema_version = 2;
    assert!(matches!(
        config.validate(),
        Err(SettingsError::UnsupportedSchemaVersion { expected: 1, found: 2 })
    ));
}

#[test]
fn test_sanitize_and_clamping() {
    let mut config = KinetiWallConfig::default();
    config.general.language = "".into();
    config.performance.target_framerate_ac = 10;
    config.performance.target_framerate_battery = 100;
    config.audio.master_volume = 1.8;
    config.diagnostics.log_level = "invalid_level".into();
    config.diagnostics.max_log_retention_days = 99;

    let adjustments = config.sanitize();
    assert!(!adjustments.is_empty());
    assert_eq!(config.general.language, "en-US");
    assert_eq!(config.performance.target_framerate_ac, 15);
    assert_eq!(config.performance.target_framerate_battery, 60);
    assert!((config.audio.master_volume - 1.0).abs() < f32::EPSILON);
    assert_eq!(config.diagnostics.log_level, "info");
    assert_eq!(config.diagnostics.max_log_retention_days, 30);

    // After sanitization, validate succeeds
    assert!(config.validate().is_ok());
}

#[test]
fn test_lossless_unknown_fields() {
    let raw_json = r#"{
        "schema_version": 1,
        "general": {
            "autostart": true,
            "start_minimized": false,
            "minimize_to_tray": true,
            "theme": "dark",
            "language": "fr-FR"
        },
        "performance": {
            "on_fullscreen_app": "mute",
            "on_maximized_window": "keep_playing",
            "on_battery_power": "pause",
            "on_remote_desktop": "pause",
            "target_framerate_ac": 120,
            "target_framerate_battery": 30,
            "enable_adaptive_quality": true,
            "enable_resource_budget": true,
            "enable_display_power_pause": true,
            "enable_inactivity_pause": true,
            "inactivity_threshold_millis": 180000,
            "pause_on_workstation_lock": true
        },
        "rendering": {
            "hardware_acceleration": true,
            "vsync": false,
            "stretch_mode_default": "center"
        },
        "audio": {
            "master_volume": 0.8,
            "master_mute": true,
            "mute_when_other_app_plays_audio": false,
            "output_device_id": "device_123"
        },
        "diagnostics": {
            "log_level": "debug",
            "max_log_retention_days": 14,
            "enable_file_logging": true,
            "enable_diagnostics_overlay": true
        },
        "experimental_feature": {
            "nested_flag": true,
            "custom_val": 42
        }
    }"#;

    let config: KinetiWallConfig = serde_json::from_str(raw_json).expect("Parse error");
    assert_eq!(config.general.theme, ThemePreference::Dark);
    assert_eq!(config.general.language, "fr-FR");
    assert_eq!(config.performance.target_framerate_ac, 120);
    assert_eq!(config.performance.on_fullscreen_app, FullscreenAction::Mute);
    assert!(config.unknown_fields.contains_key("experimental_feature"));

    let re_serialized = serde_json::to_string(&config).expect("Re-serialization failed");
    assert!(re_serialized.contains("experimental_feature"));
    assert!(re_serialized.contains("nested_flag"));
}

#[test]
fn test_schema_migration_v0_to_v1() {
    let unversioned_json = r#"{
        "general": {
            "autostart": false,
            "start_minimized": true,
            "minimize_to_tray": true,
            "theme": "light",
            "language": "en-US"
        }
    }"#;

    let parsed_val: serde_json::Value = serde_json::from_str(unversioned_json).unwrap();
    let (migrated, was_migrated) = migrate_raw(parsed_val).expect("Migration failed");

    assert!(was_migrated);
    assert_eq!(migrated.schema_version, 1);
    assert_eq!(migrated.general.theme, ThemePreference::Light);
    // Missing sections populated with defaults
    assert_eq!(migrated.performance.target_framerate_ac, 60);
    assert_eq!(migrated.rendering.stretch_mode_default, StretchMode::Fill);
    assert!(migrated.validate().is_ok());
}

#[test]
fn test_rejection_of_future_schema_version() {
    let future_json = r#"{
        "schema_version": 99,
        "general": {}
    }"#;

    let parsed_val: serde_json::Value = serde_json::from_str(future_json).unwrap();
    let result = migrate_raw(parsed_val);

    assert!(matches!(
        result,
        Err(SettingsError::UnsupportedSchemaVersion { expected: 1, found: 99 })
    ));
}

#[test]
fn test_file_repository_atomic_save_and_load() {
    let store_path = create_temp_store_path("atomic_save");
    let repo = FileSettingsRepository::new(&store_path);

    // Initial load creates default file and backup
    let loaded = repo.load_config().expect("Initial load failed");
    assert_eq!(loaded.schema_version, CURRENT_CONFIG_SCHEMA_VERSION);
    assert!(store_path.exists());

    let backup_path = store_path.with_extension(BACKUP_CONFIG_EXTENSION);
    assert!(backup_path.exists());

    // Modify and save
    let mut updated = loaded.clone();
    updated.general.theme = ThemePreference::Dark;
    updated.performance.target_framerate_ac = 144;
    repo.save_config(&updated).expect("Save failed");

    // Re-load
    let reloaded = repo.load_config().expect("Reload failed");
    assert_eq!(reloaded.general.theme, ThemePreference::Dark);
    assert_eq!(reloaded.performance.target_framerate_ac, 144);

    // Clean up
    let _ = fs::remove_dir_all(store_path.parent().unwrap());
}

#[test]
fn test_file_repository_backup_recovery_on_corrupt_primary() {
    let store_path = create_temp_store_path("corrupt_primary");
    let repo = FileSettingsRepository::new(&store_path);

    // 1. Save valid custom configuration
    let mut valid_config = KinetiWallConfig::default();
    valid_config.general.theme = ThemePreference::Light;
    valid_config.performance.target_framerate_ac = 90;
    repo.save_config(&valid_config).expect("Save failed");

    // 2. Corrupt the primary config file (simulate power cut mid-write)
    fs::write(&store_path, b"{\"corrupt\": invalid_json_syntax").unwrap();

    // 3. Load config should automatically recover from verified backup
    let recovered = repo.load_config().expect("Auto-recovery from backup failed");
    assert_eq!(recovered.general.theme, ThemePreference::Light);
    assert_eq!(recovered.performance.target_framerate_ac, 90);

    // Clean up
    let _ = fs::remove_dir_all(store_path.parent().unwrap());
}

#[test]
fn test_file_repository_quarantine_when_backup_also_missing_or_corrupt() {
    let store_path = create_temp_store_path("quarantine");
    let repo = FileSettingsRepository::new(&store_path);

    // Write corrupted JSON to primary and delete or corrupt backup
    fs::write(&store_path, b"{ completely broken invalid json").unwrap();
    let backup_path = store_path.with_extension(BACKUP_CONFIG_EXTENSION);
    if backup_path.exists() {
        let _ = fs::remove_file(&backup_path);
    }

    // Loading should quarantine the corrupt file and generate fresh factory defaults
    let config = repo.load_config().expect("Quarantine recovery failed");
    assert_eq!(config.schema_version, CURRENT_CONFIG_SCHEMA_VERSION);
    assert_eq!(config.performance.target_framerate_ac, 60);

    // Verify a quarantined file was created in the directory
    let parent = store_path.parent().unwrap();
    let mut found_quarantine = false;
    for entry in fs::read_dir(parent).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.contains(".corrupt_") {
            found_quarantine = true;
            break;
        }
    }
    assert!(found_quarantine, "Quarantined file must exist on disk");

    // Clean up
    let _ = fs::remove_dir_all(parent);
}

#[test]
fn test_policy_integration_bridges() {
    let mut config = KinetiWallConfig::default();
    config.performance.on_fullscreen_app = FullscreenAction::Pause;
    config.performance.on_battery_power = BatteryAction::Throttle30fps;
    config.performance.enable_display_power_pause = true;
    config.performance.enable_inactivity_pause = true;
    config.performance.inactivity_threshold_millis = 120_000;
    config.performance.pause_on_workstation_lock = true;
    config.performance.on_remote_desktop = RemoteDesktopAction::Pause;

    // Fullscreen bridge
    let fs_settings = config.to_fullscreen_settings();
    assert!(fs_settings.enabled);

    // Power bridge
    let pwr_settings = config.to_power_settings();
    assert_eq!(pwr_settings.mode, kinetiwall_desktop::power::PowerPolicyMode::Balanced);
    assert!(!pwr_settings.pause_on_battery); // Balanced throttles, doesn't pause immediately

    config.performance.on_battery_power = BatteryAction::Pause;
    let pwr_settings_pause = config.to_power_settings();
    assert_eq!(pwr_settings_pause.mode, kinetiwall_desktop::power::PowerPolicyMode::SaveBattery);
    assert!(pwr_settings_pause.pause_on_battery);

    // Display power bridge
    let dp_settings = config.to_display_power_settings();
    assert!(dp_settings.enabled);
    assert!(dp_settings.pause_on_display_off);

    // Inactivity bridge
    let inact_settings = config.to_inactivity_settings();
    assert!(inact_settings.enabled);
    assert_eq!(inact_settings.idle_threshold_millis, 120_000);

    // Session bridge
    let sess_settings = config.to_session_settings();
    assert!(sess_settings.enabled);
    assert!(sess_settings.pause_on_lock);
    assert!(sess_settings.optimize_remote_session);

    // Adaptive bridge
    let adapt_settings = config.to_adaptive_settings();
    assert!(adapt_settings.enabled);

    // Budget bridge
    let budget_settings = config.to_budget_settings();
    assert!(budget_settings.enabled);
}

#[test]
fn test_in_memory_repository() {
    let repo = InMemorySettingsRepository::new(KinetiWallConfig::default());
    let config = repo.load_config().unwrap();
    assert_eq!(config.schema_version, 1);

    let mut updated = config;
    updated.general.theme = ThemePreference::Dark;
    repo.save_config(&updated).unwrap();

    let reloaded = repo.load_config().unwrap();
    assert_eq!(reloaded.general.theme, ThemePreference::Dark);

    // Test simulated failure
    repo.set_fail_on_load(true);
    assert!(repo.load_config().is_err());
    repo.set_fail_on_load(false);

    repo.set_fail_on_save(true);
    assert!(repo.save_config(&updated).is_err());
    repo.set_fail_on_save(false);

    // Restore defaults
    let defaulted = repo.restore_defaults().unwrap();
    assert_eq!(defaulted.general.theme, ThemePreference::System);
}

