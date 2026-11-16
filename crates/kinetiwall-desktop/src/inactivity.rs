//! Windows User-Inactivity Awareness, Idle Detection & Privacy-Preserving Playback Policies (Step 55).
//!
//! Provides:
//! - Native Windows `GetLastInputInfo` / `LASTINPUTINFO` integration to observe session input timestamps.
//! - Safe elapsed-inactivity duration calculations with 32-bit tick-count rollover handling.
//! - Typed session-inactivity state model (`InactivitySnapshot`, `InactivityState`: Active, Idle, Transitioning, Unknown, Stale).
//! - Strict privacy preservation: NO keystrokes, mouse positions, window titles, or raw input events are ever captured or stored.
//! - Opt-in inactivity playback policy (`InactivityPlaybackSettings`) with safe defaults (`enabled: false`).
//! - Provider abstraction (`DesktopUserInactivityProvider`) with production `NativeDesktopUserInactivityProvider`
//!   and deterministic `MockDesktopUserInactivityProvider` for automated testing.
//! - Bounded diagnostic ring buffer for inactivity lifecycle auditing.

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

// -------------------------------------------------------------------------------------------------
// Win32 FFI Bindings
// -------------------------------------------------------------------------------------------------

#[cfg(windows)]
#[allow(dead_code)]
pub mod win32 {
    pub type BOOL = i32;
    pub type DWORD = u32;
    pub type UINT = u32;

    #[repr(C)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct LASTINPUTINFO {
        pub cb_size: UINT,
        pub dw_time: DWORD,
    }

    #[link(name = "user32")]
    extern "system" {
        pub fn GetLastInputInfo(plii: *mut LASTINPUTINFO) -> BOOL;
        pub fn GetLastError() -> DWORD;
    }

    #[link(name = "kernel32")]
    extern "system" {
        pub fn GetTickCount() -> DWORD;
        pub fn GetTickCount64() -> u64;
    }
}

// -------------------------------------------------------------------------------------------------
// Domain Models
// -------------------------------------------------------------------------------------------------

/// Operational state of session-input activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum InactivityState {
    /// Recent input observed within the configured idle threshold.
    Active,
    /// Sustained inactivity has reached or exceeded the configured idle threshold.
    Idle,
    /// Previously known state is being reconciled after a session or power transition.
    Transitioning,
    /// Input state cannot be determined, API query failed, or observation is uninitialized.
    #[default]
    Unknown,
    /// The last-input observation is too old to support a reliable policy decision.
    Stale,
}

impl InactivityState {
    /// Returns true if the session is currently confirmed idle.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }

    /// Returns true if the session has active input.
    #[must_use]
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active)
    }

    /// Returns true if the state is verified and reliable (Active or Idle).
    #[must_use]
    pub fn is_reliable(&self) -> bool {
        matches!(self, Self::Active | Self::Idle)
    }
}

/// Safe calculation of elapsed milliseconds between two Win32 32-bit tick counts.
/// Correctly handles tick-count wraparound (which occurs approximately every 49.7 days).
#[must_use]
pub fn calculate_elapsed_ticks(last_input_tick: u32, current_tick: u32) -> u64 {
    if current_tick >= last_input_tick {
        u64::from(current_tick - last_input_tick)
    } else {
        // Wraparound occurred (current_tick wrapped back past 0)
        let before_wrap = u64::from(u32::MAX - last_input_tick);
        let after_wrap = u64::from(current_tick);
        before_wrap.saturating_add(after_wrap).saturating_add(1)
    }
}

/// Validated immutable snapshot of user inactivity state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InactivitySnapshot {
    /// Current activity/idle state.
    pub state: InactivityState,

    /// Elapsed milliseconds since last recorded session input.
    pub elapsed_inactivity_millis: u64,

    /// Configured threshold in milliseconds used for this evaluation.
    pub idle_threshold_millis: u64,

    /// Unix epoch timestamp in milliseconds when this sample was recorded.
    pub sampled_at_millis: u64,

    /// Whether this snapshot is verified and fresh.
    pub is_fresh: bool,
}

impl Default for InactivitySnapshot {
    fn default() -> Self {
        Self {
            state: InactivityState::Unknown,
            elapsed_inactivity_millis: 0,
            idle_threshold_millis: DEFAULT_INACTIVITY_THRESHOLD_MILLIS,
            sampled_at_millis: 0,
            is_fresh: false,
        }
    }
}

impl InactivitySnapshot {
    /// Creates a verified snapshot.
    #[must_use]
    pub fn new(
        state: InactivityState,
        elapsed_inactivity_millis: u64,
        idle_threshold_millis: u64,
        sampled_at_millis: u64,
    ) -> Self {
        Self {
            state,
            elapsed_inactivity_millis,
            idle_threshold_millis,
            sampled_at_millis,
            is_fresh: true,
        }
    }

    /// Creates an unknown baseline snapshot.
    #[must_use]
    pub fn unknown() -> Self {
        Self::default()
    }
}

/// Default inactivity threshold: 10 minutes (600,000 ms).
pub const DEFAULT_INACTIVITY_THRESHOLD_MILLIS: u64 = 600_000;
/// Minimum allowed threshold: 10 seconds (10,000 ms).
pub const MIN_INACTIVITY_THRESHOLD_MILLIS: u64 = 10_000;
/// Maximum allowed threshold: 24 hours (86,400,000 ms).
pub const MAX_INACTIVITY_THRESHOLD_MILLIS: u64 = 86_400_000;

/// User-configurable inactivity playback policy settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InactivityPlaybackSettings {
    /// Whether user-inactivity awareness is enabled.
    /// Default: false (opt-in preservation of existing behavior).
    pub enabled: bool,

    /// Inactivity threshold in milliseconds before playback suspension.
    /// Default: 600,000 ms (10 minutes). Clamped to 10s .. 24h.
    pub idle_threshold_millis: u64,

    /// Whether to resume playback when user activity returns.
    /// Default: true (subject to surviving pause reasons).
    pub resume_on_activity: bool,

    /// Transition eligible live wallpaper sessions into static fallback presentation during inactivity.
    /// Default: true.
    pub enable_static_fallback: bool,
}

impl Default for InactivityPlaybackSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            idle_threshold_millis: DEFAULT_INACTIVITY_THRESHOLD_MILLIS,
            resume_on_activity: true,
            enable_static_fallback: true,
        }
    }
}

impl InactivityPlaybackSettings {
    /// Returns clamped valid threshold in milliseconds.
    #[must_use]
    pub fn clamped_threshold(&self) -> u64 {
        self.idle_threshold_millis
            .clamp(MIN_INACTIVITY_THRESHOLD_MILLIS, MAX_INACTIVITY_THRESHOLD_MILLIS)
    }
}

/// Result of evaluating an inactivity snapshot against configured policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InactivityPolicyEvaluation {
    /// Whether wallpaper sessions should be suspended due to user inactivity.
    pub should_pause: bool,

    /// Descriptive trigger reason if pause is required.
    pub trigger_reason: Option<String>,

    /// Whether eligible sessions should attempt static fallback presentation.
    pub prefer_static_fallback: bool,
}

impl InactivityPolicyEvaluation {
    /// Evaluates an inactivity snapshot against user settings.
    #[must_use]
    pub fn evaluate(
        settings: &InactivityPlaybackSettings,
        snapshot: &InactivitySnapshot,
    ) -> Self {
        if !settings.enabled {
            return Self {
                should_pause: false,
                trigger_reason: None,
                prefer_static_fallback: false,
            };
        }

        match snapshot.state {
            InactivityState::Idle => Self {
                should_pause: true,
                trigger_reason: Some(format!(
                    "Session input inactive for {} ms (threshold: {} ms)",
                    snapshot.elapsed_inactivity_millis,
                    settings.clamped_threshold(),
                )),
                prefer_static_fallback: settings.enable_static_fallback,
            },
            InactivityState::Active
            | InactivityState::Transitioning
            | InactivityState::Unknown
            | InactivityState::Stale => Self {
                should_pause: false,
                trigger_reason: None,
                prefer_static_fallback: false,
            },
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Errors & Diagnostics
// -------------------------------------------------------------------------------------------------

/// Typed errors relating to inactivity queries.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum InactivityError {
    #[error("Operating system failed to query last input information (code: {code})")]
    QueryFailed { code: u32 },

    #[error("Inactivity detection is unsupported on this platform")]
    Unsupported,
}

/// Diagnostic event kinds for inactivity lifecycle auditing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InactivityDiagnosticEventType {
    ObservationQueried,
    IdleThresholdReached,
    ActivityResumed,
    PolicyApplied,
    PolicyCleared,
    ProviderError,
}

/// Structured diagnostic log record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InactivityDiagnosticEvent {
    pub timestamp_millis: u64,
    pub event_type: InactivityDiagnosticEventType,
    pub details: String,
}

pub const MAX_INACTIVITY_DIAGNOSTICS: usize = 128;

// -------------------------------------------------------------------------------------------------
// Desktop User Inactivity Provider Trait & Implementations
// -------------------------------------------------------------------------------------------------

/// Abstraction for querying session last-input information.
pub trait DesktopUserInactivityProvider: Send + Sync {
    /// Queries the last input tick and current tick from the operating system.
    /// Returns `(last_input_tick, current_tick)`.
    fn query_last_input_ticks(&self) -> Result<(u32, u32), InactivityError>;

    /// Queries a complete evaluated inactivity snapshot for the given threshold.
    fn query_inactivity_snapshot(&self, threshold_millis: u64) -> InactivitySnapshot {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        match self.query_last_input_ticks() {
            Ok((last_input, current)) => {
                let elapsed = calculate_elapsed_ticks(last_input, current);
                let state = if elapsed >= threshold_millis {
                    InactivityState::Idle
                } else {
                    InactivityState::Active
                };
                InactivitySnapshot::new(state, elapsed, threshold_millis, now)
            }
            Err(_) => InactivitySnapshot::unknown(),
        }
    }
}

/// Production Win32 implementation using `GetLastInputInfo` and `GetTickCount`.
#[derive(Default)]
pub struct NativeDesktopUserInactivityProvider;

impl NativeDesktopUserInactivityProvider {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl DesktopUserInactivityProvider for NativeDesktopUserInactivityProvider {
    #[cfg(windows)]
    fn query_last_input_ticks(&self) -> Result<(u32, u32), InactivityError> {
        let mut lii = win32::LASTINPUTINFO {
            cb_size: std::mem::size_of::<win32::LASTINPUTINFO>() as u32,
            dw_time: 0,
        };

        let success = unsafe { win32::GetLastInputInfo(&mut lii) };
        if success == 0 {
            let code = unsafe { win32::GetLastError() };
            return Err(InactivityError::QueryFailed { code });
        }

        let current_tick = unsafe { win32::GetTickCount() };
        Ok((lii.dw_time, current_tick))
    }

    #[cfg(not(windows))]
    fn query_last_input_ticks(&self) -> Result<(u32, u32), InactivityError> {
        Err(InactivityError::Unsupported)
    }
}

// -------------------------------------------------------------------------------------------------
// Mock Provider for Deterministic Testing
// -------------------------------------------------------------------------------------------------

/// Configurable mock provider for deterministic automated testing.
#[derive(Clone)]
pub struct MockDesktopUserInactivityProvider {
    last_input_tick: Arc<Mutex<u32>>,
    current_tick: Arc<Mutex<u32>>,
    fail_query: Arc<Mutex<Option<u32>>>,
}

impl MockDesktopUserInactivityProvider {
    #[must_use]
    pub fn new(last_input_tick: u32, current_tick: u32) -> Self {
        Self {
            last_input_tick: Arc::new(Mutex::new(last_input_tick)),
            current_tick: Arc::new(Mutex::new(current_tick)),
            fail_query: Arc::new(Mutex::new(None)),
        }
    }

    /// Sets the mock's tick values.
    pub fn set_ticks(&self, last_input_tick: u32, current_tick: u32) {
        *self.last_input_tick.lock().unwrap() = last_input_tick;
        *self.current_tick.lock().unwrap() = current_tick;
    }

    /// Advances current tick without new input (simulates passing time).
    pub fn advance_time(&self, delta_millis: u32) {
        let mut ct = self.current_tick.lock().unwrap();
        *ct = ct.wrapping_add(delta_millis);
    }

    /// Simulates fresh user input at current tick.
    pub fn record_user_input(&self) {
        let ct = *self.current_tick.lock().unwrap();
        *self.last_input_tick.lock().unwrap() = ct;
    }

    /// Configures the mock to fail queries with a specified Win32 error code.
    pub fn set_query_failure(&self, code: Option<u32>) {
        *self.fail_query.lock().unwrap() = code;
    }
}

impl Default for MockDesktopUserInactivityProvider {
    fn default() -> Self {
        Self::new(100_000, 100_000)
    }
}

impl DesktopUserInactivityProvider for MockDesktopUserInactivityProvider {
    fn query_last_input_ticks(&self) -> Result<(u32, u32), InactivityError> {
        if let Some(code) = *self.fail_query.lock().unwrap() {
            return Err(InactivityError::QueryFailed { code });
        }
        let lit = *self.last_input_tick.lock().unwrap();
        let ct = *self.current_tick.lock().unwrap();
        Ok((lit, ct))
    }
}
