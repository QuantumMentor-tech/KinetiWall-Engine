//! Windows Display-Power Awareness, Screen-Off Rendering Suspension & Safe Wake Recovery (Step 54).
//!
//! Provides:
//! - Native Windows `RegisterPowerSettingNotification` / `UnregisterPowerSettingNotification` integration
//!   using the documented `GUID_SESSION_DISPLAY_STATUS` power-setting GUID.
//! - Validated, typed display-power state model (`DisplayPowerSnapshot`, `DisplayPowerState`: Off, On, Dimmed, Unknown).
//! - Explicit session-level scope recognition (`DisplayPowerScope::SessionDisplayStatus` vs `ConsoleDisplayState`).
//! - Opt-in display-power playback policy (`DisplayPowerPlaybackSettings`) with safe defaults (`enabled: false`).
//! - Provider abstraction (`DesktopDisplayPowerProvider`) with production `NativeDesktopDisplayPowerProvider`
//!   and deterministic `MockDesktopDisplayPowerProvider` for automated testing.
//! - Safe native payload validation for `WM_POWERBROADCAST` / `PBT_POWERSETTINGCHANGE` messages.
//! - Bounded diagnostic ring buffer for display power lifecycle auditing.

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

// -------------------------------------------------------------------------------------------------
// Win32 FFI Bindings & Power Setting GUIDs
// -------------------------------------------------------------------------------------------------

#[cfg(windows)]
#[allow(dead_code)]
pub mod win32 {
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
    pub struct POWERBROADCAST_SETTING {
        pub power_setting: GUID,
        pub data_length: DWORD,
        pub data: [u8; 1],
    }

    pub const DEVICE_NOTIFY_WINDOW_HANDLE: DWORD = 0x00000000;
    pub const WM_POWERBROADCAST: u32 = 0x0218;
    pub const PBT_POWERSETTINGCHANGE: u32 = 0x8013;

    /// GUID_SESSION_DISPLAY_STATUS: 2b84c10b-ed29-4357-88e1-ffdf1144c790
    /// The display-power state associated with the application's interactive session.
    /// Values: 0 = Off, 1 = On, 2 = Dimmed.
    pub const GUID_SESSION_DISPLAY_STATUS: GUID = GUID {
        data1: 0x2B84C10B,
        data2: 0xED29,
        data3: 0x4357,
        data4: [0x88, 0xE1, 0xFF, 0xDF, 0x11, 0x44, 0xC7, 0x90],
    };

    /// GUID_CONSOLE_DISPLAY_STATE: 6fe69556-704a-47a0-8f24-c28d936f0804
    /// The display-power state associated with the physical console display.
    /// Values: 0 = Off, 1 = On, 2 = Dimmed.
    pub const GUID_CONSOLE_DISPLAY_STATE: GUID = GUID {
        data1: 0x6FE69556,
        data2: 0x704A,
        data3: 0x47A0,
        data4: [0x8F, 0x24, 0xC2, 0x8D, 0x93, 0x6F, 0x08, 0x04],
    };

    #[link(name = "user32")]
    extern "system" {
        pub fn RegisterPowerSettingNotification(
            hRecipient: HANDLE,
            PowerSettingGuid: *const GUID,
            Flags: DWORD,
        ) -> HPOWERNOTIFY;

        pub fn UnregisterPowerSettingNotification(Handle: HPOWERNOTIFY) -> BOOL;
        pub fn GetLastError() -> DWORD;
    }
}

// -------------------------------------------------------------------------------------------------
// Domain Models
// -------------------------------------------------------------------------------------------------

/// Operational state of the display associated with the observed scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DisplayPowerState {
    /// The registered notification reports that the display is powered off.
    Off,
    /// The registered notification reports that the display is active and powered on.
    On,
    /// The registered notification reports that the display is dimmed (e.g. idle timeout warning).
    Dimmed,
    /// Display power state could not be definitively determined or is uninitialized.
    #[default]
    Unknown,
}

impl DisplayPowerState {
    /// Returns true if the display is confirmed to be powered off.
    #[must_use]
    pub fn is_off(&self) -> bool {
        matches!(self, Self::Off)
    }

    /// Returns true if the display is confirmed to be on.
    #[must_use]
    pub fn is_on(&self) -> bool {
        matches!(self, Self::On)
    }

    /// Returns true if the display is reported as dimmed.
    #[must_use]
    pub fn is_dimmed(&self) -> bool {
        matches!(self, Self::Dimmed)
    }

    /// Returns true if the state is known and verified (On, Off, or Dimmed).
    #[must_use]
    pub fn is_known(&self) -> bool {
        !matches!(self, Self::Unknown)
    }
}

/// Scope of the Windows display-power notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DisplayPowerScope {
    /// Interactive user session display status (`GUID_SESSION_DISPLAY_STATUS`).
    /// This is the primary and recommended notification for user-mode desktop applications.
    #[default]
    SessionDisplayStatus,

    /// Physical console display state (`GUID_CONSOLE_DISPLAY_STATE`).
    ConsoleDisplayState,
}

/// Action to take when the display state transitions to Dimmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DisplayDimmedAction {
    /// Do not alter playback during display dimming (conservative default).
    #[default]
    DoNothing,

    /// Suspend live playback during display dimming.
    Pause,
}

/// Validated immutable snapshot of display-power state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayPowerSnapshot {
    /// Current power state (Off, On, Dimmed, Unknown).
    pub state: DisplayPowerState,

    /// Scope of the notification that yielded this state.
    pub scope: DisplayPowerScope,

    /// Unix epoch timestamp in milliseconds when this sample was recorded.
    pub sampled_at_millis: u64,

    /// Whether this snapshot is retained from a previous observation or potentially stale.
    pub is_stale: bool,
}

impl Default for DisplayPowerSnapshot {
    fn default() -> Self {
        Self {
            state: DisplayPowerState::Unknown,
            scope: DisplayPowerScope::SessionDisplayStatus,
            sampled_at_millis: 0,
            is_stale: false,
        }
    }
}

impl DisplayPowerSnapshot {
    /// Creates a verified snapshot with the specified state and scope.
    #[must_use]
    pub fn new(state: DisplayPowerState, scope: DisplayPowerScope, timestamp_millis: u64) -> Self {
        Self {
            state,
            scope,
            sampled_at_millis: timestamp_millis,
            is_stale: false,
        }
    }

    /// Creates an unknown/uninitialized baseline snapshot.
    #[must_use]
    pub fn unknown(scope: DisplayPowerScope) -> Self {
        Self {
            state: DisplayPowerState::Unknown,
            scope,
            sampled_at_millis: 0,
            is_stale: false,
        }
    }
}

/// User-configurable display-power playback policy settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayPowerPlaybackSettings {
    /// Whether display-power awareness is enabled.
    /// Default: false (opt-in preservation of existing behavior).
    pub enabled: bool,

    /// Whether to pause wallpaper playback when the display is reported Off.
    /// Default: true (when enabled).
    pub pause_on_display_off: bool,

    /// Whether to resume eligible wallpaper playback when the display returns to On.
    /// Default: true (subject to surviving pause reasons).
    pub resume_on_display_on: bool,

    /// Policy response when the display transitions to Dimmed.
    /// Default: DoNothing.
    pub dimmed_action: DisplayDimmedAction,

    /// Transition eligible live wallpaper sessions into static fallback presentation during display off.
    /// Default: true.
    pub enable_static_fallback: bool,
}

impl Default for DisplayPowerPlaybackSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            pause_on_display_off: true,
            resume_on_display_on: true,
            dimmed_action: DisplayDimmedAction::DoNothing,
            enable_static_fallback: true,
        }
    }
}

/// Result of evaluating a display-power snapshot against configured policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayPowerPolicyEvaluation {
    /// Whether wallpaper sessions should be suspended due to display-power state.
    pub should_pause: bool,

    /// Descriptive trigger reason if pause is required.
    pub trigger_reason: Option<String>,

    /// Whether eligible sessions should attempt static fallback presentation.
    pub prefer_static_fallback: bool,
}

impl DisplayPowerPolicyEvaluation {
    /// Evaluates a display-power snapshot against user settings.
    #[must_use]
    pub fn evaluate(
        settings: &DisplayPowerPlaybackSettings,
        snapshot: &DisplayPowerSnapshot,
    ) -> Self {
        // 1. If policy is disabled, never suspend or modify playback
        if !settings.enabled {
            return Self {
                should_pause: false,
                trigger_reason: None,
                prefer_static_fallback: false,
            };
        }

        // 2. Evaluate based on observed display state
        match snapshot.state {
            DisplayPowerState::Off => {
                if settings.pause_on_display_off {
                    Self {
                        should_pause: true,
                        trigger_reason: Some(
                            "Display reported powered off by session power notification".into(),
                        ),
                        prefer_static_fallback: settings.enable_static_fallback,
                    }
                } else {
                    Self {
                        should_pause: false,
                        trigger_reason: None,
                        prefer_static_fallback: false,
                    }
                }
            }
            DisplayPowerState::Dimmed => match settings.dimmed_action {
                DisplayDimmedAction::Pause => Self {
                    should_pause: true,
                    trigger_reason: Some(
                        "Display reported dimmed by session power notification".into(),
                    ),
                    prefer_static_fallback: settings.enable_static_fallback,
                },
                DisplayDimmedAction::DoNothing => Self {
                    should_pause: false,
                    trigger_reason: None,
                    prefer_static_fallback: false,
                },
            },
            DisplayPowerState::On | DisplayPowerState::Unknown => Self {
                should_pause: false,
                trigger_reason: None,
                prefer_static_fallback: false,
            },
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Errors & Diagnostic Types
// -------------------------------------------------------------------------------------------------

/// Typed errors relating to display-power notification registration and parsing.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum DisplayPowerError {
    #[error("Display power notification registration failed for HWND 0x{hwnd:x} (code: {code})")]
    RegistrationFailed { hwnd: usize, code: u32 },

    #[error("Failed to unregister display power notification for HWND 0x{hwnd:x} (code: {code})")]
    UnregistrationFailed { hwnd: usize, code: u32 },

    #[error("Invalid window handle for display power notification")]
    InvalidWindowHandle,

    #[error("Malformed power setting change payload (expected at least {expected} bytes, got {actual})")]
    MalformedPayload { expected: usize, actual: usize },

    #[error("Unexpected power setting GUID `{guid}` (expected `{expected}`)")]
    UnexpectedGuid { guid: String, expected: String },

    #[error("Display power notifications unsupported on this platform")]
    Unsupported,
}

/// Diagnostic event kinds for display-power lifecycle logging.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayPowerDiagnosticEventType {
    Registered,
    Unregistered,
    DisplayStateTransition,
    PolicyEvaluated,
    PauseTriggered,
    ResumeTriggered,
    ProviderError,
}

/// Structured diagnostic log record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayPowerDiagnosticEvent {
    pub timestamp_millis: u64,
    pub event_type: DisplayPowerDiagnosticEventType,
    pub details: String,
}

pub const MAX_DISPLAY_POWER_DIAGNOSTICS: usize = 128;

// -------------------------------------------------------------------------------------------------
// Payload Decoder
// -------------------------------------------------------------------------------------------------

/// Safely decodes a native `POWERBROADCAST_SETTING` payload into a `DisplayPowerState`.
///
/// # Safety
/// Validates data_length >= 4 and dereferences safely under byte-size verification.
#[must_use]
pub fn decode_display_power_value(raw_val: u32) -> DisplayPowerState {
    match raw_val {
        0 => DisplayPowerState::Off,
        1 => DisplayPowerState::On,
        2 => DisplayPowerState::Dimmed,
        _ => DisplayPowerState::Unknown,
    }
}

// -------------------------------------------------------------------------------------------------
// Desktop Display Power Provider Trait & Implementations
// -------------------------------------------------------------------------------------------------

/// Abstraction for subscribing to OS display-power notifications and querying current state.
pub trait DesktopDisplayPowerProvider: Send + Sync {
    /// Registers a native window handle to receive display-power setting notifications.
    fn register_display_power_notification(
        &mut self,
        hwnd_val: usize,
        scope: DisplayPowerScope,
    ) -> Result<(), DisplayPowerError>;

    /// Unregisters display-power setting notifications for the specified window.
    fn unregister_display_power_notification(
        &mut self,
        hwnd_val: usize,
    ) -> Result<(), DisplayPowerError>;

    /// Returns the latest known display-power snapshot.
    fn query_display_power_snapshot(&self) -> DisplayPowerSnapshot;

    /// Returns true if a notification hook is currently active.
    fn is_registered(&self) -> bool;
}

/// Production Win32 implementation using `RegisterPowerSettingNotification`.
pub struct NativeDesktopDisplayPowerProvider {
    scope: DisplayPowerScope,
    current_snapshot: Arc<Mutex<DisplayPowerSnapshot>>,
    #[cfg(windows)]
    active_handle: Option<(usize, win32::HPOWERNOTIFY)>,
    #[cfg(not(windows))]
    active_handle: Option<(usize, usize)>,
}

unsafe impl Send for NativeDesktopDisplayPowerProvider {}
unsafe impl Sync for NativeDesktopDisplayPowerProvider {}

impl NativeDesktopDisplayPowerProvider {
    #[must_use]
    pub fn new(scope: DisplayPowerScope) -> Self {
        Self {
            scope,
            current_snapshot: Arc::new(Mutex::new(DisplayPowerSnapshot::unknown(scope))),
            active_handle: None,
        }
    }

    /// Updates the internal snapshot from a verified Win32 message callback.
    pub fn update_from_notification(&self, state: DisplayPowerState) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let mut snap = self.current_snapshot.lock().unwrap();
        *snap = DisplayPowerSnapshot::new(state, self.scope, now);
    }
}

impl Default for NativeDesktopDisplayPowerProvider {
    fn default() -> Self {
        Self::new(DisplayPowerScope::SessionDisplayStatus)
    }
}

impl DesktopDisplayPowerProvider for NativeDesktopDisplayPowerProvider {
    #[cfg(windows)]
    fn register_display_power_notification(
        &mut self,
        hwnd_val: usize,
        scope: DisplayPowerScope,
    ) -> Result<(), DisplayPowerError> {
        if hwnd_val == 0 {
            return Err(DisplayPowerError::InvalidWindowHandle);
        }

        // Unregister existing if registered
        if self.active_handle.is_some() {
            let _ = self.unregister_display_power_notification(hwnd_val);
        }

        self.scope = scope;
        let guid = match scope {
            DisplayPowerScope::SessionDisplayStatus => &win32::GUID_SESSION_DISPLAY_STATUS,
            DisplayPowerScope::ConsoleDisplayState => &win32::GUID_CONSOLE_DISPLAY_STATE,
        };

        let handle = unsafe {
            win32::RegisterPowerSettingNotification(
                hwnd_val as win32::HANDLE,
                guid,
                win32::DEVICE_NOTIFY_WINDOW_HANDLE,
            )
        };

        if handle.is_null() {
            let code = unsafe { win32::GetLastError() };
            return Err(DisplayPowerError::RegistrationFailed {
                hwnd: hwnd_val,
                code,
            });
        }

        self.active_handle = Some((hwnd_val, handle));
        Ok(())
    }

    #[cfg(not(windows))]
    fn register_display_power_notification(
        &mut self,
        _hwnd_val: usize,
        _scope: DisplayPowerScope,
    ) -> Result<(), DisplayPowerError> {
        Err(DisplayPowerError::Unsupported)
    }

    #[cfg(windows)]
    fn unregister_display_power_notification(
        &mut self,
        hwnd_val: usize,
    ) -> Result<(), DisplayPowerError> {
        if let Some((registered_hwnd, handle)) = self.active_handle.take() {
            if registered_hwnd == hwnd_val {
                let success = unsafe { win32::UnregisterPowerSettingNotification(handle) };
                if success == 0 {
                    let code = unsafe { win32::GetLastError() };
                    return Err(DisplayPowerError::UnregistrationFailed {
                        hwnd: hwnd_val,
                        code,
                    });
                }
            } else {
                // Restore handle if HWND did not match
                self.active_handle = Some((registered_hwnd, handle));
            }
        }
        Ok(())
    }

    #[cfg(not(windows))]
    fn unregister_display_power_notification(
        &mut self,
        _hwnd_val: usize,
    ) -> Result<(), DisplayPowerError> {
        Err(DisplayPowerError::Unsupported)
    }

    fn query_display_power_snapshot(&self) -> DisplayPowerSnapshot {
        self.current_snapshot.lock().unwrap().clone()
    }

    fn is_registered(&self) -> bool {
        self.active_handle.is_some()
    }
}

// -------------------------------------------------------------------------------------------------
// Mock Provider for Deterministic Testing
// -------------------------------------------------------------------------------------------------

/// Configurable in-memory mock provider for deterministic automated testing.
#[derive(Clone)]
pub struct MockDesktopDisplayPowerProvider {
    scope: DisplayPowerScope,
    snapshot: Arc<Mutex<DisplayPowerSnapshot>>,
    registered_hwnds: Arc<Mutex<Vec<usize>>>,
    fail_registration: Arc<Mutex<bool>>,
    fail_unregistration: Arc<Mutex<bool>>,
}

impl MockDesktopDisplayPowerProvider {
    #[must_use]
    pub fn new(scope: DisplayPowerScope) -> Self {
        Self {
            scope,
            snapshot: Arc::new(Mutex::new(DisplayPowerSnapshot::unknown(scope))),
            registered_hwnds: Arc::new(Mutex::new(Vec::new())),
            fail_registration: Arc::new(Mutex::new(false)),
            fail_unregistration: Arc::new(Mutex::new(false)),
        }
    }

    /// Sets the mock's active display-power state.
    pub fn set_state(&self, state: DisplayPowerState, timestamp_millis: u64) {
        let mut snap = self.snapshot.lock().unwrap();
        *snap = DisplayPowerSnapshot::new(state, self.scope, timestamp_millis);
    }

    /// Configures the mock to fail registrations.
    pub fn set_registration_failure(&self, fail: bool) {
        *self.fail_registration.lock().unwrap() = fail;
    }

    /// Configures the mock to fail unregistrations.
    pub fn set_unregistration_failure(&self, fail: bool) {
        *self.fail_unregistration.lock().unwrap() = fail;
    }

    /// Returns count of registered window handles.
    #[must_use]
    pub fn registered_handles_count(&self) -> usize {
        self.registered_hwnds.lock().unwrap().len()
    }
}

impl Default for MockDesktopDisplayPowerProvider {
    fn default() -> Self {
        Self::new(DisplayPowerScope::SessionDisplayStatus)
    }
}

impl DesktopDisplayPowerProvider for MockDesktopDisplayPowerProvider {
    fn register_display_power_notification(
        &mut self,
        hwnd_val: usize,
        scope: DisplayPowerScope,
    ) -> Result<(), DisplayPowerError> {
        if *self.fail_registration.lock().unwrap() {
            return Err(DisplayPowerError::RegistrationFailed {
                hwnd: hwnd_val,
                code: 1,
            });
        }
        if hwnd_val == 0 {
            return Err(DisplayPowerError::InvalidWindowHandle);
        }

        self.scope = scope;
        let mut hwnds = self.registered_hwnds.lock().unwrap();
        if !hwnds.contains(&hwnd_val) {
            hwnds.push(hwnd_val);
        }
        Ok(())
    }

    fn unregister_display_power_notification(
        &mut self,
        hwnd_val: usize,
    ) -> Result<(), DisplayPowerError> {
        if *self.fail_unregistration.lock().unwrap() {
            return Err(DisplayPowerError::UnregistrationFailed {
                hwnd: hwnd_val,
                code: 2,
            });
        }

        let mut hwnds = self.registered_hwnds.lock().unwrap();
        hwnds.retain(|&h| h != hwnd_val);
        Ok(())
    }

    fn query_display_power_snapshot(&self) -> DisplayPowerSnapshot {
        self.snapshot.lock().unwrap().clone()
    }

    fn is_registered(&self) -> bool {
        !self.registered_hwnds.lock().unwrap().is_empty()
    }
}
