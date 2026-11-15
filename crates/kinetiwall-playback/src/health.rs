//! Health signals, failure classification, and player instance identity.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::{PlaybackErrorKind, PlaybackSessionId};

/// Unique generation token identifying a specific supervised player instance.
/// Changing this prevents stale callbacks/events from older, terminated instances from affecting newer ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PlayerInstanceId(pub Uuid);

impl PlayerInstanceId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for PlayerInstanceId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for PlayerInstanceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Structured classification of player failures for diagnostic and recovery decisions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "classification", rename_all = "snake_case")]
pub enum FailureClassification {
    /// Unexpected process exit.
    ProcessTerminated { exit_code: Option<i32> },
    /// Startup deadline expired before backend confirmed readiness.
    StartupTimeout,
    /// Shutdown deadline expired before backend finished teardown.
    ShutdownTimeout,
    /// Lost communication over standard I/O pipes.
    IpcDisconnection,
    /// Authoritative failure report from backend.
    BackendFailure { error_kind: PlaybackErrorKind },
    /// Missing, unreadable, or corrupted asset file.
    InvalidAsset,
    /// Unrecoverable internal failure or memory condition.
    Unrecoverable { detail: String },
}

impl FailureClassification {
    /// Determines whether the failure category is eligible for automatic recovery.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::ProcessTerminated { .. } => true,
            Self::IpcDisconnection => true,
            Self::BackendFailure { error_kind } => matches!(
                error_kind,
                PlaybackErrorKind::BackendExited | PlaybackErrorKind::Timeout
            ),
            Self::StartupTimeout => false, // Deterministic slow startup or hang
            Self::ShutdownTimeout => false,
            Self::InvalidAsset => false, // Deterministic file defect
            Self::Unrecoverable { .. } => false,
        }
    }
}

/// Runtime health status of a supervised player instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PlayerHealthStatus {
    /// Actively running and confirming healthy operation.
    Healthy,
    /// Running but experiencing transient delay or missing reports.
    Degraded { detail: String },
    /// No reports received within the expected heartbeat window.
    Unresponsive,
    /// Process has terminated or unrecoverable error occurred.
    Dead { reason: FailureClassification },
}

impl PlayerHealthStatus {
    #[must_use]
    pub fn is_healthy(&self) -> bool {
        matches!(self, Self::Healthy)
    }

    #[must_use]
    pub fn is_dead(&self) -> bool {
        matches!(self, Self::Dead { .. })
    }
}

/// Health report snapshot for external observability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthReport {
    pub instance_id: PlayerInstanceId,
    pub session_id: PlaybackSessionId,
    pub status: PlayerHealthStatus,
    pub failure: Option<FailureClassification>,
}
