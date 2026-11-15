//! Desktop surface synchronization, display-change handling, and safe reattachment.
//!
//! Provides runtime coordination to ensure an already-attached KinetiWall wallpaper
//! surface remains correctly sized, positioned, and safely hosted across display resolution
//! changes, DPI transitions, host window movement/resizing, and unexpected shell host disappearance.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use uuid::Uuid;

use crate::types::{DiscoveryError, NativeWindowHandle, WindowRect};

/// Maximum retained diagnostic events for surface synchronization.
pub const MAX_SYNC_DIAGNOSTIC_EVENTS: usize = 100;

// -------------------------------------------------------------------------------------------------
// Synchronization States
// -------------------------------------------------------------------------------------------------

/// Explicit operational state of a wallpaper surface in the synchronization lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceSyncState {
    /// No wallpaper surface is currently attached or tracked for synchronization.
    NoSurfaceAttached,
    /// Surface is verified attached, correctly sized, and aligned with target host bounds.
    Synchronized,
    /// Display or host geometry event received; synchronization is pending execution.
    GeometryUpdatePending,
    /// Native window positioning (`SetWindowPos`) and post-verification are actively in progress.
    Synchronizing,
    /// Revalidation of the host handle and parent relationship is queued.
    HostValidationPending,
    /// The target host window has disappeared, been destroyed, or unparented the surface.
    HostUnavailable,
    /// Invariant violated (e.g. style or parent changed); revalidation required before mutation.
    RevalidationRequired,
    /// Safe host rediscovery and transactional reattachment are actively in progress.
    RecoveryInProgress,
    /// Synchronization or recovery failed; manual or supervisor intervention required.
    SynchronizationFailed,
    /// Surface was detached cleanly or the coordinator is shutting down.
    DetachedOrShuttingDown,
}

impl fmt::Display for SurfaceSyncState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

// -------------------------------------------------------------------------------------------------
// Display and Host Change Events
// -------------------------------------------------------------------------------------------------

/// External or system events that trigger surface geometry evaluation or recovery.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SurfaceChangeEvent {
    /// System display resolution or color depth changed (e.g. `WM_DISPLAYCHANGE`).
    DisplayResolutionChanged {
        width: i32,
        height: i32,
        bits_per_pixel: u32,
    },
    /// Per-monitor or system DPI awareness changed (e.g. `WM_DPICHANGED`).
    DpiChanged {
        new_dpi: u32,
        suggested_rect: Option<WindowRect>,
    },
    /// The parent host window moved or changed dimensions (e.g. `WM_WINDOWPOSCHANGED`).
    HostGeometryChanged {
        host_handle: NativeWindowHandle,
        new_bounds: WindowRect,
    },
    /// The parent desktop host window was destroyed or closed.
    HostWindowDestroyed(NativeWindowHandle),
    /// The parent relationship was broken (e.g. `GetParent(source) != host`).
    ParentRelationshipLost(NativeWindowHandle),
    /// Explicit request to re-evaluate and synchronize surface geometry.
    ManualSyncRequested,
}

// -------------------------------------------------------------------------------------------------
// Synchronization Metadata & Diagnostic Records
// -------------------------------------------------------------------------------------------------

/// State and tracking metadata for an attached wallpaper surface undergoing synchronization.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SurfaceSyncMetadata {
    pub source_handle: NativeWindowHandle,
    pub target_handle: NativeWindowHandle,
    pub player_instance_id: Uuid,
    pub sync_generation: u64,
    pub state: SurfaceSyncState,
    pub last_verified_bounds: Option<WindowRect>,
    pub last_sync_timestamp_millis: u64,
    pub recovery_attempts: u32,
    pub max_recovery_attempts: u32,
    pub is_synchronizing: bool,
}

impl SurfaceSyncMetadata {
    #[must_use]
    pub fn new(
        source_handle: NativeWindowHandle,
        target_handle: NativeWindowHandle,
        player_instance_id: Uuid,
        initial_bounds: WindowRect,
    ) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            source_handle,
            target_handle,
            player_instance_id,
            sync_generation: 1,
            state: SurfaceSyncState::Synchronized,
            last_verified_bounds: Some(initial_bounds),
            last_sync_timestamp_millis: now,
            recovery_attempts: 0,
            max_recovery_attempts: 3,
            is_synchronizing: false,
        }
    }
}

/// Confirmation report produced after an attempted surface synchronization.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SurfaceSyncReport {
    pub source_handle: NativeWindowHandle,
    pub host_handle: NativeWindowHandle,
    pub player_instance_id: Uuid,
    pub sync_generation: u64,
    pub previous_bounds: Option<WindowRect>,
    pub effective_bounds: WindowRect,
    /// Whether an actual Win32 `SetWindowPos` mutation was applied (`false` if skipped as a no-op).
    pub mutation_applied: bool,
    pub state: SurfaceSyncState,
    pub synced_at_millis: u64,
}

/// Structured diagnostic event recorded during surface synchronization operations.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SurfaceSyncEvent {
    pub source_handle: NativeWindowHandle,
    pub host_handle: NativeWindowHandle,
    pub player_instance_id: Uuid,
    pub generation: u64,
    pub description: String,
    pub old_bounds: Option<WindowRect>,
    pub new_bounds: Option<WindowRect>,
    pub outcome_state: SurfaceSyncState,
    pub timestamp_millis: u64,
}

// -------------------------------------------------------------------------------------------------
// Error Taxonomy
// -------------------------------------------------------------------------------------------------

/// Strongly typed errors encountered during surface synchronization.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, serde::Serialize, serde::Deserialize)]
pub enum SurfaceSyncError {
    #[error("Window {0} is not currently attached to any desktop host")]
    NotAttached(NativeWindowHandle),

    #[error("Source window handle {0} is invalid or has been destroyed")]
    SourceInvalidHandle(NativeWindowHandle),

    #[error("Desktop host handle {0} is invalid or has been destroyed")]
    HostInvalidHandle(NativeWindowHandle),

    #[error("Invalid surface bounds: {bounds:?}, reason: {reason}")]
    InvalidGeometry { bounds: WindowRect, reason: String },

    #[error("Stale player instance ID: expected {expected}, actual {actual}")]
    StaleInstance { expected: Uuid, actual: Uuid },

    #[error("Stale synchronization generation: expected {expected}, actual {actual}")]
    StaleGeneration { expected: u64, actual: u64 },

    #[error("Desktop host was lost and recovery failed: {0}")]
    HostLostAndRecoveryFailed(String),

    #[error("Host recovery budget exhausted ({attempts}/{max} attempts failed)")]
    RecoveryBudgetExhausted { attempts: u32, max: u32 },

    #[error("Native positioning failure (SetWindowPos): {0}")]
    NativePositioningFailed(String),

    #[error("Post-positioning geometry verification failed: expected {expected:?}, observed {observed:?}")]
    VerificationFailed {
        expected: WindowRect,
        observed: WindowRect,
    },

    #[error("Attachment coordinator is shut down and rejecting synchronization requests")]
    CoordinatorShutDown,

    #[error("Underlying desktop discovery error: {0}")]
    DiscoveryError(#[from] DiscoveryError),

    #[error("Underlying attachment error: {0}")]
    AttachmentError(#[from] crate::attachment::AttachmentError),
}
