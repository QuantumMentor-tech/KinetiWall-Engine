//! Player-adapter contract.
//!
//! # Semantics
//! - All methods are bounded, non-blocking-in-spirit calls: real backends run in an isolated
//!   player process behind the supervisor (future step) and must never decode/render inside the
//!   caller. Long-running confirmation arrives asynchronously as [`PlayerReport`] events.
//! - `prepare` → `Ready` guarantees the backend holds what it needs to start but shows nothing.
//!   `Pending` means a `PlayerReport::Ready` event will follow.
//! - `start` → `Ok(())` means the request was **accepted only**. Playing is confirmed solely by a
//!   `PlayerReport::Playing` event.
//! - `cancel` → `Accepted` guarantees no further playback results from this session and that the
//!   adapter releases its resources; late events for it are ignored. `Unsupported` leaves the
//!   session untouched (the orchestrator falls back to `stop`).
//! - `stop` is idempotent: stopping an already-stopped session returns `AlreadyStopped`.
//! - A failed `prepare` must leave no resources. After any other failure the orchestrator calls
//!   `stop` for cleanup (`cleanup_pending`).
//! - `shutdown` releases adapter-global resources; it is idempotent.
//!
//! # Ownership
//! The orchestrator owns each adapter (`Box`) and every session; the adapter owns its backend
//! handles, tasks, IPC connections, and process handles, and releases them in `stop`/`cancel`/
//! `shutdown` (explicit, never reliant on `Drop` timing for completion).

use kinetiwall_catalog::WallpaperId;
use serde::{Deserialize, Serialize};

use crate::config::{EffectivePlaybackConfig, RuntimeConfigUpdate};
use crate::diagnostics::{BackendPerformanceMetrics, ProcessResourceSample};
use crate::negotiation::BackendCapabilities;
use crate::types::{
    DisplayTargetId, PlaybackError, PlaybackErrorKind, PlaybackRequestId, PlaybackSessionId,
    PlayerAvailability, PlayerCapabilities,
};

/// Identity and configuration handed to an adapter. Contains no paths; adapters resolve assets via the trusted catalog.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionContext {
    pub session_id: PlaybackSessionId,
    pub request_id: PlaybackRequestId,
    pub wallpaper_id: WallpaperId,
    pub target: DisplayTargetId,
    pub config: Option<EffectivePlaybackConfig>,
}

impl SessionContext {
    #[must_use]
    pub fn new(
        session_id: PlaybackSessionId,
        request_id: PlaybackRequestId,
        wallpaper_id: WallpaperId,
        target: DisplayTargetId,
    ) -> Self {
        Self {
            session_id,
            request_id,
            wallpaper_id,
            target,
            config: None,
        }
    }

    #[must_use]
    pub fn with_config(mut self, config: EffectivePlaybackConfig) -> Self {
        self.config = Some(config);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrepareProgress {
    Ready,
    Pending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelAck {
    Accepted,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopAck {
    Stopped,
    AlreadyStopped,
}

/// Asynchronous, untrusted report from a backend. Validated by the orchestrator before use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "report", rename_all = "snake_case")]
pub enum PlayerReport {
    Ready,
    Playing,
    Stopped,
    Failed { kind: PlaybackErrorKind },
}

pub trait PlayerAdapter: Send {
    fn capabilities(&self) -> PlayerCapabilities;
    fn availability(&self) -> PlayerAvailability;
    fn prepare(&mut self, ctx: &SessionContext) -> Result<PrepareProgress, PlaybackError>;
    fn start(&mut self, ctx: &SessionContext) -> Result<(), PlaybackError>;
    fn cancel(&mut self, ctx: &SessionContext) -> Result<CancelAck, PlaybackError>;
    fn stop(&mut self, ctx: &SessionContext) -> Result<StopAck, PlaybackError>;
    fn shutdown(&mut self) -> Result<(), PlaybackError>;

    /// Polls for unhandled asynchronous events from the backend (reports, crash notifications).
    fn poll_reports(&mut self) -> Vec<(PlaybackSessionId, PlayerReport)> {
        Vec::new()
    }

    /// Checks whether the underlying player process or runtime is currently alive.
    fn is_alive(&mut self, _ctx: &SessionContext) -> bool {
        true
    }

    /// Provides structured backend capabilities used for configuration negotiation.
    fn backend_capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::default_for(self.capabilities().wallpaper_type)
    }

    /// Applies dynamic runtime configuration updates (e.g. volume, mute, pause) to an active player.
    fn apply_runtime_config(
        &mut self,
        _ctx: &SessionContext,
        _update: &RuntimeConfigUpdate,
    ) -> Result<(), PlaybackError> {
        Ok(())
    }

    /// Samples current process resource usage (working set, CPU, thread count) for an active player.
    fn sample_metrics(&mut self, _ctx: &SessionContext) -> Option<ProcessResourceSample> {
        None
    }

    /// Reports backend-specific performance metrics (e.g. decoder mode, media load duration).
    fn backend_metrics(&self, _ctx: &SessionContext) -> BackendPerformanceMetrics {
        BackendPerformanceMetrics::default()
    }

    /// Returns the OS process ID of the backend player process, if running.
    fn process_id(&self, _ctx: &SessionContext) -> Option<u32> {
        None
    }
}
