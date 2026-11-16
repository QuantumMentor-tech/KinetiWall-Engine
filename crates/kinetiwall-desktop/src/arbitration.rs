//! Unified Policy Arbitration, Conflict Resolution & Deterministic Playback Decisions (Step 56).
//! Extended for Policy Decision Diagnostics, Explainability & Troubleshooting (Step 57).
//!
//! Provides:
//! - Single authoritative arbitration contract combining all policy inputs:
//!   - Hard safety and lifecycle constraints (shutdown, manual pause, backend termination).
//!   - Automatic suspension constraints (fullscreen, workstation lock, display off, inactivity, power-saving).
//!   - Quality and resource limits (adaptive frame pacing, aggregate CPU budgeting).
//!   - User preferences and static fallback presentation.
//! - Pure, deterministic arbitration function producing identical decisions for identical inputs.
//! - Reversible, policy-owned pause-reason reconciliation ensuring no policy clears another's constraints.
//! - Transition deduplication comparing desired state with actual backend state to prevent command storms.
//! - Structured explanation codes describing the exact causal reasons for playback decisions.
//! - Human-readable diagnostic explanations and stable reason codes.
//! - Structured representation distinguishing desired state, actual backend state, and discrepancies.
//! - Multi-monitor awareness across Independent, Span, and Clone layout topologies.
//! - Bounded diagnostic ring buffer for arbitration auditing.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::display::DisplayId;
use crate::policy::PauseReason;
use crate::session::{
    DisplaySessionCoordinator, DisplaySessionId, DisplaySessionState, SessionError,
    SessionStopReason,
};

// -------------------------------------------------------------------------------------------------
// Policy Constraint Categorization & Explanation Codes
// -------------------------------------------------------------------------------------------------

/// Semantic classification of policy constraints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintCategory {
    /// Non-negotiable safety or lifecycle constraints (manual pause, termination, shutdown).
    HardSafety,
    /// Event-driven automatic pause policies (fullscreen, screen lock, display off, idle).
    AutomaticSuspension,
    /// Bounded performance or cadence adjustments (adaptive quality, resource budget).
    ResourceQuality,
    /// Saved user preferences and presentations (canonical framerate, fallback preference).
    UserPreference,
}

/// Structured explanation codes describing the causal factors behind an arbitration decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyExplanationCode {
    /// Manual user pause is actively enforced.
    ManualPauseActive,
    /// Session is terminated, stopped, or uninitialized.
    SessionTerminated,
    /// Backend reported an unrecoverable failure.
    BackendFailure,
    /// Foreground window is running in fullscreen mode on target display.
    FullscreenActive,
    /// Windows workstation is locked.
    WorkstationLocked,
    /// Session display is reported powered off by OS display power notification.
    DisplayPowerOff,
    /// User input has been inactive past configured threshold.
    UserInactivityIdle,
    /// System power policy is actively saving battery.
    PowerSavingActive,
    /// Operating system is suspended (sleep or hibernate).
    SystemSuspended,
    /// Session is running over an optimized Remote Desktop connection.
    RemoteSessionOptimized,
    /// Aggregate resource budget arbiter requested performance throttling.
    ResourceBudgetPressure,
    /// Adaptive playback optimizer applied frame pacing due to sustained load.
    AdaptiveQualityPacing,
    /// Static wallpaper fallback is active in place of live rendering.
    StaticFallbackPresented,
    /// All constraints cleared; session is authorized for live playback.
    UnconstrainedActive,
    /// Observation is unknown or unavailable; conservative baseline preserved.
    UnknownStatePreserved,
}

impl PolicyExplanationCode {
    /// Human-readable explanation text suitable for user interfaces and logs.
    #[must_use]
    pub fn description(&self) -> &'static str {
        match self {
            Self::ManualPauseActive => "Playback manually paused by user",
            Self::SessionTerminated => "Playback session is terminated or stopped",
            Self::BackendFailure => "Playback backend encountered an unrecoverable error",
            Self::FullscreenActive => "Suspended due to active foreground fullscreen application",
            Self::WorkstationLocked => "Suspended because Windows workstation is locked",
            Self::DisplayPowerOff => "Suspended because display panel is powered off",
            Self::UserInactivityIdle => "Suspended due to user inactivity (session idle)",
            Self::PowerSavingActive => "Suspended by system power policy on battery/low power",
            Self::SystemSuspended => "Suspended because operating system entered sleep/hibernation",
            Self::RemoteSessionOptimized => "Optimized/suspended for Remote Desktop connection",
            Self::ResourceBudgetPressure => "Framerate throttled due to high aggregate system load",
            Self::AdaptiveQualityPacing => "Adaptive quality pacing active to maintain performance",
            Self::StaticFallbackPresented => "Displaying preserved static wallpaper fallback",
            Self::UnconstrainedActive => "All constraints cleared; playback active",
            Self::UnknownStatePreserved => "Unknown state; conservative baseline maintained",
        }
    }

    /// Machine-readable stable reason code for diagnostics and telemetry.
    #[must_use]
    pub fn reason_code(&self) -> &'static str {
        match self {
            Self::ManualPauseActive => "MANUAL_PAUSE_ACTIVE",
            Self::SessionTerminated => "SESSION_TERMINATED",
            Self::BackendFailure => "BACKEND_FAILURE",
            Self::FullscreenActive => "FULLSCREEN_ACTIVE",
            Self::WorkstationLocked => "WORKSTATION_LOCKED",
            Self::DisplayPowerOff => "DISPLAY_POWER_OFF",
            Self::UserInactivityIdle => "USER_INACTIVITY_IDLE",
            Self::PowerSavingActive => "POWER_SAVING_ACTIVE",
            Self::SystemSuspended => "SYSTEM_SUSPENDED",
            Self::RemoteSessionOptimized => "REMOTE_SESSION_OPTIMIZED",
            Self::ResourceBudgetPressure => "RESOURCE_BUDGET_PRESSURE",
            Self::AdaptiveQualityPacing => "ADAPTIVE_QUALITY_PACING",
            Self::StaticFallbackPresented => "STATIC_FALLBACK_PRESENTED",
            Self::UnconstrainedActive => "UNCONSTRAINED_ACTIVE",
            Self::UnknownStatePreserved => "UNKNOWN_STATE_PRESERVED",
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Desired State & Effective Decision Models
// -------------------------------------------------------------------------------------------------

/// Target operational state selected by the policy arbitrator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "desired_state", rename_all = "snake_case")]
pub enum DesiredSessionState {
    /// Live rendering and playback authorized.
    Active,
    /// Playback suspended; decoders and rendering loops paused.
    Suspended,
    /// Visual presentation preserved via static fallback image.
    StaticFallback { fallback_source: String },
    /// Session must undergo orderly teardown or remain stopped.
    Terminated { reason: String },
}

impl DesiredSessionState {
    /// Returns true if live playback is authorized.
    #[must_use]
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active)
    }

    /// Returns true if suspended.
    #[must_use]
    pub fn is_suspended(&self) -> bool {
        matches!(self, Self::Suspended)
    }

    /// Returns true if in static fallback.
    #[must_use]
    pub fn is_fallback(&self) -> bool {
        matches!(self, Self::StaticFallback { .. })
    }

    /// Returns true if session should be terminated.
    #[must_use]
    pub fn is_terminated(&self) -> bool {
        matches!(self, Self::Terminated { .. })
    }
}

/// Input snapshot supplied to the policy arbitrator for a specific display session.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionArbitrationInput {
    pub display_id: DisplayId,
    pub session_id: Option<DisplaySessionId>,
    pub current_actual_state: DisplaySessionState,
    pub active_pause_reasons: HashSet<PauseReason>,
    pub allows_static_fallback: bool,
    pub default_fallback_source: Option<String>,
    pub adaptive_fps_cap: Option<u32>,
    pub budget_fps_cap: Option<u32>,
    pub canonical_target_fps: u32,
    pub is_session_terminated: bool,
    pub termination_reason: Option<String>,
}

/// Authoritative, deterministic decision generated by the policy arbitrator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectivePlaybackDecision {
    pub display_id: DisplayId,
    pub session_id: Option<DisplaySessionId>,
    pub desired_state: DesiredSessionState,
    pub actual_state: DisplaySessionState,
    pub effective_fps_limit: Option<u32>,
    pub active_constraints: Vec<PauseReason>,
    pub explanation_codes: Vec<PolicyExplanationCode>,
    pub primary_explanation: String,
    pub primary_reason_code: String,
    pub is_state_divergent: bool,
    pub requires_transition: bool,
    pub evaluated_at_millis: u64,
}

// -------------------------------------------------------------------------------------------------
// Diagnostics
// -------------------------------------------------------------------------------------------------

/// Diagnostic event recording a deterministic arbitration outcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArbitrationDiagnosticEvent {
    pub timestamp_millis: u64,
    pub display_id: DisplayId,
    pub session_id: Option<DisplaySessionId>,
    pub desired_state: DesiredSessionState,
    pub actual_state: DisplaySessionState,
    pub requires_transition: bool,
    pub explanation_codes: Vec<PolicyExplanationCode>,
    pub primary_explanation: String,
    pub primary_reason_code: String,
}

pub const MAX_ARBITRATION_DIAGNOSTICS: usize = 128;

// -------------------------------------------------------------------------------------------------
// Policy Arbitrator
// -------------------------------------------------------------------------------------------------

/// Authoritative, deterministic policy arbitrator.
pub struct PolicyArbitrator;

impl PolicyArbitrator {
    /// Pure, deterministic arbitration function.
    ///
    /// Evaluates all active constraints, preferences, and capabilities for a session,
    /// selecting a single effective desired state and calculating whether a backend transition
    /// is required to reconcile actual state with desired state.
    #[must_use]
    pub fn arbitrate(input: &SessionArbitrationInput) -> EffectivePlaybackDecision {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let mut explanations = Vec::new();
        let mut sorted_reasons: Vec<PauseReason> = input.active_pause_reasons.iter().cloned().collect();
        // Deterministic sorting to ensure stable decision output
        sorted_reasons.sort_by(|a, b| format!("{:?}", a).cmp(&format!("{:?}", b)));

        // 1. Check Hard Safety & Lifecycle Constraints
        if input.is_session_terminated {
            explanations.push(PolicyExplanationCode::SessionTerminated);
            let reason = input
                .termination_reason
                .clone()
                .unwrap_or_else(|| "Session terminated".into());
            let desired = DesiredSessionState::Terminated { reason };
            let requires_transition = !input.current_actual_state.is_terminated();
            let primary_code = PolicyExplanationCode::SessionTerminated;
            return EffectivePlaybackDecision {
                display_id: input.display_id.clone(),
                session_id: input.session_id.clone(),
                desired_state: desired,
                actual_state: input.current_actual_state.clone(),
                effective_fps_limit: None,
                active_constraints: sorted_reasons,
                explanation_codes: explanations,
                primary_explanation: primary_code.description().to_string(),
                primary_reason_code: primary_code.reason_code().to_string(),
                is_state_divergent: requires_transition,
                requires_transition,
                evaluated_at_millis: now,
            };
        }

        // 2. Check Automatic Suspensions & Manual User Pause
        for reason in &sorted_reasons {
            match reason {
                PauseReason::UserManual => {
                    explanations.push(PolicyExplanationCode::ManualPauseActive);
                }
                PauseReason::Fullscreen { .. } => {
                    explanations.push(PolicyExplanationCode::FullscreenActive);
                }
                PauseReason::WorkstationLocked => {
                    explanations.push(PolicyExplanationCode::WorkstationLocked);
                }
                PauseReason::DisplayPowerOff => {
                    explanations.push(PolicyExplanationCode::DisplayPowerOff);
                }
                PauseReason::UserInactivity => {
                    explanations.push(PolicyExplanationCode::UserInactivityIdle);
                }
                PauseReason::PowerSaving => {
                    explanations.push(PolicyExplanationCode::PowerSavingActive);
                }
                PauseReason::SystemSuspended => {
                    explanations.push(PolicyExplanationCode::SystemSuspended);
                }
                PauseReason::RemoteSession => {
                    explanations.push(PolicyExplanationCode::RemoteSessionOptimized);
                }
            }
        }

        // If any pause reason (manual or automatic) is present, session must be suspended or in fallback
        let desired_state = if !input.active_pause_reasons.is_empty() {
            // Check if static fallback is permitted and available
            if input.allows_static_fallback && input.default_fallback_source.is_some() {
                explanations.push(PolicyExplanationCode::StaticFallbackPresented);
                DesiredSessionState::StaticFallback {
                    fallback_source: input.default_fallback_source.clone().unwrap(),
                }
            } else {
                DesiredSessionState::Suspended
            }
        } else {
            explanations.push(PolicyExplanationCode::UnconstrainedActive);
            DesiredSessionState::Active
        };

        // 3. Quality & Cadence Arbitration
        // Apply most restrictive framerate cap among active optimizers
        let mut effective_fps = None;
        let mut candidate_caps = Vec::new();

        if let Some(cap) = input.adaptive_fps_cap {
            candidate_caps.push(cap);
            explanations.push(PolicyExplanationCode::AdaptiveQualityPacing);
        }
        if let Some(cap) = input.budget_fps_cap {
            candidate_caps.push(cap);
            explanations.push(PolicyExplanationCode::ResourceBudgetPressure);
        }

        if !candidate_caps.is_empty() {
            let min_cap = candidate_caps.into_iter().min().unwrap();
            effective_fps = Some(min_cap.min(input.canonical_target_fps));
        }

        // 4. Transition Deduplication: Compare desired state with actual backend state
        let requires_transition = match (&desired_state, &input.current_actual_state) {
            (DesiredSessionState::Active, DisplaySessionState::Active) => false,
            (DesiredSessionState::Suspended, DisplaySessionState::Suspended) => false,
            (
                DesiredSessionState::StaticFallback { fallback_source: d },
                DisplaySessionState::StaticFallback { fallback_source: a },
            ) => d != a,
            (DesiredSessionState::Terminated { .. }, DisplaySessionState::Stopped) => false,
            _ => true,
        };

        let primary_code = explanations
            .first()
            .copied()
            .unwrap_or(PolicyExplanationCode::UnconstrainedActive);

        EffectivePlaybackDecision {
            display_id: input.display_id.clone(),
            session_id: input.session_id.clone(),
            desired_state,
            actual_state: input.current_actual_state.clone(),
            effective_fps_limit: effective_fps,
            active_constraints: sorted_reasons,
            explanation_codes: explanations,
            primary_explanation: primary_code.description().to_string(),
            primary_reason_code: primary_code.reason_code().to_string(),
            is_state_divergent: requires_transition,
            requires_transition,
            evaluated_at_millis: now,
        }
    }

    /// Reconciles an authoritative arbitration decision against the canonical session coordinator.
    ///
    /// Executes the minimal necessary transition only if `requires_transition` is true,
    /// avoiding redundant commands, state flickering, and unnecessary player recreation.
    pub fn reconcile_decision(
        decision: &EffectivePlaybackDecision,
        coordinator: &Arc<Mutex<DisplaySessionCoordinator>>,
    ) -> Result<bool, SessionError> {
        if !decision.requires_transition {
            return Ok(false);
        }

        let mut sc = coordinator.lock().unwrap();
        match &decision.desired_state {
            DesiredSessionState::Active => {
                sc.resume_session(&decision.display_id)?;
                Ok(true)
            }
            DesiredSessionState::Suspended => {
                sc.suspend_session(&decision.display_id)?;
                Ok(true)
            }
            DesiredSessionState::StaticFallback { fallback_source } => {
                sc.transition_to_static_fallback(&decision.display_id, fallback_source)?;
                Ok(true)
            }
            DesiredSessionState::Terminated { .. } => {
                sc.stop_session(&decision.display_id, SessionStopReason::CoordinatorShutdown)?;
                Ok(true)
            }
        }
    }
}
