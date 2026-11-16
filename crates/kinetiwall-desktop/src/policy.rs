//! Intelligent Playback Policy, Foreground Window Awareness & Fullscreen Detection (Step 48).
//!
//! Provides:
//! - Native Windows foreground-window observation and candidate window revalidation.
//! - Privacy-preserving window state modeling (no window titles, no keystrokes, no screen capture).
//! - Geometry-based fullscreen classification distinguishing borderless fullscreen, maximized work area,
//!   desktop shell elements, and KinetiWall's own surfaces.
//! - Configurable smart pause/resume policy with per-display pause, multi-display support,
//!   independent pause-reason tracking, debouncing, hysteresis, and bounded diagnostics.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::arbitration::{
    ArbitrationDiagnosticEvent, EffectivePlaybackDecision, PolicyArbitrator,
    SessionArbitrationInput, MAX_ARBITRATION_DIAGNOSTICS,
};
use crate::display::{DisplayId, DisplayTopologySnapshot};
use crate::display_power::{
    DisplayPowerPlaybackSettings, DisplayPowerPolicyEvaluation, DisplayPowerSnapshot,
    DisplayPowerState,
};
use crate::inactivity::{
    InactivityPlaybackSettings, InactivityPolicyEvaluation, InactivitySnapshot, InactivityState,
};
use crate::layout::MultiMonitorLayoutMode;
use crate::power::{PowerPlaybackSettings, PowerPolicyEvaluation, SystemPowerSnapshot};
use crate::provider::DesktopWindowProvider;
use crate::session::{DisplaySessionCoordinator, DisplaySessionState, SessionError};
use crate::types::{DiscoveryError, NativeWindowHandle, WindowRect};
use crate::win_session::{
    SessionPlaybackSettings, SessionPolicyEvaluation, WindowsSessionSnapshot, WorkstationLockState,
};

/// Maximum diagnostic event history entries retained in memory.
pub const MAX_POLICY_DIAGNOSTIC_EVENTS: usize = 256;

// -------------------------------------------------------------------------------------------------
// Privacy-Preserving Window Observation Model
// -------------------------------------------------------------------------------------------------

/// Minimal, privacy-preserving observation of a candidate foreground window.
///
/// Contains strictly necessary geometry, visibility, and process metadata needed for
/// fullscreen policy decisions. Intentionally omits window titles, text contents,
/// document paths, URLs, keystrokes, screenshots, and persistent application logs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForegroundObservation {
    /// Window handle at observation time.
    pub handle: NativeWindowHandle,

    /// Owning OS process identifier (used to exclude KinetiWall itself).
    pub process_id: u32,

    /// Owning thread identifier.
    pub thread_id: u32,

    /// Registered window class name (used to exclude shell classes like WorkerW, Shell_TrayWnd).
    pub class_name: String,

    /// Visible frame bounding rectangle in virtual desktop screen coordinates.
    pub bounds: WindowRect,

    /// True if the window has the `WS_VISIBLE` style.
    pub is_visible: bool,

    /// True if the window is currently minimized (`IsIconic`).
    pub is_minimized: bool,

    /// True if the window is cloaked by DWM (e.g. on another virtual desktop).
    pub is_cloaked: bool,

    /// Monotonic timestamp when this observation occurred (Unix epoch millis).
    pub timestamp_millis: u64,

    /// Monotonically increasing observation sequence number.
    pub observation_generation: u64,
}

// -------------------------------------------------------------------------------------------------
// Fullscreen Classification
// -------------------------------------------------------------------------------------------------

/// Result of evaluating a candidate foreground window against active monitor geometry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "classification", rename_all = "snake_case")]
pub enum FullscreenClassification {
    /// Window is not fullscreen (e.g. normal windowed app, dialog, or floating tool).
    NotFullscreen,

    /// Probable fullscreen application covering a specific logical display.
    ProbableFullscreen {
        display_id: DisplayId,
        coverage_percent: f32,
    },

    /// Fullscreen application spanning multiple connected displays.
    FullscreenSpanning {
        display_ids: Vec<DisplayId>,
    },

    /// Ordinary maximized window conforming to the monitor's work area (taskbar remains visible).
    MaximizedWorkArea {
        display_id: DisplayId,
    },

    /// Window excluded from fullscreen evaluation (e.g. KinetiWall window, desktop shell, taskbar).
    ExcludedWindow {
        reason: String,
    },

    /// Window state or display topology could not be reliably determined.
    Uncertain {
        reason: String,
    },
}

impl FullscreenClassification {
    /// Returns true if the classification represents an active fullscreen condition.
    #[must_use]
    pub fn is_fullscreen(&self) -> bool {
        matches!(
            self,
            Self::ProbableFullscreen { .. } | Self::FullscreenSpanning { .. }
        )
    }

    /// Retrieves the primary affected display ID if single-display fullscreen.
    #[must_use]
    pub fn affected_display_id(&self) -> Option<&DisplayId> {
        match self {
            Self::ProbableFullscreen { display_id, .. } => Some(display_id),
            _ => None,
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Fullscreen Classifier Engine
// -------------------------------------------------------------------------------------------------

/// High-precision geometry classifier distinguishing fullscreen apps from ordinary maximized windows.
pub struct FullscreenClassifier;

impl FullscreenClassifier {
    /// Default borderless fullscreen tolerance in pixels (to accommodate borderless games
    /// that extend slightly beyond display borders, e.g. -1px to +3px).
    pub const FULLSCREEN_PIXEL_TOLERANCE: i32 = 4;

    /// Work area matching tolerance in pixels for identifying ordinary maximized windows.
    pub const WORK_AREA_PIXEL_TOLERANCE: i32 = 4;

    /// Classifies an observed foreground window against the current display topology snapshot.
    #[must_use]
    pub fn classify(
        observation: Option<&ForegroundObservation>,
        snapshot: &DisplayTopologySnapshot,
        known_kinetiwall_pids: &HashSet<u32>,
    ) -> FullscreenClassification {
        let Some(obs) = observation else {
            return FullscreenClassification::NotFullscreen;
        };

        // 1. Minimized or cloaked windows cannot be active fullscreen
        if obs.is_minimized {
            return FullscreenClassification::NotFullscreen;
        }
        if obs.is_cloaked {
            return FullscreenClassification::NotFullscreen;
        }
        if !obs.is_visible {
            return FullscreenClassification::NotFullscreen;
        }

        // 2. Exclude KinetiWall's own processes (management UI, player surfaces)
        if known_kinetiwall_pids.contains(&obs.process_id) {
            return FullscreenClassification::ExcludedWindow {
                reason: "Window belongs to KinetiWall application process".into(),
            };
        }

        // 3. Exclude Windows desktop shell elements (Progman, WorkerW, Shell_TrayWnd, etc.)
        let class_lower = obs.class_name.to_lowercase();
        if class_lower == "workerw"
            || class_lower == "progman"
            || class_lower == "shelldll_defview"
            || class_lower == "shell_traywnd"
            || class_lower == "shell_secondarytraywnd"
            || class_lower == "windows.ui.core.corewindow" && obs.bounds.width() == 0
        {
            return FullscreenClassification::ExcludedWindow {
                reason: format!("Desktop shell or taskbar element: {}", obs.class_name),
            };
        }

        if obs.bounds.width() <= 0 || obs.bounds.height() <= 0 {
            return FullscreenClassification::Uncertain {
                reason: "Window bounding box has non-positive dimensions".into(),
            };
        }

        if snapshot.displays.is_empty() {
            return FullscreenClassification::Uncertain {
                reason: "Active display topology snapshot has no connected displays".into(),
            };
        }

        let mut covered_displays = Vec::new();

        for desc in &snapshot.displays {
            let mon_rect = desc.geometry.monitor_rect;
            let work_rect = desc.geometry.work_area_rect;

            // 1. Check if window conforms to work area (ordinary maximized window with taskbar visible)
            if mon_rect != work_rect {
                let matches_work_left = (obs.bounds.left - work_rect.left).abs() <= Self::WORK_AREA_PIXEL_TOLERANCE;
                let matches_work_right = (obs.bounds.right - work_rect.right).abs() <= Self::WORK_AREA_PIXEL_TOLERANCE;
                let matches_work_top = (obs.bounds.top - work_rect.top).abs() <= Self::WORK_AREA_PIXEL_TOLERANCE;
                let matches_work_bottom = (obs.bounds.bottom - work_rect.bottom).abs() <= Self::WORK_AREA_PIXEL_TOLERANCE;

                if matches_work_left && matches_work_right && matches_work_top && matches_work_bottom {
                    return FullscreenClassification::MaximizedWorkArea {
                        display_id: desc.id.clone(),
                    };
                }
            }

            // 2. Check if window substantially covers the monitor rect (fullscreen)
            let covers_left = obs.bounds.left <= mon_rect.left + Self::FULLSCREEN_PIXEL_TOLERANCE;
            let covers_right = obs.bounds.right >= mon_rect.right - Self::FULLSCREEN_PIXEL_TOLERANCE;
            let covers_top = obs.bounds.top <= mon_rect.top + Self::FULLSCREEN_PIXEL_TOLERANCE;
            let covers_bottom = obs.bounds.bottom >= mon_rect.bottom - Self::FULLSCREEN_PIXEL_TOLERANCE;

            let covers_monitor = covers_left && covers_right && covers_top && covers_bottom;

            if covers_monitor {
                // Window covers the full monitor rectangle including taskbar area -> Probable Fullscreen!
                let mon_area = (mon_rect.width() as f64) * (mon_rect.height() as f64);
                let win_area = (obs.bounds.width() as f64) * (obs.bounds.height() as f64);
                let coverage_percent = if mon_area > 0.0 {
                    ((win_area.min(mon_area) / mon_area) * 100.0) as f32
                } else {
                    100.0
                };

                covered_displays.push((desc.id.clone(), coverage_percent));
            }
        }

        match covered_displays.len() {
            0 => FullscreenClassification::NotFullscreen,
            1 => {
                let (did, pct) = covered_displays.remove(0);
                FullscreenClassification::ProbableFullscreen {
                    display_id: did,
                    coverage_percent: pct,
                }
            }
            _ => {
                let dids = covered_displays.into_iter().map(|(id, _)| id).collect();
                FullscreenClassification::FullscreenSpanning { display_ids: dids }
            }
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Policy Configuration & State Management
// -------------------------------------------------------------------------------------------------

/// Multi-monitor pause behavior when a fullscreen application is detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FullscreenPauseMode {
    /// Pauses only the wallpaper session on the display covered by the fullscreen app (default).
    #[default]
    PerDisplay,

    /// Pauses wallpaper sessions on all displays whenever any monitor has fullscreen activity.
    AllDisplays,
}

/// User-controlled settings for fullscreen-aware playback optimization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FullscreenPlaybackSettings {
    /// Whether smart pause on fullscreen is enabled.
    pub enabled: bool,

    /// Multi-monitor pause scoping mode.
    pub pause_mode: FullscreenPauseMode,

    /// Hysteresis debounce threshold in milliseconds (default: 300ms).
    pub debounce_duration_millis: u64,
}

impl Default for FullscreenPlaybackSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            pause_mode: FullscreenPauseMode::PerDisplay,
            debounce_duration_millis: 300,
        }
    }
}

/// Independent reason tracking why a wallpaper session is suspended.
///
/// Ensures fullscreen and power policies only resume sessions they suspended, never overriding
/// explicit user pause, application stop, or competing policies.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum PauseReason {
    /// Suspended due to detected fullscreen application.
    Fullscreen { display_id: DisplayId },
    /// Manually suspended by the user.
    UserManual,
    /// Suspended for system power saving.
    PowerSaving,
    /// Suspended due to system suspension (sleep/hibernate).
    SystemSuspended,
    /// Suspended due to Windows workstation lock (Step 53).
    WorkstationLocked,
    /// Suspended due to Remote Desktop session optimization policy (Step 53).
    RemoteSession,
    /// Suspended due to display powered off or dimmed according to display power policy (Step 54).
    DisplayPowerOff,
    /// Suspended due to user inactivity / session input idle policy (Step 55).
    UserInactivity,
}

// -------------------------------------------------------------------------------------------------
// Policy Diagnostics
// -------------------------------------------------------------------------------------------------

/// Classification of policy diagnostic events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyEventType {
    ForegroundChanged,
    FullscreenClassified,
    PauseRequested,
    PauseApplied,
    ResumeRequested,
    ResumeApplied,
    ActionDebounced,
    ManualOverrideRespected,
    SettingUpdated,
    PowerStateChanged,
    PowerPolicyApplied,
    PowerPolicyCleared,
    SystemSuspendDetected,
    SystemResumeDetected,
    SessionStateChanged,
    SessionLockDetected,
    SessionUnlockDetected,
    SessionPolicyApplied,
    SessionPolicyCleared,
    DisplayPowerStateChanged,
    DisplayPowerOffDetected,
    DisplayPowerOnDetected,
    DisplayPowerPolicyApplied,
    DisplayPowerPolicyCleared,
    UserInactivityDetected,
    UserActivityResumed,
    InactivityPolicyApplied,
    InactivityPolicyCleared,
    ArbitrationReconciled,
    Error,
}

/// Bounded diagnostic event log for smart playback decisions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyDiagnosticEvent {
    pub timestamp_millis: u64,
    pub event_type: PolicyEventType,
    pub display_id: Option<DisplayId>,
    pub description: String,
    pub classification: Option<FullscreenClassification>,
}

// -------------------------------------------------------------------------------------------------
// Foreground Window Observer
// -------------------------------------------------------------------------------------------------

/// Observes Windows foreground window events and revalidates active handles.
pub struct DesktopForegroundObserver {
    provider: Arc<dyn DesktopWindowProvider>,
    known_kinetiwall_pids: HashSet<u32>,
    observation_generation: u64,
}

impl DesktopForegroundObserver {
    /// Creates a new `DesktopForegroundObserver`.
    #[must_use]
    pub fn new(provider: Arc<dyn DesktopWindowProvider>) -> Self {
        let mut known_pids = HashSet::new();
        // Add current process PID by default
        #[cfg(windows)]
        known_pids.insert(std::process::id());

        Self {
            provider,
            known_kinetiwall_pids: known_pids,
            observation_generation: 0,
        }
    }

    /// Registers a known KinetiWall process ID (e.g. isolated player process or UI).
    pub fn register_kinetiwall_pid(&mut self, pid: u32) {
        self.known_kinetiwall_pids.insert(pid);
    }

    /// Retrieves current known KinetiWall process IDs.
    #[must_use]
    pub fn known_kinetiwall_pids(&self) -> &HashSet<u32> {
        &self.known_kinetiwall_pids
    }

    /// Gathers and revalidates an observation of the current foreground window.
    pub fn observe_foreground(&mut self) -> Result<Option<ForegroundObservation>, DiscoveryError> {
        let Some(hwnd) = self.provider.get_foreground_window() else {
            return Ok(None);
        };

        if hwnd.is_null() || !self.provider.is_window(hwnd) {
            return Ok(None);
        }

        self.observation_generation = self.observation_generation.wrapping_add(1);

        let (tid, pid) = self.provider.get_window_thread_process_id(hwnd);
        let class_name = self.provider.get_class_name(hwnd).unwrap_or_default();
        let is_visible = self.provider.is_window_visible(hwnd);
        let is_minimized = self.provider.is_iconic(hwnd);
        let is_cloaked = self.provider.is_window_cloaked(hwnd);
        let bounds = self.provider.get_extended_frame_bounds(hwnd)?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Ok(Some(ForegroundObservation {
            handle: hwnd,
            process_id: pid,
            thread_id: tid,
            class_name,
            bounds,
            is_visible,
            is_minimized,
            is_cloaked,
            timestamp_millis: now,
            observation_generation: self.observation_generation,
        }))
    }
}

// -------------------------------------------------------------------------------------------------
// Smart Playback Policy Evaluator
// -------------------------------------------------------------------------------------------------

/// Evaluates foreground observations against user settings and coordinates reversible pause/resume.
pub struct SmartPlaybackPolicyEvaluator {
    settings: FullscreenPlaybackSettings,
    power_settings: PowerPlaybackSettings,
    current_power_snapshot: SystemPowerSnapshot,
    session_settings: SessionPlaybackSettings,
    current_session_snapshot: WindowsSessionSnapshot,
    display_power_settings: DisplayPowerPlaybackSettings,
    current_display_power_snapshot: DisplayPowerSnapshot,
    inactivity_settings: InactivityPlaybackSettings,
    current_inactivity_snapshot: InactivitySnapshot,
    session_coordinator: Arc<Mutex<DisplaySessionCoordinator>>,
    observer: DesktopForegroundObserver,
    active_pause_reasons: HashMap<DisplayId, HashSet<PauseReason>>,
    last_classification: FullscreenClassification,
    pending_fullscreen_candidate: Option<(DisplayId, u64)>, // (DisplayId, first_detected_millis)
    diagnostic_history: Vec<PolicyDiagnosticEvent>,
    arbitration_diagnostics: Vec<ArbitrationDiagnosticEvent>,
}

impl SmartPlaybackPolicyEvaluator {
    /// Creates a new `SmartPlaybackPolicyEvaluator`.
    #[must_use]
    pub fn new(
        settings: FullscreenPlaybackSettings,
        session_coordinator: Arc<Mutex<DisplaySessionCoordinator>>,
        provider: Arc<dyn DesktopWindowProvider>,
    ) -> Self {
        let observer = DesktopForegroundObserver::new(provider);
        Self {
            settings,
            power_settings: PowerPlaybackSettings::default(),
            current_power_snapshot: SystemPowerSnapshot::default(),
            session_settings: SessionPlaybackSettings::default(),
            current_session_snapshot: WindowsSessionSnapshot::default(),
            display_power_settings: DisplayPowerPlaybackSettings::default(),
            current_display_power_snapshot: DisplayPowerSnapshot::default(),
            inactivity_settings: InactivityPlaybackSettings::default(),
            current_inactivity_snapshot: InactivitySnapshot::default(),
            session_coordinator,
            observer,
            active_pause_reasons: HashMap::new(),
            last_classification: FullscreenClassification::NotFullscreen,
            pending_fullscreen_candidate: None,
            diagnostic_history: Vec::new(),
            arbitration_diagnostics: Vec::new(),
        }
    }

    /// Creates a `SmartPlaybackPolicyEvaluator` with custom power playback settings.
    #[must_use]
    pub fn with_power_settings(
        settings: FullscreenPlaybackSettings,
        power_settings: PowerPlaybackSettings,
        session_coordinator: Arc<Mutex<DisplaySessionCoordinator>>,
        provider: Arc<dyn DesktopWindowProvider>,
    ) -> Self {
        let observer = DesktopForegroundObserver::new(provider);
        Self {
            settings,
            power_settings,
            current_power_snapshot: SystemPowerSnapshot::default(),
            session_settings: SessionPlaybackSettings::default(),
            current_session_snapshot: WindowsSessionSnapshot::default(),
            display_power_settings: DisplayPowerPlaybackSettings::default(),
            current_display_power_snapshot: DisplayPowerSnapshot::default(),
            inactivity_settings: InactivityPlaybackSettings::default(),
            current_inactivity_snapshot: InactivitySnapshot::default(),
            session_coordinator,
            observer,
            active_pause_reasons: HashMap::new(),
            last_classification: FullscreenClassification::NotFullscreen,
            pending_fullscreen_candidate: None,
            diagnostic_history: Vec::new(),
            arbitration_diagnostics: Vec::new(),
        }
    }

    /// Retrieves current fullscreen settings.
    #[must_use]
    pub fn settings(&self) -> &FullscreenPlaybackSettings {
        &self.settings
    }

    /// Retrieves current power settings.
    #[must_use]
    pub fn power_settings(&self) -> &PowerPlaybackSettings {
        &self.power_settings
    }

    /// Retrieves current system power snapshot.
    #[must_use]
    pub fn power_snapshot(&self) -> &SystemPowerSnapshot {
        &self.current_power_snapshot
    }

    /// Returns true if a display currently has an active power saving pause reason.
    #[must_use]
    pub fn is_display_power_paused(&self, display_id: &DisplayId) -> bool {
        self.active_pause_reasons
            .get(display_id)
            .map_or(false, |set| set.contains(&PauseReason::PowerSaving))
    }

    /// Returns true if a display currently has an active system suspended pause reason.
    #[must_use]
    pub fn is_display_system_suspended(&self, display_id: &DisplayId) -> bool {
        self.active_pause_reasons
            .get(display_id)
            .map_or(false, |set| set.contains(&PauseReason::SystemSuspended))
    }

    /// Retrieves current Windows session policy settings.
    #[must_use]
    pub fn session_settings(&self) -> &SessionPlaybackSettings {
        &self.session_settings
    }

    /// Retrieves current Windows session snapshot.
    #[must_use]
    pub fn session_snapshot(&self) -> &WindowsSessionSnapshot {
        &self.current_session_snapshot
    }

    /// Returns true if a display currently has an active workstation locked pause reason.
    #[must_use]
    pub fn is_display_workstation_locked(&self, display_id: &DisplayId) -> bool {
        self.active_pause_reasons
            .get(display_id)
            .map_or(false, |set| set.contains(&PauseReason::WorkstationLocked))
    }

    /// Returns true if a display currently has an active remote session pause reason.
    #[must_use]
    pub fn is_display_remote_session_paused(&self, display_id: &DisplayId) -> bool {
        self.active_pause_reasons
            .get(display_id)
            .map_or(false, |set| set.contains(&PauseReason::RemoteSession))
    }

    /// Returns true if a display currently has an active user manual pause reason.
    #[must_use]
    pub fn is_display_manually_paused(&self, display_id: &DisplayId) -> bool {
        self.active_pause_reasons
            .get(display_id)
            .map_or(false, |set| set.contains(&PauseReason::UserManual))
    }

    /// Retrieves current display power policy settings.
    #[must_use]
    pub fn display_power_settings(&self) -> &DisplayPowerPlaybackSettings {
        &self.display_power_settings
    }

    /// Retrieves current display power snapshot.
    #[must_use]
    pub fn display_power_snapshot(&self) -> &DisplayPowerSnapshot {
        &self.current_display_power_snapshot
    }

    /// Returns true if a display currently has an active display power off pause reason.
    #[must_use]
    pub fn is_display_power_off_paused(&self, display_id: &DisplayId) -> bool {
        self.active_pause_reasons
            .get(display_id)
            .map_or(false, |set| set.contains(&PauseReason::DisplayPowerOff))
    }

    /// Retrieves current user inactivity policy settings.
    #[must_use]
    pub fn inactivity_settings(&self) -> &InactivityPlaybackSettings {
        &self.inactivity_settings
    }

    /// Retrieves current user inactivity snapshot.
    #[must_use]
    pub fn inactivity_snapshot(&self) -> &InactivitySnapshot {
        &self.current_inactivity_snapshot
    }

    /// Returns true if a display currently has an active user inactivity pause reason.
    #[must_use]
    pub fn is_display_inactivity_paused(&self, display_id: &DisplayId) -> bool {
        self.active_pause_reasons
            .get(display_id)
            .map_or(false, |set| set.contains(&PauseReason::UserInactivity))
    }

    /// Evaluates the deterministic effective playback decision for a specific display.
    #[must_use]
    pub fn arbitrate_display_session(&self, display_id: &DisplayId) -> EffectivePlaybackDecision {
        let (current_state, session_id) = {
            let sc = self.session_coordinator.lock().unwrap();
            if let Some(session) = sc.get_session(display_id) {
                (session.state.clone(), Some(session.session_id))
            } else {
                (DisplaySessionState::Uninitialized, None)
            }
        };

        let reasons = self
            .active_pause_reasons
            .get(display_id)
            .cloned()
            .unwrap_or_default();

        let input = SessionArbitrationInput {
            display_id: display_id.clone(),
            session_id,
            current_actual_state: current_state,
            active_pause_reasons: reasons,
            allows_static_fallback: self.power_settings.enable_static_fallback
                || self.display_power_settings.enable_static_fallback
                || self.inactivity_settings.enable_static_fallback,
            default_fallback_source: None,
            adaptive_fps_cap: None,
            budget_fps_cap: None,
            canonical_target_fps: 60,
            is_session_terminated: false,
            termination_reason: None,
        };

        PolicyArbitrator::arbitrate(&input)
    }

    /// Evaluates and reconciles deterministic playback decisions across all displays in the topology.
    pub fn reconcile_arbitration(
        &mut self,
        snapshot: &DisplayTopologySnapshot,
    ) -> Vec<EffectivePlaybackDecision> {
        let mut decisions = Vec::new();

        for desc in &snapshot.displays {
            let decision = self.arbitrate_display_session(&desc.id);
            if decision.requires_transition {
                let _ = PolicyArbitrator::reconcile_decision(&decision, &self.session_coordinator);
                self.record_diagnostic(
                    PolicyEventType::ArbitrationReconciled,
                    Some(desc.id.clone()),
                    format!(
                        "Arbitrated decision applied: desired={:?}, primary_reason={}, explanations={:?}",
                        decision.desired_state, decision.primary_reason_code, decision.explanation_codes
                    ),
                    None,
                );
            }

            // Record bounded arbitration diagnostic event for state tracking
            if self.arbitration_diagnostics.len() >= MAX_ARBITRATION_DIAGNOSTICS {
                self.arbitration_diagnostics.remove(0);
            }
            self.arbitration_diagnostics.push(ArbitrationDiagnosticEvent {
                timestamp_millis: decision.evaluated_at_millis,
                display_id: decision.display_id.clone(),
                session_id: decision.session_id,
                desired_state: decision.desired_state.clone(),
                actual_state: decision.actual_state.clone(),
                requires_transition: decision.requires_transition,
                explanation_codes: decision.explanation_codes.clone(),
                primary_explanation: decision.primary_explanation.clone(),
                primary_reason_code: decision.primary_reason_code.clone(),
            });

            decisions.push(decision);
        }

        decisions
    }

    /// Retrieves bounded arbitration diagnostic history.
    #[must_use]
    pub fn arbitration_diagnostics(&self) -> &[ArbitrationDiagnosticEvent] {
        &self.arbitration_diagnostics
    }

    /// Updates fullscreen policy settings and reconciles active sessions if disabled.
    pub fn update_settings(&mut self, new_settings: FullscreenPlaybackSettings) {
        let was_enabled = self.settings.enabled;
        self.settings = new_settings;

        self.record_diagnostic(
            PolicyEventType::SettingUpdated,
            None,
            format!(
                "Fullscreen policy settings updated (enabled: {}, mode: {:?})",
                self.settings.enabled, self.settings.pause_mode
            ),
            None,
        );

        if was_enabled && !self.settings.enabled {
            // Reconcile: resume any sessions that were paused exclusively due to fullscreen policy
            self.clear_all_fullscreen_pauses();
        }
    }

    /// Mutable reference to the foreground observer (e.g. to register known PIDs).
    pub fn observer_mut(&mut self) -> &mut DesktopForegroundObserver {
        &mut self.observer
    }

    /// Retrieves the most recent classification.
    #[must_use]
    pub fn last_classification(&self) -> &FullscreenClassification {
        &self.last_classification
    }

    /// Returns true if a display currently has an active fullscreen pause reason.
    #[must_use]
    pub fn is_display_fullscreen_paused(&self, display_id: &DisplayId) -> bool {
        self.active_pause_reasons
            .get(display_id)
            .map_or(false, |set| set.iter().any(|r| matches!(r, PauseReason::Fullscreen { .. })))
    }

    /// Registers a known KinetiWall process ID.
    pub fn register_kinetiwall_pid(&mut self, pid: u32) {
        self.observer.register_kinetiwall_pid(pid);
    }

    /// Sets an explicit manual pause reason on a display session.
    pub fn set_user_manual_pause(&mut self, display_id: &DisplayId, paused: bool) -> Result<(), SessionError> {
        let reasons = self.active_pause_reasons.entry(display_id.clone()).or_default();
        if paused {
            reasons.insert(PauseReason::UserManual);
            let mut sc = self.session_coordinator.lock().unwrap();
            sc.suspend_session(display_id)?;
        } else {
            reasons.remove(&PauseReason::UserManual);
            if reasons.is_empty() {
                let mut sc = self.session_coordinator.lock().unwrap();
                sc.resume_session(display_id)?;
            }
        }
        Ok(())
    }

    /// Registers a user manual pause for the specified display.
    pub fn register_user_pause(&mut self, display_id: &DisplayId) -> Result<(), SessionError> {
        self.set_user_manual_pause(display_id, true)
    }

    /// Clears a user manual pause for the specified display.
    pub fn clear_user_pause(&mut self, display_id: &DisplayId) -> Result<(), SessionError> {
        self.set_user_manual_pause(display_id, false)
    }

    /// Evaluates current foreground window and applies smart pause/resume across display sessions.
    pub fn evaluate_and_reconcile(
        &mut self,
        snapshot: &DisplayTopologySnapshot,
        layout_mode: MultiMonitorLayoutMode,
    ) -> Result<FullscreenClassification, DiscoveryError> {
        let observation = self.observer.observe_foreground()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Ok(self.evaluate_observation(
            observation.as_ref(),
            snapshot,
            layout_mode,
            now,
        ))
    }

    /// Directly evaluates a provided observation with explicit timestamp (useful for testing or event pumps).
    pub fn evaluate_observation(
        &mut self,
        observation: Option<&ForegroundObservation>,
        snapshot: &DisplayTopologySnapshot,
        layout_mode: MultiMonitorLayoutMode,
        now_millis: u64,
    ) -> FullscreenClassification {
        let classification = FullscreenClassifier::classify(
            observation,
            snapshot,
            self.observer.known_kinetiwall_pids(),
        );

        self.last_classification = classification.clone();

        if !self.settings.enabled {
            return classification;
        }

        match &classification {
            FullscreenClassification::ProbableFullscreen { display_id, .. } => {
                self.handle_fullscreen_detected(display_id, now_millis, snapshot, layout_mode);
            }
            FullscreenClassification::FullscreenSpanning { display_ids } => {
                for did in display_ids {
                    self.handle_fullscreen_detected(did, now_millis, snapshot, layout_mode);
                }
            }
            FullscreenClassification::NotFullscreen
            | FullscreenClassification::MaximizedWorkArea { .. }
            | FullscreenClassification::ExcludedWindow { .. }
            | FullscreenClassification::Uncertain { .. } => {
                self.pending_fullscreen_candidate = None;
                self.reconcile_fullscreen_cleared(snapshot);
            }
        }

        classification
    }

    /// Handles detected fullscreen on a display with debouncing.
    fn handle_fullscreen_detected(
        &mut self,
        display_id: &DisplayId,
        now: u64,
        snapshot: &DisplayTopologySnapshot,
        layout_mode: MultiMonitorLayoutMode,
    ) {
        let is_debounced = if self.settings.debounce_duration_millis == 0 {
            true
        } else {
            match self.pending_fullscreen_candidate {
                Some((ref cand_id, first_time)) if cand_id == display_id => {
                    now.saturating_sub(first_time) >= self.settings.debounce_duration_millis
                }
                _ => {
                    self.pending_fullscreen_candidate = Some((display_id.clone(), now));
                    self.record_diagnostic(
                        PolicyEventType::ActionDebounced,
                        Some(display_id.clone()),
                        format!(
                            "Debouncing fullscreen candidate for display `{}` (threshold: {}ms)",
                            display_id, self.settings.debounce_duration_millis
                        ),
                        Some(self.last_classification.clone()),
                    );
                    false
                }
            }
        };

        if is_debounced {
            self.apply_fullscreen_pause(display_id, snapshot, layout_mode);
        }
    }

    /// Applies fullscreen pause according to configured mode (PerDisplay or AllDisplays).
    fn apply_fullscreen_pause(
        &mut self,
        target_display_id: &DisplayId,
        snapshot: &DisplayTopologySnapshot,
        layout_mode: MultiMonitorLayoutMode,
    ) {
        let displays_to_pause: Vec<DisplayId> = match (self.settings.pause_mode, layout_mode) {
            (FullscreenPauseMode::AllDisplays, _) | (_, MultiMonitorLayoutMode::Clone) => {
                // Pause all connected displays
                snapshot.displays.iter().map(|d| d.id.clone()).collect()
            }
            (FullscreenPauseMode::PerDisplay, MultiMonitorLayoutMode::Independent) => {
                // Per-display: only pause the covered display
                vec![target_display_id.clone()]
            }
            (FullscreenPauseMode::PerDisplay, MultiMonitorLayoutMode::Span) => {
                // In Span mode, virtual desktop surface is shared; pause group
                snapshot.displays.iter().map(|d| d.id.clone()).collect()
            }
        };

        let mut suspended = Vec::new();
        {
            let mut sc = self.session_coordinator.lock().unwrap();

            for did in displays_to_pause {
                let reasons = self.active_pause_reasons.entry(did.clone()).or_default();
                let reason = PauseReason::Fullscreen {
                    display_id: target_display_id.clone(),
                };

                if !reasons.contains(&reason) {
                    reasons.insert(reason);

                    if let Some(session) = sc.get_session(&did) {
                        if session.state.is_active() {
                            let _ = sc.suspend_session(&did);
                            suspended.push(did);
                        }
                    }
                }
            }
        }

        for did in suspended {
            self.record_diagnostic(
                PolicyEventType::PauseApplied,
                Some(did.clone()),
                format!("Paused wallpaper session on display `{}` due to fullscreen app", did),
                Some(self.last_classification.clone()),
            );
        }
    }

    /// Reconciles sessions when fullscreen condition clears.
    fn reconcile_fullscreen_cleared(&mut self, snapshot: &DisplayTopologySnapshot) {
        let mut to_resume = Vec::new();
        let mut manual_overrides = Vec::new();

        for desc in &snapshot.displays {
            let did = &desc.id;

            if let Some(reasons) = self.active_pause_reasons.get_mut(did) {
                // Remove fullscreen pause reasons
                reasons.retain(|r| !matches!(r, PauseReason::Fullscreen { .. }));

                // Resume ONLY if no other pause reasons remain (e.g. no UserManual pause!)
                if reasons.is_empty() {
                    to_resume.push(did.clone());
                } else {
                    manual_overrides.push((did.clone(), format!("{:?}", reasons)));
                }
            }
        }

        for (did, reasons_str) in manual_overrides {
            self.record_diagnostic(
                PolicyEventType::ManualOverrideRespected,
                Some(did.clone()),
                format!(
                    "Display `{}` retained in suspended state due to surviving reasons: {}",
                    did, reasons_str
                ),
                Some(self.last_classification.clone()),
            );
        }

        let mut resumed = Vec::new();
        {
            let mut sc = self.session_coordinator.lock().unwrap();

            for did in to_resume {
                if let Some(session) = sc.get_session(&did) {
                    if matches!(
                        session.state,
                        DisplaySessionState::Suspended | DisplaySessionState::StaticFallback { .. }
                    ) {
                        let _ = sc.resume_session(&did);
                        resumed.push(did);
                    }
                }
            }
        }

        for did in resumed {
            self.record_diagnostic(
                PolicyEventType::ResumeApplied,
                Some(did.clone()),
                format!("Resumed wallpaper session on display `{}` as fullscreen cleared", did),
                Some(self.last_classification.clone()),
            );
        }
    }

    /// Clears all fullscreen pause reasons (e.g. when policy is disabled).
    fn clear_all_fullscreen_pauses(&mut self) {
        let mut sc = self.session_coordinator.lock().unwrap();

        for (did, reasons) in &mut self.active_pause_reasons {
            reasons.retain(|r| !matches!(r, PauseReason::Fullscreen { .. }));

            if reasons.is_empty() {
                if let Some(session) = sc.get_session(did) {
                    if matches!(
                        session.state,
                        DisplaySessionState::Suspended | DisplaySessionState::StaticFallback { .. }
                    ) {
                        let _ = sc.resume_session(did);
                    }
                }
            }
        }
    }

    /// Updates power playback settings and re-evaluates active sessions.
    pub fn update_power_settings(
        &mut self,
        new_settings: PowerPlaybackSettings,
        snapshot: &DisplayTopologySnapshot,
    ) {
        self.power_settings = new_settings;
        self.record_diagnostic(
            PolicyEventType::SettingUpdated,
            None,
            format!(
                "Power policy settings updated (mode: {:?}, pause_on_battery: {})",
                self.power_settings.mode, self.power_settings.pause_on_battery
            ),
            None,
        );

        let power_snap = self.current_power_snapshot.clone();
        self.evaluate_power_state(&power_snap, snapshot);
    }

    /// Evaluates current power state and reconciles wallpaper sessions across all displays.
    pub fn evaluate_power_state(
        &mut self,
        power_snapshot: &SystemPowerSnapshot,
        snapshot: &DisplayTopologySnapshot,
    ) {
        self.current_power_snapshot = power_snapshot.clone();

        self.record_diagnostic(
            PolicyEventType::PowerStateChanged,
            None,
            format!(
                "Power state observed: source={:?}, battery={:?}%, saver={:?}",
                power_snapshot.power_source,
                power_snapshot.battery_percent,
                power_snapshot.battery_saver
            ),
            None,
        );

        let eval = PowerPolicyEvaluation::evaluate(&self.power_settings, power_snapshot);

        if eval.should_pause {
            let trigger = eval
                .trigger_reason
                .unwrap_or_else(|| "Power saving policy active".into());
            self.apply_power_pause(&trigger, snapshot);
        } else {
            self.reconcile_power_cleared(snapshot);
        }
    }

    /// Handles system suspend notification (sleep/hibernate).
    pub fn handle_system_suspend(&mut self, snapshot: &DisplayTopologySnapshot) {
        self.record_diagnostic(
            PolicyEventType::SystemSuspendDetected,
            None,
            "Operating system suspension (sleep/hibernate) detected; suspending sessions".into(),
            None,
        );

        let mut suspended = Vec::new();
        {
            let mut sc = self.session_coordinator.lock().unwrap();
            for desc in &snapshot.displays {
                let did = &desc.id;
                let reasons = self.active_pause_reasons.entry(did.clone()).or_default();
                reasons.insert(PauseReason::SystemSuspended);

                if let Some(session) = sc.get_session(did) {
                    if session.state.is_active() {
                        let _ = sc.suspend_session(did);
                        suspended.push(did.clone());
                    }
                }
            }
        }

        for did in suspended {
            self.record_diagnostic(
                PolicyEventType::PauseApplied,
                Some(did.clone()),
                format!("Paused wallpaper session on display `{}` due to system suspend", did),
                None,
            );
        }
    }

    /// Handles system resume notification and re-evaluates with freshly queried power state.
    pub fn handle_system_resume(
        &mut self,
        fresh_power_snapshot: &SystemPowerSnapshot,
        snapshot: &DisplayTopologySnapshot,
    ) {
        self.record_diagnostic(
            PolicyEventType::SystemResumeDetected,
            None,
            "Operating system resume detected; reconciling power state and display sessions".into(),
            None,
        );

        for desc in &snapshot.displays {
            if let Some(reasons) = self.active_pause_reasons.get_mut(&desc.id) {
                reasons.remove(&PauseReason::SystemSuspended);
            }
        }

        self.evaluate_power_state(fresh_power_snapshot, snapshot);
    }

    /// Applies power-saving pause across all displays participating in desktop presentation.
    fn apply_power_pause(&mut self, trigger: &str, snapshot: &DisplayTopologySnapshot) {
        let mut suspended = Vec::new();
        {
            let mut sc = self.session_coordinator.lock().unwrap();
            for desc in &snapshot.displays {
                let did = &desc.id;
                let reasons = self.active_pause_reasons.entry(did.clone()).or_default();
                let reason = PauseReason::PowerSaving;

                if !reasons.contains(&reason) {
                    reasons.insert(reason);

                    if let Some(session) = sc.get_session(did) {
                        if session.state.is_active() {
                            let _ = sc.suspend_session(did);
                            suspended.push(did.clone());
                        }
                    }
                }
            }
        }

        for did in suspended {
            self.record_diagnostic(
                PolicyEventType::PowerPolicyApplied,
                Some(did.clone()),
                format!("Paused display `{}` due to power policy: {}", did, trigger),
                None,
            );
        }
    }

    /// Reconciles displays when power-saving constraints clear.
    fn reconcile_power_cleared(&mut self, snapshot: &DisplayTopologySnapshot) {
        let mut to_resume = Vec::new();
        let mut manual_overrides = Vec::new();

        for desc in &snapshot.displays {
            let did = &desc.id;

            if let Some(reasons) = self.active_pause_reasons.get_mut(did) {
                // Remove power saving reason
                reasons.remove(&PauseReason::PowerSaving);

                // Resume ONLY if no other pause reasons remain (e.g. no UserManual, Fullscreen, or SystemSuspended!)
                if reasons.is_empty() {
                    to_resume.push(did.clone());
                } else {
                    manual_overrides.push((did.clone(), format!("{:?}", reasons)));
                }
            }
        }

        for (did, reasons_str) in manual_overrides {
            self.record_diagnostic(
                PolicyEventType::ManualOverrideRespected,
                Some(did.clone()),
                format!(
                    "Display `{}` retained in suspended state after power cleared due to surviving reasons: {}",
                    did, reasons_str
                ),
                None,
            );
        }

        let mut resumed = Vec::new();
        {
            let mut sc = self.session_coordinator.lock().unwrap();
            for did in to_resume {
                if let Some(session) = sc.get_session(&did) {
                    if matches!(
                        session.state,
                        DisplaySessionState::Suspended | DisplaySessionState::StaticFallback { .. }
                    ) {
                        let _ = sc.resume_session(&did);
                        resumed.push(did);
                    }
                }
            }
        }

        for did in resumed {
            self.record_diagnostic(
                PolicyEventType::PowerPolicyCleared,
                Some(did.clone()),
                format!("Resumed wallpaper session on display `{}` as power constraints cleared", did),
                None,
            );
        }
    }

    /// Updates session policy settings and reconciles active sessions if disabled.
    pub fn update_session_settings(
        &mut self,
        new_settings: SessionPlaybackSettings,
        snapshot: &DisplayTopologySnapshot,
    ) {
        let was_enabled = self.session_settings.enabled;
        self.session_settings = new_settings;

        self.record_diagnostic(
            PolicyEventType::SettingUpdated,
            None,
            format!(
                "Session policy settings updated (enabled: {}, pause_on_lock: {}, optimize_remote: {})",
                self.session_settings.enabled,
                self.session_settings.pause_on_lock,
                self.session_settings.optimize_remote_session
            ),
            None,
        );

        let snap = self.current_session_snapshot.clone();
        if was_enabled && !self.session_settings.enabled {
            self.reconcile_lock_cleared(snapshot);
            self.reconcile_remote_cleared(snapshot);
        } else {
            self.evaluate_session_state(&snap, snapshot);
        }
    }

    /// Evaluates current Windows session state and reconciles wallpaper sessions across all displays.
    pub fn evaluate_session_state(
        &mut self,
        session_snapshot: &WindowsSessionSnapshot,
        snapshot: &DisplayTopologySnapshot,
    ) {
        self.current_session_snapshot = session_snapshot.clone();

        self.record_diagnostic(
            PolicyEventType::SessionStateChanged,
            None,
            format!(
                "Windows session state observed: lock={:?}, connection={:?}, session_id={:?}",
                session_snapshot.lock_state,
                session_snapshot.connection_state,
                session_snapshot.session_id
            ),
            None,
        );

        let eval = SessionPolicyEvaluation::evaluate(&self.session_settings, session_snapshot);

        if eval.should_pause {
            let trigger = eval
                .trigger_reason
                .unwrap_or_else(|| "Session policy active".into());
            if session_snapshot.lock_state == WorkstationLockState::Locked {
                self.apply_lock_pause(&trigger, snapshot);
            } else {
                self.apply_remote_pause(&trigger, snapshot);
            }
        } else {
            if session_snapshot.lock_state != WorkstationLockState::Locked {
                self.reconcile_lock_cleared(snapshot);
            }
            if session_snapshot.connection_state != crate::win_session::SessionConnectionState::RemoteConnected {
                self.reconcile_remote_cleared(snapshot);
            }
        }
    }

    /// Handles explicit workstation lock event.
    pub fn handle_workstation_lock(&mut self, snapshot: &DisplayTopologySnapshot) {
        self.current_session_snapshot.lock_state = WorkstationLockState::Locked;
        self.record_diagnostic(
            PolicyEventType::SessionLockDetected,
            None,
            "Windows workstation lock detected".into(),
            None,
        );

        if self.session_settings.enabled && self.session_settings.pause_on_lock {
            self.apply_lock_pause("Workstation is locked", snapshot);
        }
    }

    /// Handles explicit workstation unlock event.
    pub fn handle_workstation_unlock(&mut self, snapshot: &DisplayTopologySnapshot) {
        self.current_session_snapshot.lock_state = WorkstationLockState::Unlocked;
        self.record_diagnostic(
            PolicyEventType::SessionUnlockDetected,
            None,
            "Windows workstation unlock detected".into(),
            None,
        );

        self.reconcile_lock_cleared(snapshot);
    }

    /// Applies workstation-lock pause across all displays participating in desktop presentation.
    fn apply_lock_pause(&mut self, trigger: &str, snapshot: &DisplayTopologySnapshot) {
        let mut suspended = Vec::new();
        {
            let mut sc = self.session_coordinator.lock().unwrap();
            for desc in &snapshot.displays {
                let did = &desc.id;
                let reasons = self.active_pause_reasons.entry(did.clone()).or_default();
                let reason = PauseReason::WorkstationLocked;

                if !reasons.contains(&reason) {
                    reasons.insert(reason);

                    if let Some(session) = sc.get_session(did) {
                        if session.state.is_active() {
                            let _ = sc.suspend_session(did);
                            suspended.push(did.clone());
                        }
                    }
                }
            }
        }

        for did in suspended {
            self.record_diagnostic(
                PolicyEventType::SessionPolicyApplied,
                Some(did.clone()),
                format!("Paused display `{}` due to workstation lock: {}", did, trigger),
                None,
            );
        }
    }

    /// Reconciles displays when workstation lock clears.
    fn reconcile_lock_cleared(&mut self, snapshot: &DisplayTopologySnapshot) {
        let mut to_resume = Vec::new();
        let mut manual_overrides = Vec::new();

        for desc in &snapshot.displays {
            let did = &desc.id;

            if let Some(reasons) = self.active_pause_reasons.get_mut(did) {
                reasons.remove(&PauseReason::WorkstationLocked);

                // Resume ONLY if no other pause reasons remain (e.g. no UserManual, Fullscreen, PowerSaving, SystemSuspended!)
                if reasons.is_empty() {
                    to_resume.push(did.clone());
                } else {
                    manual_overrides.push((did.clone(), format!("{:?}", reasons)));
                }
            }
        }

        for (did, reasons_str) in manual_overrides {
            self.record_diagnostic(
                PolicyEventType::ManualOverrideRespected,
                Some(did.clone()),
                format!(
                    "Display `{}` retained in suspended state after workstation unlock due to surviving reasons: {}",
                    did, reasons_str
                ),
                None,
            );
        }

        let mut resumed = Vec::new();
        {
            let mut sc = self.session_coordinator.lock().unwrap();
            for did in to_resume {
                if let Some(session) = sc.get_session(&did) {
                    if matches!(
                        session.state,
                        DisplaySessionState::Suspended | DisplaySessionState::StaticFallback { .. }
                    ) {
                        let _ = sc.resume_session(&did);
                        resumed.push(did);
                    }
                }
            }
        }

        for did in resumed {
            self.record_diagnostic(
                PolicyEventType::SessionPolicyCleared,
                Some(did.clone()),
                format!("Resumed wallpaper session on display `{}` as workstation unlocked", did),
                None,
            );
        }
    }

    /// Applies Remote Desktop optimization pause across all displays.
    fn apply_remote_pause(&mut self, trigger: &str, snapshot: &DisplayTopologySnapshot) {
        let mut suspended = Vec::new();
        {
            let mut sc = self.session_coordinator.lock().unwrap();
            for desc in &snapshot.displays {
                let did = &desc.id;
                let reasons = self.active_pause_reasons.entry(did.clone()).or_default();
                let reason = PauseReason::RemoteSession;

                if !reasons.contains(&reason) {
                    reasons.insert(reason);

                    if let Some(session) = sc.get_session(did) {
                        if session.state.is_active() {
                            let _ = sc.suspend_session(did);
                            suspended.push(did.clone());
                        }
                    }
                }
            }
        }

        for did in suspended {
            self.record_diagnostic(
                PolicyEventType::SessionPolicyApplied,
                Some(did.clone()),
                format!("Paused display `{}` due to Remote Desktop policy: {}", did, trigger),
                None,
            );
        }
    }

    /// Reconciles displays when Remote Desktop session constraints clear.
    fn reconcile_remote_cleared(&mut self, snapshot: &DisplayTopologySnapshot) {
        let mut to_resume = Vec::new();
        let mut manual_overrides = Vec::new();

        for desc in &snapshot.displays {
            let did = &desc.id;

            if let Some(reasons) = self.active_pause_reasons.get_mut(did) {
                reasons.remove(&PauseReason::RemoteSession);

                if reasons.is_empty() {
                    to_resume.push(did.clone());
                } else {
                    manual_overrides.push((did.clone(), format!("{:?}", reasons)));
                }
            }
        }

        for (did, reasons_str) in manual_overrides {
            self.record_diagnostic(
                PolicyEventType::ManualOverrideRespected,
                Some(did.clone()),
                format!(
                    "Display `{}` retained in suspended state after remote session cleared due to surviving reasons: {}",
                    did, reasons_str
                ),
                None,
            );
        }

        let mut resumed = Vec::new();
        {
            let mut sc = self.session_coordinator.lock().unwrap();
            for did in to_resume {
                if let Some(session) = sc.get_session(&did) {
                    if matches!(
                        session.state,
                        DisplaySessionState::Suspended | DisplaySessionState::StaticFallback { .. }
                    ) {
                        let _ = sc.resume_session(&did);
                        resumed.push(did);
                    }
                }
            }
        }

        for did in resumed {
            self.record_diagnostic(
                PolicyEventType::SessionPolicyCleared,
                Some(did.clone()),
                format!("Resumed wallpaper session on display `{}` as remote session cleared", did),
                None,
            );
        }
    }

    /// Updates display power policy settings and reconciles active sessions if disabled.
    pub fn update_display_power_settings(
        &mut self,
        new_settings: DisplayPowerPlaybackSettings,
        snapshot: &DisplayTopologySnapshot,
    ) {
        let was_enabled = self.display_power_settings.enabled;
        self.display_power_settings = new_settings;

        self.record_diagnostic(
            PolicyEventType::SettingUpdated,
            None,
            format!(
                "Display power policy settings updated (enabled: {}, pause_on_off: {}, resume_on_on: {}, dimmed: {:?})",
                self.display_power_settings.enabled,
                self.display_power_settings.pause_on_display_off,
                self.display_power_settings.resume_on_display_on,
                self.display_power_settings.dimmed_action
            ),
            None,
        );

        let snap = self.current_display_power_snapshot.clone();
        if was_enabled && !self.display_power_settings.enabled {
            self.reconcile_display_power_cleared(snapshot);
        } else {
            self.evaluate_display_power_state(&snap, snapshot);
        }
    }

    /// Evaluates current display power state and reconciles wallpaper sessions across all displays.
    pub fn evaluate_display_power_state(
        &mut self,
        display_power_snapshot: &DisplayPowerSnapshot,
        snapshot: &DisplayTopologySnapshot,
    ) {
        self.current_display_power_snapshot = display_power_snapshot.clone();

        self.record_diagnostic(
            PolicyEventType::DisplayPowerStateChanged,
            None,
            format!(
                "Display power state observed: state={:?}, scope={:?}",
                display_power_snapshot.state, display_power_snapshot.scope
            ),
            None,
        );

        let eval = DisplayPowerPolicyEvaluation::evaluate(
            &self.display_power_settings,
            display_power_snapshot,
        );

        if eval.should_pause {
            let trigger = eval
                .trigger_reason
                .unwrap_or_else(|| "Display power policy active".into());
            self.apply_display_power_pause(&trigger, snapshot);
        } else if display_power_snapshot.state == DisplayPowerState::On {
            self.reconcile_display_power_cleared(snapshot);
        }
    }

    /// Handles explicit display power off notification.
    pub fn handle_display_off(&mut self, snapshot: &DisplayTopologySnapshot) {
        self.current_display_power_snapshot.state = DisplayPowerState::Off;
        self.record_diagnostic(
            PolicyEventType::DisplayPowerOffDetected,
            None,
            "Display power off detected via session notification".into(),
            None,
        );

        if self.display_power_settings.enabled && self.display_power_settings.pause_on_display_off {
            self.apply_display_power_pause("Session display is powered off", snapshot);
        }
    }

    /// Handles explicit display power on notification.
    pub fn handle_display_on(&mut self, snapshot: &DisplayTopologySnapshot) {
        self.current_display_power_snapshot.state = DisplayPowerState::On;
        self.record_diagnostic(
            PolicyEventType::DisplayPowerOnDetected,
            None,
            "Display power on detected via session notification".into(),
            None,
        );

        self.reconcile_display_power_cleared(snapshot);
    }

    /// Applies display-power pause across all displays participating in desktop presentation.
    fn apply_display_power_pause(&mut self, trigger: &str, snapshot: &DisplayTopologySnapshot) {
        let mut suspended = Vec::new();
        {
            let mut sc = self.session_coordinator.lock().unwrap();
            for desc in &snapshot.displays {
                let did = &desc.id;
                let reasons = self.active_pause_reasons.entry(did.clone()).or_default();
                let reason = PauseReason::DisplayPowerOff;

                if !reasons.contains(&reason) {
                    reasons.insert(reason);

                    if let Some(session) = sc.get_session(did) {
                        if session.state.is_active() {
                            let _ = sc.suspend_session(did);
                            suspended.push(did.clone());
                        }
                    }
                }
            }
        }

        for did in suspended {
            self.record_diagnostic(
                PolicyEventType::DisplayPowerPolicyApplied,
                Some(did.clone()),
                format!("Paused display `{}` due to display power policy: {}", did, trigger),
                None,
            );
        }
    }

    /// Reconciles displays when display-power constraints clear (display returns to On).
    fn reconcile_display_power_cleared(&mut self, snapshot: &DisplayTopologySnapshot) {
        let mut to_resume = Vec::new();
        let mut manual_overrides = Vec::new();

        for desc in &snapshot.displays {
            let did = &desc.id;

            if let Some(reasons) = self.active_pause_reasons.get_mut(did) {
                reasons.remove(&PauseReason::DisplayPowerOff);

                // Resume ONLY if no other pause reasons remain (e.g. no UserManual, Fullscreen, PowerSaving, SystemSuspended, WorkstationLocked, RemoteSession!)
                if reasons.is_empty() {
                    to_resume.push(did.clone());
                } else {
                    manual_overrides.push((did.clone(), format!("{:?}", reasons)));
                }
            }
        }

        for (did, reasons_str) in manual_overrides {
            self.record_diagnostic(
                PolicyEventType::ManualOverrideRespected,
                Some(did.clone()),
                format!(
                    "Display `{}` retained in suspended state after display power on due to surviving reasons: {}",
                    did, reasons_str
                ),
                None,
            );
        }

        let mut resumed = Vec::new();
        {
            let mut sc = self.session_coordinator.lock().unwrap();
            for did in to_resume {
                if let Some(session) = sc.get_session(&did) {
                    if matches!(
                        session.state,
                        DisplaySessionState::Suspended | DisplaySessionState::StaticFallback { .. }
                    ) {
                        let _ = sc.resume_session(&did);
                        resumed.push(did);
                    }
                }
            }
        }

        for did in resumed {
            self.record_diagnostic(
                PolicyEventType::DisplayPowerPolicyCleared,
                Some(did.clone()),
                format!("Resumed wallpaper session on display `{}` as display powered on", did),
                None,
            );
        }
    }

    /// Updates user inactivity policy settings and reconciles active sessions if disabled.
    pub fn update_inactivity_settings(
        &mut self,
        new_settings: InactivityPlaybackSettings,
        snapshot: &DisplayTopologySnapshot,
    ) {
        let was_enabled = self.inactivity_settings.enabled;
        self.inactivity_settings = new_settings;

        self.record_diagnostic(
            PolicyEventType::SettingUpdated,
            None,
            format!(
                "User inactivity policy settings updated (enabled: {}, threshold_ms: {}, resume_on_activity: {})",
                self.inactivity_settings.enabled,
                self.inactivity_settings.clamped_threshold(),
                self.inactivity_settings.resume_on_activity
            ),
            None,
        );

        let snap = self.current_inactivity_snapshot.clone();
        if was_enabled && !self.inactivity_settings.enabled {
            self.reconcile_inactivity_cleared(snapshot);
        } else {
            self.evaluate_inactivity_state(&snap, snapshot);
        }
    }

    /// Evaluates current user inactivity state and reconciles wallpaper sessions across all displays.
    pub fn evaluate_inactivity_state(
        &mut self,
        inactivity_snapshot: &InactivitySnapshot,
        snapshot: &DisplayTopologySnapshot,
    ) {
        self.current_inactivity_snapshot = inactivity_snapshot.clone();

        let eval = InactivityPolicyEvaluation::evaluate(
            &self.inactivity_settings,
            inactivity_snapshot,
        );

        if eval.should_pause {
            let trigger = eval
                .trigger_reason
                .unwrap_or_else(|| "User inactivity policy active".into());
            self.apply_inactivity_pause(&trigger, snapshot);
        } else if inactivity_snapshot.state == InactivityState::Active {
            self.reconcile_inactivity_cleared(snapshot);
        }
    }

    /// Handles explicit user inactivity detection.
    pub fn handle_user_inactivity(&mut self, elapsed_millis: u64, snapshot: &DisplayTopologySnapshot) {
        self.current_inactivity_snapshot.state = InactivityState::Idle;
        self.current_inactivity_snapshot.elapsed_inactivity_millis = elapsed_millis;
        self.record_diagnostic(
            PolicyEventType::UserInactivityDetected,
            None,
            format!("User inactivity detected (idle: {} ms)", elapsed_millis),
            None,
        );

        if self.inactivity_settings.enabled {
            self.apply_inactivity_pause("User input inactive", snapshot);
        }
    }

    /// Handles explicit user activity resumed in the session.
    pub fn handle_user_activity(&mut self, snapshot: &DisplayTopologySnapshot) {
        self.current_inactivity_snapshot.state = InactivityState::Active;
        self.current_inactivity_snapshot.elapsed_inactivity_millis = 0;
        self.record_diagnostic(
            PolicyEventType::UserActivityResumed,
            None,
            "User activity resumed in interactive session".into(),
            None,
        );

        self.reconcile_inactivity_cleared(snapshot);
    }

    /// Applies user inactivity pause across all displays participating in desktop presentation.
    fn apply_inactivity_pause(&mut self, trigger: &str, snapshot: &DisplayTopologySnapshot) {
        let mut suspended = Vec::new();
        {
            let mut sc = self.session_coordinator.lock().unwrap();
            for desc in &snapshot.displays {
                let did = &desc.id;
                let reasons = self.active_pause_reasons.entry(did.clone()).or_default();
                let reason = PauseReason::UserInactivity;

                if !reasons.contains(&reason) {
                    reasons.insert(reason);

                    if let Some(session) = sc.get_session(did) {
                        if session.state.is_active() {
                            let _ = sc.suspend_session(did);
                            suspended.push(did.clone());
                        }
                    }
                }
            }
        }

        for did in suspended {
            self.record_diagnostic(
                PolicyEventType::InactivityPolicyApplied,
                Some(did.clone()),
                format!("Paused display `{}` due to user inactivity: {}", did, trigger),
                None,
            );
        }
    }

    /// Reconciles displays when user inactivity constraints clear (user activity resumes).
    fn reconcile_inactivity_cleared(&mut self, snapshot: &DisplayTopologySnapshot) {
        let mut to_resume = Vec::new();
        let mut manual_overrides = Vec::new();

        for desc in &snapshot.displays {
            let did = &desc.id;

            if let Some(reasons) = self.active_pause_reasons.get_mut(did) {
                reasons.remove(&PauseReason::UserInactivity);

                // Resume ONLY if no other pause reasons remain (e.g. no UserManual, Fullscreen, PowerSaving, SystemSuspended, WorkstationLocked, RemoteSession, DisplayPowerOff!)
                if reasons.is_empty() {
                    to_resume.push(did.clone());
                } else {
                    manual_overrides.push((did.clone(), format!("{:?}", reasons)));
                }
            }
        }

        for (did, reasons_str) in manual_overrides {
            self.record_diagnostic(
                PolicyEventType::ManualOverrideRespected,
                Some(did.clone()),
                format!(
                    "Display `{}` retained in suspended state after user activity resumed due to surviving reasons: {}",
                    did, reasons_str
                ),
                None,
            );
        }

        let mut resumed = Vec::new();
        {
            let mut sc = self.session_coordinator.lock().unwrap();
            for did in to_resume {
                if let Some(session) = sc.get_session(&did) {
                    if matches!(
                        session.state,
                        DisplaySessionState::Suspended | DisplaySessionState::StaticFallback { .. }
                    ) {
                        let _ = sc.resume_session(&did);
                        resumed.push(did);
                    }
                }
            }
        }

        for did in resumed {
            self.record_diagnostic(
                PolicyEventType::InactivityPolicyCleared,
                Some(did.clone()),
                format!("Resumed wallpaper session on display `{}` as user activity resumed", did),
                None,
            );
        }
    }

    /// Records a bounded diagnostic event.
    fn record_diagnostic(
        &mut self,
        event_type: PolicyEventType,
        display_id: Option<DisplayId>,
        description: String,
        classification: Option<FullscreenClassification>,
    ) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        if self.diagnostic_history.len() >= MAX_POLICY_DIAGNOSTIC_EVENTS {
            self.diagnostic_history.remove(0);
        }

        self.diagnostic_history.push(PolicyDiagnosticEvent {
            timestamp_millis: now,
            event_type,
            display_id,
            description,
            classification,
        });
    }

    /// Retrieves diagnostic history.
    #[must_use]
    pub fn get_diagnostics(&self) -> &[PolicyDiagnosticEvent] {
        &self.diagnostic_history
    }

    /// Alias for `get_diagnostics`.
    #[must_use]
    pub fn diagnostics(&self) -> &[PolicyDiagnosticEvent] {
        &self.diagnostic_history
    }
}
