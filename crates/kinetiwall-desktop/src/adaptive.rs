//! Resource-Aware Adaptive Playback Quality, Frame Pacing & Performance Budgets (Step 51).
//!
//! Provides:
//! - Typed resource signal observations with scope, freshness, and signal quality semantics.
//! - Bounded sustained-pressure detection filtering transient spikes with hysteresis and cooldowns.
//! - Separation of canonical user-configured playback settings from temporary optimizer effective overrides.
//! - Backend capability negotiation for supported runtime quality controls (Video vs Web vs Static).
//! - Multi-monitor coordination across Independent, Span, and Clone presentations.
//! - Strict coordination preserving pause reasons, power-saving policies, and static wallpaper fallback.
//! - Bounded diagnostic event history for verifiable operational transparency.

use std::collections::{HashMap, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

use kinetiwall_catalog::WallpaperType;
use serde::{Deserialize, Serialize};

use crate::display::DisplayId;
use crate::layout::MultiMonitorLayoutMode;
use crate::policy::PauseReason;
use crate::power::PowerPolicyEvaluation;
use crate::session::{DisplaySessionCoordinator, DisplaySessionState};

// -------------------------------------------------------------------------------------------------
// Timing, Threshold & Budget Constants
// -------------------------------------------------------------------------------------------------

/// Default duration (in milliseconds) of sustained resource pressure required before downgrading.
pub const DEFAULT_SUSTAINED_PRESSURE_THRESHOLD_MS: u64 = 3000;

/// Default dwell duration (in milliseconds) below recovery threshold required before restoring quality.
pub const DEFAULT_RECOVERY_DWELL_THRESHOLD_MS: u64 = 5000;

/// Default process CPU threshold (percentage, 0.0 - 100.0) triggering adaptive pressure evaluation.
pub const DEFAULT_CPU_PRESSURE_THRESHOLD_PERCENT: f32 = 75.0;

/// Default process CPU threshold for hysteresis recovery (exit threshold).
pub const DEFAULT_CPU_RECOVERY_THRESHOLD_PERCENT: f32 = 40.0;

/// Default minimum cooldown duration between successive quality adjustments (in milliseconds).
pub const DEFAULT_ADJUSTMENT_COOLDOWN_MS: u64 = 4000;

/// Default minimum frame rate floor (frames per second).
pub const DEFAULT_TARGET_FPS_FLOOR: u32 = 30;

/// Absolute minimum safe frame rate floor below which the engine will never degrade.
pub const MIN_SAFE_FPS_FLOOR: u32 = 15;

/// Maximum number of diagnostic events retained in the adaptive policy ring buffer.
pub const MAX_ADAPTIVE_DIAGNOSTICS: usize = 128;

/// Maximum number of resource observation samples retained per session detector.
pub const MAX_RESOURCE_SAMPLE_HISTORY: usize = 30;

// -------------------------------------------------------------------------------------------------
// Resource Observation Models
// -------------------------------------------------------------------------------------------------

/// Category of measurable resource performance metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceMetricKind {
    /// Process-level CPU utilization across available cores (0.0 to 100.0%).
    ProcessCpuPercent,
    /// Working set memory allocated to the player process in bytes.
    ProcessWorkingSetBytes,
    /// Time spent rendering an individual frame in milliseconds.
    RenderFrameTimeMs,
    /// Number of dropped or late frames observed in the sample window.
    DroppedFramesCount,
    /// Observed presentation cadence / refresh rate in frames per second (Hz).
    RenderCadenceFps,
}

/// Measurement unit associated with a resource observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceMetricUnit {
    Percent,
    Bytes,
    Megabytes,
    Milliseconds,
    Frames,
    Fps,
}

/// Scope of attribution for a resource observation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "scope_type", content = "id", rename_all = "snake_case")]
pub enum ResourceObservationScope {
    /// Entire KinetiWall desktop application process.
    Application,
    /// Dedicated playback session for a specific display.
    DisplaySession(DisplayId),
    /// Specific child player host process ID.
    Process(u32),
}

/// Data quality classification for a resource observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceSignalQuality {
    /// Observation is fresh, verified, and ready for policy evaluation.
    Valid,
    /// Observation was collected under degraded conditions or partial sampling.
    Degraded,
    /// Measurement was unavailable, failed to read, or expired.
    Unavailable,
}

/// Typed, privacy-preserving performance measurement sample.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaybackResourceObservation {
    /// Identifier of the measurement collector or metric name.
    pub measurement_id: String,
    /// Category of metric.
    pub metric_kind: ResourceMetricKind,
    /// Numeric value.
    pub value: f64,
    /// Associated unit.
    pub unit: ResourceMetricUnit,
    /// Scope of attribution.
    pub scope: ResourceObservationScope,
    /// Monotonic observation timestamp (Unix epoch millis).
    pub timestamp_millis: u64,
    /// Duration in milliseconds for which this observation is considered fresh.
    pub validity_window_millis: u64,
    /// Quality classification.
    pub quality: ResourceSignalQuality,
}

impl PlaybackResourceObservation {
    /// Constructs a valid fresh resource observation.
    #[must_use]
    pub fn new_valid(
        measurement_id: impl Into<String>,
        metric_kind: ResourceMetricKind,
        value: f64,
        unit: ResourceMetricUnit,
        scope: ResourceObservationScope,
        timestamp_millis: u64,
        validity_window_millis: u64,
    ) -> Self {
        Self {
            measurement_id: measurement_id.into(),
            metric_kind,
            value,
            unit,
            scope,
            timestamp_millis,
            validity_window_millis,
            quality: ResourceSignalQuality::Valid,
        }
    }

    /// Constructs an unavailable or failed observation placeholder.
    #[must_use]
    pub fn new_unavailable(
        measurement_id: impl Into<String>,
        metric_kind: ResourceMetricKind,
        scope: ResourceObservationScope,
        timestamp_millis: u64,
    ) -> Self {
        Self {
            measurement_id: measurement_id.into(),
            metric_kind,
            value: 0.0,
            unit: ResourceMetricUnit::Percent,
            scope,
            timestamp_millis,
            validity_window_millis: 0,
            quality: ResourceSignalQuality::Unavailable,
        }
    }

    /// Returns true if the observation is within its validity window and has Valid quality.
    #[must_use]
    pub fn is_fresh(&self, now_millis: u64) -> bool {
        self.quality == ResourceSignalQuality::Valid
            && now_millis.saturating_sub(self.timestamp_millis) <= self.validity_window_millis
    }
}

// -------------------------------------------------------------------------------------------------
// Adaptive Quality Levels & Backend Capabilities
// -------------------------------------------------------------------------------------------------

/// Discrete adaptive quality tier for active playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AdaptiveQualityLevel {
    /// Normal: 100% full user configuration, native refresh cadence (e.g. 60 FPS).
    #[default]
    Normal,
    /// Reduced: Bounded first-tier throttling (e.g. 30 FPS cap or reduced render cadence).
    Reduced,
    /// MinimumEligible: Conservative floor tier (e.g. 20-24 FPS, safe floor).
    MinimumEligible,
}

impl AdaptiveQualityLevel {
    /// Returns human-readable label for diagnostics and UI presentation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Reduced => "reduced",
            Self::MinimumEligible => "minimum_eligible",
        }
    }
}

/// Capability descriptor declaring what runtime quality controls a backend can safely execute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdaptiveBackendCapability {
    /// True if the backend supports dynamic frame-rate clamping (e.g. libmpv).
    pub supports_framerate_cap: bool,
    /// Minimum frame rate in FPS the backend supports when capped.
    pub min_supported_fps: u32,
    /// Maximum frame rate in FPS supported by the backend.
    pub max_supported_fps: u32,
    /// True if the backend supports render cadence or update interval reduction.
    pub supports_render_cadence_throttling: bool,
    /// True if the backend supports downsampling audio-reactive visualizer updates.
    pub supports_visualizer_downsampling: bool,
    /// True if the backend can reliably restore previous canonical settings without player reload.
    pub can_restore_accurately: bool,
    /// True if the backend runtime is currently available.
    pub is_available: bool,
}

impl AdaptiveBackendCapability {
    /// Capabilities for video backend (`libmpv`).
    /// Supports dynamic frame-rate capping and render cadence throttling.
    #[must_use]
    pub fn for_video(is_available: bool) -> Self {
        Self {
            supports_framerate_cap: true,
            min_supported_fps: 15,
            max_supported_fps: 60,
            supports_render_cadence_throttling: true,
            supports_visualizer_downsampling: false,
            can_restore_accurately: true,
            is_available,
        }
    }

    /// Capabilities for interactive web backend (`WebView2`).
    /// Supports visualizer downsampling and render cadence, but does NOT support arbitrary JS throttling.
    #[must_use]
    pub fn for_web(is_available: bool) -> Self {
        Self {
            supports_framerate_cap: false,
            min_supported_fps: 30,
            max_supported_fps: 60,
            supports_render_cadence_throttling: true,
            supports_visualizer_downsampling: true,
            can_restore_accurately: true,
            is_available,
        }
    }

    /// Derives capability descriptor from catalog `WallpaperType`.
    #[must_use]
    pub fn from_wallpaper_type(wallpaper_type: WallpaperType, is_available: bool) -> Self {
        match wallpaper_type {
            WallpaperType::Video => Self::for_video(is_available),
            WallpaperType::Web => Self::for_web(is_available),
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Configuration & Settings Model
// -------------------------------------------------------------------------------------------------

/// User-configurable policy settings governing resource-aware adaptive quality optimization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdaptiveQualitySettings {
    /// Master toggle for adaptive quality optimization.
    /// STRICT REQUIREMENT: Disabled by default for existing users!
    pub enabled: bool,

    /// Minimum target frame rate floor (clamped in [MIN_SAFE_FPS_FLOOR, 60]).
    pub target_fps_floor: u32,

    /// Duration of sustained pressure required before applying quality reduction (millis).
    pub sustained_pressure_threshold_ms: u64,

    /// Duration of low-pressure dwell required before restoring toward normal quality (millis).
    pub recovery_dwell_threshold_ms: u64,

    /// Process CPU threshold (percentage) triggering pressure evaluation.
    pub cpu_pressure_threshold_percent: f32,

    /// Process CPU threshold (percentage) required for hysteresis recovery.
    pub cpu_recovery_threshold_percent: f32,

    /// Cooldown between consecutive quality transitions (millis).
    pub adjustment_cooldown_ms: u64,

    /// True if power policy (battery/saver) can coordinate adaptive quality adjustments.
    pub allow_power_policy_coordination: bool,
}

impl Default for AdaptiveQualitySettings {
    fn default() -> Self {
        Self {
            enabled: false, // Opt-in by default
            target_fps_floor: DEFAULT_TARGET_FPS_FLOOR,
            sustained_pressure_threshold_ms: DEFAULT_SUSTAINED_PRESSURE_THRESHOLD_MS,
            recovery_dwell_threshold_ms: DEFAULT_RECOVERY_DWELL_THRESHOLD_MS,
            cpu_pressure_threshold_percent: DEFAULT_CPU_PRESSURE_THRESHOLD_PERCENT,
            cpu_recovery_threshold_percent: DEFAULT_CPU_RECOVERY_THRESHOLD_PERCENT,
            adjustment_cooldown_ms: DEFAULT_ADJUSTMENT_COOLDOWN_MS,
            allow_power_policy_coordination: true,
        }
    }
}

impl AdaptiveQualitySettings {
    /// Validates and sanitizes settings values within bounded constraints.
    #[must_use]
    pub fn sanitized(mut self) -> Self {
        if self.target_fps_floor < MIN_SAFE_FPS_FLOOR {
            self.target_fps_floor = MIN_SAFE_FPS_FLOOR;
        } else if self.target_fps_floor > 60 {
            self.target_fps_floor = 60;
        }

        if self.cpu_pressure_threshold_percent <= self.cpu_recovery_threshold_percent {
            // Guarantee hysteresis gap
            self.cpu_recovery_threshold_percent = (self.cpu_pressure_threshold_percent - 10.0).max(10.0);
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

        self
    }
}

// -------------------------------------------------------------------------------------------------
// Effective Quality Override & Canonical State Separation
// -------------------------------------------------------------------------------------------------

/// Separates the user's canonical desired settings from temporary optimizer effective overrides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveQualityOverride {
    /// User's canonical desired target frame rate (if configured).
    pub canonical_fps: Option<u32>,
    /// Optimizer's active effective target frame rate.
    pub effective_fps: Option<u32>,
    /// Active adaptive tier level.
    pub level: AdaptiveQualityLevel,
    /// Timestamp when this override was applied (Unix epoch millis).
    pub applied_at_millis: u64,
    /// Reason or trigger description for this override.
    pub reason: String,
}

impl EffectiveQualityOverride {
    #[must_use]
    pub fn new_normal(canonical_fps: Option<u32>, now_millis: u64) -> Self {
        Self {
            canonical_fps,
            effective_fps: canonical_fps,
            level: AdaptiveQualityLevel::Normal,
            applied_at_millis: now_millis,
            reason: "User canonical settings active".into(),
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Sustained Pressure Detector & Hysteresis
// -------------------------------------------------------------------------------------------------

/// Current pressure classification derived from historical observation samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PressureState {
    /// Performance is within acceptable operational thresholds.
    #[default]
    Normal,
    /// Sustained pressure confirmed exceeding the configured duration threshold.
    SustainedPressure,
    /// Readings are currently missing, stale, or unavailable.
    Unknown,
}

/// Evaluates a sliding window of performance measurements with hysteresis and spike rejection.
#[derive(Debug, Clone)]
pub struct SustainedPressureDetector {
    /// Bounded sliding sample history.
    samples: VecDeque<PlaybackResourceObservation>,
    /// Timestamp when high pressure was first continuously observed (Unix epoch millis).
    high_pressure_since: Option<u64>,
    /// Timestamp when low pressure (recovery) was first continuously observed (Unix epoch millis).
    low_pressure_since: Option<u64>,
    /// Current confirmed pressure classification.
    current_state: PressureState,
}

impl Default for SustainedPressureDetector {
    fn default() -> Self {
        Self {
            samples: VecDeque::with_capacity(MAX_RESOURCE_SAMPLE_HISTORY),
            high_pressure_since: None,
            low_pressure_since: None,
            current_state: PressureState::Normal,
        }
    }
}

impl SustainedPressureDetector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a new resource observation sample, pruning stale and excess entries.
    pub fn record_sample(&mut self, sample: PlaybackResourceObservation, now_millis: u64) {
        let is_cpu = sample.metric_kind == ResourceMetricKind::ProcessCpuPercent
            && sample.quality == ResourceSignalQuality::Valid;
        let cpu_val = sample.value as f32;
        let sample_time = sample.timestamp_millis;

        while self.samples.len() >= MAX_RESOURCE_SAMPLE_HISTORY {
            self.samples.pop_front();
        }
        self.samples.retain(|s| s.is_fresh(now_millis));
        self.samples.push_back(sample);

        if is_cpu {
            if cpu_val >= DEFAULT_CPU_PRESSURE_THRESHOLD_PERCENT {
                self.high_pressure_since.get_or_insert(sample_time);
                self.low_pressure_since = None;
            } else if cpu_val <= DEFAULT_CPU_RECOVERY_THRESHOLD_PERCENT {
                self.low_pressure_since.get_or_insert(sample_time);
                self.high_pressure_since = None;
            } else {
                self.high_pressure_since = None;
                self.low_pressure_since = None;
            }
        }
    }

    /// Evaluates current pressure state against settings thresholds with hysteresis.
    pub fn evaluate(
        &mut self,
        settings: &AdaptiveQualitySettings,
        now_millis: u64,
    ) -> PressureState {
        // Filter for fresh CPU observations
        let valid_cpu_samples: Vec<&PlaybackResourceObservation> = self
            .samples
            .iter()
            .filter(|s| s.is_fresh(now_millis) && s.metric_kind == ResourceMetricKind::ProcessCpuPercent)
            .collect();

        if valid_cpu_samples.is_empty() {
            // Missing/stale measurements: DO NOT treat as zero! Return Unknown.
            self.high_pressure_since = None;
            self.low_pressure_since = None;
            self.current_state = PressureState::Unknown;
            return PressureState::Unknown;
        }

        let latest = valid_cpu_samples.last().unwrap();
        let latest_cpu = latest.value as f32;

        match self.current_state {
            PressureState::Normal | PressureState::Unknown => {
                if latest_cpu >= settings.cpu_pressure_threshold_percent {
                    let first = self.high_pressure_since.get_or_insert(latest.timestamp_millis);
                    let elapsed = now_millis.saturating_sub(*first);
                    if elapsed >= settings.sustained_pressure_threshold_ms {
                        self.current_state = PressureState::SustainedPressure;
                        self.low_pressure_since = None;
                    }
                } else {
                    self.high_pressure_since = None;
                    self.current_state = PressureState::Normal;
                }
            }
            PressureState::SustainedPressure => {
                if latest_cpu <= settings.cpu_recovery_threshold_percent {
                    let first = self.low_pressure_since.get_or_insert(latest.timestamp_millis);
                    let elapsed = now_millis.saturating_sub(*first);
                    if elapsed >= settings.recovery_dwell_threshold_ms {
                        self.current_state = PressureState::Normal;
                        self.high_pressure_since = None;
                    }
                } else {
                    self.low_pressure_since = None;
                }
            }
        }

        self.current_state
    }

    /// Returns current confirmed pressure state.
    #[must_use]
    pub fn current_state(&self) -> PressureState {
        self.current_state
    }

    /// Resets internal timing states (e.g. on session recreation or mode change).
    pub fn reset(&mut self) {
        self.samples.clear();
        self.high_pressure_since = None;
        self.low_pressure_since = None;
        self.current_state = PressureState::Normal;
    }
}

// -------------------------------------------------------------------------------------------------
// Policy Action & Decision Model
// -------------------------------------------------------------------------------------------------

/// Operational action commanded by the adaptive policy evaluator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdaptiveAction {
    /// No change required; existing operational parameters are optimal.
    NoChange,
    /// Bounded quality adjustment requested.
    ApplyAdjustment,
    /// Reversion toward user's canonical configuration requested.
    RestoreSettings,
    /// Adjustment suppressed due to active cooldown timer.
    HoldCooldown,
}

/// Evaluated policy decision for an individual display session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdaptivePolicyDecision {
    pub display_id: DisplayId,
    pub current_level: AdaptiveQualityLevel,
    pub target_level: AdaptiveQualityLevel,
    pub effective_fps: Option<u32>,
    pub action: AdaptiveAction,
    pub reason: String,
}

// -------------------------------------------------------------------------------------------------
// Diagnostic Log Event Model
// -------------------------------------------------------------------------------------------------

/// Classification of adaptive quality diagnostic events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdaptiveDiagnosticEventType {
    PolicyEnabled,
    PolicyDisabled,
    SustainedPressureDetected,
    PressureCleared,
    AdjustmentApplied,
    SettingsRestored,
    CooldownActive,
    MeasurementStale,
    BackendUnsupported,
    Error,
}

/// Bounded diagnostic event log entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdaptiveDiagnosticEvent {
    pub timestamp_millis: u64,
    pub display_id: Option<DisplayId>,
    pub event_type: AdaptiveDiagnosticEventType,
    pub message: String,
}

// -------------------------------------------------------------------------------------------------
// Central Deterministic Policy Evaluator
// -------------------------------------------------------------------------------------------------

/// Central coordinator evaluating performance signals, capability boundaries,
/// cooldowns, and multi-monitor presentations.
pub struct AdaptivePlaybackPolicyEvaluator {
    /// Active policy settings.
    settings: AdaptiveQualitySettings,
    /// Per-display sustained pressure detectors.
    detectors: HashMap<DisplayId, SustainedPressureDetector>,
    /// Per-display active effective quality overrides.
    effective_overrides: HashMap<DisplayId, EffectiveQualityOverride>,
    /// Per-display timestamp of last applied adjustment (for cooldown enforcement).
    last_adjustment_time: HashMap<DisplayId, u64>,
    /// Bounded diagnostic event history.
    diagnostics: Vec<AdaptiveDiagnosticEvent>,
}

impl AdaptivePlaybackPolicyEvaluator {
    /// Constructs a new evaluator with default (opt-in disabled) settings.
    pub fn new(settings: AdaptiveQualitySettings) -> Self {
        Self {
            settings: settings.sanitized(),
            detectors: HashMap::new(),
            effective_overrides: HashMap::new(),
            last_adjustment_time: HashMap::new(),
            diagnostics: Vec::with_capacity(MAX_ADAPTIVE_DIAGNOSTICS),
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

    /// Updates adaptive policy configuration, recording diagnostic events.
    pub fn update_settings(&mut self, new_settings: AdaptiveQualitySettings) {
        let prev_enabled = self.settings.enabled;
        self.settings = new_settings.sanitized();

        if !prev_enabled && self.settings.enabled {
            self.record_diagnostic(
                None,
                AdaptiveDiagnosticEventType::PolicyEnabled,
                "Adaptive playback quality optimization enabled".into(),
            );
        } else if prev_enabled && !self.settings.enabled {
            self.record_diagnostic(
                None,
                AdaptiveDiagnosticEventType::PolicyDisabled,
                "Adaptive playback quality optimization disabled; restoring canonical settings".into(),
            );
        }
    }

    /// Returns reference to active settings.
    #[must_use]
    pub fn settings(&self) -> &AdaptiveQualitySettings {
        &self.settings
    }

    /// Feeds a new resource measurement observation into the target display detector.
    pub fn record_observation(&mut self, observation: PlaybackResourceObservation, now_millis: u64) {
        if let ResourceObservationScope::DisplaySession(ref did) = observation.scope {
            self.detectors
                .entry(did.clone())
                .or_default()
                .record_sample(observation, now_millis);
        } else {
            // Application-wide observation: replicate to all active detectors
            for detector in self.detectors.values_mut() {
                detector.record_sample(observation.clone(), now_millis);
            }
        }
    }

    /// Registers canonical user settings for a display session.
    pub fn register_session(
        &mut self,
        display_id: &DisplayId,
        canonical_fps: Option<u32>,
        now_millis: u64,
    ) {
        self.detectors.entry(display_id.clone()).or_default();
        self.effective_overrides.entry(display_id.clone()).or_insert_with(|| {
            EffectiveQualityOverride::new_normal(canonical_fps, now_millis)
        });
    }

    /// Unregisters a terminated session, cleaning up state without memory leaks.
    pub fn unregister_session(&mut self, display_id: &DisplayId) {
        self.detectors.remove(display_id);
        self.effective_overrides.remove(display_id);
        self.last_adjustment_time.remove(display_id);
    }

    /// Returns current effective override for a display.
    #[must_use]
    pub fn get_effective_override(&self, display_id: &DisplayId) -> Option<&EffectiveQualityOverride> {
        self.effective_overrides.get(display_id)
    }

    /// Evaluates adaptive quality policy for an individual display session.
    pub fn evaluate_session(
        &mut self,
        display_id: &DisplayId,
        capability: &AdaptiveBackendCapability,
        session_state: &DisplaySessionState,
        active_pause_reasons: &std::collections::HashSet<PauseReason>,
        power_eval: Option<&PowerPolicyEvaluation>,
        now_millis: u64,
    ) -> AdaptivePolicyDecision {
        let current_override = self
            .effective_overrides
            .entry(display_id.clone())
            .or_insert_with(|| EffectiveQualityOverride::new_normal(None, now_millis));

        let current_level = current_override.level;
        let canonical_fps = current_override.canonical_fps;

        // 1. If optimization is disabled: guarantee canonical configuration
        if !self.settings.enabled {
            if current_level != AdaptiveQualityLevel::Normal {
                current_override.level = AdaptiveQualityLevel::Normal;
                current_override.effective_fps = canonical_fps;
                current_override.applied_at_millis = now_millis;
                current_override.reason = "Optimization disabled; restored canonical settings".into();
                return AdaptivePolicyDecision {
                    display_id: display_id.clone(),
                    current_level,
                    target_level: AdaptiveQualityLevel::Normal,
                    effective_fps: canonical_fps,
                    action: AdaptiveAction::RestoreSettings,
                    reason: "Adaptive optimization is disabled".into(),
                };
            }
            return AdaptivePolicyDecision {
                display_id: display_id.clone(),
                current_level,
                target_level: AdaptiveQualityLevel::Normal,
                effective_fps: canonical_fps,
                action: AdaptiveAction::NoChange,
                reason: "Adaptive optimization is disabled".into(),
            };
        }

        // 2. Policy Interlock: If session is Suspended, StaticFallback, or Paused by Fullscreen / User
        if session_state.is_fallback()
            || matches!(session_state, DisplaySessionState::Suspended)
            || active_pause_reasons.contains(&PauseReason::UserManual)
            || active_pause_reasons.iter().any(|r| matches!(r, PauseReason::Fullscreen { .. }))
        {
            // Do not issue redundant quality adjustments to paused or fallback sessions
            return AdaptivePolicyDecision {
                display_id: display_id.clone(),
                current_level,
                target_level: current_level,
                effective_fps: current_override.effective_fps,
                action: AdaptiveAction::NoChange,
                reason: "Session is paused or in static fallback; throttling suppressed".into(),
            };
        }

        // 3. Backend Capability Gate: If backend does not support dynamic adjustments
        if !capability.supports_framerate_cap && !capability.supports_render_cadence_throttling {
            self.record_diagnostic(
                Some(display_id.clone()),
                AdaptiveDiagnosticEventType::BackendUnsupported,
                format!("Backend for display `{}` does not support runtime quality adjustments", display_id),
            );
            return AdaptivePolicyDecision {
                display_id: display_id.clone(),
                current_level,
                target_level: AdaptiveQualityLevel::Normal,
                effective_fps: canonical_fps,
                action: AdaptiveAction::NoChange,
                reason: "Backend does not support runtime quality adjustments".into(),
            };
        }

        // 4. Evaluate sustained resource pressure
        let detector = self.detectors.entry(display_id.clone()).or_default();
        let pressure_state = detector.evaluate(&self.settings, now_millis);

        // Check power policy coordination (e.g. Battery Saver or Low Battery triggers conservative quality)
        let is_power_pressure = self.settings.allow_power_policy_coordination
            && power_eval.map(|p| p.should_pause || p.prefer_static_fallback).unwrap_or(false);

        // Determine target level
        let target_level = if is_power_pressure || pressure_state == PressureState::SustainedPressure {
            if is_power_pressure {
                AdaptiveQualityLevel::MinimumEligible
            } else {
                AdaptiveQualityLevel::Reduced
            }
        } else if pressure_state == PressureState::Normal {
            AdaptiveQualityLevel::Normal
        } else {
            // Unknown/missing measurement: preserve existing safe settings without abrupt jumps
            current_level
        };

        if target_level == current_level {
            return AdaptivePolicyDecision {
                display_id: display_id.clone(),
                current_level,
                target_level,
                effective_fps: current_override.effective_fps,
                action: AdaptiveAction::NoChange,
                reason: "Operational quality already optimal".into(),
            };
        }

        // 5. Cooldown Guard: Prevent rapid oscillation
        if let Some(last_adj) = self.last_adjustment_time.get(display_id) {
            let elapsed = now_millis.saturating_sub(*last_adj);
            if elapsed < self.settings.adjustment_cooldown_ms {
                return AdaptivePolicyDecision {
                    display_id: display_id.clone(),
                    current_level,
                    target_level,
                    effective_fps: current_override.effective_fps,
                    action: AdaptiveAction::HoldCooldown,
                    reason: format!(
                        "Adjustment cooldown active ({}ms remaining)",
                        self.settings.adjustment_cooldown_ms - elapsed
                    ),
                };
            }
        }

        // 6. Compute bounded effective FPS
        let effective_fps = match target_level {
            AdaptiveQualityLevel::Normal => canonical_fps,
            AdaptiveQualityLevel::Reduced => {
                let floor = self.settings.target_fps_floor.max(MIN_SAFE_FPS_FLOOR);
                Some(canonical_fps.unwrap_or(60).min(floor))
            }
            AdaptiveQualityLevel::MinimumEligible => {
                let floor = (self.settings.target_fps_floor / 2).max(MIN_SAFE_FPS_FLOOR);
                Some(canonical_fps.unwrap_or(60).min(floor))
            }
        };

        // Update effective override state
        current_override.level = target_level;
        current_override.effective_fps = effective_fps;
        current_override.applied_at_millis = now_millis;
        current_override.reason = format!("Transitioned to {:?}", target_level);
        self.last_adjustment_time.insert(display_id.clone(), now_millis);

        let action = if target_level > current_level {
            self.record_diagnostic(
                Some(display_id.clone()),
                AdaptiveDiagnosticEventType::AdjustmentApplied,
                format!(
                    "Applied adaptive downgrade for display `{}` to {:?} (effective FPS: {:?})",
                    display_id, target_level, effective_fps
                ),
            );
            AdaptiveAction::ApplyAdjustment
        } else {
            self.record_diagnostic(
                Some(display_id.clone()),
                AdaptiveDiagnosticEventType::SettingsRestored,
                format!(
                    "Restored quality settings for display `{}` to {:?} (effective FPS: {:?})",
                    display_id, target_level, effective_fps
                ),
            );
            AdaptiveAction::RestoreSettings
        };

        AdaptivePolicyDecision {
            display_id: display_id.clone(),
            current_level,
            target_level,
            effective_fps,
            action,
            reason: format!("Sustained pressure transition to {:?}", target_level),
        }
    }

    /// Evaluates multi-monitor layouts:
    /// - Independent: Each display evaluated on its own signals.
    /// - Clone: Secondary displays follow primary display's level.
    /// - Span: Shared virtual surface receives single unified level.
    pub fn evaluate_layout_group(
        &mut self,
        displays: &[DisplayId],
        layout_mode: MultiMonitorLayoutMode,
        capability: &AdaptiveBackendCapability,
        coordinator: &DisplaySessionCoordinator,
        active_pause_reasons: &HashMap<DisplayId, std::collections::HashSet<PauseReason>>,
        power_eval: Option<&PowerPolicyEvaluation>,
        now_millis: u64,
    ) -> HashMap<DisplayId, AdaptivePolicyDecision> {
        let mut results = HashMap::new();

        match layout_mode {
            MultiMonitorLayoutMode::Independent => {
                for did in displays {
                    let (state, pause_reasons) = {
                        let state = coordinator
                            .get_session(did)
                            .map(|s| s.state.clone())
                            .unwrap_or(DisplaySessionState::Uninitialized);
                        let reasons = active_pause_reasons.get(did).cloned().unwrap_or_default();
                        (state, reasons)
                    };
                    let dec = self.evaluate_session(
                        did,
                        capability,
                        &state,
                        &pause_reasons,
                        power_eval,
                        now_millis,
                    );
                    results.insert(did.clone(), dec);
                }
            }
            MultiMonitorLayoutMode::Clone => {
                if let Some(primary_id) = displays.first() {
                    let (state, pause_reasons) = {
                        let state = coordinator
                            .get_session(primary_id)
                            .map(|s| s.state.clone())
                            .unwrap_or(DisplaySessionState::Uninitialized);
                        let reasons = active_pause_reasons.get(primary_id).cloned().unwrap_or_default();
                        (state, reasons)
                    };
                    let primary_dec = self.evaluate_session(
                        primary_id,
                        capability,
                        &state,
                        &pause_reasons,
                        power_eval,
                        now_millis,
                    );
                    results.insert(primary_id.clone(), primary_dec.clone());

                    // Cloned displays follow primary decision synchronously
                    for secondary_id in &displays[1..] {
                        let mut cloned_dec = primary_dec.clone();
                        cloned_dec.display_id = secondary_id.clone();
                        results.insert(secondary_id.clone(), cloned_dec);
                    }
                }
            }
            MultiMonitorLayoutMode::Span => {
                // Spanned surface uses primary display evaluation across all participants
                if let Some(primary_id) = displays.first() {
                    let (state, pause_reasons) = {
                        let state = coordinator
                            .get_session(primary_id)
                            .map(|s| s.state.clone())
                            .unwrap_or(DisplaySessionState::Uninitialized);
                        let reasons = active_pause_reasons.get(primary_id).cloned().unwrap_or_default();
                        (state, reasons)
                    };
                    let span_dec = self.evaluate_session(
                        primary_id,
                        capability,
                        &state,
                        &pause_reasons,
                        power_eval,
                        now_millis,
                    );
                    for did in displays {
                        let mut dec = span_dec.clone();
                        dec.display_id = did.clone();
                        results.insert(did.clone(), dec);
                    }
                }
            }
        }

        results
    }

    /// Records a diagnostic event into the bounded ring log.
    pub fn record_diagnostic(
        &mut self,
        display_id: Option<DisplayId>,
        event_type: AdaptiveDiagnosticEventType,
        message: String,
    ) {
        if self.diagnostics.len() >= MAX_ADAPTIVE_DIAGNOSTICS {
            self.diagnostics.remove(0);
        }
        self.diagnostics.push(AdaptiveDiagnosticEvent {
            timestamp_millis: Self::current_timestamp_millis(),
            display_id,
            event_type,
            message,
        });
    }

    /// Returns diagnostic event history.
    #[must_use]
    pub fn get_diagnostics(&self) -> &[AdaptiveDiagnosticEvent] {
        &self.diagnostics
    }
}
