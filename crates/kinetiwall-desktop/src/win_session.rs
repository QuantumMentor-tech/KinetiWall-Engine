//! Windows Session-Lifecycle Awareness, Lock/Unlock & Remote-Session Playback Policies (Step 53).
//!
//! Provides:
//! - Native Windows `WTSRegisterSessionNotification` and `WM_WTSSESSION_CHANGE` message integration.
//! - Validated, typed Windows session-state model (`WindowsSessionSnapshot`).
//! - Distinction between workstation lock/unlock events, console connect/disconnect, and remote desktop sessions.
//! - Honest representation of unknown and unavailable session states (never guessing lock state at startup).
//! - Non-blocking event-driven updates, event decoding, and validation.
//! - Opt-in user-configurable session playback policies (`pause_on_lock`, `optimize_remote_session`).
//! - Independent pause-reason ownership (`PauseReason::WorkstationLocked`, `PauseReason::RemoteSession`).
//! - Complete coordination preserving manual user pauses, fullscreen pauses, power-saving pauses, and static fallback.
//! - Clean registration lifecycle and orderly unregistration on shutdown.

use std::sync::Mutex;

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
    pub type HWND = *mut c_void;

    pub const NOTIFY_FOR_THIS_SESSION: DWORD = 0;
    pub const NOTIFY_FOR_ALL_SESSIONS: DWORD = 1;

    pub const WM_WTSSESSION_CHANGE: u32 = 0x02B1;

    pub const WTS_CONSOLE_CONNECT: usize = 0x1;
    pub const WTS_CONSOLE_DISCONNECT: usize = 0x2;
    pub const WTS_REMOTE_CONNECT: usize = 0x3;
    pub const WTS_REMOTE_DISCONNECT: usize = 0x4;
    pub const WTS_SESSION_LOGON: usize = 0x5;
    pub const WTS_SESSION_LOGOFF: usize = 0x6;
    pub const WTS_SESSION_LOCK: usize = 0x7;
    pub const WTS_SESSION_UNLOCK: usize = 0x8;
    pub const WTS_SESSION_REMOTE_CONTROL: usize = 0x9;

    #[link(name = "wtsapi32")]
    extern "system" {
        pub fn WTSRegisterSessionNotification(hWnd: HWND, dwFlags: DWORD) -> BOOL;
        pub fn WTSUnRegisterSessionNotification(hWnd: HWND) -> BOOL;
    }

    #[link(name = "kernel32")]
    extern "system" {
        pub fn GetLastError() -> DWORD;
    }
}

// -------------------------------------------------------------------------------------------------
// Domain Models: Session State & Event Taxonomy
// -------------------------------------------------------------------------------------------------

/// Operational workstation lock state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkstationLockState {
    /// Workstation is confirmed locked (`WTS_SESSION_LOCK`).
    Locked,
    /// Workstation is confirmed unlocked (`WTS_SESSION_UNLOCK`).
    Unlocked,
    /// Workstation lock state is unknown or has not yet been established.
    /// STRICT REQUIREMENT: Initial state at application startup!
    #[default]
    Unknown,
}

impl WorkstationLockState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Locked => "locked",
            Self::Unlocked => "unlocked",
            Self::Unknown => "unknown",
        }
    }
}

/// Operational connection and console/remote session state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionConnectionState {
    /// Connected to the local physical console (`WTS_CONSOLE_CONNECT`).
    ConsoleConnected,
    /// Disconnected from the local physical console (`WTS_CONSOLE_DISCONNECT`).
    ConsoleDisconnected,
    /// Connected via remote session or RDP client (`WTS_REMOTE_CONNECT`).
    RemoteConnected,
    /// Disconnected from remote session (`WTS_REMOTE_DISCONNECT`).
    RemoteDisconnected,
    /// Session connection state is unknown or not yet determined.
    #[default]
    Unknown,
}

impl SessionConnectionState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ConsoleConnected => "console_connected",
            Self::ConsoleDisconnected => "console_disconnected",
            Self::RemoteConnected => "remote_connected",
            Self::RemoteDisconnected => "remote_disconnected",
            Self::Unknown => "unknown",
        }
    }
}

/// Notification scope requested during WTS session notification registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionNotificationScope {
    /// Receive session notifications only for the current user session (narrowest, safest scope).
    #[default]
    ThisSession,
    /// Receive session notifications for all sessions on the terminal server.
    AllSessions,
}

/// Category of decoded Windows session change notification (`WM_WTSSESSION_CHANGE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionNotificationEventKind {
    ConsoleConnect,
    ConsoleDisconnect,
    RemoteConnect,
    RemoteDisconnect,
    SessionLogon,
    SessionLogoff,
    SessionLock,
    SessionUnlock,
    SessionRemoteControl,
    Unknown(usize),
}

/// Decoded Windows session change notification event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionNotificationEvent {
    pub kind: SessionNotificationEventKind,
    pub session_id: u32,
    pub timestamp_millis: u64,
}

/// Validated point-in-time snapshot of Windows workstation and session state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowsSessionSnapshot {
    /// Current workstation lock state.
    pub lock_state: WorkstationLockState,
    /// Current session connection state.
    pub connection_state: SessionConnectionState,
    /// Current Windows session ID if determined.
    pub session_id: Option<u32>,
    /// Most recently received session event kind.
    pub last_event: Option<SessionNotificationEventKind>,
    /// Timestamp of most recently processed session event (Unix epoch millis).
    pub last_event_timestamp_millis: u64,
    /// Whether session notification listener is currently registered.
    pub is_registered: bool,
    /// Notification scope of active registration.
    pub registration_scope: SessionNotificationScope,
}

impl Default for WindowsSessionSnapshot {
    fn default() -> Self {
        Self {
            lock_state: WorkstationLockState::Unknown,
            connection_state: SessionConnectionState::Unknown,
            session_id: None,
            last_event: None,
            last_event_timestamp_millis: 0,
            is_registered: false,
            registration_scope: SessionNotificationScope::ThisSession,
        }
    }
}

impl WindowsSessionSnapshot {
    /// Applies a decoded session event, updating state idempotently and recording the event timestamp.
    pub fn apply_event(&mut self, event: &SessionNotificationEvent) {
        self.last_event = Some(event.kind);
        self.last_event_timestamp_millis = event.timestamp_millis;
        self.session_id = Some(event.session_id);

        match event.kind {
            SessionNotificationEventKind::SessionLock => {
                self.lock_state = WorkstationLockState::Locked;
            }
            SessionNotificationEventKind::SessionUnlock => {
                self.lock_state = WorkstationLockState::Unlocked;
            }
            SessionNotificationEventKind::ConsoleConnect => {
                self.connection_state = SessionConnectionState::ConsoleConnected;
            }
            SessionNotificationEventKind::ConsoleDisconnect => {
                self.connection_state = SessionConnectionState::ConsoleDisconnected;
            }
            SessionNotificationEventKind::RemoteConnect => {
                self.connection_state = SessionConnectionState::RemoteConnected;
            }
            SessionNotificationEventKind::RemoteDisconnect => {
                self.connection_state = SessionConnectionState::RemoteDisconnected;
            }
            _ => {}
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Policy Configuration Model
// -------------------------------------------------------------------------------------------------

/// User-configurable session-awareness playback policy settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPlaybackSettings {
    /// Master toggle for session-lifecycle awareness policy.
    /// STRICT REQUIREMENT: Opt-in by default (`false` for existing users).
    pub enabled: bool,

    /// Pause active wallpaper playback when the workstation is locked.
    pub pause_on_lock: bool,

    /// Pause or reduce active wallpaper playback when operating inside a Remote Desktop session.
    pub optimize_remote_session: bool,

    /// Transition eligible live wallpaper sessions into static fallback presentation during lock.
    pub enable_static_fallback: bool,
}

impl Default for SessionPlaybackSettings {
    fn default() -> Self {
        Self {
            enabled: false, // Opt-in by default
            pause_on_lock: true,
            optimize_remote_session: false,
            enable_static_fallback: false,
        }
    }
}

/// Result of evaluating session state against user-configured policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPolicyEvaluation {
    /// Whether wallpaper sessions should be suspended due to session constraints.
    pub should_pause: bool,
    /// Descriptive trigger reason if pause is required.
    pub trigger_reason: Option<String>,
    /// Whether eligible sessions should attempt static fallback presentation.
    pub prefer_static_fallback: bool,
}

impl SessionPolicyEvaluation {
    /// Evaluates current session snapshot against policy settings.
    #[must_use]
    pub fn evaluate(settings: &SessionPlaybackSettings, snapshot: &WindowsSessionSnapshot) -> Self {
        if !settings.enabled {
            return Self {
                should_pause: false,
                trigger_reason: None,
                prefer_static_fallback: false,
            };
        }

        // 1. Workstation Lock trigger
        if settings.pause_on_lock && snapshot.lock_state == WorkstationLockState::Locked {
            return Self {
                should_pause: true,
                trigger_reason: Some("Workstation is locked".into()),
                prefer_static_fallback: settings.enable_static_fallback,
            };
        }

        // 2. Remote Desktop connection optimization
        if settings.optimize_remote_session
            && snapshot.connection_state == SessionConnectionState::RemoteConnected
        {
            return Self {
                should_pause: true,
                trigger_reason: Some("Operating in Remote Desktop session".into()),
                prefer_static_fallback: settings.enable_static_fallback,
            };
        }

        Self {
            should_pause: false,
            trigger_reason: None,
            prefer_static_fallback: false,
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Errors & Notification Handles
// -------------------------------------------------------------------------------------------------

/// Typed errors relating to Windows session notifications and state management.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum WinSessionError {
    #[error("Operating system failed to register session notification (code: {code})")]
    RegistrationFailed { code: u32 },

    #[error("Operating system failed to unregister session notification (code: {code})")]
    UnregistrationFailed { code: u32 },

    #[error("Session notifications already registered for window handle `{0}`")]
    AlreadyRegistered(NativeWindowHandle),

    #[error("Window handle is not currently registered for session notifications")]
    NotRegistered,

    #[error("Target window handle `{0}` is invalid")]
    InvalidHandle(NativeWindowHandle),

    #[error("Session notification functionality is unsupported on this platform")]
    Unsupported,
}

/// Represents a successfully acquired native session notification registration.
#[derive(Debug, Default)]
pub struct SessionNotificationRegistration {
    pub hwnd: Option<NativeWindowHandle>,
    pub scope: SessionNotificationScope,
    pub is_registered: bool,
}

unsafe impl Send for SessionNotificationRegistration {}
unsafe impl Sync for SessionNotificationRegistration {}

// -------------------------------------------------------------------------------------------------
// Provider Trait & Implementations
// -------------------------------------------------------------------------------------------------

/// Abstraction for registering session notifications and decoding `WM_WTSSESSION_CHANGE` messages.
pub trait DesktopSessionNotificationProvider: Send + Sync {
    /// Registers a native window handle to receive `WM_WTSSESSION_CHANGE` notifications.
    fn register_session_notifications(
        &self,
        hwnd: NativeWindowHandle,
        scope: SessionNotificationScope,
    ) -> Result<SessionNotificationRegistration, WinSessionError>;

    /// Unregisters a previously registered native window handle.
    fn unregister_session_notifications(
        &self,
        registration: &mut SessionNotificationRegistration,
    ) -> Result<(), WinSessionError>;

    /// Decodes a raw `WM_WTSSESSION_CHANGE` message payload into a strongly-typed event.
    fn decode_session_message(
        &self,
        w_param: usize,
        l_param: isize,
        now_millis: u64,
    ) -> Option<SessionNotificationEvent>;
}

/// Native Windows implementation of `DesktopSessionNotificationProvider` using `wtsapi32.dll`.
#[derive(Debug, Default)]
pub struct NativeDesktopSessionNotificationProvider;

impl NativeDesktopSessionNotificationProvider {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl DesktopSessionNotificationProvider for NativeDesktopSessionNotificationProvider {
    fn register_session_notifications(
        &self,
        hwnd: NativeWindowHandle,
        scope: SessionNotificationScope,
    ) -> Result<SessionNotificationRegistration, WinSessionError> {
        #[cfg(windows)]
        {
            if hwnd.is_null() {
                return Err(WinSessionError::InvalidHandle(hwnd));
            }

            let flags = match scope {
                SessionNotificationScope::ThisSession => win32::NOTIFY_FOR_THIS_SESSION,
                SessionNotificationScope::AllSessions => win32::NOTIFY_FOR_ALL_SESSIONS,
            };

            let res = unsafe {
                win32::WTSRegisterSessionNotification(hwnd.as_raw() as win32::HWND, flags)
            };

            if res == 0 {
                let code = unsafe { win32::GetLastError() };
                return Err(WinSessionError::RegistrationFailed { code });
            }

            Ok(SessionNotificationRegistration {
                hwnd: Some(hwnd),
                scope,
                is_registered: true,
            })
        }
        #[cfg(not(windows))]
        {
            let _ = (hwnd, scope);
            Err(WinSessionError::Unsupported)
        }
    }

    fn unregister_session_notifications(
        &self,
        registration: &mut SessionNotificationRegistration,
    ) -> Result<(), WinSessionError> {
        #[cfg(windows)]
        {
            if !registration.is_registered {
                return Ok(());
            }

            if let Some(hwnd) = registration.hwnd {
                let res = unsafe {
                    win32::WTSUnRegisterSessionNotification(hwnd.as_raw() as win32::HWND)
                };

                if res == 0 {
                    let code = unsafe { win32::GetLastError() };
                    return Err(WinSessionError::UnregistrationFailed { code });
                }
            }

            registration.is_registered = false;
            registration.hwnd = None;
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = registration;
            Err(WinSessionError::Unsupported)
        }
    }

    fn decode_session_message(
        &self,
        w_param: usize,
        l_param: isize,
        now_millis: u64,
    ) -> Option<SessionNotificationEvent> {
        let kind = match w_param {
            0x1 => SessionNotificationEventKind::ConsoleConnect,
            0x2 => SessionNotificationEventKind::ConsoleDisconnect,
            0x3 => SessionNotificationEventKind::RemoteConnect,
            0x4 => SessionNotificationEventKind::RemoteDisconnect,
            0x5 => SessionNotificationEventKind::SessionLogon,
            0x6 => SessionNotificationEventKind::SessionLogoff,
            0x7 => SessionNotificationEventKind::SessionLock,
            0x8 => SessionNotificationEventKind::SessionUnlock,
            0x9 => SessionNotificationEventKind::SessionRemoteControl,
            other => SessionNotificationEventKind::Unknown(other),
        };

        let session_id = l_param as u32;

        Some(SessionNotificationEvent {
            kind,
            session_id,
            timestamp_millis: now_millis,
        })
    }
}

/// In-memory mock session notification provider for deterministic testing.
#[derive(Debug, Default)]
pub struct MockDesktopSessionNotificationProvider {
    pub registrations: Mutex<Vec<NativeWindowHandle>>,
    pub fail_registration: Mutex<bool>,
}

impl MockDesktopSessionNotificationProvider {
    #[must_use]
    pub fn new() -> Self {
        Self {
            registrations: Mutex::new(Vec::new()),
            fail_registration: Mutex::new(false),
        }
    }

    pub fn set_fail_registration(&self, fail: bool) {
        *self.fail_registration.lock().unwrap() = fail;
    }

    #[must_use]
    pub fn registered_handles_count(&self) -> usize {
        self.registrations.lock().unwrap().len()
    }
}

impl DesktopSessionNotificationProvider for MockDesktopSessionNotificationProvider {
    fn register_session_notifications(
        &self,
        hwnd: NativeWindowHandle,
        scope: SessionNotificationScope,
    ) -> Result<SessionNotificationRegistration, WinSessionError> {
        if *self.fail_registration.lock().unwrap() {
            return Err(WinSessionError::RegistrationFailed { code: 5 }); // Access denied
        }

        let mut regs = self.registrations.lock().unwrap();
        if regs.contains(&hwnd) {
            return Err(WinSessionError::AlreadyRegistered(hwnd));
        }
        regs.push(hwnd);

        Ok(SessionNotificationRegistration {
            hwnd: Some(hwnd),
            scope,
            is_registered: true,
        })
    }

    fn unregister_session_notifications(
        &self,
        registration: &mut SessionNotificationRegistration,
    ) -> Result<(), WinSessionError> {
        if !registration.is_registered {
            return Ok(());
        }

        if let Some(hwnd) = registration.hwnd {
            let mut regs = self.registrations.lock().unwrap();
            regs.retain(|h| h != &hwnd);
        }

        registration.is_registered = false;
        registration.hwnd = None;
        Ok(())
    }

    fn decode_session_message(
        &self,
        w_param: usize,
        l_param: isize,
        now_millis: u64,
    ) -> Option<SessionNotificationEvent> {
        let kind = match w_param {
            0x1 => SessionNotificationEventKind::ConsoleConnect,
            0x2 => SessionNotificationEventKind::ConsoleDisconnect,
            0x3 => SessionNotificationEventKind::RemoteConnect,
            0x4 => SessionNotificationEventKind::RemoteDisconnect,
            0x5 => SessionNotificationEventKind::SessionLogon,
            0x6 => SessionNotificationEventKind::SessionLogoff,
            0x7 => SessionNotificationEventKind::SessionLock,
            0x8 => SessionNotificationEventKind::SessionUnlock,
            0x9 => SessionNotificationEventKind::SessionRemoteControl,
            other => SessionNotificationEventKind::Unknown(other),
        };

        Some(SessionNotificationEvent {
            kind,
            session_id: l_param as u32,
            timestamp_millis: now_millis,
        })
    }
}

// -------------------------------------------------------------------------------------------------
// Diagnostics Model
// -------------------------------------------------------------------------------------------------

/// Maximum number of diagnostic events retained in the session policy ring buffer.
pub const MAX_SESSION_POLICY_DIAGNOSTICS: usize = 128;

/// Classification of session-lifecycle diagnostic events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionDiagnosticEventType {
    RegistrationSucceeded,
    RegistrationFailed,
    RegistrationReleased,
    LockEventObserved,
    UnlockEventObserved,
    ConsoleConnectObserved,
    ConsoleDisconnectObserved,
    RemoteConnectObserved,
    RemoteDisconnectObserved,
    PolicyPauseApplied,
    PolicyResumeApplied,
    PolicyBlockedByExistingReason,
    UnrecognizedEvent,
    Error,
}

/// Structured diagnostic event record for session-awareness operations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionDiagnosticEvent {
    pub timestamp_millis: u64,
    pub event_type: SessionDiagnosticEventType,
    pub message: String,
    pub session_id: Option<u32>,
}
