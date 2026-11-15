//! Controlled native Windows wallpaper window attachment, window styles, and transactional rollback.
//!
//! Provides a safe, verifiable state machine for attaching a dedicated KinetiWall player
//! window to a validated desktop-host candidate (e.g. `WorkerW`), enforcing native Win32
//! style contracts, non-intrusive visibility, and automatic rollback on failure.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use uuid::Uuid;

use crate::compatibility::CompatibilityPolicy;
use crate::discovery::{DesktopDiscoveryOptions, DesktopHostDiscovery};
use crate::provider::DesktopWindowProvider;
use crate::synchronization::{
    SurfaceChangeEvent, SurfaceSyncError, SurfaceSyncEvent, SurfaceSyncMetadata, SurfaceSyncReport,
    SurfaceSyncState, MAX_SYNC_DIAGNOSTIC_EVENTS,
};
use crate::types::{
    CandidateRole, DesktopCandidateWindow, DiscoveryError, NativeWindowHandle, WindowRect,
};

// -------------------------------------------------------------------------------------------------
// Win32 Window Styles & Flags Constants
// -------------------------------------------------------------------------------------------------

pub mod constants {
    use crate::types::NativeWindowHandle;

    pub const GWL_STYLE: i32 = -16;
    pub const GWL_EXSTYLE: i32 = -20;

    pub const WS_OVERLAPPED: isize = 0x0000_0000;
    pub const WS_POPUP: isize = 0x8000_0000_u32 as isize;
    pub const WS_CHILD: isize = 0x4000_0000;
    pub const WS_VISIBLE: isize = 0x1000_0000;
    pub const WS_CLIPSIBLINGS: isize = 0x0400_0000;
    pub const WS_CLIPCHILDREN: isize = 0x0200_0000;

    pub const WS_EX_TOOLWINDOW: isize = 0x0000_0080;
    pub const WS_EX_APPWINDOW: isize = 0x0004_0000;
    pub const WS_EX_NOACTIVATE: isize = 0x0800_0000;

    pub const SWP_NOSIZE: u32 = 0x0001;
    pub const SWP_NOMOVE: u32 = 0x0002;
    pub const SWP_NOZORDER: u32 = 0x0004;
    pub const SWP_NOACTIVATE: u32 = 0x0010;
    pub const SWP_FRAMECHANGED: u32 = 0x0020;
    pub const SWP_SHOWWINDOW: u32 = 0x0040;
    pub const SWP_HIDEWINDOW: u32 = 0x0080;

    pub const HWND_TOP: NativeWindowHandle = NativeWindowHandle(0);
    pub const HWND_BOTTOM: NativeWindowHandle = NativeWindowHandle(1);
}

// -------------------------------------------------------------------------------------------------
// Lifecycle & Attachment States
// -------------------------------------------------------------------------------------------------

/// Explicit operational state of a player window in the native desktop attachment lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentState {
    /// Window is not attached to any desktop host window.
    Detached,
    /// Window eligibility and target compatibility are currently being evaluated.
    Validating,
    /// Transactional mutations (styles, parenting, positioning) are actively in progress.
    Attaching,
    /// Window is confirmed attached behind desktop icons and verified.
    Attached,
    /// Detachment mutations are actively in progress.
    Detaching,
    /// Attachment or validation failed; rollback to original state is in progress.
    Restoring,
    /// Attachment transaction failed and could not be completed.
    Failed,
    /// Rollback failed or was incomplete; requires external supervisor intervention.
    RecoveryRequired,
}

impl std::fmt::Display for AttachmentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// Explicit progressive stages of the attachment transaction.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentStage {
    /// Stage 0: Initial validation of source window, target host, and ownership.
    Validation,
    /// Stage 1: Original window styles, bounds, and parent captured.
    StateCaptured,
    /// Stage 2: Source window styles updated (WS_CHILD added, WS_POPUP stripped).
    StylePrepared,
    /// Stage 3: SetParent successfully called to reparent window into desktop host.
    ParentAssigned,
    /// Stage 4: Window positioned and sized to target host dimensions.
    BoundsApplied,
    /// Stage 5: Post-mutation verification completed (parent, styles, bounds, visibility).
    Verified,
}

/// The recorded outcome of a rollback operation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RollbackOutcome {
    /// All mutations successfully reversed and restored state verified.
    RollbackCompleted { verified: bool },
    /// Rollback encountered an error during native API restoration.
    RollbackFailed { error: String },
    /// Rollback was partially applied but some original attributes could not be confirmed.
    RollbackIncomplete { reason: String },
    /// Original parent window is no longer available.
    OriginalParentUnavailable,
    /// Source window was destroyed during the transaction or rollback.
    SourceWindowDestroyed,
}

// -------------------------------------------------------------------------------------------------
// Error Taxonomy
// -------------------------------------------------------------------------------------------------

/// Strongly typed errors for wallpaper window attachment operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, serde::Serialize, serde::Deserialize)]
pub enum AttachmentError {
    #[error("Source window handle {0} is invalid or NULL")]
    SourceInvalidHandle(NativeWindowHandle),

    #[error("Target desktop host handle {0} is invalid or NULL")]
    TargetInvalidHandle(NativeWindowHandle),

    #[error("Source window {0} is the protected management UI window and cannot be attached")]
    SourceIsManagementUi(NativeWindowHandle),

    #[error("Source window {0} is the desktop root window and cannot be attached")]
    SourceIsDesktopRoot(NativeWindowHandle),

    #[error("Source window {0} is a desktop icon view or carrier window")]
    SourceIsIconHost(NativeWindowHandle),

    #[error("Source window PID mismatch: expected {expected}, actual {actual}")]
    SourceProcessMismatch { expected: u32, actual: u32 },

    #[error("Target candidate is incompatible: {0}")]
    TargetIncompatible(String),

    #[error(
        "DPI awareness contexts are incompatible: source={source_ctx:?}, target={target_ctx:?}"
    )]
    DpiIncompatible {
        source_ctx: Option<isize>,
        target_ctx: Option<isize>,
    },

    #[error("Failed to mutate window styles: {0}")]
    StyleMutationFailed(String),

    #[error("Failed to update parent relationship (SetParent): {0}")]
    ParentMutationFailed(String),

    #[error("Failed to position and size window (SetWindowPos): {0}")]
    BoundsMutationFailed(String),

    #[error("Post-attachment verification check failed: {0}")]
    VerificationFailed(String),

    #[error("Attachment failed and triggered rollback: cause={cause}, outcome={outcome:?}")]
    RollbackTriggered {
        cause: Box<AttachmentError>,
        outcome: RollbackOutcome,
    },

    #[error("Window {0} is already attached to a desktop host")]
    AlreadyAttached(NativeWindowHandle),

    #[error("Window {0} is not currently attached")]
    NotAttached(NativeWindowHandle),

    #[error("Stale player instance ID: expected {expected}, actual {actual}")]
    StalePlayerInstance { expected: Uuid, actual: Uuid },

    #[error("Native desktop provider error: {0}")]
    NativeApiFailure(#[from] DiscoveryError),

    #[error("Attachment coordinator is shut down and rejecting new requests")]
    CoordinatorShutDown,
}

// -------------------------------------------------------------------------------------------------
// Domain Records & Snapshots
// -------------------------------------------------------------------------------------------------

/// Preserved original window state captured prior to any native window mutation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OriginalWindowState {
    pub source_handle: NativeWindowHandle,
    pub original_parent: Option<NativeWindowHandle>,
    pub original_style: isize,
    pub original_ex_style: isize,
    pub original_bounds: WindowRect,
    pub original_visible: bool,
    pub process_id: u32,
    pub thread_id: u32,
    pub player_instance_id: Uuid,
    pub target_handle: NativeWindowHandle,
    pub captured_at_millis: u64,
}

/// Request to attach a validated player window to a candidate desktop host.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AttachmentRequest {
    pub source_handle: NativeWindowHandle,
    pub player_instance_id: Uuid,
    pub owning_process_id: u32,
    pub target_candidate: DesktopCandidateWindow,
    pub target_display_bounds: Option<WindowRect>,
}

/// Confirmation record emitted upon successful attachment and post-mutation verification.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AttachmentRecord {
    pub source_handle: NativeWindowHandle,
    pub target_handle: NativeWindowHandle,
    pub player_instance_id: Uuid,
    pub attached_at_millis: u64,
    pub effective_style: isize,
    pub effective_ex_style: isize,
    pub effective_bounds: WindowRect,
    pub is_verified: bool,
}

/// Confirmation record emitted upon successful detachment and restoration.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DetachmentRecord {
    pub source_handle: NativeWindowHandle,
    pub restored_parent: Option<NativeWindowHandle>,
    pub restored_style: isize,
    pub restored_ex_style: isize,
    pub restored_bounds: WindowRect,
    pub detached_at_millis: u64,
    pub is_verified: bool,
}

// -------------------------------------------------------------------------------------------------
// Attachment Coordinator
// -------------------------------------------------------------------------------------------------

/// Coordinates transactional attachment, window style changes, verification, and rollback.
pub struct AttachmentCoordinator {
    provider: Arc<dyn DesktopWindowProvider>,
    prohibited_handles: HashSet<NativeWindowHandle>,
    allowed_process_ids: HashSet<u32>,
    attached_windows: HashMap<NativeWindowHandle, (OriginalWindowState, AttachmentRecord)>,
    window_states: HashMap<NativeWindowHandle, AttachmentState>,
    active_requests: HashMap<NativeWindowHandle, Uuid>,
    sync_metadata: HashMap<NativeWindowHandle, SurfaceSyncMetadata>,
    sync_diagnostic_events: Vec<SurfaceSyncEvent>,
    is_shut_down: bool,
}

impl AttachmentCoordinator {
    /// Creates a new `AttachmentCoordinator` backed by the given window provider.
    #[must_use]
    pub fn new(provider: Arc<dyn DesktopWindowProvider>) -> Self {
        Self {
            provider,
            prohibited_handles: HashSet::new(),
            allowed_process_ids: HashSet::new(),
            attached_windows: HashMap::new(),
            window_states: HashMap::new(),
            active_requests: HashMap::new(),
            sync_metadata: HashMap::new(),
            sync_diagnostic_events: Vec::new(),
            is_shut_down: false,
        }
    }

    /// Registers a window handle that must NEVER be reparented or attached (e.g. Management UI).
    pub fn register_prohibited_handle(&mut self, hwnd: NativeWindowHandle) {
        if !hwnd.is_null() {
            self.prohibited_handles.insert(hwnd);
        }
    }

    /// Registers an authorized process ID permitted to own wallpaper player windows.
    pub fn register_allowed_process_id(&mut self, pid: u32) {
        if pid > 0 {
            self.allowed_process_ids.insert(pid);
        }
    }

    /// Queries the tracked attachment state of a window.
    #[must_use]
    pub fn get_state(&self, hwnd: NativeWindowHandle) -> AttachmentState {
        self.window_states
            .get(&hwnd)
            .copied()
            .unwrap_or(AttachmentState::Detached)
    }

    /// Queries the active attachment record for a window, if currently attached.
    #[must_use]
    pub fn get_attachment_record(&self, hwnd: NativeWindowHandle) -> Option<&AttachmentRecord> {
        self.attached_windows.get(&hwnd).map(|(_, rec)| rec)
    }

    /// Revalidates that an attached window remains valid, visible, and parented to the host.
    #[must_use]
    pub fn revalidate_attached(&self, hwnd: NativeWindowHandle) -> bool {
        if let Some((_, record)) = self.attached_windows.get(&hwnd) {
            if !self.provider.is_window(hwnd) || !self.provider.is_window(record.target_handle) {
                return false;
            }
            if self.provider.get_parent(hwnd) != Some(record.target_handle) {
                return false;
            }
            if let Ok(style) = self.provider.get_window_long(hwnd, constants::GWL_STYLE) {
                if (style & constants::WS_CHILD) == 0 || (style & constants::WS_POPUP) != 0 {
                    return false;
                }
            } else {
                return false;
            }
            true
        } else {
            false
        }
    }

    /// Executes the transactional attachment procedure with explicit rollback on any failure.
    pub fn attach(
        &mut self,
        request: &AttachmentRequest,
    ) -> Result<AttachmentRecord, AttachmentError> {
        if self.is_shut_down {
            return Err(AttachmentError::CoordinatorShutDown);
        }

        let source = request.source_handle;
        let target = request.target_candidate.handle;

        tracing::info!(
            source = %source,
            target = %target,
            instance = %request.player_instance_id,
            "Initiating wallpaper window attachment transaction"
        );

        if self.attached_windows.contains_key(&source) {
            return Err(AttachmentError::AlreadyAttached(source));
        }

        self.window_states
            .insert(source, AttachmentState::Validating);

        // Stage 0: Validation
        if let Err(err) = self.validate_attachment_eligibility(request) {
            self.window_states.insert(source, AttachmentState::Failed);
            tracing::warn!(source = %source, error = %err, "Attachment validation rejected");
            return Err(err);
        }

        // Stage 1: Capture Original Window State
        let original_state = match self.capture_original_state(request) {
            Ok(s) => s,
            Err(err) => {
                self.window_states.insert(source, AttachmentState::Failed);
                return Err(err);
            }
        };

        self.window_states
            .insert(source, AttachmentState::Attaching);
        self.active_requests
            .insert(source, request.player_instance_id);

        let mut current_stage = AttachmentStage::StateCaptured;

        // Stage 2: Prepare Window Styles
        let original_style = original_state.original_style;
        let original_ex_style = original_state.original_ex_style;

        // In Win32, child windows must have WS_CHILD and must NOT have WS_POPUP.
        let new_style = (original_style & !constants::WS_POPUP)
            | constants::WS_CHILD
            | constants::WS_CLIPSIBLINGS;
        let new_ex_style =
            original_ex_style | constants::WS_EX_TOOLWINDOW | constants::WS_EX_NOACTIVATE;

        if let Err(err) = self
            .provider
            .set_window_long(source, constants::GWL_STYLE, new_style)
        {
            return self.abort_and_rollback(
                source,
                &original_state,
                current_stage,
                AttachmentError::StyleMutationFailed(err.to_string()),
            );
        }

        if let Err(err) =
            self.provider
                .set_window_long(source, constants::GWL_EXSTYLE, new_ex_style)
        {
            return self.abort_and_rollback(
                source,
                &original_state,
                current_stage,
                AttachmentError::StyleMutationFailed(err.to_string()),
            );
        }

        let _ = self.provider.set_window_pos(
            source,
            None,
            0,
            0,
            0,
            0,
            constants::SWP_NOMOVE
                | constants::SWP_NOSIZE
                | constants::SWP_NOZORDER
                | constants::SWP_NOACTIVATE
                | constants::SWP_FRAMECHANGED,
        );

        current_stage = AttachmentStage::StylePrepared;

        // Stage 3: Apply Parent Relationship (SetParent)
        if let Err(err) = self.provider.set_parent(source, Some(target)) {
            return self.abort_and_rollback(
                source,
                &original_state,
                current_stage,
                AttachmentError::ParentMutationFailed(err.to_string()),
            );
        }

        current_stage = AttachmentStage::ParentAssigned;

        // Stage 4: Position and Size Surface to Target Bounds
        let target_bounds = if let Some(bounds) = request.target_display_bounds {
            bounds
        } else if let Ok(bounds) = self.provider.get_window_rect(target) {
            bounds
        } else {
            WindowRect::new(0, 0, 1920, 1080)
        };

        let width = target_bounds.width();
        let height = target_bounds.height();

        if let Err(err) = self.provider.set_window_pos(
            source,
            Some(constants::HWND_TOP),
            target_bounds.left,
            target_bounds.top,
            width,
            height,
            constants::SWP_NOACTIVATE | constants::SWP_SHOWWINDOW,
        ) {
            return self.abort_and_rollback(
                source,
                &original_state,
                current_stage,
                AttachmentError::BoundsMutationFailed(err.to_string()),
            );
        }

        current_stage = AttachmentStage::BoundsApplied;

        // Stage 5: Post-Mutation Verification
        if let Err(err) = self.verify_attached_state(source, target, new_style) {
            return self.abort_and_rollback(source, &original_state, current_stage, err);
        }

        // Stage 6: Commit
        let now_millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let effective_bounds = self
            .provider
            .get_window_rect(source)
            .unwrap_or(target_bounds);

        let record = AttachmentRecord {
            source_handle: source,
            target_handle: target,
            player_instance_id: request.player_instance_id,
            attached_at_millis: now_millis,
            effective_style: new_style,
            effective_ex_style: new_ex_style,
            effective_bounds,
            is_verified: true,
        };

        self.attached_windows
            .insert(source, (original_state, record.clone()));
        self.window_states.insert(source, AttachmentState::Attached);

        let sync_meta =
            SurfaceSyncMetadata::new(source, target, request.player_instance_id, effective_bounds);
        self.sync_metadata.insert(source, sync_meta);
        self.record_sync_diagnostic(
            source,
            target,
            request.player_instance_id,
            1,
            "Surface initial synchronization committed upon attachment".to_string(),
            None,
            Some(effective_bounds),
            SurfaceSyncState::Synchronized,
        );

        tracing::info!(
            source = %source,
            target = %target,
            instance = %request.player_instance_id,
            "Attachment transaction successfully committed and verified"
        );

        Ok(record)
    }

    /// Detaches an attached player window and deterministically restores its original state.
    pub fn detach(
        &mut self,
        source: NativeWindowHandle,
    ) -> Result<DetachmentRecord, AttachmentError> {
        tracing::info!(source = %source, "Detachment requested");

        let (original_state, _) = match self.attached_windows.remove(&source) {
            Some(entry) => entry,
            None => {
                let current_state = self.get_state(source);
                if current_state == AttachmentState::Detached {
                    let now_millis = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;

                    return Ok(DetachmentRecord {
                        source_handle: source,
                        restored_parent: None,
                        restored_style: 0,
                        restored_ex_style: 0,
                        restored_bounds: WindowRect::default(),
                        detached_at_millis: now_millis,
                        is_verified: true,
                    });
                }
                return Err(AttachmentError::NotAttached(source));
            }
        };

        self.window_states
            .insert(source, AttachmentState::Detaching);

        // Rollback from Verified stage to restore original state
        let outcome = self.execute_restoration(source, &original_state, AttachmentStage::Verified);

        let now_millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let verified = match outcome {
            RollbackOutcome::RollbackCompleted { verified: v } => v,
            _ => false,
        };

        self.window_states.insert(source, AttachmentState::Detached);
        self.active_requests.remove(&source);

        if let Some(mut meta) = self.sync_metadata.remove(&source) {
            meta.state = SurfaceSyncState::DetachedOrShuttingDown;
            self.record_sync_diagnostic(
                source,
                meta.target_handle,
                meta.player_instance_id,
                meta.sync_generation,
                "Surface synchronization tracking terminated upon detachment".to_string(),
                meta.last_verified_bounds,
                None,
                SurfaceSyncState::DetachedOrShuttingDown,
            );
        }

        let record = DetachmentRecord {
            source_handle: source,
            restored_parent: original_state.original_parent,
            restored_style: original_state.original_style,
            restored_ex_style: original_state.original_ex_style,
            restored_bounds: original_state.original_bounds,
            detached_at_millis: now_millis,
            is_verified: verified,
        };

        tracing::info!(source = %source, verified = verified, "Detachment completed");

        Ok(record)
    }

    /// Detaches all active wallpaper windows during application shutdown.
    pub fn detach_all_shutdown(&mut self) -> Vec<Result<DetachmentRecord, AttachmentError>> {
        self.is_shut_down = true;
        let attached_handles: Vec<NativeWindowHandle> =
            self.attached_windows.keys().copied().collect();
        let mut results = Vec::new();
        for hwnd in attached_handles {
            results.push(self.detach(hwnd));
        }
        results
    }

    // ---------------------------------------------------------------------------------------------
    // Surface Synchronization & Display-Change Handling (Step 41)
    // ---------------------------------------------------------------------------------------------

    /// Queries the surface synchronization state for an attached window.
    #[must_use]
    pub fn get_sync_state(&self, hwnd: NativeWindowHandle) -> SurfaceSyncState {
        self.sync_metadata
            .get(&hwnd)
            .map(|m| m.state)
            .unwrap_or(SurfaceSyncState::NoSurfaceAttached)
    }

    /// Queries the full surface synchronization metadata for an attached window.
    #[must_use]
    pub fn get_sync_metadata(&self, hwnd: NativeWindowHandle) -> Option<&SurfaceSyncMetadata> {
        self.sync_metadata.get(&hwnd)
    }

    /// Returns the diagnostic history for surface synchronization events.
    #[must_use]
    pub fn get_sync_diagnostic_events(&self) -> &[SurfaceSyncEvent] {
        &self.sync_diagnostic_events
    }

    /// Sets the synchronizing flag for testing re-entrancy guards.
    pub fn set_synchronizing_flag_for_testing(
        &mut self,
        hwnd: NativeWindowHandle,
        synchronizing: bool,
    ) {
        if let Some(m) = self.sync_metadata.get_mut(&hwnd) {
            m.is_synchronizing = synchronizing;
        }
    }

    /// Clears recorded diagnostic synchronization events.
    pub fn clear_sync_diagnostic_events(&mut self) {
        self.sync_diagnostic_events.clear();
    }

    /// Records a diagnostic event in the bounded synchronization event ring.
    #[allow(clippy::too_many_arguments)]
    pub fn record_sync_diagnostic(
        &mut self,
        source: NativeWindowHandle,
        target: NativeWindowHandle,
        instance: Uuid,
        generation: u64,
        description: String,
        old_bounds: Option<WindowRect>,
        new_bounds: Option<WindowRect>,
        outcome_state: SurfaceSyncState,
    ) {
        let now_millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        if self.sync_diagnostic_events.len() >= MAX_SYNC_DIAGNOSTIC_EVENTS {
            self.sync_diagnostic_events.remove(0);
        }

        self.sync_diagnostic_events.push(SurfaceSyncEvent {
            source_handle: source,
            host_handle: target,
            player_instance_id: instance,
            generation,
            description,
            old_bounds,
            new_bounds,
            outcome_state,
            timestamp_millis: now_millis,
        });
    }

    /// Synchronizes the geometry of an already-attached wallpaper surface to the target bounds.
    ///
    /// Implements coalescing (skips Win32 calls if bounds unchanged), anti-loop guards
    /// (ignoring self-induced repositioning events), and focus preservation.
    pub fn synchronize_surface_geometry(
        &mut self,
        source: NativeWindowHandle,
        target_bounds: WindowRect,
    ) -> Result<SurfaceSyncReport, SurfaceSyncError> {
        if self.is_shut_down {
            return Err(SurfaceSyncError::CoordinatorShutDown);
        }

        // Validate window is tracked and attached
        let (host_handle, player_instance_id) = {
            let meta = self
                .sync_metadata
                .get_mut(&source)
                .ok_or(SurfaceSyncError::NotAttached(source))?;

            if meta.is_synchronizing {
                // Re-entrancy / self-generated event guard
                tracing::debug!(source = %source, "Re-entrant synchronization event ignored");
                let now_millis = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                return Ok(SurfaceSyncReport {
                    source_handle: source,
                    host_handle: meta.target_handle,
                    player_instance_id: meta.player_instance_id,
                    sync_generation: meta.sync_generation,
                    previous_bounds: meta.last_verified_bounds,
                    effective_bounds: meta.last_verified_bounds.unwrap_or(target_bounds),
                    mutation_applied: false,
                    state: meta.state,
                    synced_at_millis: now_millis,
                });
            }

            // Validate requested dimensions
            if target_bounds.is_empty() || target_bounds.width() <= 0 || target_bounds.height() <= 0
            {
                meta.state = SurfaceSyncState::SynchronizationFailed;
                return Err(SurfaceSyncError::InvalidGeometry {
                    bounds: target_bounds,
                    reason: "Requested surface bounds must have strictly positive width and height"
                        .to_string(),
                });
            }

            // Anti-thrashing / coalescing check:
            // If already at target bounds and host is still valid, return no-op report without invoking Win32 SetWindowPos
            if meta.last_verified_bounds == Some(target_bounds)
                && meta.state == SurfaceSyncState::Synchronized
            {
                let now_millis = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;

                return Ok(SurfaceSyncReport {
                    source_handle: source,
                    host_handle: meta.target_handle,
                    player_instance_id: meta.player_instance_id,
                    sync_generation: meta.sync_generation,
                    previous_bounds: meta.last_verified_bounds,
                    effective_bounds: target_bounds,
                    mutation_applied: false,
                    state: SurfaceSyncState::Synchronized,
                    synced_at_millis: now_millis,
                });
            }

            meta.is_synchronizing = true;
            meta.state = SurfaceSyncState::Synchronizing;
            (meta.target_handle, meta.player_instance_id)
        };

        // Check if windows still exist
        if !self.provider.is_window(source) {
            if let Some(m) = self.sync_metadata.get_mut(&source) {
                m.is_synchronizing = false;
                m.state = SurfaceSyncState::SynchronizationFailed;
            }
            return Err(SurfaceSyncError::SourceInvalidHandle(source));
        }

        if !self.provider.is_window(host_handle) {
            if let Some(m) = self.sync_metadata.get_mut(&source) {
                m.is_synchronizing = false;
                m.state = SurfaceSyncState::HostUnavailable;
            }
            return Err(SurfaceSyncError::HostInvalidHandle(host_handle));
        }

        // Apply repositioning:
        // SWP_NOZORDER ensures we don't disturb Z-order (stays behind desktop icons).
        // SWP_NOACTIVATE ensures foreground focus is NEVER stolen.
        // SWP_SHOWWINDOW ensures the surface remains visible.
        let flags = constants::SWP_NOACTIVATE | constants::SWP_NOZORDER | constants::SWP_SHOWWINDOW;
        let set_res = self.provider.set_window_pos(
            source,
            None,
            target_bounds.left,
            target_bounds.top,
            target_bounds.width(),
            target_bounds.height(),
            flags,
        );

        if let Err(e) = set_res {
            if let Some(m) = self.sync_metadata.get_mut(&source) {
                m.is_synchronizing = false;
                m.state = SurfaceSyncState::SynchronizationFailed;
            }
            return Err(SurfaceSyncError::NativePositioningFailed(e.to_string()));
        }

        // Verification phase
        let observed_bounds = match self.provider.get_window_rect(source) {
            Ok(b) => b,
            Err(e) => {
                if let Some(m) = self.sync_metadata.get_mut(&source) {
                    m.is_synchronizing = false;
                    m.state = SurfaceSyncState::SynchronizationFailed;
                }
                return Err(SurfaceSyncError::NativePositioningFailed(e.to_string()));
            }
        };

        // For child windows, get_window_rect returns screen coordinates; width & height must match target_bounds
        if observed_bounds.width() != target_bounds.width()
            || observed_bounds.height() != target_bounds.height()
        {
            if let Some(m) = self.sync_metadata.get_mut(&source) {
                m.is_synchronizing = false;
                m.state = SurfaceSyncState::SynchronizationFailed;
            }
            return Err(SurfaceSyncError::VerificationFailed {
                expected: target_bounds,
                observed: observed_bounds,
            });
        }

        // Finalize metadata update
        let now_millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let (sync_generation, previous_bounds) = {
            let meta = self
                .sync_metadata
                .get_mut(&source)
                .ok_or(SurfaceSyncError::NotAttached(source))?;
            let prev = meta.last_verified_bounds;
            meta.sync_generation += 1;
            meta.last_verified_bounds = Some(target_bounds);
            meta.state = SurfaceSyncState::Synchronized;
            meta.is_synchronizing = false;
            meta.last_sync_timestamp_millis = now_millis;
            (meta.sync_generation, prev)
        };

        // Also update the cached attachment record
        if let Some((_, record)) = self.attached_windows.get_mut(&source) {
            record.effective_bounds = target_bounds;
        }

        self.record_sync_diagnostic(
            source,
            host_handle,
            player_instance_id,
            sync_generation,
            format!(
                "Surface geometry synchronized to {}x{} at ({}, {})",
                target_bounds.width(),
                target_bounds.height(),
                target_bounds.left,
                target_bounds.top
            ),
            previous_bounds,
            Some(target_bounds),
            SurfaceSyncState::Synchronized,
        );

        Ok(SurfaceSyncReport {
            source_handle: source,
            host_handle,
            player_instance_id,
            sync_generation,
            previous_bounds,
            effective_bounds: target_bounds,
            mutation_applied: true,
            state: SurfaceSyncState::Synchronized,
            synced_at_millis: now_millis,
        })
    }

    /// Dispatches and processes a display or host change notification event.
    pub fn handle_surface_event(
        &mut self,
        source: NativeWindowHandle,
        event: SurfaceChangeEvent,
    ) -> Result<SurfaceSyncReport, SurfaceSyncError> {
        let host_handle = match self.sync_metadata.get(&source) {
            Some(m) => m.target_handle,
            None => return Err(SurfaceSyncError::NotAttached(source)),
        };

        match event {
            SurfaceChangeEvent::DisplayResolutionChanged { width, height, .. } => {
                let new_bounds = WindowRect::new(0, 0, width, height);
                self.synchronize_surface_geometry(source, new_bounds)
            }
            SurfaceChangeEvent::DpiChanged { suggested_rect, .. } => {
                let new_bounds = if let Some(rect) = suggested_rect {
                    rect
                } else {
                    self.provider
                        .get_window_rect(host_handle)
                        .map_err(|e| SurfaceSyncError::NativePositioningFailed(e.to_string()))?
                };
                self.synchronize_surface_geometry(source, new_bounds)
            }
            SurfaceChangeEvent::HostGeometryChanged {
                host_handle: event_host,
                new_bounds,
            } => {
                if event_host != host_handle {
                    return Err(SurfaceSyncError::HostInvalidHandle(event_host));
                }
                self.synchronize_surface_geometry(source, new_bounds)
            }
            SurfaceChangeEvent::HostWindowDestroyed(event_host)
            | SurfaceChangeEvent::ParentRelationshipLost(event_host) => {
                if event_host != host_handle {
                    return Err(SurfaceSyncError::HostInvalidHandle(event_host));
                }
                self.handle_host_invalidation(source)
            }
            SurfaceChangeEvent::ManualSyncRequested => {
                let bounds = self
                    .provider
                    .get_window_rect(host_handle)
                    .map_err(|e| SurfaceSyncError::NativePositioningFailed(e.to_string()))?;
                self.synchronize_surface_geometry(source, bounds)
            }
        }
    }

    /// Recovers an attached surface when its hosting window is destroyed or invalidated.
    ///
    /// Safeguards the surface by unparenting it cleanly, re-discovering a fresh valid
    /// desktop host candidate via Step 39 discovery & compatibility policy, and executing
    /// a reattachment transaction via Step 40. Bounded by a maximum of 3 recovery attempts.
    pub fn handle_host_invalidation(
        &mut self,
        source: NativeWindowHandle,
    ) -> Result<SurfaceSyncReport, SurfaceSyncError> {
        if self.is_shut_down {
            return Err(SurfaceSyncError::CoordinatorShutDown);
        }

        let (attempts, max_attempts, instance_id, owning_pid, prev_bounds) = {
            let meta = self
                .sync_metadata
                .get_mut(&source)
                .ok_or(SurfaceSyncError::NotAttached(source))?;

            meta.state = SurfaceSyncState::HostUnavailable;

            if meta.recovery_attempts >= meta.max_recovery_attempts {
                meta.state = SurfaceSyncState::SynchronizationFailed;
                return Err(SurfaceSyncError::RecoveryBudgetExhausted {
                    attempts: meta.recovery_attempts,
                    max: meta.max_recovery_attempts,
                });
            }

            meta.recovery_attempts += 1;
            meta.state = SurfaceSyncState::RecoveryInProgress;
            (
                meta.recovery_attempts,
                meta.max_recovery_attempts,
                meta.player_instance_id,
                self.attached_windows
                    .get(&source)
                    .map(|(orig, _)| orig.process_id)
                    .unwrap_or(0),
                meta.last_verified_bounds,
            )
        };

        tracing::warn!(
            source = %source,
            attempt = attempts,
            max = max_attempts,
            "Host invalidation detected; initiating desktop host rediscovery and safe reattachment"
        );

        // Step A: Safely unparent the source window first to prevent hanging references.
        let _ = self.provider.set_parent(source, None);

        // Remove from attached_windows temporarily so attach() will accept it
        let (original_state, _) = match self.attached_windows.remove(&source) {
            Some(pair) => pair,
            None => {
                if let Some(m) = self.sync_metadata.get_mut(&source) {
                    m.state = SurfaceSyncState::SynchronizationFailed;
                }
                return Err(SurfaceSyncError::NotAttached(source));
            }
        };

        // Step B: Run discovery and compatibility policy
        let discovery = DesktopHostDiscovery::new(self.provider.clone());
        let options = DesktopDiscoveryOptions::default();

        let topology_classification = match discovery.discover_topology(&options) {
            Ok(c) => c,
            Err(e) => {
                self.attached_windows.insert(
                    source,
                    (
                        original_state,
                        AttachmentRecord {
                            source_handle: source,
                            target_handle: NativeWindowHandle(0),
                            player_instance_id: instance_id,
                            attached_at_millis: 0,
                            effective_style: 0,
                            effective_ex_style: 0,
                            effective_bounds: prev_bounds.unwrap_or_default(),
                            is_verified: false,
                        },
                    ),
                );
                if let Some(m) = self.sync_metadata.get_mut(&source) {
                    m.state = SurfaceSyncState::SynchronizationFailed;
                }
                return Err(SurfaceSyncError::HostLostAndRecoveryFailed(format!(
                    "Rediscovery failed: {e}"
                )));
            }
        };

        let new_candidate =
            match CompatibilityPolicy::evaluate_attachment_suitability(&topology_classification) {
                Ok(c) => c,
                Err(e) => {
                    self.attached_windows.insert(
                        source,
                        (
                            original_state,
                            AttachmentRecord {
                                source_handle: source,
                                target_handle: NativeWindowHandle(0),
                                player_instance_id: instance_id,
                                attached_at_millis: 0,
                                effective_style: 0,
                                effective_ex_style: 0,
                                effective_bounds: prev_bounds.unwrap_or_default(),
                                is_verified: false,
                            },
                        ),
                    );
                    if let Some(m) = self.sync_metadata.get_mut(&source) {
                        m.state = SurfaceSyncState::SynchronizationFailed;
                    }
                    return Err(SurfaceSyncError::HostLostAndRecoveryFailed(format!(
                        "Candidate compatibility rejected: {e}"
                    )));
                }
            };

        let target_handle = new_candidate.handle;

        // Step C: Re-attach to new candidate using attach()
        let attach_request = AttachmentRequest {
            source_handle: source,
            target_candidate: new_candidate,
            owning_process_id: owning_pid,
            player_instance_id: instance_id,
            target_display_bounds: prev_bounds,
        };

        let attach_result = self.attach(&attach_request);

        match attach_result {
            Ok(new_record) => {
                if let Some(entry) = self.attached_windows.get_mut(&source) {
                    entry.0 = original_state;
                }

                if let Some(m) = self.sync_metadata.get_mut(&source) {
                    m.target_handle = target_handle;
                    m.state = SurfaceSyncState::Synchronized;
                    m.recovery_attempts = 0;
                    m.sync_generation += 1;
                    m.last_verified_bounds = Some(new_record.effective_bounds);
                }

                let now_millis = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;

                let generation = self
                    .sync_metadata
                    .get(&source)
                    .map(|m| m.sync_generation)
                    .unwrap_or(1);

                self.record_sync_diagnostic(
                    source,
                    target_handle,
                    instance_id,
                    generation,
                    format!("Successfully recovered and reattached to host {target_handle}"),
                    prev_bounds,
                    Some(new_record.effective_bounds),
                    SurfaceSyncState::Synchronized,
                );

                Ok(SurfaceSyncReport {
                    source_handle: source,
                    host_handle: target_handle,
                    player_instance_id: instance_id,
                    sync_generation: generation,
                    previous_bounds: prev_bounds,
                    effective_bounds: new_record.effective_bounds,
                    mutation_applied: true,
                    state: SurfaceSyncState::Synchronized,
                    synced_at_millis: now_millis,
                })
            }
            Err(e) => {
                self.attached_windows.insert(
                    source,
                    (
                        original_state,
                        AttachmentRecord {
                            source_handle: source,
                            target_handle: NativeWindowHandle(0),
                            player_instance_id: instance_id,
                            attached_at_millis: 0,
                            effective_style: 0,
                            effective_ex_style: 0,
                            effective_bounds: prev_bounds.unwrap_or_default(),
                            is_verified: false,
                        },
                    ),
                );
                if let Some(m) = self.sync_metadata.get_mut(&source) {
                    m.state = SurfaceSyncState::SynchronizationFailed;
                }
                Err(SurfaceSyncError::HostLostAndRecoveryFailed(format!(
                    "Reattachment transaction failed: {e}"
                )))
            }
        }
    }

    // ---------------------------------------------------------------------------------------------
    // Internal Validation & Restoration Subroutines
    // ---------------------------------------------------------------------------------------------

    fn validate_attachment_eligibility(
        &self,
        request: &AttachmentRequest,
    ) -> Result<(), AttachmentError> {
        let source = request.source_handle;
        let target = request.target_candidate.handle;

        if source.is_null() || !self.provider.is_window(source) {
            return Err(AttachmentError::SourceInvalidHandle(source));
        }

        if target.is_null() || !self.provider.is_window(target) {
            return Err(AttachmentError::TargetInvalidHandle(target));
        }

        if self.prohibited_handles.contains(&source) {
            return Err(AttachmentError::SourceIsManagementUi(source));
        }

        if source == target {
            return Err(AttachmentError::SourceIsDesktopRoot(source));
        }

        // Validate source class name: cannot be shell / desktop windows
        if let Ok(class_name) = self.provider.get_class_name(source) {
            let lower = class_name.to_ascii_lowercase();
            if lower == "progman" || lower == "workstation" {
                return Err(AttachmentError::SourceIsDesktopRoot(source));
            }
            if lower == "shelldll_defview" || lower == "syslistview32" {
                return Err(AttachmentError::SourceIsIconHost(source));
            }
        }

        // Validate source PID ownership
        let (_, pid) = self.provider.get_window_thread_process_id(source);
        if pid != request.owning_process_id && !self.allowed_process_ids.contains(&pid) {
            return Err(AttachmentError::SourceProcessMismatch {
                expected: request.owning_process_id,
                actual: pid,
            });
        }

        // Validate target candidate role: strictly WallpaperHostCandidate
        if request.target_candidate.role != CandidateRole::WallpaperHostCandidate {
            return Err(AttachmentError::TargetIncompatible(format!(
                "Target role `{:?}` is not a valid WallpaperHostCandidate",
                request.target_candidate.role
            )));
        }

        if request.target_candidate.has_shelldll_defview_child {
            return Err(AttachmentError::TargetIncompatible(
                "Target candidate window contains desktop icons (SHELLDLL_DefView)".to_string(),
            ));
        }

        // Check DPI awareness compatibility
        let source_dpi = self.provider.get_window_dpi_awareness_context(source);
        let target_dpi = self.provider.get_window_dpi_awareness_context(target);
        if let (Some(s), Some(t)) = (source_dpi, target_dpi) {
            if !self.provider.are_dpi_awareness_contexts_equal(s, t) {
                return Err(AttachmentError::DpiIncompatible {
                    source_ctx: source_dpi,
                    target_ctx: target_dpi,
                });
            }
        }

        Ok(())
    }

    fn capture_original_state(
        &self,
        request: &AttachmentRequest,
    ) -> Result<OriginalWindowState, AttachmentError> {
        let source = request.source_handle;
        let parent = self.provider.get_parent(source);
        let style = self
            .provider
            .get_window_long(source, constants::GWL_STYLE)?;
        let ex_style = self
            .provider
            .get_window_long(source, constants::GWL_EXSTYLE)?;
        let bounds = self.provider.get_window_rect(source)?;
        let visible = self.provider.is_window_visible(source);
        let (tid, pid) = self.provider.get_window_thread_process_id(source);

        let now_millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Ok(OriginalWindowState {
            source_handle: source,
            original_parent: parent,
            original_style: style,
            original_ex_style: ex_style,
            original_bounds: bounds,
            original_visible: visible,
            process_id: pid,
            thread_id: tid,
            player_instance_id: request.player_instance_id,
            target_handle: request.target_candidate.handle,
            captured_at_millis: now_millis,
        })
    }

    fn verify_attached_state(
        &self,
        source: NativeWindowHandle,
        target: NativeWindowHandle,
        expected_style: isize,
    ) -> Result<(), AttachmentError> {
        let current_parent = self.provider.get_parent(source);
        if current_parent != Some(target) {
            return Err(AttachmentError::VerificationFailed(format!(
                "Parent verification mismatch: expected {target}, actual {current_parent:?}"
            )));
        }

        let current_style = self
            .provider
            .get_window_long(source, constants::GWL_STYLE)?;
        if (current_style & constants::WS_CHILD) == 0 {
            return Err(AttachmentError::VerificationFailed(
                "Style verification mismatch: WS_CHILD bit is not set".to_string(),
            ));
        }
        if (current_style & constants::WS_POPUP) != 0 {
            return Err(AttachmentError::VerificationFailed(
                "Style verification mismatch: WS_POPUP bit is still set".to_string(),
            ));
        }
        if current_style != expected_style {
            return Err(AttachmentError::VerificationFailed(format!(
                "Style verification mismatch: expected 0x{expected_style:08X}, actual 0x{current_style:08X}"
            )));
        }

        let bounds = self.provider.get_window_rect(source)?;
        if bounds.is_empty() {
            return Err(AttachmentError::VerificationFailed(format!(
                "Bounds verification mismatch: bounds are empty {bounds:?}"
            )));
        }

        if !self.provider.is_window_visible(source) {
            return Err(AttachmentError::VerificationFailed(
                "Visibility verification mismatch: window is not visible".to_string(),
            ));
        }

        Ok(())
    }

    fn abort_and_rollback(
        &mut self,
        source: NativeWindowHandle,
        original_state: &OriginalWindowState,
        stage: AttachmentStage,
        cause: AttachmentError,
    ) -> Result<AttachmentRecord, AttachmentError> {
        self.window_states
            .insert(source, AttachmentState::Restoring);
        tracing::warn!(
            source = %source,
            stage = ?stage,
            cause = %cause,
            "Attachment mutation failed; initiating rollback"
        );

        let outcome = self.execute_restoration(source, original_state, stage);
        self.window_states.insert(source, AttachmentState::Failed);
        self.active_requests.remove(&source);

        Err(AttachmentError::RollbackTriggered {
            cause: Box::new(cause),
            outcome,
        })
    }

    fn execute_restoration(
        &self,
        source: NativeWindowHandle,
        orig: &OriginalWindowState,
        stage: AttachmentStage,
    ) -> RollbackOutcome {
        if !self.provider.is_window(source) {
            return RollbackOutcome::SourceWindowDestroyed;
        }

        // Restore parent if SetParent was called
        if stage >= AttachmentStage::ParentAssigned {
            if let Err(e) = self.provider.set_parent(source, orig.original_parent) {
                return RollbackOutcome::RollbackFailed {
                    error: format!("Failed to restore original parent: {e}"),
                };
            }
        }

        // Restore styles if styles were changed
        if stage >= AttachmentStage::StylePrepared {
            let _ =
                self.provider
                    .set_window_long(source, constants::GWL_STYLE, orig.original_style);
            let _ = self.provider.set_window_long(
                source,
                constants::GWL_EXSTYLE,
                orig.original_ex_style,
            );
            let _ = self.provider.set_window_pos(
                source,
                None,
                0,
                0,
                0,
                0,
                constants::SWP_NOMOVE
                    | constants::SWP_NOSIZE
                    | constants::SWP_NOZORDER
                    | constants::SWP_NOACTIVATE
                    | constants::SWP_FRAMECHANGED,
            );
        }

        // Restore bounds and visibility if bounds were changed
        if stage >= AttachmentStage::BoundsApplied {
            let show_flag = if orig.original_visible {
                constants::SWP_SHOWWINDOW
            } else {
                constants::SWP_HIDEWINDOW
            };

            let _ = self.provider.set_window_pos(
                source,
                None,
                orig.original_bounds.left,
                orig.original_bounds.top,
                orig.original_bounds.width(),
                orig.original_bounds.height(),
                constants::SWP_NOACTIVATE | show_flag,
            );
        }

        // Verify restoration
        let restored_parent = self.provider.get_parent(source);
        let restored_style = self
            .provider
            .get_window_long(source, constants::GWL_STYLE)
            .unwrap_or(0);

        let parent_ok = restored_parent == orig.original_parent;
        let style_ok = restored_style == orig.original_style;

        RollbackOutcome::RollbackCompleted {
            verified: parent_ok && style_ok,
        }
    }
}
