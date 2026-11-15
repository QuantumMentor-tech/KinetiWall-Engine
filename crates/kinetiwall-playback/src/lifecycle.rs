//! Typed session lifecycle and transition rules.

use kinetiwall_catalog::{WallpaperId, WallpaperType};
use serde::{Deserialize, Serialize};

use crate::types::{DisplayTargetId, PlaybackErrorKind, PlaybackRequestId, PlaybackSessionId};

/// Observed lifecycle state of a session (what has been confirmed, not what was requested).
///
/// `Starting` means *start was accepted by the adapter*; `Playing` is reached only on a backend
/// `PlayerReport::Playing` confirmation. `Paused` is deferred to the pause-policy step (`CTR-04`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "failure", rename_all = "snake_case")]
pub enum SessionState {
    Preparing,
    Ready,
    Starting,
    Playing,
    Stopping,
    Stopped,
    Failed(PlaybackErrorKind),
    Canceled,
}

impl SessionState {
    /// Terminal states never transition again. A `Failed` session may still need adapter cleanup,
    /// tracked separately by the orchestrator (`cleanup_pending`).
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Stopped | Self::Failed(_) | Self::Canceled)
    }

    #[must_use]
    pub fn is_live(&self) -> bool {
        !self.is_terminal()
    }

    /// The complete set of valid transitions.
    #[must_use]
    pub fn can_transition_to(&self, next: &SessionState) -> bool {
        use SessionState::*;
        matches!(
            (self, next),
            (Preparing, Ready | Stopping | Failed(_) | Canceled)
                | (Ready, Starting | Failed(_) | Canceled | Stopping)
                | (Starting, Playing | Failed(_) | Canceled | Stopping)
                | (Playing, Stopping | Failed(_))
                | (Stopping, Stopped | Failed(_))
        )
    }
}

/// What KinetiWall has asked for. Kept separate from [`SessionState`]: a desired `Playing` never
/// implies an observed `Playing`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesiredState {
    Playing,
    Stopped,
}

/// Read-only copy of a session for callers and presentation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub session_id: PlaybackSessionId,
    pub request_id: PlaybackRequestId,
    pub wallpaper_id: WallpaperId,
    pub wallpaper_type: WallpaperType,
    pub target: DisplayTargetId,
    pub desired: DesiredState,
    pub observed: SessionState,
    /// True when a timeout means the backend may or may not have acted; requires stop/reconcile.
    pub outcome_uncertain: bool,
    /// True when adapter resources may still be held and `stop_session` must run cleanup.
    pub cleanup_pending: bool,
}
