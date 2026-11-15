//! Unified player supervisor coordinating lifecycle, health monitoring, and crash recovery.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use kinetiwall_catalog::{WallpaperId, WallpaperType};
use tracing::{error, info, warn};

use crate::adapter::PlayerReport;
use crate::config::{ConfigUpdateResult, PlaybackConfig, RuntimeConfigUpdate};
use crate::diagnostics::{
    DiagnosticCategory, DiagnosticEvent, DiagnosticsAggregator, DiagnosticsSnapshot,
    LifecycleTimings,
};
use crate::health::{FailureClassification, HealthReport, PlayerHealthStatus, PlayerInstanceId};
use crate::lifecycle::{DesiredState, SessionState};
use crate::orchestrator::{PendingPhase, PlaybackOrchestrator, StartOutcome, StopOutcome};
use crate::recovery::{RecoveryBudget, RecoveryOutcome, RecoveryPolicy};
use crate::types::{
    DisplayTargetId, PlaybackError, PlaybackErrorKind, PlaybackRequest, PlaybackRequestId,
    PlaybackSessionId, WallpaperPlaybackLookup,
};

/// Optional lifecycle coordinator for audio-reactive wallpapers.
pub trait AudioCoordinator: Send {
    fn on_session_started(&mut self, wallpaper_id: WallpaperId, wallpaper_type: WallpaperType);
    fn on_session_stopped(&mut self);
    fn on_session_failed(&mut self);
}

/// Represents the supervised state of a display target and its active player instance.
#[derive(Debug)]
pub struct SupervisedTarget {
    pub instance_id: PlayerInstanceId,
    pub session_id: PlaybackSessionId,
    pub request_id: PlaybackRequestId,
    pub wallpaper_id: WallpaperId,
    pub wallpaper_type: WallpaperType,
    pub target: DisplayTargetId,
    pub config: Option<PlaybackConfig>,
    pub desired: DesiredState,
    pub observed: SessionState,
    pub health: PlayerHealthStatus,
    pub failure_reason: Option<FailureClassification>,
    pub started_at: Instant,
    pub last_health_confirm: Instant,
    pub recovery_budget: RecoveryBudget,
    pub is_recovering: bool,
    pub timings: LifecycleTimings,
    pub aggregator: DiagnosticsAggregator,
    pub pid: Option<u32>,
}

/// Unified Player Supervisor providing health monitoring, crash recovery, and shutdown coordination.
pub struct PlayerSupervisor {
    orchestrator: PlaybackOrchestrator,
    recovery_policy: RecoveryPolicy,
    audio_coordinator: Option<Box<dyn AudioCoordinator>>,
    targets: HashMap<DisplayTargetId, SupervisedTarget>,
    recent_snapshots: HashMap<DisplayTargetId, DiagnosticsSnapshot>,
    startup_timeout: Duration,
    shutdown_timeout: Duration,
    shutting_down: bool,
}

impl PlayerSupervisor {
    #[must_use]
    pub fn new(orchestrator: PlaybackOrchestrator) -> Self {
        Self {
            orchestrator,
            recovery_policy: RecoveryPolicy::default(),
            audio_coordinator: None,
            targets: HashMap::new(),
            recent_snapshots: HashMap::new(),
            startup_timeout: Duration::from_secs(10),
            shutdown_timeout: Duration::from_secs(5),
            shutting_down: false,
        }
    }

    /// Attaches an audio-reactive coordinator hook.
    pub fn set_audio_coordinator(&mut self, coordinator: Box<dyn AudioCoordinator>) {
        self.audio_coordinator = Some(coordinator);
    }

    /// Sets custom crash recovery policy.
    pub fn set_recovery_policy(&mut self, policy: RecoveryPolicy) {
        self.recovery_policy = policy;
    }

    /// Sets custom startup timeout.
    pub fn set_startup_timeout(&mut self, timeout: Duration) {
        self.startup_timeout = timeout;
    }

    /// Sets custom shutdown timeout.
    pub fn set_shutdown_timeout(&mut self, timeout: Duration) {
        self.shutdown_timeout = timeout;
    }

    #[must_use]
    pub fn is_shutting_down(&self) -> bool {
        self.shutting_down
    }

    #[must_use]
    pub fn target_status(&self, target: DisplayTargetId) -> Option<&SupervisedTarget> {
        self.targets.get(&target)
    }

    #[must_use]
    pub fn all_targets(&self) -> Vec<&SupervisedTarget> {
        self.targets.values().collect()
    }

    /// Mutable access to the inner orchestrator (e.g. for registering adapters).
    pub fn orchestrator_mut(&mut self) -> &mut PlaybackOrchestrator {
        &mut self.orchestrator
    }

    /// Starts wallpaper playback on a target under supervision.
    pub fn play_wallpaper(
        &mut self,
        lookup: &dyn WallpaperPlaybackLookup,
        request: PlaybackRequest,
    ) -> Result<PlayerInstanceId, PlaybackError> {
        if self.shutting_down {
            return Err(PlaybackError::new(
                PlaybackErrorKind::ShuttingDown,
                "Supervisor is shutting down",
            ));
        }

        let target_id = request.target;
        let wallpaper_id = request.wallpaper_id;
        let wallpaper_type = request.wallpaper_type;
        let request_id = request.request_id;
        let config = request.config.clone().or_else(|| match wallpaper_type {
            WallpaperType::Video => Some(PlaybackConfig::video_default()),
            WallpaperType::Web => Some(PlaybackConfig::web_default()),
        });

        // If target already has an active session, stop it first cleanly
        if self.targets.contains_key(&target_id) {
            let _ = self.stop_target(target_id);
        }
        self.recent_snapshots.remove(&target_id);

        let outcome = self.orchestrator.start_playback(lookup, request);
        match outcome {
            StartOutcome::Accepted { session_id, state } => {
                let instance_id = PlayerInstanceId::new();
                let now = Instant::now();
                let mut timings = LifecycleTimings::new(now);
                let mut aggregator = DiagnosticsAggregator::new();
                let pid = self.orchestrator.process_id(session_id);

                aggregator.record_event(DiagnosticEvent::new(
                    instance_id,
                    session_id,
                    target_id,
                    wallpaper_id,
                    wallpaper_type,
                    DiagnosticCategory::StartupRequested,
                    0,
                ));
                aggregator.record_event(DiagnosticEvent::new(
                    instance_id,
                    session_id,
                    target_id,
                    wallpaper_id,
                    wallpaper_type,
                    DiagnosticCategory::BackendInitStarted,
                    0,
                ));
                timings.record_init_started(now);

                if state == SessionState::Ready {
                    timings.record_ready(now);
                    let ready_dur = timings.time_to_ready().map(|d| d.as_millis() as u64);
                    let mut ev = DiagnosticEvent::new(
                        instance_id,
                        session_id,
                        target_id,
                        wallpaper_id,
                        wallpaper_type,
                        DiagnosticCategory::BackendReady,
                        0,
                    );
                    if let Some(d) = ready_dur {
                        ev = ev.with_duration_ms(d);
                    }
                    aggregator.record_event(ev);
                } else if state == SessionState::Playing {
                    timings.record_ready(now);
                    timings.record_playing(now);
                    let play_dur = timings.time_to_playing().map(|d| d.as_millis() as u64);
                    let mut ev = DiagnosticEvent::new(
                        instance_id,
                        session_id,
                        target_id,
                        wallpaper_id,
                        wallpaper_type,
                        DiagnosticCategory::PlaybackStarted,
                        0,
                    );
                    if let Some(d) = play_dur {
                        ev = ev.with_duration_ms(d);
                    }
                    aggregator.record_event(ev);
                }

                let supervised = SupervisedTarget {
                    instance_id,
                    session_id,
                    request_id,
                    wallpaper_id,
                    wallpaper_type,
                    target: target_id,
                    config,
                    desired: DesiredState::Playing,
                    observed: state,
                    health: PlayerHealthStatus::Healthy,
                    failure_reason: None,
                    started_at: now,
                    last_health_confirm: now,
                    recovery_budget: RecoveryBudget::new(),
                    is_recovering: false,
                    timings,
                    aggregator,
                    pid,
                };

                self.targets.insert(target_id, supervised);

                if let Some(coord) = &mut self.audio_coordinator {
                    coord.on_session_started(wallpaper_id, wallpaper_type);
                }

                info!(
                    %instance_id,
                    %session_id,
                    target = target_id.0,
                    "Supervised player started"
                );
                Ok(instance_id)
            }
            StartOutcome::AlreadyActive { session_id, .. }
            | StartOutcome::Duplicate { session_id, .. } => {
                if let Some(target) = self.targets.get(&target_id) {
                    if target.session_id == session_id {
                        return Ok(target.instance_id);
                    }
                }
                let instance_id = PlayerInstanceId::new();
                Ok(instance_id)
            }
            StartOutcome::Rejected { error } | StartOutcome::Failed { error, .. } => Err(error),
        }
    }

    /// Stops playback on a target, releasing player resources and cancelling recovery.
    pub fn stop_target(&mut self, target: DisplayTargetId) -> Result<StopOutcome, PlaybackError> {
        let Some(mut supervised) = self.targets.remove(&target) else {
            return Ok(StopOutcome::AlreadyStopped);
        };

        let now = Instant::now();
        supervised.desired = DesiredState::Stopped;
        supervised.is_recovering = false;
        supervised.timings.record_stop_requested(now);

        if let Some(coord) = &mut self.audio_coordinator {
            coord.on_session_stopped();
        }

        let outcome = self.orchestrator.stop_session(supervised.session_id);
        supervised.timings.record_stopped(Instant::now());
        let stop_dur = supervised
            .timings
            .stop_duration()
            .map(|d| d.as_millis() as u64);

        let mut ev = DiagnosticEvent::new(
            supervised.instance_id,
            supervised.session_id,
            target,
            supervised.wallpaper_id,
            supervised.wallpaper_type,
            DiagnosticCategory::PlaybackStopped,
            0,
        );
        if let Some(d) = stop_dur {
            ev = ev.with_duration_ms(d);
        }
        supervised.aggregator.record_event(ev);

        let final_snapshot = DiagnosticsSnapshot {
            instance_id: supervised.instance_id,
            session_id: supervised.session_id,
            target_id: target,
            wallpaper_id: supervised.wallpaper_id,
            wallpaper_type: supervised.wallpaper_type,
            health_status: supervised.health.clone(),
            timings: supervised.timings.summary(Instant::now()),
            latest_sample: supervised.aggregator.latest_sample(),
            resource_summary: supervised.aggregator.summarize_resources(),
            backend_metrics: supervised.aggregator.backend_metrics().clone(),
            recovery_attempts: supervised.recovery_budget.attempts(),
            recent_events: supervised.aggregator.events(),
        };
        self.recent_snapshots.insert(target, final_snapshot);

        info!(
            instance_id = %supervised.instance_id,
            target = target.0,
            outcome = ?outcome,
            "Supervised player stopped"
        );
        Ok(outcome)
    }

    /// Periodic supervision tick:
    /// 1. Polls asynchronous backend reports and updates observed states.
    /// 2. Checks process liveness and startup deadlines.
    /// 3. Executes bounded automatic crash recovery for eligible dead instances.
    /// 4. Emits health reports.
    pub fn poll_and_supervise(
        &mut self,
        lookup: &dyn WallpaperPlaybackLookup,
    ) -> Vec<HealthReport> {
        let now = Instant::now();

        // 1. Ingest asynchronous reports from all adapters
        let reports = self.orchestrator.poll_adapter_events();
        for (session_id, report) in reports {
            // Find target matching session_id
            if let Some((_, target)) = self
                .targets
                .iter_mut()
                .find(|(_, t)| t.session_id == session_id)
            {
                let outcome = self.orchestrator.handle_event(session_id, report.clone());
                match report {
                    PlayerReport::Ready => {
                        target.timings.record_ready(now);
                        let dur = target.timings.time_to_ready().map(|d| d.as_millis() as u64);
                        let mut ev = DiagnosticEvent::new(
                            target.instance_id,
                            target.session_id,
                            target.target,
                            target.wallpaper_id,
                            target.wallpaper_type,
                            DiagnosticCategory::BackendReady,
                            0,
                        );
                        if let Some(d) = dur {
                            ev = ev.with_duration_ms(d);
                        }
                        target.aggregator.record_event(ev);

                        target.observed = match outcome {
                            crate::orchestrator::EventOutcome::Applied { state } => state,
                            _ => target.observed,
                        };
                        target.health = PlayerHealthStatus::Healthy;
                        target.last_health_confirm = now;
                    }
                    PlayerReport::Playing => {
                        target.timings.record_ready(now);
                        target.timings.record_playing(now);
                        let dur = target
                            .timings
                            .time_to_playing()
                            .map(|d| d.as_millis() as u64);
                        let mut ev = DiagnosticEvent::new(
                            target.instance_id,
                            target.session_id,
                            target.target,
                            target.wallpaper_id,
                            target.wallpaper_type,
                            DiagnosticCategory::PlaybackStarted,
                            0,
                        );
                        if let Some(d) = dur {
                            ev = ev.with_duration_ms(d);
                        }
                        target.aggregator.record_event(ev);

                        target.observed = match outcome {
                            crate::orchestrator::EventOutcome::Applied { state } => state,
                            _ => target.observed,
                        };
                        target.health = PlayerHealthStatus::Healthy;
                        target.last_health_confirm = now;
                    }
                    PlayerReport::Stopped => {
                        target.timings.record_stopped(now);
                        target.aggregator.record_event(DiagnosticEvent::new(
                            target.instance_id,
                            target.session_id,
                            target.target,
                            target.wallpaper_id,
                            target.wallpaper_type,
                            DiagnosticCategory::PlaybackStopped,
                            0,
                        ));
                        target.observed = SessionState::Stopped;
                        target.health = PlayerHealthStatus::Healthy;
                    }
                    PlayerReport::Failed { kind } => {
                        target.timings.record_failed(now);
                        target.aggregator.record_event(
                            DiagnosticEvent::new(
                                target.instance_id,
                                target.session_id,
                                target.target,
                                target.wallpaper_id,
                                target.wallpaper_type,
                                DiagnosticCategory::BackendReportedError,
                                0,
                            )
                            .with_detail(kind.user_message()),
                        );
                        let failure = FailureClassification::BackendFailure { error_kind: kind };
                        target.observed = SessionState::Failed(kind);
                        target.health = PlayerHealthStatus::Dead {
                            reason: failure.clone(),
                        };
                        target.failure_reason = Some(failure);
                        if let Some(coord) = &mut self.audio_coordinator {
                            coord.on_session_failed();
                        }
                    }
                }
            }
        }

        // 2. Liveness and startup deadline checks
        let target_ids: Vec<DisplayTargetId> = self.targets.keys().copied().collect();
        for tid in &target_ids {
            if let Some(target) = self.targets.get_mut(tid) {
                if target.desired != DesiredState::Playing || target.is_recovering {
                    continue;
                }

                // Check startup timeout
                if (target.observed == SessionState::Starting
                    || target.observed == SessionState::Preparing)
                    && target.started_at.elapsed() > self.startup_timeout
                {
                    warn!(
                        instance_id = %target.instance_id,
                        target = tid.0,
                        "Startup timeout expired"
                    );
                    let _ = self
                        .orchestrator
                        .handle_timeout(target.session_id, PendingPhase::Start);
                    let failure = FailureClassification::StartupTimeout;
                    target.timings.record_failed(now);
                    target.aggregator.record_event(DiagnosticEvent::new(
                        target.instance_id,
                        target.session_id,
                        *tid,
                        target.wallpaper_id,
                        target.wallpaper_type,
                        DiagnosticCategory::InitTimedOut,
                        0,
                    ));
                    target.observed = SessionState::Failed(PlaybackErrorKind::Timeout);
                    target.health = PlayerHealthStatus::Dead {
                        reason: failure.clone(),
                    };
                    target.failure_reason = Some(failure);
                    if let Some(coord) = &mut self.audio_coordinator {
                        coord.on_session_failed();
                    }
                    continue;
                }

                // Check process liveness
                if target.observed.is_live()
                    && !self.orchestrator.check_session_liveness(target.session_id)
                {
                    warn!(
                        instance_id = %target.instance_id,
                        target = tid.0,
                        "Process unexpectedly terminated"
                    );
                    let _ = self.orchestrator.handle_event(
                        target.session_id,
                        PlayerReport::Failed {
                            kind: PlaybackErrorKind::BackendExited,
                        },
                    );
                    let failure = FailureClassification::ProcessTerminated { exit_code: None };
                    target.timings.record_failed(now);
                    target.aggregator.record_event(DiagnosticEvent::new(
                        target.instance_id,
                        target.session_id,
                        *tid,
                        target.wallpaper_id,
                        target.wallpaper_type,
                        DiagnosticCategory::ProcessExitedUnexpectedly,
                        0,
                    ));
                    target.observed = SessionState::Failed(PlaybackErrorKind::BackendExited);
                    target.health = PlayerHealthStatus::Dead {
                        reason: failure.clone(),
                    };
                    target.failure_reason = Some(failure);
                    if let Some(coord) = &mut self.audio_coordinator {
                        coord.on_session_failed();
                    }
                }
            }
        }

        // Sample resource metrics and backend performance for active targets
        for tid in &target_ids {
            if let Some(target) = self.targets.get_mut(tid) {
                if target.observed.is_live() {
                    if target.pid.is_none() {
                        target.pid = self.orchestrator.process_id(target.session_id);
                    }
                    if let Some(sample) = self.orchestrator.sample_metrics(target.session_id) {
                        target.aggregator.record_sample(sample);
                    }
                    let bm = self.orchestrator.backend_metrics(target.session_id);
                    target.aggregator.update_backend_metrics(bm);
                }
            }
        }

        // 3. Auto-recovery for eligible dead targets
        let dead_targets: Vec<DisplayTargetId> = self
            .targets
            .iter()
            .filter(|(_, t)| {
                t.desired == DesiredState::Playing
                    && t.health.is_dead()
                    && !t.is_recovering
                    && !self.shutting_down
            })
            .map(|(tid, _)| *tid)
            .collect();

        for tid in dead_targets {
            let outcome = self.execute_recovery(tid, lookup);
            info!(target = tid.0, ?outcome, "Automatic recovery evaluation");
        }

        // 4. Generate health reports
        self.targets
            .values()
            .map(|t| HealthReport {
                instance_id: t.instance_id,
                session_id: t.session_id,
                status: t.health.clone(),
                failure: t.failure_reason.clone(),
            })
            .collect()
    }

    /// Evaluates eligibility and performs crash recovery for a target.
    pub fn execute_recovery(
        &mut self,
        target: DisplayTargetId,
        lookup: &dyn WallpaperPlaybackLookup,
    ) -> RecoveryOutcome {
        if self.shutting_down {
            return RecoveryOutcome::IneligibleShuttingDown;
        }

        let Some(t) = self.targets.get_mut(&target) else {
            return RecoveryOutcome::IneligibleNotDesired;
        };

        if t.desired != DesiredState::Playing {
            return RecoveryOutcome::IneligibleNotDesired;
        }

        let failure = t
            .failure_reason
            .clone()
            .unwrap_or(FailureClassification::Unrecoverable {
                detail: "Unknown failure".into(),
            });

        if !failure.is_retryable() {
            return RecoveryOutcome::IneligibleNonRetryable { reason: failure };
        }

        let now = Instant::now();
        if !t.recovery_budget.can_attempt(&self.recovery_policy, now) {
            warn!(
                target = target.0,
                attempts = t.recovery_budget.attempts(),
                "Recovery budget exhausted; circuit breaker open"
            );
            t.aggregator.record_event(DiagnosticEvent::new(
                t.instance_id,
                t.session_id,
                target,
                t.wallpaper_id,
                t.wallpaper_type,
                DiagnosticCategory::RecoveryExhausted,
                0,
            ));
            return RecoveryOutcome::BudgetExhausted {
                attempts: t.recovery_budget.attempts(),
            };
        }

        let _backoff = t.recovery_budget.record_attempt(&self.recovery_policy, now);
        t.is_recovering = true;

        // Cleanup dead session in orchestrator
        let old_session_id = t.session_id;
        let wallpaper_id = t.wallpaper_id;
        let wallpaper_type = t.wallpaper_type;
        let _ = self.orchestrator.stop_session(old_session_id);

        // Generate fresh generation token & request preserving previously effective config
        let new_instance_id = PlayerInstanceId::new();
        t.aggregator.record_event(
            DiagnosticEvent::new(
                new_instance_id,
                old_session_id,
                target,
                wallpaper_id,
                wallpaper_type,
                DiagnosticCategory::RecoveryAttemptStarted,
                0,
            )
            .with_detail(format!("attempt {}", t.recovery_budget.attempts())),
        );

        let new_request = PlaybackRequest {
            request_id: PlaybackRequestId::new(),
            wallpaper_id,
            wallpaper_type,
            target,
            config: t.config.clone(),
        };

        match self.orchestrator.start_playback(lookup, new_request) {
            StartOutcome::Accepted { session_id, state } => {
                if let Some(supervised) = self.targets.get_mut(&target) {
                    supervised.instance_id = new_instance_id;
                    supervised.session_id = session_id;
                    supervised.observed = state;
                    supervised.health = PlayerHealthStatus::Healthy;
                    supervised.failure_reason = None;
                    supervised.started_at = Instant::now();
                    supervised.last_health_confirm = Instant::now();
                    supervised.is_recovering = false;
                    supervised.timings = LifecycleTimings::new(Instant::now());
                    supervised.pid = self.orchestrator.process_id(session_id);
                    supervised.aggregator.record_event(DiagnosticEvent::new(
                        new_instance_id,
                        session_id,
                        target,
                        wallpaper_id,
                        wallpaper_type,
                        DiagnosticCategory::RecoveryCompleted,
                        0,
                    ));
                }

                if let Some(coord) = &mut self.audio_coordinator {
                    coord.on_session_started(wallpaper_id, wallpaper_type);
                }

                info!(
                    %new_instance_id,
                    %session_id,
                    target = target.0,
                    "Player successfully recovered"
                );
                RecoveryOutcome::Recovered {
                    new_instance_id,
                    new_session_id: session_id,
                }
            }
            StartOutcome::Failed { error, .. } | StartOutcome::Rejected { error } => {
                if let Some(supervised) = self.targets.get_mut(&target) {
                    supervised.is_recovering = false;
                }
                error!(target = target.0, ?error, "Recovery start failed");
                RecoveryOutcome::RecoveryFailed { error }
            }
            _ => {
                if let Some(supervised) = self.targets.get_mut(&target) {
                    supervised.is_recovering = false;
                }
                RecoveryOutcome::RecoveryFailed {
                    error: PlaybackError::new(
                        PlaybackErrorKind::Internal,
                        "Unexpected start outcome during recovery",
                    ),
                }
            }
        }
    }

    /// Applies dynamic runtime configuration updates to a target's player instance.
    ///
    /// Validates instance identity (`instance_id`), ensuring that stale updates intended for an
    /// earlier or replaced instance do not mutate the newly recovered replacement instance.
    pub fn update_runtime_config(
        &mut self,
        target_id: DisplayTargetId,
        instance_id: PlayerInstanceId,
        update: RuntimeConfigUpdate,
    ) -> ConfigUpdateResult {
        let Some(target) = self.targets.get_mut(&target_id) else {
            return ConfigUpdateResult::TargetNotFound;
        };
        if target.instance_id != instance_id {
            return ConfigUpdateResult::StaleInstance;
        }
        if target.desired != DesiredState::Playing {
            return ConfigUpdateResult::TargetNotActive;
        }

        let session_id = target.session_id;
        match self.orchestrator.apply_runtime_config(session_id, &update) {
            Ok(res @ ConfigUpdateResult::Applied { .. }) => {
                // If applied, update the target's stored config so future crash recovery preserves it!
                let cfg = target
                    .config
                    .get_or_insert_with(|| match target.wallpaper_type {
                        WallpaperType::Video => PlaybackConfig::video_default(),
                        WallpaperType::Web => PlaybackConfig::web_default(),
                    });
                if let Some(vol) = update.volume {
                    let (clamped, _) = crate::config::NormalizedVolume::clamped(vol);
                    cfg.common.volume = clamped;
                }
                if let Some(muted) = update.muted {
                    cfg.common.muted = muted;
                }
                if let Some(paused) = update.paused {
                    cfg.common.start_mode = if paused {
                        crate::config::StartMode::Paused
                    } else {
                        crate::config::StartMode::AutoPlay
                    };
                }
                res
            }
            Ok(other) => other,
            Err(e) => ConfigUpdateResult::BackendFailed {
                error: e.diagnostic,
            },
        }
    }

    /// Returns an authoritative diagnostics snapshot for the specified display target.
    pub fn diagnostics_snapshot(&self, target_id: DisplayTargetId) -> Option<DiagnosticsSnapshot> {
        if let Some(target) = self.targets.get(&target_id) {
            let now = Instant::now();
            Some(DiagnosticsSnapshot {
                instance_id: target.instance_id,
                session_id: target.session_id,
                target_id: target.target,
                wallpaper_id: target.wallpaper_id,
                wallpaper_type: target.wallpaper_type,
                health_status: target.health.clone(),
                timings: target.timings.summary(now),
                latest_sample: target.aggregator.latest_sample(),
                resource_summary: target.aggregator.summarize_resources(),
                backend_metrics: target.aggregator.backend_metrics().clone(),
                recovery_attempts: target.recovery_budget.attempts(),
                recent_events: target.aggregator.events(),
            })
        } else {
            self.recent_snapshots.get(&target_id).cloned()
        }
    }

    /// Exports a privacy-safe, redacted, formatted JSON diagnostic report for a target.
    pub fn export_diagnostics_json(
        &self,
        target_id: DisplayTargetId,
    ) -> Result<String, PlaybackError> {
        let snapshot = self.diagnostics_snapshot(target_id).ok_or_else(|| {
            PlaybackError::new(PlaybackErrorKind::InvalidRequest, "Target not found")
        })?;
        serde_json::to_string_pretty(&snapshot).map_err(|e| {
            PlaybackError::new(
                PlaybackErrorKind::Internal,
                format!("Serialization failure: {e}"),
            )
        })
    }

    /// Clean, deterministic, idempotent shutdown.
    pub fn shutdown(&mut self) -> Result<(), PlaybackError> {
        if self.shutting_down {
            return Ok(());
        }
        self.shutting_down = true;

        if let Some(coord) = &mut self.audio_coordinator {
            coord.on_session_stopped();
        }

        // Cancel recovery on all targets and record shutdown events
        for t in self.targets.values_mut() {
            t.desired = DesiredState::Stopped;
            t.is_recovering = false;
            t.aggregator.record_event(DiagnosticEvent::new(
                t.instance_id,
                t.session_id,
                t.target,
                t.wallpaper_id,
                t.wallpaper_type,
                DiagnosticCategory::ShutdownCompleted,
                0,
            ));
        }

        let report = self.orchestrator.shutdown();
        self.targets.clear();

        info!(
            released = report.sessions_released,
            failures = report.session_failures.len(),
            "Supervisor shutdown completed successfully"
        );
        Ok(())
    }
}

impl Drop for PlayerSupervisor {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
