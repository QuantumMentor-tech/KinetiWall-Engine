//! Playback orchestration boundary.
//!
//! Single-owner, `&mut self` API: all state mutation is serialized by Rust ownership. A future
//! controller must run it on one actor thread/task fed by channels (or wrap it in a `Mutex` that is
//! never held across anything but these bounded calls). Nothing here decodes, renders, spawns
//! processes, or touches the desktop.

use std::collections::HashMap;

use kinetiwall_catalog::{AvailabilityState, WallpaperType};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::adapter::{
    CancelAck, PlayerAdapter, PlayerReport, PrepareProgress, SessionContext, StopAck,
};
use crate::config::{
    ConfigUpdateResult, EffectivePlaybackConfig, PlaybackConfig, RuntimeConfigUpdate,
};
use crate::diagnostics::{BackendPerformanceMetrics, ProcessResourceSample};
use crate::lifecycle::{DesiredState, SessionSnapshot, SessionState};
use crate::negotiation::Negotiator;
use crate::types::{
    AllowAllPolicy, PlaybackError, PlaybackErrorKind, PlaybackPolicy, PlaybackRequest,
    PlaybackSessionId, PlayerAvailability, WallpaperPlaybackLookup,
};

/// Result of `start_playback`. `Accepted` is never a claim that playback is visible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum StartOutcome {
    /// Session created and the adapter accepted work; `state` is `Preparing` or `Starting`.
    Accepted {
        session_id: PlaybackSessionId,
        state: SessionState,
    },
    /// Same `request_id` seen before: returns the existing session, launches nothing.
    Duplicate {
        session_id: PlaybackSessionId,
        state: SessionState,
    },
    /// Same wallpaper already live on the target: returns it, launches nothing.
    AlreadyActive {
        session_id: PlaybackSessionId,
        state: SessionState,
    },
    /// Rejected before any session or adapter work existed.
    Rejected { error: PlaybackError },
    /// A session was created but adapter work failed; the session is `Failed`.
    Failed {
        session_id: PlaybackSessionId,
        error: PlaybackError,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum StopOutcome {
    Stopped,
    AlreadyStopped,
    /// Stop requested during preparation; adapter accepted cancellation.
    Canceled,
    /// Cleanup of a failed session completed; the session stays `Failed`.
    CleanedUpFailedSession,
    Failed {
        error: PlaybackError,
    },
    UnknownSession,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum CancelOutcome {
    /// Adapter accepted cancellation; session is `Canceled` and late events are ignored.
    Canceled,
    /// Adapter cannot cancel; the orchestrator performed an ordinary stop instead.
    FellBackToStop {
        stop: Box<StopOutcome>,
    },
    /// State does not permit cancellation (e.g. already `Playing` or terminal).
    NotCancellable {
        state: SessionState,
    },
    Failed {
        error: PlaybackError,
    },
    UnknownSession,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum EventOutcome {
    Applied {
        state: SessionState,
    },
    IgnoredUnknownSession,
    IgnoredTerminalSession {
        state: SessionState,
    },
    IgnoredInvalidTransition {
        state: SessionState,
        report: PlayerReport,
    },
}

/// Which pending confirmation timed out (timer ownership belongs to the future supervisor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingPhase {
    Prepare,
    Start,
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ShutdownReport {
    pub already_shut_down: bool,
    pub sessions_released: usize,
    pub session_failures: Vec<(PlaybackSessionId, PlaybackErrorKind)>,
    pub adapter_failures: Vec<(WallpaperType, PlaybackErrorKind)>,
}

struct Session {
    snapshot: SessionSnapshot,
    effective_config: Option<EffectivePlaybackConfig>,
}

pub struct PlaybackOrchestrator {
    adapters: Vec<Box<dyn PlayerAdapter>>,
    policy: Box<dyn PlaybackPolicy>,
    sessions: HashMap<PlaybackSessionId, Session>,
    by_request: HashMap<crate::types::PlaybackRequestId, PlaybackSessionId>,
    by_target: HashMap<crate::types::DisplayTargetId, PlaybackSessionId>,
    shut_down: bool,
}

impl Default for PlaybackOrchestrator {
    fn default() -> Self {
        Self::new()
    }
}

impl PlaybackOrchestrator {
    #[must_use]
    pub fn new() -> Self {
        Self::with_policy(Box::new(AllowAllPolicy))
    }

    #[must_use]
    pub fn with_policy(policy: Box<dyn PlaybackPolicy>) -> Self {
        Self {
            adapters: Vec::new(),
            policy,
            sessions: HashMap::new(),
            by_request: HashMap::new(),
            by_target: HashMap::new(),
            shut_down: false,
        }
    }

    /// Registers the single adapter for its declared wallpaper type. No silent fallback between types.
    pub fn register_adapter(
        &mut self,
        adapter: Box<dyn PlayerAdapter>,
    ) -> Result<(), PlaybackError> {
        let ty = adapter.capabilities().wallpaper_type;
        if self
            .adapters
            .iter()
            .any(|a| a.capabilities().wallpaper_type == ty)
        {
            return Err(PlaybackError::new(
                PlaybackErrorKind::InvalidRequest,
                format!("adapter already registered for {ty:?}"),
            ));
        }
        self.adapters.push(adapter);
        Ok(())
    }

    #[must_use]
    pub fn session(&self, id: PlaybackSessionId) -> Option<SessionSnapshot> {
        self.sessions.get(&id).map(|s| s.snapshot.clone())
    }

    #[must_use]
    pub fn sessions(&self) -> Vec<SessionSnapshot> {
        self.sessions.values().map(|s| s.snapshot.clone()).collect()
    }

    /// Drops finished sessions with no pending cleanup. Their request IDs lose idempotency.
    pub fn prune_finished(&mut self) -> usize {
        let ids: Vec<_> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.snapshot.observed.is_terminal() && !s.snapshot.cleanup_pending)
            .map(|(id, _)| *id)
            .collect();
        for id in &ids {
            self.sessions.remove(id);
            self.by_request.retain(|_, s| s != id);
        }
        ids.len()
    }

    pub fn start_playback(
        &mut self,
        lookup: &dyn WallpaperPlaybackLookup,
        request: PlaybackRequest,
    ) -> StartOutcome {
        let reject = |kind, diag: &str| StartOutcome::Rejected {
            error: PlaybackError::new(kind, diag),
        };

        if self.shut_down {
            return reject(PlaybackErrorKind::ShuttingDown, "orchestrator shut down");
        }
        if request.request_id.is_nil() {
            return reject(PlaybackErrorKind::InvalidRequest, "nil request id");
        }
        if let Some(sid) = self.by_request.get(&request.request_id).copied() {
            let state = self.sessions[&sid].snapshot.observed;
            return StartOutcome::Duplicate {
                session_id: sid,
                state,
            };
        }
        let Some(info) = lookup.playback_info(request.wallpaper_id) else {
            return reject(PlaybackErrorKind::InvalidRequest, "unknown wallpaper id");
        };
        if info.wallpaper_type != request.wallpaper_type {
            return reject(
                PlaybackErrorKind::InvalidRequest,
                "wallpaper type does not match catalog",
            );
        }
        if let Some(sid) = self.by_target.get(&request.target).copied() {
            let snap = &self.sessions[&sid].snapshot;
            if snap.wallpaper_id == request.wallpaper_id && snap.observed.is_live() {
                return StartOutcome::AlreadyActive {
                    session_id: sid,
                    state: snap.observed,
                };
            }
            return reject(
                PlaybackErrorKind::TargetBusy,
                "target has an unreleased session",
            );
        }
        let Some(idx) = self
            .adapters
            .iter()
            .position(|a| a.capabilities().wallpaper_type == request.wallpaper_type)
        else {
            return reject(
                PlaybackErrorKind::UnsupportedCapability,
                "no adapter registered for type",
            );
        };
        if !self.adapters[idx].capabilities().reports_observed_state {
            return reject(
                PlaybackErrorKind::UnsupportedCapability,
                "adapter cannot report observed state",
            );
        }
        if let PlayerAvailability::Unavailable { reason } = self.adapters[idx].availability() {
            return reject(PlaybackErrorKind::BackendUnavailable, &reason);
        }
        if info.availability != AvailabilityState::Available {
            return reject(
                PlaybackErrorKind::AssetUnavailable,
                "catalog availability is not Available",
            );
        }
        if let Err(reason) = self.policy.authorize(&request) {
            return reject(PlaybackErrorKind::PolicyDenied, &reason);
        }

        let requested_config =
            request
                .config
                .clone()
                .unwrap_or_else(|| match request.wallpaper_type {
                    WallpaperType::Video => PlaybackConfig::video_default(),
                    WallpaperType::Web => PlaybackConfig::web_default(),
                });
        let backend_caps = self.adapters[idx].backend_capabilities();
        let negotiation_report = match Negotiator::negotiate(&requested_config, &backend_caps) {
            Ok(rep) => rep,
            Err(err) => {
                return StartOutcome::Rejected { error: err };
            }
        };
        let effective_config = negotiation_report.effective_config;

        let session_id = PlaybackSessionId::new();
        self.sessions.insert(
            session_id,
            Session {
                snapshot: SessionSnapshot {
                    session_id,
                    request_id: request.request_id,
                    wallpaper_id: request.wallpaper_id,
                    wallpaper_type: request.wallpaper_type,
                    target: request.target,
                    desired: DesiredState::Playing,
                    observed: SessionState::Preparing,
                    outcome_uncertain: false,
                    cleanup_pending: false,
                },
                effective_config: Some(effective_config),
            },
        );
        self.by_request.insert(request.request_id, session_id);
        self.by_target.insert(request.target, session_id);
        info!(%session_id, request_id = %request.request_id, ?request.wallpaper_type, "playback requested; session created");

        let ctx = self.ctx(session_id);
        match self.adapters[idx].prepare(&ctx) {
            Err(error) => {
                // Contract: failed prepare leaves no adapter resources.
                self.fail(session_id, error.kind, false);
                self.release_target(session_id);
                warn!(%session_id, kind = ?error.kind, diag = %error.diagnostic, "prepare failed");
                StartOutcome::Failed { session_id, error }
            }
            Ok(PrepareProgress::Pending) => StartOutcome::Accepted {
                session_id,
                state: SessionState::Preparing,
            },
            Ok(PrepareProgress::Ready) => {
                self.set_state(session_id, SessionState::Ready);
                match self.begin_start(session_id) {
                    Ok(state) => StartOutcome::Accepted { session_id, state },
                    Err(error) => StartOutcome::Failed { session_id, error },
                }
            }
        }
    }

    /// Applies an untrusted backend report after validating session identity and transition.
    pub fn handle_event(
        &mut self,
        session_id: PlaybackSessionId,
        report: PlayerReport,
    ) -> EventOutcome {
        let Some(state) = self.sessions.get(&session_id).map(|s| s.snapshot.observed) else {
            warn!(%session_id, "ignored report for unknown/obsolete session");
            return EventOutcome::IgnoredUnknownSession;
        };
        if state.is_terminal() {
            return EventOutcome::IgnoredTerminalSession { state };
        }
        let invalid = |state| EventOutcome::IgnoredInvalidTransition {
            state,
            report: report.clone(),
        };
        match &report {
            PlayerReport::Ready if state == SessionState::Preparing => {
                self.set_state(session_id, SessionState::Ready);
                match self.begin_start(session_id) {
                    Ok(s) => EventOutcome::Applied { state: s },
                    Err(_) => EventOutcome::Applied {
                        state: self.state_of(session_id),
                    },
                }
            }
            PlayerReport::Playing if state == SessionState::Starting => {
                self.set_state(session_id, SessionState::Playing);
                info!(%session_id, "backend confirmed playing");
                EventOutcome::Applied {
                    state: SessionState::Playing,
                }
            }
            PlayerReport::Stopped if state == SessionState::Stopping => {
                self.set_state(session_id, SessionState::Stopped);
                self.release_target(session_id);
                EventOutcome::Applied {
                    state: SessionState::Stopped,
                }
            }
            PlayerReport::Stopped => {
                // Unsolicited stop of a live session: backend exited, not a clean stop.
                self.fail(session_id, PlaybackErrorKind::BackendExited, true);
                EventOutcome::Applied {
                    state: self.state_of(session_id),
                }
            }
            PlayerReport::Failed { kind } => {
                self.fail(session_id, *kind, true);
                EventOutcome::Applied {
                    state: self.state_of(session_id),
                }
            }
            _ => {
                warn!(%session_id, ?state, ?report, "ignored invalid transition report");
                invalid(state)
            }
        }
    }

    /// Records that an expected confirmation did not arrive. Outcome is *uncertain*: the session
    /// becomes `Failed(Timeout)` with `outcome_uncertain`, and must be reconciled via `stop_session`
    /// rather than blindly retried.
    pub fn handle_timeout(
        &mut self,
        session_id: PlaybackSessionId,
        phase: PendingPhase,
    ) -> EventOutcome {
        let Some(state) = self.sessions.get(&session_id).map(|s| s.snapshot.observed) else {
            return EventOutcome::IgnoredUnknownSession;
        };
        let matches = matches!(
            (phase, state),
            (PendingPhase::Prepare, SessionState::Preparing)
                | (PendingPhase::Start, SessionState::Starting)
                | (PendingPhase::Stop, SessionState::Stopping)
        );
        if state.is_terminal() {
            return EventOutcome::IgnoredTerminalSession { state };
        }
        if !matches {
            return EventOutcome::IgnoredInvalidTransition {
                state,
                report: PlayerReport::Failed {
                    kind: PlaybackErrorKind::Timeout,
                },
            };
        }
        self.fail(session_id, PlaybackErrorKind::Timeout, true);
        if let Some(s) = self.sessions.get_mut(&session_id) {
            s.snapshot.outcome_uncertain = true;
        }
        warn!(%session_id, ?phase, "confirmation timed out; outcome uncertain");
        EventOutcome::Applied {
            state: self.state_of(session_id),
        }
    }

    pub fn cancel_session(&mut self, session_id: PlaybackSessionId) -> CancelOutcome {
        let Some(state) = self.sessions.get(&session_id).map(|s| s.snapshot.observed) else {
            return CancelOutcome::UnknownSession;
        };
        if !matches!(
            state,
            SessionState::Preparing | SessionState::Ready | SessionState::Starting
        ) {
            return CancelOutcome::NotCancellable { state };
        }
        self.set_desired(session_id, DesiredState::Stopped);
        let ctx = self.ctx(session_id);
        let result = match self.adapter_for(session_id) {
            Some(a) => a.cancel(&ctx),
            None => Err(PlaybackError::new(
                PlaybackErrorKind::Internal,
                "adapter missing",
            )),
        };
        match result {
            Ok(CancelAck::Accepted) => {
                self.set_state(session_id, SessionState::Canceled);
                self.release_target(session_id);
                info!(%session_id, "cancellation accepted");
                CancelOutcome::Canceled
            }
            Ok(CancelAck::Unsupported) => CancelOutcome::FellBackToStop {
                stop: Box::new(self.do_stop(session_id)),
            },
            Err(error) => {
                self.fail(session_id, error.kind, true);
                CancelOutcome::Failed { error }
            }
        }
    }

    /// Idempotent stop. Cleans up failed sessions without rewriting their `Failed` state.
    pub fn stop_session(&mut self, session_id: PlaybackSessionId) -> StopOutcome {
        let Some((state, pending)) = self
            .sessions
            .get(&session_id)
            .map(|s| (s.snapshot.observed, s.snapshot.cleanup_pending))
        else {
            return StopOutcome::UnknownSession;
        };
        match state {
            SessionState::Stopped | SessionState::Canceled => StopOutcome::AlreadyStopped,
            SessionState::Failed(_) => {
                if !pending {
                    return StopOutcome::AlreadyStopped;
                }
                let ctx = self.ctx(session_id);
                let r = self.adapter_for(session_id).map(|a| a.stop(&ctx));
                match r {
                    Some(Ok(_)) => {
                        if let Some(s) = self.sessions.get_mut(&session_id) {
                            s.snapshot.cleanup_pending = false;
                            s.snapshot.desired = DesiredState::Stopped;
                        }
                        self.release_target(session_id);
                        StopOutcome::CleanedUpFailedSession
                    }
                    Some(Err(error)) => StopOutcome::Failed { error },
                    None => StopOutcome::Failed {
                        error: PlaybackError::new(PlaybackErrorKind::Internal, "adapter missing"),
                    },
                }
            }
            SessionState::Stopping => StopOutcome::Failed {
                error: PlaybackError::new(
                    PlaybackErrorKind::OperationRejected,
                    "stop already in progress",
                ),
            },
            SessionState::Preparing => match self.cancel_session(session_id) {
                CancelOutcome::Canceled => StopOutcome::Canceled,
                CancelOutcome::FellBackToStop { stop } => *stop,
                CancelOutcome::Failed { error } => StopOutcome::Failed { error },
                _ => StopOutcome::Failed {
                    error: PlaybackError::new(
                        PlaybackErrorKind::Internal,
                        "unexpected cancel result",
                    ),
                },
            },
            SessionState::Ready | SessionState::Starting | SessionState::Playing => {
                self.set_desired(session_id, DesiredState::Stopped);
                self.do_stop(session_id)
            }
        }
    }

    /// Stops accepting requests, releases every session and adapter once. Idempotent.
    pub fn shutdown(&mut self) -> ShutdownReport {
        if self.shut_down {
            return ShutdownReport {
                already_shut_down: true,
                ..ShutdownReport::default()
            };
        }
        self.shut_down = true;
        let mut report = ShutdownReport::default();
        let ids: Vec<_> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.snapshot.observed.is_live() || s.snapshot.cleanup_pending)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            match self.stop_session(id) {
                StopOutcome::Failed { error } => report.session_failures.push((id, error.kind)),
                _ => report.sessions_released += 1,
            }
        }
        for a in &mut self.adapters {
            if let Err(e) = a.shutdown() {
                report
                    .adapter_failures
                    .push((a.capabilities().wallpaper_type, e.kind));
            }
        }
        info!(
            released = report.sessions_released,
            failures = report.session_failures.len(),
            "playback shutdown complete"
        );
        report
    }

    /// Polls all registered adapters for unhandled asynchronous events.
    pub fn poll_adapter_events(&mut self) -> Vec<(PlaybackSessionId, PlayerReport)> {
        let mut reports = Vec::new();
        for adapter in &mut self.adapters {
            reports.extend(adapter.poll_reports());
        }
        reports
    }

    /// Checks whether the adapter backing a session is currently alive.
    pub fn check_session_liveness(&mut self, session_id: PlaybackSessionId) -> bool {
        let Some(sess) = self.sessions.get(&session_id) else {
            return false;
        };
        let ty = sess.snapshot.wallpaper_type;
        let ctx = self.ctx(session_id);
        if let Some(adapter) = self
            .adapters
            .iter_mut()
            .find(|a| a.capabilities().wallpaper_type == ty)
        {
            adapter.is_alive(&ctx)
        } else {
            false
        }
    }

    /// Returns session context if session exists.
    #[must_use]
    pub fn session_context(&self, session_id: PlaybackSessionId) -> Option<SessionContext> {
        if self.sessions.contains_key(&session_id) {
            Some(self.ctx(session_id))
        } else {
            None
        }
    }

    // ---- internals ----

    fn begin_start(&mut self, id: PlaybackSessionId) -> Result<SessionState, PlaybackError> {
        let ctx = self.ctx(id);
        let r = match self.adapter_for(id) {
            Some(a) => a.start(&ctx),
            None => Err(PlaybackError::new(
                PlaybackErrorKind::Internal,
                "adapter missing",
            )),
        };
        match r {
            Ok(()) => {
                self.set_state(id, SessionState::Starting);
                Ok(SessionState::Starting)
            }
            Err(error) => {
                self.fail(id, error.kind, true);
                warn!(session_id = %id, kind = ?error.kind, diag = %error.diagnostic, "start failed");
                Err(error)
            }
        }
    }

    fn do_stop(&mut self, id: PlaybackSessionId) -> StopOutcome {
        self.set_state(id, SessionState::Stopping);
        let ctx = self.ctx(id);
        let r = match self.adapter_for(id) {
            Some(a) => a.stop(&ctx),
            None => Err(PlaybackError::new(
                PlaybackErrorKind::Internal,
                "adapter missing",
            )),
        };
        match r {
            Ok(ack) => {
                self.set_state(id, SessionState::Stopped);
                self.release_target(id);
                match ack {
                    StopAck::Stopped => StopOutcome::Stopped,
                    StopAck::AlreadyStopped => StopOutcome::AlreadyStopped,
                }
            }
            Err(error) => {
                self.fail(id, PlaybackErrorKind::StopFailed, true);
                StopOutcome::Failed { error }
            }
        }
    }

    fn adapter_for(&mut self, id: PlaybackSessionId) -> Option<&mut Box<dyn PlayerAdapter>> {
        let ty = self.sessions.get(&id)?.snapshot.wallpaper_type;
        self.adapters
            .iter_mut()
            .find(|a| a.capabilities().wallpaper_type == ty)
    }

    fn ctx(&self, id: PlaybackSessionId) -> SessionContext {
        let session = &self.sessions[&id];
        let s = &session.snapshot;
        SessionContext {
            session_id: id,
            request_id: s.request_id,
            wallpaper_id: s.wallpaper_id,
            target: s.target,
            config: session.effective_config.clone(),
        }
    }

    /// Returns the effective playback configuration for an active session.
    #[must_use]
    pub fn effective_config(&self, id: PlaybackSessionId) -> Option<&EffectivePlaybackConfig> {
        self.sessions
            .get(&id)
            .and_then(|s| s.effective_config.as_ref())
    }

    /// Applies dynamic runtime configuration updates to an active session.
    pub fn apply_runtime_config(
        &mut self,
        session_id: PlaybackSessionId,
        update: &RuntimeConfigUpdate,
    ) -> Result<ConfigUpdateResult, PlaybackError> {
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return Ok(ConfigUpdateResult::TargetNotFound);
        };
        if !session.snapshot.observed.is_live() {
            return Ok(ConfigUpdateResult::TargetNotActive);
        }
        if update.is_empty() {
            return Ok(ConfigUpdateResult::NoChange);
        }

        let wallpaper_type = session.snapshot.wallpaper_type;
        let Some(idx) = self
            .adapters
            .iter()
            .position(|a| a.capabilities().wallpaper_type == wallpaper_type)
        else {
            return Ok(ConfigUpdateResult::UnsupportedSetting {
                setting: "adapter".into(),
            });
        };

        let caps = self.adapters[idx].backend_capabilities();
        if update.volume.is_some() && !caps.supports_dynamic_volume {
            return Ok(ConfigUpdateResult::UnsupportedSetting {
                setting: "volume".into(),
            });
        }
        if update.muted.is_some() && !caps.supports_dynamic_mute {
            return Ok(ConfigUpdateResult::UnsupportedSetting {
                setting: "muted".into(),
            });
        }
        if update.paused.is_some() && !caps.supports_dynamic_pause {
            return Ok(ConfigUpdateResult::UnsupportedSetting {
                setting: "paused".into(),
            });
        }

        let ctx = SessionContext {
            session_id,
            request_id: session.snapshot.request_id,
            wallpaper_id: session.snapshot.wallpaper_id,
            target: session.snapshot.target,
            config: session.effective_config.clone(),
        };

        match self.adapters[idx].apply_runtime_config(&ctx, update) {
            Ok(()) => {
                if let Some(ref mut eff) = session.effective_config {
                    if let Some(vol) = update.volume {
                        let (clamped, _) = crate::config::NormalizedVolume::clamped(vol);
                        eff.common.volume = clamped;
                    }
                    if let Some(muted) = update.muted {
                        eff.common.muted = muted;
                    }
                    if let Some(paused) = update.paused {
                        eff.common.start_mode = if paused {
                            crate::config::StartMode::Paused
                        } else {
                            crate::config::StartMode::AutoPlay
                        };
                    }
                }
                Ok(ConfigUpdateResult::Applied {
                    effective_volume: update
                        .volume
                        .map(|v| crate::config::NormalizedVolume::clamped(v).0.value()),
                    effective_muted: update.muted,
                    effective_paused: update.paused,
                })
            }
            Err(e) => Ok(ConfigUpdateResult::BackendFailed {
                error: e.diagnostic,
            }),
        }
    }

    /// Samples current process resource usage for an active session.
    pub fn sample_metrics(
        &mut self,
        session_id: PlaybackSessionId,
    ) -> Option<ProcessResourceSample> {
        let ty = self.sessions.get(&session_id)?.snapshot.wallpaper_type;
        let ctx = self.ctx(session_id);
        self.adapters
            .iter_mut()
            .find(|a| a.capabilities().wallpaper_type == ty)?
            .sample_metrics(&ctx)
    }

    /// Retrieves backend-specific performance telemetry for an active session.
    pub fn backend_metrics(&self, session_id: PlaybackSessionId) -> BackendPerformanceMetrics {
        let Some(sess) = self.sessions.get(&session_id) else {
            return BackendPerformanceMetrics::default();
        };
        let ty = sess.snapshot.wallpaper_type;
        let ctx = self.ctx(session_id);
        self.adapters
            .iter()
            .find(|a| a.capabilities().wallpaper_type == ty)
            .map(|a| a.backend_metrics(&ctx))
            .unwrap_or_default()
    }

    /// Returns the OS process ID of the backend player process, if running.
    pub fn process_id(&self, session_id: PlaybackSessionId) -> Option<u32> {
        let ty = self.sessions.get(&session_id)?.snapshot.wallpaper_type;
        let ctx = self.ctx(session_id);
        self.adapters
            .iter()
            .find(|a| a.capabilities().wallpaper_type == ty)?
            .process_id(&ctx)
    }

    fn state_of(&self, id: PlaybackSessionId) -> SessionState {
        self.sessions[&id].snapshot.observed
    }

    fn set_desired(&mut self, id: PlaybackSessionId, d: DesiredState) {
        if let Some(s) = self.sessions.get_mut(&id) {
            s.snapshot.desired = d;
        }
    }

    fn set_state(&mut self, id: PlaybackSessionId, next: SessionState) {
        if let Some(s) = self.sessions.get_mut(&id) {
            if s.snapshot.observed.can_transition_to(&next) {
                info!(session_id = %id, from = ?s.snapshot.observed, to = ?next, "lifecycle transition");
                s.snapshot.observed = next;
            } else {
                warn!(session_id = %id, from = ?s.snapshot.observed, to = ?next, "rejected invalid lifecycle transition");
            }
        }
    }

    fn fail(&mut self, id: PlaybackSessionId, kind: PlaybackErrorKind, cleanup_pending: bool) {
        self.set_state(id, SessionState::Failed(kind));
        if let Some(s) = self.sessions.get_mut(&id) {
            if matches!(s.snapshot.observed, SessionState::Failed(_)) {
                s.snapshot.cleanup_pending = cleanup_pending;
            }
        }
    }

    fn release_target(&mut self, id: PlaybackSessionId) {
        let target = self.sessions[&id].snapshot.target;
        if self.by_target.get(&target) == Some(&id) {
            self.by_target.remove(&target);
        }
    }
}
