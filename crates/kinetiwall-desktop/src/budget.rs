//! Dynamic Resource Budgeting, Multi-Session Load Balancing & Optimization Arbitration (Step 52).
//!
//! Provides:
//! - Canonical resource inventory distinguishing dedicated vs shared playback resources.
//! - Non-overlapping aggregate workload calculation from verified measurements with coverage tracking.
//! - Explicit handling of unknown, stale, and partial measurement coverage (never treated as zero).
//! - Bounded soft application resource budget model with hysteresis and dwell timing.
//! - Centralized optimization arbitration allocating supported adjustments across eligible sessions.
//! - Heavy-session prioritization preventing one high-demand session from unnecessarily degrading all wallpapers.
//! - Shared-resource coordination across Span and Clone presentation layouts.
//! - Strict interlocks preserving manual pause, fullscreen pause, battery power policies, and static fallback.
//! - Lossless restoration of canonical user settings when pressure clears or the feature is disabled.
//! - Bounded diagnostic event log (`MAX_BUDGET_DIAGNOSTICS = 128`).

use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::adaptive::{
    AdaptiveBackendCapability, AdaptiveQualityLevel, EffectiveQualityOverride,
    PlaybackResourceObservation, ResourceMetricKind, ResourceObservationScope,
    ResourceSignalQuality,
};
use crate::display::DisplayId;
use crate::layout::MultiMonitorLayoutMode;
use crate::policy::PauseReason;
use crate::power::PowerPolicyEvaluation;
use crate::session::{DisplaySessionCoordinator, DisplaySessionId, DisplaySessionState};

// -------------------------------------------------------------------------------------------------
// Timing, Threshold & Budget Constants
// -------------------------------------------------------------------------------------------------

/// Default aggregate application CPU pressure threshold triggering optimization (percentage, 0.0 - 100.0).
pub const DEFAULT_BUDGET_PRESSURE_CPU_PERCENT: f32 = 80.0;

/// Default aggregate application CPU recovery threshold (hysteresis exit threshold).
pub const DEFAULT_BUDGET_RECOVERY_CPU_PERCENT: f32 = 45.0;

/// Default duration (in milliseconds) of sustained aggregate pressure required before downgrading.
pub const DEFAULT_BUDGET_SUSTAINED_PRESSURE_MS: u64 = 3000;

/// Default dwell duration (in milliseconds) below recovery threshold required before restoring settings.
pub const DEFAULT_BUDGET_RECOVERY_DWELL_MS: u64 = 5000;

/// Default minimum cooldown duration between successive arbitration adjustments (in milliseconds).
pub const DEFAULT_BUDGET_ADJUSTMENT_COOLDOWN_MS: u64 = 4000;

/// Default maximum number of simultaneous session adjustments per arbitration cycle.
pub const DEFAULT_MAX_SIMULTANEOUS_ADJUSTMENTS: usize = 1;

/// Maximum number of diagnostic events retained in the budget arbiter ring buffer.
pub const MAX_BUDGET_DIAGNOSTICS: usize = 128;

/// Maximum number of measurement samples retained per tracked resource.
pub const MAX_RESOURCE_SAMPLES: usize = 30;

// -------------------------------------------------------------------------------------------------
// Resource Ownership & Tracked Resource Models
// -------------------------------------------------------------------------------------------------

/// Unique identifier for an accounted playback resource.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TrackedResourceId(pub String);

impl TrackedResourceId {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    #[must_use]
    pub fn for_session(session_id: &DisplaySessionId) -> Self {
        Self(format!("res_session_{}", session_id))
    }

    #[must_use]
    pub fn for_shared_group(group_id: &str) -> Self {
        Self(format!("res_shared_{}", group_id))
    }

    #[must_use]
    pub fn for_process(pid: u32) -> Self {
        Self(format!("res_proc_{}", pid))
    }
}

impl std::fmt::Display for TrackedResourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Ownership classification of a playback resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "ownership_type", rename_all = "snake_case")]
pub enum ResourceOwnership {
    /// Dedicated to a single display session.
    Dedicated {
        display_id: DisplayId,
        session_id: DisplaySessionId,
        process_id: Option<u32>,
    },
    /// Shared among multiple participating displays (e.g. Span or shared-renderer Clone).
    SharedGroup {
        layout_mode: MultiMonitorLayoutMode,
        primary_display_id: DisplayId,
        participating_displays: Vec<DisplayId>,
        process_id: Option<u32>,
    },
    /// Application-wide or unassigned host process.
    ApplicationProcess {
        process_id: u32,
    },
}

impl ResourceOwnership {
    /// Returns true if this ownership involves the specified display ID.
    #[must_use]
    pub fn involves_display(&self, target_display: &DisplayId) -> bool {
        match self {
            Self::Dedicated { display_id, .. } => display_id == target_display,
            Self::SharedGroup {
                primary_display_id,
                participating_displays,
                ..
            } => primary_display_id == target_display || participating_displays.contains(target_display),
            Self::ApplicationProcess { .. } => false,
        }
    }

    /// Returns associated child process ID, if known.
    #[must_use]
    pub fn process_id(&self) -> Option<u32> {
        match self {
            Self::Dedicated { process_id, .. } => *process_id,
            Self::SharedGroup { process_id, .. } => *process_id,
            Self::ApplicationProcess { process_id } => Some(*process_id),
        }
    }
}

/// Status of measurement coverage for a tracked resource.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum MeasurementCoverageStatus {
    /// Verified, fresh measurement available.
    Verified {
        cpu_percent: f32,
        working_set_bytes: Option<u64>,
        sampled_at_millis: u64,
    },
    /// Measurement is stale or expired.
    Stale {
        last_observed_millis: u64,
    },
    /// Measurement is unknown or not supported by the backend/platform.
    Unknown {
        reason: String,
    },
}

/// Detailed descriptor of a live tracked playback resource.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackedPlaybackResource {
    pub resource_id: TrackedResourceId,
    pub ownership: ResourceOwnership,
    pub backend_capability: AdaptiveBackendCapability,
    pub coverage_status: MeasurementCoverageStatus,
    pub session_generation: u64,
    pub effective_override: EffectiveQualityOverride,
    pub last_adjustment_millis: Option<u64>,
}

// -------------------------------------------------------------------------------------------------
// Aggregate Workload Model
// -------------------------------------------------------------------------------------------------

/// Point-in-time calculation of aggregate application workload across active sessions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AggregateWorkloadSnapshot {
    /// Sum of verified CPU percentages across unique player processes (0.0 to N*100.0%).
    pub aggregate_cpu_percent: f32,
    /// Total working set memory across unique player processes in bytes.
    pub total_working_set_bytes: u64,
    /// Total number of active playback sessions currently tracked.
    pub total_sessions_count: usize,
    /// Number of active sessions with verified, fresh measurements.
    pub measured_sessions_count: usize,
    /// Number of active sessions with unknown or stale measurements.
    pub unknown_sessions_count: usize,
    /// True if all active sessions have valid measurements (100% coverage).
    pub is_coverage_complete: bool,
    /// Timestamp when this aggregate workload snapshot was computed (Unix epoch millis).
    pub timestamp_millis: u64,
}

impl AggregateWorkloadSnapshot {
    /// Calculates aggregate workload from a set of tracked resources, strictly preventing
    /// double-counting by deduplicating unique child process IDs.
    #[must_use]
    pub fn compute(
        resources: &[&TrackedPlaybackResource],
        now_millis: u64,
    ) -> Self {
        let mut total_cpu = 0.0f32;
        let mut total_mem = 0u64;
        let mut counted_pids = HashSet::new();
        let mut measured_count = 0usize;
        let mut unknown_count = 0usize;
        let total_count = resources.len();

        for res in resources {
            match &res.coverage_status {
                MeasurementCoverageStatus::Verified {
                    cpu_percent,
                    working_set_bytes,
                    sampled_at_millis,
                } => {
                    // Check freshness (validity window e.g. 3000ms)
                    if now_millis.saturating_sub(*sampled_at_millis) <= 3000 {
                        measured_count += 1;
                        if let Some(pid) = res.ownership.process_id() {
                            if counted_pids.insert(pid) {
                                total_cpu += *cpu_percent;
                                if let Some(mem) = working_set_bytes {
                                    total_mem = total_mem.saturating_add(*mem);
                                }
                            }
                        } else {
                            // Dedicated session without explicit PID: count its verified CPU once
                            total_cpu += *cpu_percent;
                            if let Some(mem) = working_set_bytes {
                                total_mem = total_mem.saturating_add(*mem);
                            }
                        }
                    } else {
                        // Stale measurement treated as unknown coverage
                        unknown_count += 1;
                    }
                }
                MeasurementCoverageStatus::Stale { .. } | MeasurementCoverageStatus::Unknown { .. } => {
                    unknown_count += 1;
                }
            }
        }

        let is_coverage_complete = total_count > 0 && unknown_count == 0;

        Self {
            aggregate_cpu_percent: total_cpu,
            total_working_set_bytes: total_mem,
            total_sessions_count: total_count,
            measured_sessions_count: measured_count,
            unknown_sessions_count: unknown_count,
            is_coverage_complete,
            timestamp_millis: now_millis,
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Resource Budget Configuration
// -------------------------------------------------------------------------------------------------

/// Configurable soft application resource budget model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApplicationResourceBudget {
    /// Master toggle for dynamic resource budgeting and arbitration.
    /// STRICT REQUIREMENT: Opt-in by default (`false` for existing users).
    pub enabled: bool,

    /// Soft target for aggregate application CPU usage (percentage, e.g. 75.0%).
    pub target_aggregate_cpu_percent: f32,

    /// Pressure threshold for aggregate application CPU usage triggering arbitration (e.g. 85.0%).
    pub pressure_threshold_cpu_percent: f32,

    /// Recovery threshold for aggregate application CPU usage (hysteresis exit, e.g. 45.0%).
    pub recovery_threshold_cpu_percent: f32,

    /// Duration of sustained pressure required before applying arbitration adjustments (millis).
    pub sustained_pressure_threshold_ms: u64,

    /// Dwell duration below recovery threshold required before restoring settings (millis).
    pub recovery_dwell_threshold_ms: u64,

    /// Minimum cooldown duration between successive arbitration adjustments (millis).
    pub adjustment_cooldown_ms: u64,

    /// Maximum number of simultaneous session adjustments per arbitration cycle (e.g. 1).
    pub max_simultaneous_adjustments: usize,

    /// True if power policy (battery/saver) can coordinate with resource budgeting.
    pub allow_power_coordination: bool,
}

impl Default for ApplicationResourceBudget {
    fn default() -> Self {
        Self {
            enabled: false, // Opt-in by default
            target_aggregate_cpu_percent: 75.0,
            pressure_threshold_cpu_percent: DEFAULT_BUDGET_PRESSURE_CPU_PERCENT,
            recovery_threshold_cpu_percent: DEFAULT_BUDGET_RECOVERY_CPU_PERCENT,
            sustained_pressure_threshold_ms: DEFAULT_BUDGET_SUSTAINED_PRESSURE_MS,
            recovery_dwell_threshold_ms: DEFAULT_BUDGET_RECOVERY_DWELL_MS,
            adjustment_cooldown_ms: DEFAULT_BUDGET_ADJUSTMENT_COOLDOWN_MS,
            max_simultaneous_adjustments: DEFAULT_MAX_SIMULTANEOUS_ADJUSTMENTS,
            allow_power_coordination: true,
        }
    }
}

impl ApplicationResourceBudget {
    /// Validates and sanitizes settings values within bounded constraints.
    #[must_use]
    pub fn sanitized(mut self) -> Self {
        if self.target_aggregate_cpu_percent < 20.0 {
            self.target_aggregate_cpu_percent = 20.0;
        } else if self.target_aggregate_cpu_percent > 100.0 {
            self.target_aggregate_cpu_percent = 100.0;
        }

        if self.pressure_threshold_cpu_percent <= self.target_aggregate_cpu_percent {
            self.pressure_threshold_cpu_percent = (self.target_aggregate_cpu_percent + 10.0).min(100.0);
        }

        if self.recovery_threshold_cpu_percent >= self.target_aggregate_cpu_percent {
            self.recovery_threshold_cpu_percent = (self.target_aggregate_cpu_percent - 15.0).max(10.0);
        }

        if self.sustained_pressure_threshold_ms < 1000 {
            self.sustained_pressure_threshold_ms = 1000;
        }
        if self.recovery_dwell_threshold_ms < 2000 {
            self.recovery_dwell_threshold_ms = 2000;
        }
        if self.adjustment_cooldown_ms < 1000 {
            self.adjustment_cooldown_ms = 1000;
        }
        if self.max_simultaneous_adjustments == 0 {
            self.max_simultaneous_adjustments = 1;
        } else if self.max_simultaneous_adjustments > 4 {
            self.max_simultaneous_adjustments = 4;
        }

        self
    }
}

// -------------------------------------------------------------------------------------------------
// Aggregate Budget State & Arbitration Action Models
// -------------------------------------------------------------------------------------------------

/// Operational state of the aggregate application resource budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AggregateBudgetState {
    /// Budgeting is disabled; canonical user settings active.
    #[default]
    Disabled,
    /// Aggregate workload is within soft budget target.
    WithinBudget,
    /// Workload is approaching soft budget target.
    ApproachingBudget,
    /// Sustained pressure exceeding threshold confirmed.
    SustainedPressure,
    /// Measurement coverage incomplete; conservative policy active.
    CoverageIncomplete,
    /// Adjustment currently applied and active.
    OptimizationActive,
    /// Target budget cannot be satisfied by available backend controls.
    TargetInfeasible,
    /// Workload is low; dwell period active prior to restoring canonical settings.
    Recovering,
}

impl AggregateBudgetState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::WithinBudget => "within_budget",
            Self::ApproachingBudget => "approaching_budget",
            Self::SustainedPressure => "sustained_pressure",
            Self::CoverageIncomplete => "coverage_incomplete",
            Self::OptimizationActive => "optimization_active",
            Self::TargetInfeasible => "target_infeasible",
            Self::Recovering => "recovering",
        }
    }
}

/// Operational arbitration action commanded by the resource budget manager.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ArbitrationAction {
    /// No change needed; operational parameters are within budget.
    NoChange {
        reason: String,
    },
    /// Apply a bounded quality reduction to an individual session.
    ApplySessionReduction {
        display_id: DisplayId,
        session_id: DisplaySessionId,
        target_level: AdaptiveQualityLevel,
        target_fps: Option<u32>,
        reason: String,
    },
    /// Apply a unified quality reduction to a shared layout group (Span or Clone).
    ApplySharedReduction {
        primary_display_id: DisplayId,
        affected_displays: Vec<DisplayId>,
        target_level: AdaptiveQualityLevel,
        target_fps: Option<u32>,
        reason: String,
    },
    /// Restore canonical user settings for an individual session.
    RestoreSession {
        display_id: DisplayId,
        session_id: DisplaySessionId,
        canonical_fps: Option<u32>,
        reason: String,
    },
    /// Restore canonical user settings for a shared layout group.
    RestoreShared {
        primary_display_id: DisplayId,
        affected_displays: Vec<DisplayId>,
        canonical_fps: Option<u32>,
        reason: String,
    },
    /// Action suppressed due to active cooldown timer.
    HoldCooldown {
        remaining_ms: u64,
    },
    /// Defer arbitration because measurement coverage is incomplete.
    CoverageIncompleteDefer {
        unmeasured_displays: Vec<DisplayId>,
        reason: String,
    },
    /// Requested budget cannot be met through verified backend controls.
    TargetInfeasible {
        reason: String,
    },
}

// -------------------------------------------------------------------------------------------------
// Diagnostic Events & Audit Trail
// -------------------------------------------------------------------------------------------------

/// Classification of resource budget diagnostic events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetDiagnosticEventType {
    BudgetEnabled,
    BudgetDisabled,
    InventoryReconciled,
    SustainedPressureDetected,
    PressureCleared,
    OptimizationArbitrated,
    RestorationArbitrated,
    CooldownActive,
    CoverageIncomplete,
    TargetInfeasible,
    StaleActionRejected,
}

/// Bounded diagnostic event log entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetDiagnosticEvent {
    pub timestamp_millis: u64,
    pub event_type: BudgetDiagnosticEventType,
    pub message: String,
    pub aggregate_cpu_percent: Option<f32>,
    pub coverage_ratio: Option<f32>,
}

// -------------------------------------------------------------------------------------------------
// Central Resource Budget Arbiter
// -------------------------------------------------------------------------------------------------

/// Central manager and arbitrator coordinating aggregate application resource budgets,
/// session prioritization, and multi-monitor presentation load balancing.
pub struct ResourceBudgetArbiter {
    /// Active budget configuration.
    budget: ApplicationResourceBudget,
    /// Canonical inventory of tracked playback resources.
    resources: HashMap<TrackedResourceId, TrackedPlaybackResource>,
    /// Display-to-Resource index for quick resolution.
    display_to_resource: HashMap<DisplayId, TrackedResourceId>,
    /// Timestamp when sustained pressure was first continuously observed (millis).
    high_pressure_since: Option<u64>,
    /// Timestamp when low pressure (recovery) was first continuously observed (millis).
    low_pressure_since: Option<u64>,
    /// Timestamp of last applied arbitration action (for cooldown enforcement).
    last_action_timestamp_millis: Option<u64>,
    /// Current confirmed budget state.
    current_state: AggregateBudgetState,
    /// Bounded diagnostic event history.
    diagnostics: Vec<BudgetDiagnosticEvent>,
}

impl ResourceBudgetArbiter {
    /// Constructs a new arbiter with given budget settings.
    pub fn new(budget: ApplicationResourceBudget) -> Self {
        let is_enabled = budget.enabled;
        Self {
            budget: budget.sanitized(),
            resources: HashMap::new(),
            display_to_resource: HashMap::new(),
            high_pressure_since: None,
            low_pressure_since: None,
            last_action_timestamp_millis: None,
            current_state: if is_enabled {
                AggregateBudgetState::WithinBudget
            } else {
                AggregateBudgetState::Disabled
            },
            diagnostics: Vec::with_capacity(MAX_BUDGET_DIAGNOSTICS),
        }
    }

    /// Returns current timestamp in milliseconds.
    #[must_use]
    pub fn current_timestamp_millis() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    /// Updates budget configuration, recording diagnostic events.
    pub fn update_budget(&mut self, new_budget: ApplicationResourceBudget) {
        let prev_enabled = self.budget.enabled;
        self.budget = new_budget.sanitized();

        if !prev_enabled && self.budget.enabled {
            self.current_state = AggregateBudgetState::WithinBudget;
            self.record_diagnostic(
                BudgetDiagnosticEventType::BudgetEnabled,
                "Dynamic resource budgeting and optimization arbitration enabled".into(),
                None,
                None,
            );
        } else if prev_enabled && !self.budget.enabled {
            self.current_state = AggregateBudgetState::Disabled;
            self.high_pressure_since = None;
            self.low_pressure_since = None;
            self.record_diagnostic(
                BudgetDiagnosticEventType::BudgetDisabled,
                "Dynamic resource budgeting disabled; canonical settings will be restored".into(),
                None,
                None,
            );
        }
    }

    /// Returns reference to active budget configuration.
    #[must_use]
    pub fn budget(&self) -> &ApplicationResourceBudget {
        &self.budget
    }

    /// Returns current confirmed aggregate budget state.
    #[must_use]
    pub fn current_state(&self) -> AggregateBudgetState {
        self.current_state
    }

    /// Registers or reconciles a dedicated session resource in the inventory.
    pub fn register_dedicated_session(
        &mut self,
        display_id: DisplayId,
        session_id: DisplaySessionId,
        generation: u64,
        capability: AdaptiveBackendCapability,
        canonical_fps: Option<u32>,
        process_id: Option<u32>,
        now_millis: u64,
    ) {
        let res_id = TrackedResourceId::for_session(&session_id);
        let ownership = ResourceOwnership::Dedicated {
            display_id: display_id.clone(),
            session_id,
            process_id,
        };

        let entry = self.resources.entry(res_id.clone()).or_insert_with(|| {
            TrackedPlaybackResource {
                resource_id: res_id.clone(),
                ownership: ownership.clone(),
                backend_capability: capability.clone(),
                coverage_status: MeasurementCoverageStatus::Unknown {
                    reason: "Awaiting initial observation".into(),
                },
                session_generation: generation,
                effective_override: EffectiveQualityOverride::new_normal(canonical_fps, now_millis),
                last_adjustment_millis: None,
            }
        });

        // Update mutable fields in place
        entry.session_generation = generation;
        entry.backend_capability = capability;
        entry.ownership = ownership;

        self.display_to_resource.insert(display_id, res_id);
    }

    /// Registers or reconciles a shared layout group resource (Span or shared-renderer Clone).
    pub fn register_shared_group(
        &mut self,
        group_id: &str,
        layout_mode: MultiMonitorLayoutMode,
        primary_display_id: DisplayId,
        participating_displays: Vec<DisplayId>,
        generation: u64,
        capability: AdaptiveBackendCapability,
        canonical_fps: Option<u32>,
        process_id: Option<u32>,
        now_millis: u64,
    ) {
        let res_id = TrackedResourceId::for_shared_group(group_id);
        let ownership = ResourceOwnership::SharedGroup {
            layout_mode,
            primary_display_id: primary_display_id.clone(),
            participating_displays: participating_displays.clone(),
            process_id,
        };

        let entry = self.resources.entry(res_id.clone()).or_insert_with(|| {
            TrackedPlaybackResource {
                resource_id: res_id.clone(),
                ownership: ownership.clone(),
                backend_capability: capability.clone(),
                coverage_status: MeasurementCoverageStatus::Unknown {
                    reason: "Awaiting initial observation".into(),
                },
                session_generation: generation,
                effective_override: EffectiveQualityOverride::new_normal(canonical_fps, now_millis),
                last_adjustment_millis: None,
            }
        });

        entry.session_generation = generation;
        entry.backend_capability = capability;
        entry.ownership = ownership;

        self.display_to_resource
            .insert(primary_display_id.clone(), res_id.clone());
        for did in participating_displays {
            self.display_to_resource.insert(did, res_id.clone());
        }
    }

    /// Unregisters a terminated session or removed display, cleaning up inventory and mappings.
    pub fn unregister_display(&mut self, display_id: &DisplayId) {
        if let Some(res_id) = self.display_to_resource.remove(display_id) {
            // Check if any other display references this resource
            let is_still_referenced = self.display_to_resource.values().any(|r| r == &res_id);
            if !is_still_referenced {
                self.resources.remove(&res_id);
            }
        }
    }

    /// Ingests a new performance measurement sample into the inventory.
    pub fn record_observation(&mut self, observation: PlaybackResourceObservation, now_millis: u64) {
        let target_res_id = match &observation.scope {
            ResourceObservationScope::DisplaySession(did) => self.display_to_resource.get(did).cloned(),
            ResourceObservationScope::Process(pid) => {
                // Find resource matching this process ID
                self.resources
                    .values()
                    .find(|r| r.ownership.process_id() == Some(*pid))
                    .map(|r| r.resource_id.clone())
            }
            ResourceObservationScope::Application => None,
        };

        if let Some(res_id) = target_res_id {
            if let Some(res) = self.resources.get_mut(&res_id) {
                if observation.is_fresh(now_millis) && observation.metric_kind == ResourceMetricKind::ProcessCpuPercent {
                    res.coverage_status = MeasurementCoverageStatus::Verified {
                        cpu_percent: observation.value as f32,
                        working_set_bytes: None,
                        sampled_at_millis: observation.timestamp_millis,
                    };
                } else if observation.quality == ResourceSignalQuality::Unavailable {
                    res.coverage_status = MeasurementCoverageStatus::Unknown {
                        reason: "Signal unavailable from backend".into(),
                    };
                }
            }
        }
    }

    /// Directly records verified process CPU and memory metrics for a resource.
    pub fn record_process_metrics(
        &mut self,
        display_id: &DisplayId,
        cpu_percent: f32,
        working_set_bytes: Option<u64>,
        now_millis: u64,
    ) {
        if let Some(res_id) = self.display_to_resource.get(display_id) {
            if let Some(res) = self.resources.get_mut(res_id) {
                res.coverage_status = MeasurementCoverageStatus::Verified {
                    cpu_percent,
                    working_set_bytes,
                    sampled_at_millis: now_millis,
                };
            }
        }
    }

    /// Computes current aggregate workload across all active resources.
    #[must_use]
    pub fn compute_workload(&self, now_millis: u64) -> AggregateWorkloadSnapshot {
        let res_vec: Vec<&TrackedPlaybackResource> = self.resources.values().collect();
        AggregateWorkloadSnapshot::compute(&res_vec, now_millis)
    }

    /// Evaluates aggregate workload and arbitrates optimization actions across eligible sessions.
    pub fn arbitrate(
        &mut self,
        coordinator: &DisplaySessionCoordinator,
        active_pause_reasons: &HashMap<DisplayId, HashSet<PauseReason>>,
        power_eval: Option<&PowerPolicyEvaluation>,
        now_millis: u64,
    ) -> Vec<ArbitrationAction> {
        let mut actions = Vec::new();

        // 1. If disabled: restore any active overrides to canonical user settings
        if !self.budget.enabled {
            self.current_state = AggregateBudgetState::Disabled;
            for res in self.resources.values_mut() {
                if res.effective_override.level != AdaptiveQualityLevel::Normal {
                    res.effective_override.level = AdaptiveQualityLevel::Normal;
                    res.effective_override.effective_fps = res.effective_override.canonical_fps;
                    res.effective_override.applied_at_millis = now_millis;
                    res.effective_override.reason = "Resource budget disabled; canonical settings restored".into();

                    match &res.ownership {
                        ResourceOwnership::Dedicated { display_id, session_id, .. } => {
                            actions.push(ArbitrationAction::RestoreSession {
                                display_id: display_id.clone(),
                                session_id: *session_id,
                                canonical_fps: res.effective_override.canonical_fps,
                                reason: "Resource budget disabled".into(),
                            });
                        }
                        ResourceOwnership::SharedGroup {
                            primary_display_id,
                            participating_displays,
                            ..
                        } => {
                            actions.push(ArbitrationAction::RestoreShared {
                                primary_display_id: primary_display_id.clone(),
                                affected_displays: participating_displays.clone(),
                                canonical_fps: res.effective_override.canonical_fps,
                                reason: "Resource budget disabled".into(),
                            });
                        }
                        ResourceOwnership::ApplicationProcess { .. } => {}
                    }
                }
            }
            if actions.is_empty() {
                actions.push(ArbitrationAction::NoChange {
                    reason: "Resource budget management is disabled".into(),
                });
            } else {
                self.record_diagnostic(
                    BudgetDiagnosticEventType::RestorationArbitrated,
                    format!("Restored canonical settings for {} resources due to budget disable", actions.len()),
                    None,
                    None,
                );
            }
            return actions;
        }

        // 2. Compute aggregate workload snapshot
        let workload = self.compute_workload(now_millis);
        let coverage_ratio = if workload.total_sessions_count > 0 {
            workload.measured_sessions_count as f32 / workload.total_sessions_count as f32
        } else {
            1.0
        };

        // 3. Handle incomplete measurement coverage
        if !workload.is_coverage_complete && workload.measured_sessions_count == 0 {
            self.current_state = AggregateBudgetState::CoverageIncomplete;
            let unmeasured: Vec<DisplayId> = self.display_to_resource.keys().cloned().collect();
            actions.push(ArbitrationAction::CoverageIncompleteDefer {
                unmeasured_displays: unmeasured,
                reason: "Zero active sessions have verified resource telemetry".into(),
            });
            return actions;
        }

        // 4. Cooldown guard: prevent flapping
        if let Some(last_action) = self.last_action_timestamp_millis {
            let elapsed = now_millis.saturating_sub(last_action);
            if elapsed < self.budget.adjustment_cooldown_ms {
                actions.push(ArbitrationAction::HoldCooldown {
                    remaining_ms: self.budget.adjustment_cooldown_ms - elapsed,
                });
                return actions;
            }
        }

        // 5. Evaluate pressure thresholds with dwell timing & hysteresis
        let is_power_pressure = self.budget.allow_power_coordination
            && power_eval.map(|p| p.should_pause || p.prefer_static_fallback).unwrap_or(false);

        if is_power_pressure || workload.aggregate_cpu_percent >= self.budget.pressure_threshold_cpu_percent {
            let sustained_start = self.high_pressure_since.get_or_insert(now_millis);
            self.low_pressure_since = None;
            let sustained_elapsed = now_millis.saturating_sub(*sustained_start);

            if is_power_pressure || sustained_elapsed >= self.budget.sustained_pressure_threshold_ms {
                self.current_state = AggregateBudgetState::SustainedPressure;
                self.record_diagnostic(
                    BudgetDiagnosticEventType::SustainedPressureDetected,
                    format!(
                        "Sustained aggregate CPU pressure confirmed ({:.1}%, threshold: {:.1}%)",
                        workload.aggregate_cpu_percent, self.budget.pressure_threshold_cpu_percent
                    ),
                    Some(workload.aggregate_cpu_percent),
                    Some(coverage_ratio),
                );

                // Select prioritization candidates for reduction
                let adjustment_actions = self.select_optimization_candidates(
                    coordinator,
                    active_pause_reasons,
                    now_millis,
                );

                if adjustment_actions.is_empty() {
                    self.current_state = AggregateBudgetState::TargetInfeasible;
                    actions.push(ArbitrationAction::TargetInfeasible {
                        reason: "All active sessions are already at minimum quality floor or backends do not support controls".into(),
                    });
                } else {
                    self.current_state = AggregateBudgetState::OptimizationActive;
                    self.last_action_timestamp_millis = Some(now_millis);
                    for act in &adjustment_actions {
                        self.record_diagnostic(
                            BudgetDiagnosticEventType::OptimizationArbitrated,
                            format!("Arbitrated optimization reduction: {:?}", act),
                            Some(workload.aggregate_cpu_percent),
                            Some(coverage_ratio),
                        );
                    }
                    actions.extend(adjustment_actions);
                }
                return actions;
            } else {
                self.current_state = AggregateBudgetState::ApproachingBudget;
                actions.push(ArbitrationAction::NoChange {
                    reason: format!(
                        "Aggregate pressure detected; awaiting sustained dwell threshold ({}ms/{}ms)",
                        sustained_elapsed, self.budget.sustained_pressure_threshold_ms
                    ),
                });
                return actions;
            }
        } else if workload.aggregate_cpu_percent <= self.budget.recovery_threshold_cpu_percent {
            // Low pressure: evaluate recovery dwell
            self.high_pressure_since = None;
            let recovery_start = self.low_pressure_since.get_or_insert(now_millis);
            let recovery_elapsed = now_millis.saturating_sub(*recovery_start);

            if recovery_elapsed >= self.budget.recovery_dwell_threshold_ms {
                self.current_state = AggregateBudgetState::WithinBudget;
                self.low_pressure_since = None;

                // Select candidates for restoration
                let restore_actions = self.select_restoration_candidates(now_millis);
                if !restore_actions.is_empty() {
                    self.last_action_timestamp_millis = Some(now_millis);
                    for act in &restore_actions {
                        self.record_diagnostic(
                            BudgetDiagnosticEventType::RestorationArbitrated,
                            format!("Arbitrated restoration of canonical settings: {:?}", act),
                            Some(workload.aggregate_cpu_percent),
                            Some(coverage_ratio),
                        );
                    }
                    actions.extend(restore_actions);
                    return actions;
                }
            } else {
                self.current_state = AggregateBudgetState::Recovering;
                actions.push(ArbitrationAction::NoChange {
                    reason: format!(
                        "Workload below recovery threshold; awaiting dwell completion ({}ms/{}ms)",
                        recovery_elapsed, self.budget.recovery_dwell_threshold_ms
                    ),
                });
                return actions;
            }
        } else {
            // In the hysteresis deadband between recovery and pressure
            self.high_pressure_since = None;
            self.low_pressure_since = None;
            if self.current_state != AggregateBudgetState::OptimizationActive {
                self.current_state = AggregateBudgetState::WithinBudget;
            }
        }

        if actions.is_empty() {
            actions.push(ArbitrationAction::NoChange {
                reason: "Aggregate resource workload is within acceptable budget target".into(),
            });
        }

        actions
    }

    /// Selects candidates for optimization reduction based on resource demand and backend capability.
    /// Heaviest CPU consumers are prioritized first to protect lighter wallpapers.
    fn select_optimization_candidates(
        &mut self,
        coordinator: &DisplaySessionCoordinator,
        active_pause_reasons: &HashMap<DisplayId, HashSet<PauseReason>>,
        now_millis: u64,
    ) -> Vec<ArbitrationAction> {
        let mut candidates: Vec<(TrackedResourceId, f32)> = Vec::new();

        for (res_id, res) in &self.resources {
            // Capability gate: must support either framerate cap or render cadence throttling
            if !res.backend_capability.supports_framerate_cap
                && !res.backend_capability.supports_render_cadence_throttling
            {
                continue;
            }

            // Cannot downgrade if already at MinimumEligible
            if res.effective_override.level == AdaptiveQualityLevel::MinimumEligible {
                continue;
            }

            // Policy Interlock: check if session is paused or fallback
            let is_interlocked = match &res.ownership {
                ResourceOwnership::Dedicated { display_id, session_id, .. } => {
                    let is_paused = active_pause_reasons
                        .get(display_id)
                        .map(|r| !r.is_empty())
                        .unwrap_or(false);
                    let is_fallback_or_suspended = coordinator
                        .get_session(display_id)
                        .map(|s| {
                            s.session_id != *session_id
                                || s.state.is_fallback()
                                || matches!(s.state, DisplaySessionState::Suspended)
                        })
                        .unwrap_or(false);
                    is_paused || is_fallback_or_suspended
                }
                ResourceOwnership::SharedGroup { primary_display_id, .. } => {
                    let is_paused = active_pause_reasons
                        .get(primary_display_id)
                        .map(|r| !r.is_empty())
                        .unwrap_or(false);
                    let is_fallback_or_suspended = coordinator
                        .get_session(primary_display_id)
                        .map(|s| s.state.is_fallback() || matches!(s.state, DisplaySessionState::Suspended))
                        .unwrap_or(false);
                    is_paused || is_fallback_or_suspended
                }
                ResourceOwnership::ApplicationProcess { .. } => false,
            };

            if is_interlocked {
                continue;
            }

            // Measured CPU weight
            let cpu_weight = match &res.coverage_status {
                MeasurementCoverageStatus::Verified { cpu_percent, .. } => *cpu_percent,
                _ => 0.0f32,
            };

            candidates.push((res_id.clone(), cpu_weight));
        }

        // Sort descending by CPU weight: heaviest consumer optimized first!
        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let mut actions = Vec::new();
        let max_adj = self.budget.max_simultaneous_adjustments;

        for (res_id, _) in candidates.into_iter().take(max_adj) {
            if let Some(res) = self.resources.get_mut(&res_id) {
                let next_level = match res.effective_override.level {
                    AdaptiveQualityLevel::Normal => AdaptiveQualityLevel::Reduced,
                    AdaptiveQualityLevel::Reduced => AdaptiveQualityLevel::MinimumEligible,
                    AdaptiveQualityLevel::MinimumEligible => AdaptiveQualityLevel::MinimumEligible,
                };

                let target_fps = match next_level {
                    AdaptiveQualityLevel::Normal => res.effective_override.canonical_fps,
                    AdaptiveQualityLevel::Reduced => Some(30),
                    AdaptiveQualityLevel::MinimumEligible => Some(20),
                };

                res.effective_override.level = next_level;
                res.effective_override.effective_fps = target_fps;
                res.effective_override.applied_at_millis = now_millis;
                res.effective_override.reason = format!("Arbitrated reduction to {:?}", next_level);
                res.last_adjustment_millis = Some(now_millis);

                match &res.ownership {
                    ResourceOwnership::Dedicated { display_id, session_id, .. } => {
                        actions.push(ArbitrationAction::ApplySessionReduction {
                            display_id: display_id.clone(),
                            session_id: *session_id,
                            target_level: next_level,
                            target_fps,
                            reason: "Arbitrated load balancing under aggregate CPU pressure".into(),
                        });
                    }
                    ResourceOwnership::SharedGroup {
                        primary_display_id,
                        participating_displays,
                        ..
                    } => {
                        actions.push(ArbitrationAction::ApplySharedReduction {
                            primary_display_id: primary_display_id.clone(),
                            affected_displays: participating_displays.clone(),
                            target_level: next_level,
                            target_fps,
                            reason: "Arbitrated shared load balancing under aggregate CPU pressure".into(),
                        });
                    }
                    ResourceOwnership::ApplicationProcess { .. } => {}
                }
            }
        }

        actions
    }

    /// Selects candidates for restoration back toward canonical user settings.
    fn select_restoration_candidates(&mut self, now_millis: u64) -> Vec<ArbitrationAction> {
        let mut actions = Vec::new();
        let max_adj = self.budget.max_simultaneous_adjustments;
        let mut restored_count = 0usize;

        for res in self.resources.values_mut() {
            if restored_count >= max_adj {
                break;
            }

            if res.effective_override.level != AdaptiveQualityLevel::Normal {
                let next_level = match res.effective_override.level {
                    AdaptiveQualityLevel::MinimumEligible => AdaptiveQualityLevel::Reduced,
                    AdaptiveQualityLevel::Reduced => AdaptiveQualityLevel::Normal,
                    AdaptiveQualityLevel::Normal => AdaptiveQualityLevel::Normal,
                };

                let target_fps = if next_level == AdaptiveQualityLevel::Normal {
                    res.effective_override.canonical_fps
                } else {
                    Some(30)
                };

                res.effective_override.level = next_level;
                res.effective_override.effective_fps = target_fps;
                res.effective_override.applied_at_millis = now_millis;
                res.effective_override.reason = format!("Restored quality toward {:?}", next_level);
                res.last_adjustment_millis = Some(now_millis);
                restored_count += 1;

                match &res.ownership {
                    ResourceOwnership::Dedicated { display_id, session_id, .. } => {
                        actions.push(ArbitrationAction::RestoreSession {
                            display_id: display_id.clone(),
                            session_id: *session_id,
                            canonical_fps: target_fps,
                            reason: "Aggregate workload recovered; restored quality tier".into(),
                        });
                    }
                    ResourceOwnership::SharedGroup {
                        primary_display_id,
                        participating_displays,
                        ..
                    } => {
                        actions.push(ArbitrationAction::RestoreShared {
                            primary_display_id: primary_display_id.clone(),
                            affected_displays: participating_displays.clone(),
                            canonical_fps: target_fps,
                            reason: "Aggregate workload recovered; restored shared quality tier".into(),
                        });
                    }
                    ResourceOwnership::ApplicationProcess { .. } => {}
                }
            }
        }

        actions
    }

    /// Records a diagnostic event into the bounded circular log.
    pub fn record_diagnostic(
        &mut self,
        event_type: BudgetDiagnosticEventType,
        message: String,
        aggregate_cpu_percent: Option<f32>,
        coverage_ratio: Option<f32>,
    ) {
        if self.diagnostics.len() >= MAX_BUDGET_DIAGNOSTICS {
            self.diagnostics.remove(0);
        }
        self.diagnostics.push(BudgetDiagnosticEvent {
            timestamp_millis: Self::current_timestamp_millis(),
            event_type,
            message,
            aggregate_cpu_percent,
            coverage_ratio,
        });
    }

    /// Returns diagnostic event history.
    #[must_use]
    pub fn get_diagnostics(&self) -> &[BudgetDiagnosticEvent] {
        &self.diagnostics
    }

    /// Returns a tracked resource by ID, if present.
    #[must_use]
    pub fn get_resource(&self, res_id: &TrackedResourceId) -> Option<&TrackedPlaybackResource> {
        self.resources.get(res_id)
    }

    /// Returns all tracked resources.
    #[must_use]
    pub fn get_all_resources(&self) -> Vec<&TrackedPlaybackResource> {
        self.resources.values().collect()
    }
}
