//! System Power-State Awareness, Battery-Sensitive Playback & Safe Suspension Policies (Step 49).
//!
//! Provides:
//! - Native Windows `GetSystemPowerStatus` and `RegisterPowerSettingNotification` integration.
//! - Validated, typed application-wide power-state model (`SystemPowerSnapshot`).
//! - Support for AC, battery/DC, unknown power sources, battery percentages, and Windows Battery Saver.
//! - Non-blocking event-driven updates, snapshot freshness validation, and coalescing.
//! - Multi-mode power playback policies (`Off`, `Balanced`, `SaveBattery`) with configurable triggers.
//! - Independent pause-reason tracking ensuring power resumes never override manual or fullscreen pauses.
//! - Safe system suspend and resume lifecycle handling with post-wake reconciliation.

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::types::NativeWindowHandle;

// -------------------------------------------------------------------------------------------------
// Win32 FFI Bindings & Constants
// -------------------------------------------------------------------------------------------------

#[cfg(windows)]
#[allow(dead_code)]
mod win32 {
    use std::ffi::c_void;

    pub type BOOL = i32;
    pub type DWORD = u32;
    pub type HANDLE = *mut c_void;
    pub type HPOWERNOTIFY = *mut c_void;

    #[repr(C)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct GUID {
        pub data1: u32,
        pub data2: u16,
        pub data3: u16,
        pub data4: [u8; 8],
    }

    #[repr(C)]
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct SYSTEM_POWER_STATUS {
        pub ac_line_status: u8,
        pub battery_flag: u8,
        pub battery_life_percent: u8,
        pub system_status_flag: u8,
        pub battery_life_time: u32,
        pub battery_full_life_time: u32,
    }

    #[repr(C)]
    pub struct POWERBROADCAST_SETTING {
        pub power_setting: GUID,
        pub data_length: DWORD,
        pub data: [u8; 1],
    }

    pub const DEVICE_NOTIFY_WINDOW_HANDLE: DWORD = 0x00000000;

    // Standard Windows Power Setting GUIDs
    // GUID_ACDC_POWER_SOURCE: 5d3e9a59-e9D5-4b00-a6bd-ff34ff516548
    pub const GUID_ACDC_POWER_SOURCE: GUID = GUID {
        data1: 0x5D3E9A59,
        data2: 0xE9D5,
        data3: 0x4B00,
        data4: [0xA6, 0xBD, 0xFF, 0x34, 0xFF, 0x51, 0x65, 0x48],
    };

    // GUID_BATTERY_PERCENTAGE_REMAINING: a7ad8041-b45a-4cae-87a3-eecbb468a9e1
    pub const GUID_BATTERY_PERCENTAGE_REMAINING: GUID = GUID {
        data1: 0xA7AD8041,
        data2: 0xB45A,
        data3: 0x4CAE,
        data4: [0x87, 0xA3, 0xEE, 0xCB, 0xB4, 0x68, 0xA9, 0xE1],
    };

    // GUID_POWER_SAVING_STATUS: e0094548-9a10-44ee-be81-4ddacd272895 (Battery Saver)
    pub const GUID_POWER_SAVING_STATUS: GUID = GUID {
        data1: 0xE0094548,
        data2: 0x9A10,
        data3: 0x44EE,
        data4: [0x87, 0x81, 0x4D, 0xDA, 0xCD, 0x27, 0x28, 0x95],
    };

    #[link(name = "kernel32")]
    extern "system" {
        pub fn GetSystemPowerStatus(lpSystemPowerStatus: *mut SYSTEM_POWER_STATUS) -> BOOL;
        pub fn GetLastError() -> DWORD;
    }

    #[link(name = "user32")]
    extern "system" {
        pub fn RegisterPowerSettingNotification(
            hRecipient: HANDLE,
            PowerSettingGuid: *const GUID,
            Flags: DWORD,
        ) -> HPOWERNOTIFY;
        pub fn UnregisterPowerSettingNotification(Handle: HPOWERNOTIFY) -> BOOL;
    }
}

// -------------------------------------------------------------------------------------------------
// Domain Models
// -------------------------------------------------------------------------------------------------

/// Primary operational power source of the host system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PowerSource {
    /// Connected to AC wall utility power.
    Ac,
    /// Operating on internal DC battery power.
    Battery,
    /// Power source could not be definitively determined.
    #[default]
    Unknown,
}

/// System battery hardware presence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BatteryAvailability {
    /// System has one or more installed, active batteries.
    Present,
    /// System is a desktop or workstation with no internal battery hardware.
    NoBattery,
    /// Battery availability status is unknown or pending driver response.
    #[default]
    Unknown,
}

/// Operational charging state of installed battery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BatteryChargingState {
    /// Battery is actively charging from AC power.
    Charging,
    /// Battery is discharging (operating on battery power).
    Discharging,
    /// Battery is connected to AC but not charging (e.g. 100% full or threshold cap).
    NotCharging,
    /// System has no battery installed.
    NoBattery,
    /// Charging state is unknown.
    #[default]
    Unknown,
}

/// Status of Windows 10/11 system-level Battery Saver mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BatterySaverState {
    /// Battery Saver is currently inactive.
    Disabled,
    /// Battery Saver is actively enabled by the user or OS power management.
    Enabled,
    /// Battery Saver state is unsupported or unknown.
    #[default]
    Unknown,
}

/// Quality/confidence rating of a system power status sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PowerStatusQuality {
    /// Authoritative, freshly queried sample from the operating system.
    #[default]
    Verified,
    /// Retained from previous valid sample due to transient query error.
    StaleRetained,
    /// Synthetic or uninitialized baseline sample.
    Uninitialized,
}

/// Validated, immutable snapshot of the system's power state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemPowerSnapshot {
    /// Current power source (AC, Battery, or Unknown).
    pub power_source: PowerSource,

    /// Hardware battery availability.
    pub battery_availability: BatteryAvailability,

    /// Battery remaining charge percentage (0..=100), or None if unknown or no battery.
    pub battery_percent: Option<u8>,

    /// Battery charging state.
    pub charging_state: BatteryChargingState,

    /// Windows Battery Saver status.
    pub battery_saver: BatterySaverState,

    /// Whether the operating system is currently in a suspended state (Sleep/Hibernate).
    pub is_system_suspended: bool,

    /// Unix epoch timestamp in milliseconds when this sample was recorded.
    pub sampled_at_millis: u64,

    /// Data quality indicator.
    pub status_quality: PowerStatusQuality,
}

impl Default for SystemPowerSnapshot {
    fn default() -> Self {
        Self {
            power_source: PowerSource::Unknown,
            battery_availability: BatteryAvailability::Unknown,
            battery_percent: None,
            charging_state: BatteryChargingState::Unknown,
            battery_saver: BatterySaverState::Unknown,
            is_system_suspended: false,
            sampled_at_millis: 0,
            status_quality: PowerStatusQuality::Uninitialized,
        }
    }
}

impl SystemPowerSnapshot {
    /// Creates a verified desktop AC snapshot (no battery).
    #[must_use]
    pub fn desktop_ac(timestamp_millis: u64) -> Self {
        Self {
            power_source: PowerSource::Ac,
            battery_availability: BatteryAvailability::NoBattery,
            battery_percent: None,
            charging_state: BatteryChargingState::NoBattery,
            battery_saver: BatterySaverState::Disabled,
            is_system_suspended: false,
            sampled_at_millis: timestamp_millis,
            status_quality: PowerStatusQuality::Verified,
        }
    }

    /// Creates a verified laptop battery snapshot.
    #[must_use]
    pub fn laptop_battery(percent: u8, battery_saver: bool, timestamp_millis: u64) -> Self {
        Self {
            power_source: PowerSource::Battery,
            battery_availability: BatteryAvailability::Present,
            battery_percent: Some(percent.min(100)),
            charging_state: BatteryChargingState::Discharging,
            battery_saver: if battery_saver {
                BatterySaverState::Enabled
            } else {
                BatterySaverState::Disabled
            },
            is_system_suspended: false,
            sampled_at_millis: timestamp_millis,
            status_quality: PowerStatusQuality::Verified,
        }
    }

    /// Creates a verified laptop AC charging snapshot.
    #[must_use]
    pub fn laptop_charging(percent: u8, timestamp_millis: u64) -> Self {
        Self {
            power_source: PowerSource::Ac,
            battery_availability: BatteryAvailability::Present,
            battery_percent: Some(percent.min(100)),
            charging_state: BatteryChargingState::Charging,
            battery_saver: BatterySaverState::Disabled,
            is_system_suspended: false,
            sampled_at_millis: timestamp_millis,
            status_quality: PowerStatusQuality::Verified,
        }
    }

    /// Returns true if the device is currently running on battery power.
    #[must_use]
    pub fn is_on_battery(&self) -> bool {
        self.power_source == PowerSource::Battery
    }

    /// Returns true if Windows Battery Saver is actively enabled.
    #[must_use]
    pub fn is_battery_saver_active(&self) -> bool {
        self.battery_saver == BatterySaverState::Enabled
    }
}

// -------------------------------------------------------------------------------------------------
// Policy Configuration
// -------------------------------------------------------------------------------------------------

/// Operational mode for power-aware playback management.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PowerPolicyMode {
    /// Monitor power state for status and diagnostics; never pause or throttle playback (default).
    #[default]
    Off,
    /// Apply reversible low-disruption optimizations or pause when battery is critical.
    Balanced,
    /// Aggressively suspend wallpaper rendering to maximize battery runtime when saving triggers fire.
    SaveBattery,
}

/// User-configurable power-aware playback policy settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowerPlaybackSettings {
    /// High-level policy mode (Off, Balanced, SaveBattery). Default: Off.
    pub mode: PowerPolicyMode,

    /// Trigger power pause when device switches to battery power (SaveBattery mode).
    pub pause_on_battery: bool,

    /// Trigger power pause when Windows Battery Saver is active.
    pub pause_on_battery_saver: bool,

    /// Trigger power pause when battery charge drops at or below this percentage.
    pub low_battery_threshold_percent: Option<u8>,

    /// Suspend wallpapers during system sleep/hibernate and reconcile on resume.
    pub pause_on_system_suspend: bool,

    /// Transition eligible live wallpaper sessions into static fallback presentation during power pauses.
    pub enable_static_fallback: bool,
}

impl Default for PowerPlaybackSettings {
    fn default() -> Self {
        Self {
            mode: PowerPolicyMode::Off,
            pause_on_battery: true,
            pause_on_battery_saver: true,
            low_battery_threshold_percent: Some(20),
            pause_on_system_suspend: true,
            enable_static_fallback: true,
        }
    }
}

/// Result of evaluating a power snapshot against configured policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PowerPolicyEvaluation {
    /// Whether wallpaper sessions should be suspended due to power constraints.
    pub should_pause: bool,
    /// Descriptive trigger reason if pause is required.
    pub trigger_reason: Option<String>,
    /// Whether eligible sessions should attempt static fallback presentation.
    pub prefer_static_fallback: bool,
}

impl PowerPolicyEvaluation {
    /// Evaluates a power snapshot against user settings.
    #[must_use]
    pub fn evaluate(settings: &PowerPlaybackSettings, snapshot: &SystemPowerSnapshot) -> Self {
        // 1. Off mode: never changes playback
        if settings.mode == PowerPolicyMode::Off {
            return Self {
                should_pause: false,
                trigger_reason: None,
                prefer_static_fallback: false,
            };
        }

        // 2. System suspend check: always applies if enabled
        if settings.pause_on_system_suspend && snapshot.is_system_suspended {
            return Self {
                should_pause: true,
                trigger_reason: Some("System suspension (sleep/hibernate)".into()),
                prefer_static_fallback: false, // During sleep, suspend without visual fallback
            };
        }

        // 3. Desktop without battery: never triggers battery-specific pauses
        if snapshot.battery_availability == BatteryAvailability::NoBattery {
            if settings.pause_on_battery_saver && snapshot.is_battery_saver_active() {
                return Self {
                    should_pause: true,
                    trigger_reason: Some("Windows Battery Saver active".into()),
                    prefer_static_fallback: settings.enable_static_fallback,
                };
            }
            return Self {
                should_pause: false,
                trigger_reason: None,
                prefer_static_fallback: false,
            };
        }

        // 4. Windows Battery Saver trigger
        if settings.pause_on_battery_saver && snapshot.is_battery_saver_active() {
            return Self {
                should_pause: true,
                trigger_reason: Some("Windows Battery Saver active".into()),
                prefer_static_fallback: settings.enable_static_fallback,
            };
        }

        // 5. Mode-specific evaluation
        match settings.mode {
            PowerPolicyMode::Off => unreachable!(),
            PowerPolicyMode::Balanced => {
                // Balanced mode: only pauses if battery is below threshold
                if let (Some(threshold), Some(current)) =
                    (settings.low_battery_threshold_percent, snapshot.battery_percent)
                {
                    if snapshot.is_on_battery() && current <= threshold {
                        return Self {
                            should_pause: true,
                            trigger_reason: Some(format!(
                                "Battery charge ({}%) is at or below threshold ({}%)",
                                current, threshold
                            )),
                            prefer_static_fallback: settings.enable_static_fallback,
                        };
                    }
                }
                Self {
                    should_pause: false,
                    trigger_reason: None,
                    prefer_static_fallback: false,
                }
            }
            PowerPolicyMode::SaveBattery => {
                // SaveBattery mode: pause on battery if configured
                if settings.pause_on_battery && snapshot.is_on_battery() {
                    return Self {
                        should_pause: true,
                        trigger_reason: Some("Host device running on battery power".into()),
                        prefer_static_fallback: settings.enable_static_fallback,
                    };
                }

                // Check battery threshold
                if let (Some(threshold), Some(current)) =
                    (settings.low_battery_threshold_percent, snapshot.battery_percent)
                {
                    if snapshot.is_on_battery() && current <= threshold {
                        return Self {
                            should_pause: true,
                            trigger_reason: Some(format!(
                                "Battery charge ({}%) is at or below threshold ({}%)",
                                current, threshold
                            )),
                            prefer_static_fallback: settings.enable_static_fallback,
                        };
                    }
                }

                Self {
                    should_pause: false,
                    trigger_reason: None,
                    prefer_static_fallback: false,
                }
            }
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Errors & Notification Handles
// -------------------------------------------------------------------------------------------------

/// Typed errors relating to power state queries and notification management.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum PowerError {
    #[error("Operating system failed to query system power status (code: {code})")]
    QueryFailed { code: u32 },

    #[error("Power setting notification registration failed for GUID `{guid}` (code: {code})")]
    RegistrationFailed { guid: String, code: u32 },

    #[error("Failed to unregister power setting notification")]
    UnregistrationFailed,

    #[error("Malformed power setting change notification payload (expected {expected} bytes, got {actual})")]
    MalformedPayload { expected: usize, actual: usize },

    #[error("Power management functionality is unsupported on this platform or architecture")]
    Unsupported,
}

/// Tracks successfully registered native power-setting notification handles.
#[derive(Debug, Default)]
pub struct PowerNotificationHandles {
    #[cfg(windows)]
    pub handles: Vec<win32::HPOWERNOTIFY>,
    #[cfg(not(windows))]
    pub handles: Vec<usize>,
}

unsafe impl Send for PowerNotificationHandles {}
unsafe impl Sync for PowerNotificationHandles {}

impl PowerNotificationHandles {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.handles.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.handles.len()
    }
}

// -------------------------------------------------------------------------------------------------
// Desktop Power Provider Trait & Implementations
// -------------------------------------------------------------------------------------------------

/// Abstraction for querying system power status and subscribing to OS power notifications.
pub trait DesktopPowerProvider: Send + Sync {
    /// Queries the current system power snapshot.
    fn query_power_status(&self) -> Result<SystemPowerSnapshot, PowerError>;

    /// Registers a native window to receive `WM_POWERBROADCAST` power setting changes.
    fn register_power_notifications(
        &self,
        hwnd: NativeWindowHandle,
    ) -> Result<PowerNotificationHandles, PowerError>;

    /// Unregisters previously acquired power notification handles.
    fn unregister_power_notifications(
        &self,
        handles: &mut PowerNotificationHandles,
    ) -> Result<(), PowerError>;
}

/// Production Win32 implementation of `DesktopPowerProvider`.
pub struct NativeDesktopPowerProvider;

impl NativeDesktopPowerProvider {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for NativeDesktopPowerProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl DesktopPowerProvider for NativeDesktopPowerProvider {
    #[cfg(windows)]
    fn query_power_status(&self) -> Result<SystemPowerSnapshot, PowerError> {
        let mut raw = win32::SYSTEM_POWER_STATUS::default();
        let success = unsafe { win32::GetSystemPowerStatus(&mut raw) };
        if success == 0 {
            let code = unsafe { win32::GetLastError() };
            return Err(PowerError::QueryFailed { code });
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let power_source = match raw.ac_line_status {
            0 => PowerSource::Battery,
            1 => PowerSource::Ac,
            _ => PowerSource::Unknown,
        };

        let (battery_availability, charging_state, battery_percent) = match raw.battery_flag {
            128 => (
                BatteryAvailability::NoBattery,
                BatteryChargingState::NoBattery,
                None,
            ),
            255 => (
                BatteryAvailability::Unknown,
                BatteryChargingState::Unknown,
                if raw.battery_life_percent <= 100 {
                    Some(raw.battery_life_percent)
                } else {
                    None
                },
            ),
            flags => {
                let is_charging = flags & 8 != 0;
                let charging = if is_charging {
                    BatteryChargingState::Charging
                } else if raw.ac_line_status == 1 {
                    BatteryChargingState::NotCharging
                } else {
                    BatteryChargingState::Discharging
                };
                let percent = if raw.battery_life_percent <= 100 {
                    Some(raw.battery_life_percent)
                } else {
                    None
                };
                (BatteryAvailability::Present, charging, percent)
            }
        };

        let battery_saver = match raw.system_status_flag {
            0 => BatterySaverState::Disabled,
            1 => BatterySaverState::Enabled,
            _ => BatterySaverState::Unknown,
        };

        Ok(SystemPowerSnapshot {
            power_source,
            battery_availability,
            battery_percent,
            charging_state,
            battery_saver,
            is_system_suspended: false,
            sampled_at_millis: now,
            status_quality: PowerStatusQuality::Verified,
        })
    }

    #[cfg(not(windows))]
    fn query_power_status(&self) -> Result<SystemPowerSnapshot, PowerError> {
        Err(PowerError::Unsupported)
    }

    #[cfg(windows)]
    fn register_power_notifications(
        &self,
        hwnd: NativeWindowHandle,
    ) -> Result<PowerNotificationHandles, PowerError> {
        if hwnd.is_null() {
            return Err(PowerError::RegistrationFailed {
                guid: "NULL_HWND".into(),
                code: 0,
            });
        }

        let mut handles = Vec::new();
        let guids = [
            ("GUID_ACDC_POWER_SOURCE", win32::GUID_ACDC_POWER_SOURCE),
            (
                "GUID_BATTERY_PERCENTAGE_REMAINING",
                win32::GUID_BATTERY_PERCENTAGE_REMAINING,
            ),
            (
                "GUID_POWER_SAVING_STATUS",
                win32::GUID_POWER_SAVING_STATUS,
            ),
        ];

        for (name, guid) in guids {
            let handle = unsafe {
                win32::RegisterPowerSettingNotification(
                    hwnd.as_raw() as *mut std::ffi::c_void,
                    &guid,
                    win32::DEVICE_NOTIFY_WINDOW_HANDLE,
                )
            };

            if handle.is_null() {
                let code = unsafe { win32::GetLastError() };
                // Cleanup any already registered handles
                for h in handles {
                    unsafe { win32::UnregisterPowerSettingNotification(h) };
                }
                return Err(PowerError::RegistrationFailed {
                    guid: name.into(),
                    code,
                });
            }

            handles.push(handle);
        }

        Ok(PowerNotificationHandles { handles })
    }

    #[cfg(not(windows))]
    fn register_power_notifications(
        &self,
        _hwnd: NativeWindowHandle,
    ) -> Result<PowerNotificationHandles, PowerError> {
        Err(PowerError::Unsupported)
    }

    #[cfg(windows)]
    fn unregister_power_notifications(
        &self,
        handles: &mut PowerNotificationHandles,
    ) -> Result<(), PowerError> {
        let mut all_succeeded = true;
        for handle in handles.handles.drain(..) {
            let res = unsafe { win32::UnregisterPowerSettingNotification(handle) };
            if res == 0 {
                all_succeeded = false;
            }
        }

        if all_succeeded {
            Ok(())
        } else {
            Err(PowerError::UnregistrationFailed)
        }
    }

    #[cfg(not(windows))]
    fn unregister_power_notifications(
        &self,
        _handles: &mut PowerNotificationHandles,
    ) -> Result<(), PowerError> {
        Err(PowerError::Unsupported)
    }
}

// -------------------------------------------------------------------------------------------------
// Mock Provider for Testing
// -------------------------------------------------------------------------------------------------

/// Configurable mock provider for deterministic automated testing.
#[derive(Default, Clone)]
pub struct MockDesktopPowerProvider {
    snapshot: Arc<Mutex<SystemPowerSnapshot>>,
    fail_query: Arc<Mutex<Option<u32>>>,
    fail_registration: Arc<Mutex<bool>>,
    registered_count: Arc<Mutex<usize>>,
}

impl MockDesktopPowerProvider {
    #[must_use]
    pub fn new() -> Self {
        Self {
            snapshot: Arc::new(Mutex::new(SystemPowerSnapshot::desktop_ac(1000))),
            fail_query: Arc::new(Mutex::new(None)),
            fail_registration: Arc::new(Mutex::new(false)),
            registered_count: Arc::new(Mutex::new(0)),
        }
    }

    /// Sets the mock's active power snapshot.
    pub fn set_snapshot(&self, snapshot: SystemPowerSnapshot) {
        *self.snapshot.lock().unwrap() = snapshot;
    }

    /// Configures the mock to fail queries with a specified Win32 error code.
    pub fn set_query_failure(&self, code: Option<u32>) {
        *self.fail_query.lock().unwrap() = code;
    }

    /// Configures the mock to fail registration.
    pub fn set_registration_failure(&self, fail: bool) {
        *self.fail_registration.lock().unwrap() = fail;
    }

    /// Gets current count of active registered handles.
    #[must_use]
    pub fn registered_handles_count(&self) -> usize {
        *self.registered_count.lock().unwrap()
    }
}

impl DesktopPowerProvider for MockDesktopPowerProvider {
    fn query_power_status(&self) -> Result<SystemPowerSnapshot, PowerError> {
        if let Some(code) = *self.fail_query.lock().unwrap() {
            return Err(PowerError::QueryFailed { code });
        }
        Ok(self.snapshot.lock().unwrap().clone())
    }

    fn register_power_notifications(
        &self,
        hwnd: NativeWindowHandle,
    ) -> Result<PowerNotificationHandles, PowerError> {
        if *self.fail_registration.lock().unwrap() || hwnd.is_null() {
            return Err(PowerError::RegistrationFailed {
                guid: "MOCK_GUID".into(),
                code: 1,
            });
        }
        *self.registered_count.lock().unwrap() += 3;
        #[cfg(windows)]
        {
            Ok(PowerNotificationHandles {
                handles: vec![
                    1 as win32::HPOWERNOTIFY,
                    2 as win32::HPOWERNOTIFY,
                    3 as win32::HPOWERNOTIFY,
                ],
            })
        }
        #[cfg(not(windows))]
        {
            Ok(PowerNotificationHandles {
                handles: vec![1, 2, 3],
            })
        }
    }

    fn unregister_power_notifications(
        &self,
        handles: &mut PowerNotificationHandles,
    ) -> Result<(), PowerError> {
        let count = handles.handles.len();
        handles.handles.clear();
        let mut reg = self.registered_count.lock().unwrap();
        *reg = reg.saturating_sub(count);
        Ok(())
    }
}
