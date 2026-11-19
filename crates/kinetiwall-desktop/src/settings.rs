//! Centralized Application Settings Architecture, Persistent Preferences, Validation & Safe Migrations (Step 60).
//!
//! Provides a single source of truth for persistent user preferences across general behavior,
//! performance policies, rendering, audio, and multi-monitor assignments, conforming to the
//! KW-DAT-004 specification (`config.json` Schema V1).
//!
//! ### Core Guarantees:
//! - **Strict Validation & Clamping**: Prevents invalid framerates, out-of-range volume, and malformed state.
//! - **Atomic Persistence**: Uses write-to-temp, `sync_all()`, atomic rename, and automatic `.bak` copy.
//! - **Quarantine & Safe Recovery**: Auto-recovers from `.bak` on corrupt load; if unrecoverable,
//!   quarantines corrupted files to `.corrupt.<timestamp>` and falls back to safe factory defaults.
//! - **Lossless Forward Compatibility**: Unknown top-level fields are preserved across serialization.
//! - **Policy Arbitration Bridge**: Translates centralized settings directly into typed policy structs
//!   without bypassing arbitration.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::adaptive::AdaptiveQualitySettings;
use crate::budget::ApplicationResourceBudget;
use crate::display_power::{DisplayDimmedAction, DisplayPowerPlaybackSettings};
use crate::inactivity::InactivityPlaybackSettings;
use crate::policy::{FullscreenPauseMode, FullscreenPlaybackSettings};
use crate::power::{PowerPlaybackSettings, PowerPolicyMode};
use crate::win_session::SessionPlaybackSettings;

/// Current configuration store schema version.
pub const CURRENT_CONFIG_SCHEMA_VERSION: u32 = 1;

/// Default filename for the primary configuration store.
pub const DEFAULT_CONFIG_FILENAME: &str = "config.json";

/// Extension used for atomic write staging.
pub const TEMP_CONFIG_EXTENSION: &str = "tmp";

/// Extension used for verified backup configuration.
pub const BACKUP_CONFIG_EXTENSION: &str = "bak";

// -------------------------------------------------------------------------------------------------
// Domain Models: Configuration Components (KW-DAT-004 Schema V1)
// -------------------------------------------------------------------------------------------------

/// UI Theme preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreference {
    #[default]
    System,
    Dark,
    Light,
}

/// Accessible reduced motion preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReducedMotionPreference {
    #[default]
    System,
    AlwaysReduce,
    NoPreference,
}

/// Action to take when a foreground application enters fullscreen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FullscreenAction {
    #[default]
    Pause,
    Mute,
    KeepPlaying,
}

/// Action to take when a window is maximized within the work area.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WindowStateAction {
    Pause,
    Mute,
    #[default]
    KeepPlaying,
}

/// Action to take when the system runs on battery power.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BatteryAction {
    Pause,
    #[default]
    Throttle30fps,
    KeepPlaying,
}

/// Action to take during a Remote Desktop session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RemoteDesktopAction {
    #[default]
    Pause,
    KeepPlaying,
}

/// Wallpaper display scaling mode (aligned with KW-DAT-004 specification).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StretchMode {
    #[default]
    Fill,
    Fit,
    Stretch,
    Center,
}

/// General application behavior settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneralConfig {
    pub autostart: bool,
    pub start_minimized: bool,
    pub minimize_to_tray: bool,
    pub theme: ThemePreference,
    #[serde(default)]
    pub reduced_motion: ReducedMotionPreference,
    pub language: String,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            autostart: false,
            start_minimized: true,
            minimize_to_tray: true,
            theme: ThemePreference::System,
            reduced_motion: ReducedMotionPreference::System,
            language: "en-US".to_string(),
        }
    }
}

/// Performance and resource policy configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerformanceConfig {
    pub on_fullscreen_app: FullscreenAction,
    pub on_maximized_window: WindowStateAction,
    pub on_battery_power: BatteryAction,
    pub on_remote_desktop: RemoteDesktopAction,
    pub target_framerate_ac: u32,
    pub target_framerate_battery: u32,
    pub enable_adaptive_quality: bool,
    pub enable_resource_budget: bool,
    pub enable_display_power_pause: bool,
    pub enable_inactivity_pause: bool,
    pub inactivity_threshold_millis: u64,
    pub pause_on_workstation_lock: bool,
}

impl Default for PerformanceConfig {
    fn default() -> Self {
        Self {
            on_fullscreen_app: FullscreenAction::Pause,
            on_maximized_window: WindowStateAction::KeepPlaying,
            on_battery_power: BatteryAction::Throttle30fps,
            on_remote_desktop: RemoteDesktopAction::Pause,
            target_framerate_ac: 60,
            target_framerate_battery: 30,
            enable_adaptive_quality: true,
            enable_resource_budget: true,
            enable_display_power_pause: true,
            enable_inactivity_pause: true,
            inactivity_threshold_millis: 300_000,
            pause_on_workstation_lock: true,
        }
    }
}

/// Rendering and graphics acceleration settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderingConfig {
    pub hardware_acceleration: bool,
    pub vsync: bool,
    pub stretch_mode_default: StretchMode,
}

impl Default for RenderingConfig {
    fn default() -> Self {
        Self {
            hardware_acceleration: true,
            vsync: true,
            stretch_mode_default: StretchMode::Fill,
        }
    }
}

/// Audio playback settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioConfig {
    pub master_volume: f32,
    pub master_mute: bool,
    pub mute_when_other_app_plays_audio: bool,
    pub output_device_id: Option<String>,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            master_volume: 0.5,
            master_mute: false,
            mute_when_other_app_plays_audio: true,
            output_device_id: None,
        }
    }
}

/// Per-monitor assignment and override configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitorAssignmentConfig {
    pub hardware_edid_hash: String,
    pub display_name: String,
    pub assigned_wallpaper_id: Option<Uuid>,
    pub stretch_mode: StretchMode,
    pub volume: f32,
    pub is_muted: bool,
    #[serde(default)]
    pub property_overrides: HashMap<String, serde_json::Value>,
}

impl Default for MonitorAssignmentConfig {
    fn default() -> Self {
        Self {
            hardware_edid_hash: String::new(),
            display_name: String::new(),
            assigned_wallpaper_id: None,
            stretch_mode: StretchMode::Fill,
            volume: 1.0,
            is_muted: false,
            property_overrides: HashMap::new(),
        }
    }
}

/// Logging and diagnostics settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticsConfig {
    pub log_level: String,
    pub max_log_retention_days: u32,
    pub enable_file_logging: bool,
    pub enable_diagnostics_overlay: bool,
}

impl Default for DiagnosticsConfig {
    fn default() -> Self {
        Self {
            log_level: "info".to_string(),
            max_log_retention_days: 7,
            enable_file_logging: true,
            enable_diagnostics_overlay: false,
        }
    }
}

/// Centralized KinetiWall global configuration model (`config.json` Schema V1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KinetiWallConfig {
    pub schema_version: u32,
    pub general: GeneralConfig,
    pub performance: PerformanceConfig,
    pub rendering: RenderingConfig,
    pub audio: AudioConfig,
    #[serde(default)]
    pub monitors: HashMap<String, MonitorAssignmentConfig>,
    #[serde(default)]
    pub wallpaper_properties: HashMap<Uuid, HashMap<String, serde_json::Value>>,
    pub diagnostics: DiagnosticsConfig,
    #[serde(flatten)]
    pub unknown_fields: HashMap<String, serde_json::Value>,
}

impl Default for KinetiWallConfig {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_CONFIG_SCHEMA_VERSION,
            general: GeneralConfig::default(),
            performance: PerformanceConfig::default(),
            rendering: RenderingConfig::default(),
            audio: AudioConfig::default(),
            monitors: HashMap::new(),
            wallpaper_properties: HashMap::new(),
            diagnostics: DiagnosticsConfig::default(),
            unknown_fields: HashMap::new(),
        }
    }
}

impl KinetiWallConfig {
    /// Validates configuration values strictly.
    pub fn validate(&self) -> Result<(), SettingsError> {
        if self.schema_version != CURRENT_CONFIG_SCHEMA_VERSION {
            return Err(SettingsError::UnsupportedSchemaVersion {
                expected: CURRENT_CONFIG_SCHEMA_VERSION,
                found: self.schema_version,
            });
        }

        if self.general.language.trim().is_empty() {
            return Err(SettingsError::ValidationFailure {
                setting: "general.language".into(),
                detail: "Language identifier cannot be empty".into(),
            });
        }

        if !(15..=240).contains(&self.performance.target_framerate_ac) {
            return Err(SettingsError::ValidationFailure {
                setting: "performance.target_framerate_ac".into(),
                detail: format!(
                    "Target framerate AC ({}) outside valid range [15, 240]",
                    self.performance.target_framerate_ac
                ),
            });
        }

        if !(15..=60).contains(&self.performance.target_framerate_battery) {
            return Err(SettingsError::ValidationFailure {
                setting: "performance.target_framerate_battery".into(),
                detail: format!(
                    "Target framerate battery ({}) outside valid range [15, 60]",
                    self.performance.target_framerate_battery
                ),
            });
        }

        if self.audio.master_volume.is_nan()
            || self.audio.master_volume.is_infinite()
            || !(0.0..=1.0).contains(&self.audio.master_volume)
        {
            return Err(SettingsError::ValidationFailure {
                setting: "audio.master_volume".into(),
                detail: format!(
                    "Master volume ({}) outside valid range [0.0, 1.0]",
                    self.audio.master_volume
                ),
            });
        }

        for (id, mon) in &self.monitors {
            if mon.volume.is_nan() || mon.volume.is_infinite() || !(0.0..=1.0).contains(&mon.volume)
            {
                return Err(SettingsError::ValidationFailure {
                    setting: format!("monitors[{}].volume", id),
                    detail: format!("Monitor volume ({}) outside valid range [0.0, 1.0]", mon.volume),
                });
            }
        }

        let valid_log_levels = ["trace", "debug", "info", "warn", "error"];
        if !valid_log_levels.contains(&self.diagnostics.log_level.to_lowercase().as_str()) {
            return Err(SettingsError::ValidationFailure {
                setting: "diagnostics.log_level".into(),
                detail: format!(
                    "Unknown log level '{}', expected one of {:?}",
                    self.diagnostics.log_level, valid_log_levels
                ),
            });
        }

        if !(1..=30).contains(&self.diagnostics.max_log_retention_days) {
            return Err(SettingsError::ValidationFailure {
                setting: "diagnostics.max_log_retention_days".into(),
                detail: format!(
                    "Log retention days ({}) outside valid range [1, 30]",
                    self.diagnostics.max_log_retention_days
                ),
            });
        }

        Ok(())
    }

    /// Sanitizes and clamps out-of-bounds fields safely, returning any recorded adjustments.
    pub fn sanitize(&mut self) -> Vec<String> {
        let mut adjustments = Vec::new();

        if self.schema_version != CURRENT_CONFIG_SCHEMA_VERSION {
            adjustments.push(format!(
                "Upgraded schema_version from {} to {}",
                self.schema_version, CURRENT_CONFIG_SCHEMA_VERSION
            ));
            self.schema_version = CURRENT_CONFIG_SCHEMA_VERSION;
        }

        if self.general.language.trim().is_empty() {
            adjustments.push("Reset empty general.language to 'en-US'".into());
            self.general.language = "en-US".into();
        }

        if self.performance.target_framerate_ac < 15 {
            adjustments.push(format!(
                "Clamped performance.target_framerate_ac from {} to 15",
                self.performance.target_framerate_ac
            ));
            self.performance.target_framerate_ac = 15;
        } else if self.performance.target_framerate_ac > 240 {
            adjustments.push(format!(
                "Clamped performance.target_framerate_ac from {} to 240",
                self.performance.target_framerate_ac
            ));
            self.performance.target_framerate_ac = 240;
        }

        if self.performance.target_framerate_battery < 15 {
            adjustments.push(format!(
                "Clamped performance.target_framerate_battery from {} to 15",
                self.performance.target_framerate_battery
            ));
            self.performance.target_framerate_battery = 15;
        } else if self.performance.target_framerate_battery > 60 {
            adjustments.push(format!(
                "Clamped performance.target_framerate_battery from {} to 60",
                self.performance.target_framerate_battery
            ));
            self.performance.target_framerate_battery = 60;
        }

        if self.audio.master_volume.is_nan() || self.audio.master_volume.is_infinite() {
            adjustments.push("Reset invalid master_volume to 0.5".into());
            self.audio.master_volume = 0.5;
        } else if self.audio.master_volume < 0.0 {
            adjustments.push("Clamped master_volume from below 0.0 to 0.0".into());
            self.audio.master_volume = 0.0;
        } else if self.audio.master_volume > 1.0 {
            adjustments.push("Clamped master_volume from above 1.0 to 1.0".into());
            self.audio.master_volume = 1.0;
        }

        for (id, mon) in self.monitors.iter_mut() {
            if mon.volume.is_nan() || mon.volume.is_infinite() {
                adjustments.push(format!("Reset invalid volume on monitor '{}' to 1.0", id));
                mon.volume = 1.0;
            } else if mon.volume < 0.0 {
                adjustments.push(format!("Clamped volume on monitor '{}' to 0.0", id));
                mon.volume = 0.0;
            } else if mon.volume > 1.0 {
                adjustments.push(format!("Clamped volume on monitor '{}' to 1.0", id));
                mon.volume = 1.0;
            }
        }

        if !(1..=30).contains(&self.diagnostics.max_log_retention_days) {
            let clamped = self.diagnostics.max_log_retention_days.clamp(1, 30);
            adjustments.push(format!(
                "Clamped max_log_retention_days from {} to {}",
                self.diagnostics.max_log_retention_days, clamped
            ));
            self.diagnostics.max_log_retention_days = clamped;
        }

        let valid_log_levels = ["trace", "debug", "info", "warn", "error"];
        if !valid_log_levels.contains(&self.diagnostics.log_level.to_lowercase().as_str()) {
            adjustments.push(format!(
                "Reset unrecognized log_level '{}' to 'info'",
                self.diagnostics.log_level
            ));
            self.diagnostics.log_level = "info".into();
        }

        adjustments
    }

    // ---------------------------------------------------------------------------------------------
    // Policy Integration Bridges
    // ---------------------------------------------------------------------------------------------

    /// Bridges configuration to `FullscreenPlaybackSettings` for policy arbitration.
    #[must_use]
    pub fn to_fullscreen_settings(&self) -> FullscreenPlaybackSettings {
        FullscreenPlaybackSettings {
            enabled: self.performance.on_fullscreen_app != FullscreenAction::KeepPlaying,
            pause_mode: FullscreenPauseMode::PerDisplay,
            debounce_duration_millis: 300,
        }
    }

    /// Bridges configuration to `PowerPlaybackSettings` for battery and power policies.
    #[must_use]
    pub fn to_power_settings(&self) -> PowerPlaybackSettings {
        let mode = match self.performance.on_battery_power {
            BatteryAction::Pause => PowerPolicyMode::SaveBattery,
            BatteryAction::Throttle30fps => PowerPolicyMode::Balanced,
            BatteryAction::KeepPlaying => PowerPolicyMode::Off,
        };

        PowerPlaybackSettings {
            mode,
            pause_on_battery: self.performance.on_battery_power == BatteryAction::Pause,
            pause_on_battery_saver: true,
            low_battery_threshold_percent: Some(20),
            pause_on_system_suspend: true,
            enable_static_fallback: true,
        }
    }

    /// Bridges configuration to `DisplayPowerPlaybackSettings`.
    #[must_use]
    pub fn to_display_power_settings(&self) -> DisplayPowerPlaybackSettings {
        DisplayPowerPlaybackSettings {
            enabled: self.performance.enable_display_power_pause,
            pause_on_display_off: true,
            resume_on_display_on: true,
            dimmed_action: DisplayDimmedAction::DoNothing,
            enable_static_fallback: true,
        }
    }

    /// Bridges configuration to `InactivityPlaybackSettings`.
    #[must_use]
    pub fn to_inactivity_settings(&self) -> InactivityPlaybackSettings {
        InactivityPlaybackSettings {
            enabled: self.performance.enable_inactivity_pause,
            idle_threshold_millis: self.performance.inactivity_threshold_millis,
            resume_on_activity: true,
            enable_static_fallback: true,
        }
    }

    /// Bridges configuration to `SessionPlaybackSettings` for workstation lock and remote desktop.
    #[must_use]
    pub fn to_session_settings(&self) -> SessionPlaybackSettings {
        SessionPlaybackSettings {
            enabled: self.performance.pause_on_workstation_lock
                || self.performance.on_remote_desktop == RemoteDesktopAction::Pause,
            pause_on_lock: self.performance.pause_on_workstation_lock,
            optimize_remote_session: self.performance.on_remote_desktop == RemoteDesktopAction::Pause,
            enable_static_fallback: true,
        }
    }

    /// Bridges configuration to `AdaptiveQualitySettings`.
    #[must_use]
    pub fn to_adaptive_settings(&self) -> AdaptiveQualitySettings {
        AdaptiveQualitySettings {
            enabled: self.performance.enable_adaptive_quality,
            target_fps_floor: self.performance.target_framerate_battery.min(30),
            sustained_pressure_threshold_ms: 3000,
            recovery_dwell_threshold_ms: 5000,
            cpu_pressure_threshold_percent: 85.0,
            cpu_recovery_threshold_percent: 60.0,
            adjustment_cooldown_ms: 2000,
            allow_power_policy_coordination: true,
        }
    }

    /// Bridges configuration to `ApplicationResourceBudget`.
    #[must_use]
    pub fn to_budget_settings(&self) -> ApplicationResourceBudget {
        ApplicationResourceBudget {
            enabled: self.performance.enable_resource_budget,
            target_aggregate_cpu_percent: 25.0,
            pressure_threshold_cpu_percent: 30.0,
            recovery_threshold_cpu_percent: 20.0,
            sustained_pressure_threshold_ms: 3000,
            recovery_dwell_threshold_ms: 5000,
            adjustment_cooldown_ms: 2000,
            max_simultaneous_adjustments: 2,
            allow_power_coordination: true,
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Error Taxonomy
// -------------------------------------------------------------------------------------------------

/// Errors produced during configuration loading, validation, migration, or persistence.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum SettingsError {
    #[error("Configuration I/O storage failure: {0}")]
    StorageFailure(String),

    #[error("Configuration serialization failure: {0}")]
    SerializationFailure(String),

    #[error("Configuration validation error for setting `{setting}`: {detail}")]
    ValidationFailure { setting: String, detail: String },

    #[error("Unsupported configuration schema version: expected <= {expected}, found {found}")]
    UnsupportedSchemaVersion { expected: u32, found: u32 },

    #[error(
        "Corrupted configuration quarantined at `{quarantine_path}` (original: `{original_path}`): {reason}"
    )]
    QuarantinedCorruptConfig {
        original_path: String,
        quarantine_path: String,
        reason: String,
    },

    #[error("Backup restoration failed for `{path}`: {detail}")]
    BackupRestoreFailed { path: String, detail: String },
}

// -------------------------------------------------------------------------------------------------
// Schema Migrations
// -------------------------------------------------------------------------------------------------

/// Safely migrates arbitrary JSON input into a validated `KinetiWallConfig` Schema V1.
///
/// Returns `(config, was_migrated)`.
pub fn migrate_raw(mut value: serde_json::Value) -> Result<(KinetiWallConfig, bool), SettingsError> {
    if !value.is_object() {
        return Err(SettingsError::SerializationFailure(
            "Root configuration must be a JSON object".into(),
        ));
    }

    let obj = value.as_object_mut().unwrap();

    let raw_version = obj.get("schema_version").and_then(|v| v.as_u64());

    match raw_version {
        None | Some(0) => {
            info!("Migrating unversioned / V0 configuration to Schema V1");
            obj.insert(
                "schema_version".to_string(),
                serde_json::Value::from(CURRENT_CONFIG_SCHEMA_VERSION),
            );

            // Populate defaults for missing mandatory objects
            if !obj.contains_key("general") {
                obj.insert(
                    "general".to_string(),
                    serde_json::to_value(GeneralConfig::default())
                        .map_err(|e| SettingsError::SerializationFailure(e.to_string()))?,
                );
            }
            if !obj.contains_key("performance") {
                obj.insert(
                    "performance".to_string(),
                    serde_json::to_value(PerformanceConfig::default())
                        .map_err(|e| SettingsError::SerializationFailure(e.to_string()))?,
                );
            }
            if !obj.contains_key("rendering") {
                obj.insert(
                    "rendering".to_string(),
                    serde_json::to_value(RenderingConfig::default())
                        .map_err(|e| SettingsError::SerializationFailure(e.to_string()))?,
                );
            }
            if !obj.contains_key("audio") {
                obj.insert(
                    "audio".to_string(),
                    serde_json::to_value(AudioConfig::default())
                        .map_err(|e| SettingsError::SerializationFailure(e.to_string()))?,
                );
            }
            if !obj.contains_key("diagnostics") {
                obj.insert(
                    "diagnostics".to_string(),
                    serde_json::to_value(DiagnosticsConfig::default())
                        .map_err(|e| SettingsError::SerializationFailure(e.to_string()))?,
                );
            }

            let mut config: KinetiWallConfig = serde_json::from_value(value)
                .map_err(|e| SettingsError::SerializationFailure(e.to_string()))?;
            config.sanitize();
            Ok((config, true))
        }
        Some(v) if v == CURRENT_CONFIG_SCHEMA_VERSION as u64 => {
            let mut config: KinetiWallConfig = serde_json::from_value(value)
                .map_err(|e| SettingsError::SerializationFailure(e.to_string()))?;
            config.sanitize();
            Ok((config, false))
        }
        Some(v) => Err(SettingsError::UnsupportedSchemaVersion {
            expected: CURRENT_CONFIG_SCHEMA_VERSION,
            found: v as u32,
        }),
    }
}

// -------------------------------------------------------------------------------------------------
// Settings Repository Abstraction & Storage
// -------------------------------------------------------------------------------------------------

/// Repository contract for loading, persisting, and defaulting application preferences.
pub trait SettingsRepository: Send + Sync {
    /// Loads and validates configuration, handling recovery or migrations if needed.
    fn load_config(&self) -> Result<KinetiWallConfig, SettingsError>;

    /// Atomically persists validated configuration, updating the verified backup.
    fn save_config(&self, config: &KinetiWallConfig) -> Result<(), SettingsError>;

    /// Restores factory defaults atomically.
    fn restore_defaults(&self) -> Result<KinetiWallConfig, SettingsError>;

    /// Target file path of the configuration store.
    fn store_path(&self) -> &Path;

    /// Backup file path for recovery.
    fn backup_path(&self) -> PathBuf {
        self.store_path().with_extension(BACKUP_CONFIG_EXTENSION)
    }
}

/// File-based atomic JSON repository with rollback, backup, and corruption quarantine.
#[derive(Debug, Clone)]
pub struct FileSettingsRepository {
    store_path: PathBuf,
}

impl FileSettingsRepository {
    /// Creates a repository targeting a specific configuration path.
    #[must_use]
    pub fn new(store_path: impl Into<PathBuf>) -> Self {
        Self {
            store_path: store_path.into(),
        }
    }

    /// Resolves the default standard configuration path (`%LOCALAPPDATA%\KinetiWall\config.json`).
    #[must_use]
    pub fn default_path() -> PathBuf {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            PathBuf::from(local_app_data)
                .join("KinetiWall")
                .join(DEFAULT_CONFIG_FILENAME)
        } else {
            PathBuf::from(DEFAULT_CONFIG_FILENAME)
        }
    }

    /// Attempts to load configuration from backup file.
    fn try_load_backup(&self) -> Result<KinetiWallConfig, SettingsError> {
        let bak_path = self.backup_path();
        if !bak_path.exists() {
            return Err(SettingsError::BackupRestoreFailed {
                path: bak_path.display().to_string(),
                detail: "Backup file does not exist".into(),
            });
        }

        let contents = fs::read_to_string(&bak_path)
            .map_err(|e| SettingsError::StorageFailure(e.to_string()))?;
        let json_val: serde_json::Value = serde_json::from_str(&contents)
            .map_err(|e| SettingsError::SerializationFailure(e.to_string()))?;
        let (config, _) = migrate_raw(json_val)?;
        Ok(config)
    }

    /// Quarantines a corrupted file and writes fresh factory defaults.
    fn quarantine_and_default(&self, reason: &str) -> Result<KinetiWallConfig, SettingsError> {
        let now_millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();

        let quarantine_filename = format!(
            "{}.corrupt_{}",
            self.store_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(DEFAULT_CONFIG_FILENAME),
            now_millis
        );

        let quarantine_path = self
            .store_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(quarantine_filename);

        if self.store_path.exists() {
            if let Err(e) = fs::rename(&self.store_path, &quarantine_path) {
                warn!(
                    "Failed to rename corrupted config to {:?}: {}",
                    quarantine_path, e
                );
            } else {
                warn!(
                    "Corrupted configuration quarantined from {:?} to {:?} (Reason: {})",
                    self.store_path, quarantine_path, reason
                );
            }
        }

        let default_config = KinetiWallConfig::default();
        if let Err(e) = self.save_config(&default_config) {
            error!("Failed to write factory defaults after quarantine: {}", e);
        }

        info!("Factory defaults initialized and saved after corruption quarantine");
        Ok(default_config)
    }
}

impl SettingsRepository for FileSettingsRepository {
    fn store_path(&self) -> &Path {
        &self.store_path
    }

    fn load_config(&self) -> Result<KinetiWallConfig, SettingsError> {
        if !self.store_path.exists() {
            // Check if backup exists (e.g. primary was deleted or crashed mid-write)
            if let Ok(recovered) = self.try_load_backup() {
                warn!(
                    "Primary config missing; auto-restoring from verified backup {:?}",
                    self.backup_path()
                );
                let _ = self.save_config(&recovered);
                return Ok(recovered);
            }

            // Fresh install: initialize default configuration atomically
            info!(
                "Configuration file not found at {:?}; creating factory defaults",
                self.store_path
            );
            let default_config = KinetiWallConfig::default();
            self.save_config(&default_config)?;
            return Ok(default_config);
        }

        let contents = match fs::read_to_string(&self.store_path) {
            Ok(c) => c,
            Err(e) => {
                error!("I/O failure reading {:?}: {}", self.store_path, e);
                // Attempt recovery from backup
                if let Ok(recovered) = self.try_load_backup() {
                    warn!("Recovered configuration from backup after primary read I/O error");
                    let _ = self.save_config(&recovered);
                    return Ok(recovered);
                }
                return self.quarantine_and_default(&format!("I/O error: {}", e));
            }
        };

        // Zero-byte or truncated file check
        if contents.trim().is_empty() {
            warn!("Configuration file at {:?} is zero-length/empty", self.store_path);
            if let Ok(recovered) = self.try_load_backup() {
                warn!("Auto-restored zero-length config from backup");
                let _ = self.save_config(&recovered);
                return Ok(recovered);
            }
            return self.quarantine_and_default("File was zero-byte or whitespace-only");
        }

        let json_val: serde_json::Value = match serde_json::from_str(&contents) {
            Ok(v) => v,
            Err(e) => {
                error!("JSON parse error in {:?}: {}", self.store_path, e);
                if let Ok(recovered) = self.try_load_backup() {
                    warn!("Auto-restored corrupted config from backup {:?}", self.backup_path());
                    let _ = self.save_config(&recovered);
                    return Ok(recovered);
                }
                return self.quarantine_and_default(&format!("JSON syntax error: {}", e));
            }
        };

        let (mut config, was_migrated) = match migrate_raw(json_val) {
            Ok(res) => res,
            Err(SettingsError::UnsupportedSchemaVersion { expected, found }) => {
                // Do NOT overwrite future versions!
                return Err(SettingsError::UnsupportedSchemaVersion { expected, found });
            }
            Err(e) => {
                error!("Migration error for {:?}: {}", self.store_path, e);
                if let Ok(recovered) = self.try_load_backup() {
                    let _ = self.save_config(&recovered);
                    return Ok(recovered);
                }
                return self.quarantine_and_default(&format!("Schema migration failure: {}", e));
            }
        };

        let adjustments = config.sanitize();
        if !adjustments.is_empty() {
            info!("Applied {} config sanitization adjustments", adjustments.len());
        }

        // If migrated or sanitized, persist the clean version
        if was_migrated || !adjustments.is_empty() {
            let _ = self.save_config(&config);
        }

        Ok(config)
    }

    fn save_config(&self, config: &KinetiWallConfig) -> Result<(), SettingsError> {
        config.validate()?;

        if let Some(parent) = self.store_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| SettingsError::StorageFailure(e.to_string()))?;
        }

        let serialized = serde_json::to_string_pretty(config)
            .map_err(|e| SettingsError::SerializationFailure(e.to_string()))?;

        let temp_path = self.store_path.with_extension(TEMP_CONFIG_EXTENSION);
        {
            let mut file = File::create(&temp_path)
                .map_err(|e| SettingsError::StorageFailure(e.to_string()))?;
            file.write_all(serialized.as_bytes())
                .map_err(|e| SettingsError::StorageFailure(e.to_string()))?;
            file.sync_all()
                .map_err(|e| SettingsError::StorageFailure(e.to_string()))?;
        }

        // Atomic replace
        fs::rename(&temp_path, &self.store_path)
            .map_err(|e| SettingsError::StorageFailure(e.to_string()))?;

        // Maintain verified backup
        let backup_path = self.backup_path();
        if let Err(e) = fs::copy(&self.store_path, &backup_path) {
            warn!("Failed to update verified backup at {:?}: {}", backup_path, e);
        } else {
            debug!("Updated verified backup at {:?}", backup_path);
        }

        Ok(())
    }

    fn restore_defaults(&self) -> Result<KinetiWallConfig, SettingsError> {
        let defaults = KinetiWallConfig::default();
        self.save_config(&defaults)?;
        info!("Restored and saved factory default configuration");
        Ok(defaults)
    }
}

/// In-memory mock repository for deterministic unit tests.
#[derive(Debug, Default, Clone)]
pub struct InMemorySettingsRepository {
    config: Arc<RwLock<KinetiWallConfig>>,
    fail_on_save: Arc<Mutex<bool>>,
    fail_on_load: Arc<Mutex<bool>>,
}

impl InMemorySettingsRepository {
    #[must_use]
    pub fn new(initial: KinetiWallConfig) -> Self {
        Self {
            config: Arc::new(RwLock::new(initial)),
            fail_on_save: Arc::new(Mutex::new(false)),
            fail_on_load: Arc::new(Mutex::new(false)),
        }
    }

    pub fn set_fail_on_save(&self, fail: bool) {
        *self.fail_on_save.lock().unwrap() = fail;
    }

    pub fn set_fail_on_load(&self, fail: bool) {
        *self.fail_on_load.lock().unwrap() = fail;
    }
}

impl SettingsRepository for InMemorySettingsRepository {
    fn store_path(&self) -> &Path {
        Path::new("memory://config.json")
    }

    fn load_config(&self) -> Result<KinetiWallConfig, SettingsError> {
        if *self.fail_on_load.lock().unwrap() {
            return Err(SettingsError::StorageFailure("Simulated load failure".into()));
        }
        let cfg = self.config.read().unwrap();
        Ok(cfg.clone())
    }

    fn save_config(&self, config: &KinetiWallConfig) -> Result<(), SettingsError> {
        if *self.fail_on_save.lock().unwrap() {
            return Err(SettingsError::StorageFailure("Simulated save failure".into()));
        }
        config.validate()?;
        let mut cfg = self.config.write().unwrap();
        *cfg = config.clone();
        Ok(())
    }

    fn restore_defaults(&self) -> Result<KinetiWallConfig, SettingsError> {
        let defaults = KinetiWallConfig::default();
        self.save_config(&defaults)?;
        Ok(defaults)
    }
}
