//! Per-monitor playback session coordinator, assignment activation, and lifecycle management (Step 44).
//!
//! Provides an orchestration layer that resolves per-monitor wallpaper assignments,
//! manages independent per-display playback sessions, synchronizes surface bounds,
//! isolates failures per display, and guarantees orderly detachment and cleanup.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use kinetiwall_catalog::WallpaperId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::assignment::{AssignmentResolutionOutcome, ResolvedDisplayAssignment};
use crate::attachment::{AttachmentCoordinator, AttachmentRequest};
use crate::display::{DisplayGeometry, DisplayId, DisplayTopologySnapshot};
use crate::provider::DesktopWindowProvider;
use crate::types::{DesktopCandidateWindow, NativeWindowHandle};

/// Maximum number of diagnostic events retained in memory.
pub const MAX_SESSION_DIAGNOSTIC_EVENTS: usize = 100;

// -------------------------------------------------------------------------------------------------
// Domain Types: Session Identity and State
// -------------------------------------------------------------------------------------------------

/// Unique identifier for a per-monitor playback session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DisplaySessionId(pub Uuid);

impl DisplaySessionId {
    /// Generates a new random session identifier.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for DisplaySessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for DisplaySessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Operational state of an individual display playback session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DisplaySessionState {
    /// Session allocated but native surface has not yet been initialized.
    Uninitialized,
    /// Surface created and player backend is preparing to render.
    Starting,
    /// Surface is attached to desktop host and actively rendering on target display bounds.
    Active,
    /// Player window is detached from host (e.g. during paused state or desktop re-hosting).
    Detached,
    /// Session is temporarily suspended (e.g. display sleep, lock screen, or battery saving).
    Suspended,
    /// Session is displaying a static fallback image (e.g. low power, poster, or frozen frame).
    StaticFallback { fallback_source: String },
    /// Session encountered an error; failure is isolated to this monitor only.
    Failed { reason: String },
    /// Session is undergoing orderly detachment and surface teardown.
    Stopping,
    /// Session has completed teardown and native resources are released.
    Stopped,
}

impl DisplaySessionState {
    /// Returns true if the session is currently active and attached.
    #[must_use]
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active)
    }

    /// Returns true if the session is currently displaying a static fallback image.
    #[must_use]
    pub fn is_fallback(&self) -> bool {
        matches!(self, Self::StaticFallback { .. })
    }

    /// Returns true if the session is in a terminal or stopped state.
    #[must_use]
    pub fn is_terminated(&self) -> bool {
        matches!(self, Self::Stopped | Self::Failed { .. })
    }
}

/// Detailed descriptor of an active or tracked per-monitor playback session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplaySessionDescriptor {
    /// Unique identifier for this session.
    pub session_id: DisplaySessionId,

    /// Target display identifier.
    pub display_id: DisplayId,

    /// Currently assigned wallpaper identifier.
    pub wallpaper_id: WallpaperId,

    /// Current operational state.
    pub state: DisplaySessionState,

    /// Native window handle for the player surface, if created.
    pub source_handle: Option<NativeWindowHandle>,

    /// Desktop host candidate window handle (e.g. WorkerW), if attached.
    pub host_handle: Option<NativeWindowHandle>,

    /// Target display geometry and bounds in virtual desktop coordinates.
    pub geometry: DisplayGeometry,

    /// Monotonically increasing generation number guarding against stale events.
    pub generation: u64,

    /// Timestamp when this session was created (Unix epoch millis).
    pub created_at_millis: u64,

    /// Timestamp when this session was last updated (Unix epoch millis).
    pub last_updated_millis: u64,
}

// -------------------------------------------------------------------------------------------------
// Error Taxonomy
// -------------------------------------------------------------------------------------------------

/// Strongly typed errors for per-monitor playback session management.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum SessionError {
    #[error("Session `{0}` not found")]
    SessionNotFound(DisplaySessionId),

    #[error("Display `{0}` already has an active session `{1}`")]
    DisplayAlreadyHasSession(DisplayId, DisplaySessionId),

    #[error("Display `{0}` has no active session")]
    DisplayHasNoSession(DisplayId),

    #[error("Display `{0}` is not connected in the active topology")]
    DisplayNotConnected(DisplayId),

    #[error("Display `{0}` has no playable assignment: outcome `{1:?}`")]
    DisplayNotPlayable(DisplayId, AssignmentResolutionOutcome),

    #[error("Host candidate `{0}` is invalid or unavailable")]
    HostCandidateInvalid(NativeWindowHandle),

    #[error("Surface creation failed for display `{0}`: {1}")]
    SurfaceCreationFailed(DisplayId, String),

    #[error("Attachment failed for display `{0}`: {1}")]
    AttachmentFailed(DisplayId, String),

    #[error("Surface synchronization failed for display `{0}`: {1}")]
    SynchronizationFailed(DisplayId, String),

    #[error("Coordinator has been shut down")]
    CoordinatorShutDown,

    #[error(
        "Stale session generation for display `{display_id}`: expected {expected}, got {actual}"
    )]
    StaleGeneration {
        display_id: DisplayId,
        expected: u64,
        actual: u64,
    },
}

// -------------------------------------------------------------------------------------------------
// Session Planning & Reconciliation Actions
// -------------------------------------------------------------------------------------------------

/// Explicit reason why an active session is being stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStopReason {
    /// Monitor was physically disconnected from the desktop topology.
    DisplayDisconnected,
    /// User removed or cleared the assignment for this display.
    AssignmentRemoved,
    /// Target wallpaper is no longer available in the catalog or missing on disk.
    WallpaperUnavailable,
    /// Explicit user request to stop wallpaper on this display.
    ExplicitRequest,
    /// Session was replaced by a new assignment or re-orchestration.
    Replaced,
    /// Playback coordinator is shutting down.
    CoordinatorShutdown,
}

impl std::fmt::Display for SessionStopReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DisplayDisconnected => write!(f, "display_disconnected"),
            Self::AssignmentRemoved => write!(f, "assignment_removed"),
            Self::WallpaperUnavailable => write!(f, "wallpaper_unavailable"),
            Self::ExplicitRequest => write!(f, "explicit_request"),
            Self::Replaced => write!(f, "replaced"),
            Self::CoordinatorShutdown => write!(f, "coordinator_shutdown"),
        }
    }
}

/// Declarative delta action for reconciling active sessions against desired state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SessionPlanAction {
    /// Create and activate a new session for an unmanaged, assigned display.
    CreateSession {
        display_id: DisplayId,
        wallpaper_id: WallpaperId,
        geometry: DisplayGeometry,
    },

    /// Retain an already-healthy session without disturbing playback or native hierarchy.
    RetainSession {
        session_id: DisplaySessionId,
        display_id: DisplayId,
        wallpaper_id: WallpaperId,
    },

    /// Update display geometry/resolution of an active session without recreating the player.
    UpdateGeometry {
        session_id: DisplaySessionId,
        display_id: DisplayId,
        new_geometry: DisplayGeometry,
    },

    /// Reconfigure the session with a new wallpaper assignment.
    ReconfigureSession {
        session_id: DisplaySessionId,
        display_id: DisplayId,
        new_wallpaper_id: WallpaperId,
        geometry: DisplayGeometry,
    },

    /// Orderly stop and detach a session whose display disconnected or assignment was removed.
    StopSession {
        session_id: DisplaySessionId,
        display_id: DisplayId,
        reason: SessionStopReason,
    },
}

/// Declarative execution plan produced by comparing active sessions with resolved assignments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPlan {
    /// Ordered list of reconciliation actions.
    pub actions: Vec<SessionPlanAction>,

    /// Topology snapshot generation this plan was computed from.
    pub snapshot_generation: u64,

    /// Timestamp when this plan was created.
    pub plan_timestamp_millis: u64,
}

impl SessionPlan {
    /// Returns true if the plan requires zero mutations (all active sessions are retained).
    #[must_use]
    pub fn is_noop(&self) -> bool {
        self.actions
            .iter()
            .all(|a| matches!(a, SessionPlanAction::RetainSession { .. }))
    }

    /// Returns the number of mutations (create, update, reconfigure, stop).
    #[must_use]
    pub fn mutation_count(&self) -> usize {
        self.actions
            .iter()
            .filter(|a| !matches!(a, SessionPlanAction::RetainSession { .. }))
            .count()
    }
}

/// Report produced after applying a `SessionPlan`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionReconciliationReport {
    /// Number of sessions retained unchanged.
    pub retained_count: usize,

    /// Sessions successfully activated.
    pub activated: Vec<DisplayId>,

    /// Sessions whose geometry was successfully updated.
    pub geometry_updated: Vec<DisplayId>,

    /// Sessions successfully reconfigured with a new wallpaper.
    pub reconfigured: Vec<DisplayId>,

    /// Sessions successfully stopped and detached.
    pub stopped: Vec<DisplayId>,

    /// Individual display failure records (failure on one display does not affect others).
    pub failures: HashMap<DisplayId, String>,

    /// Generation of the coordinator after applying the plan.
    pub coordinator_generation: u64,
}

// -------------------------------------------------------------------------------------------------
// Surface Factory Trait
// -------------------------------------------------------------------------------------------------

/// Abstraction for creating and destroying native player window surfaces.
pub trait SurfaceFactory: Send + Sync {
    /// Creates a dedicated native player surface window for the given display.
    fn create_surface(
        &self,
        display_id: &DisplayId,
        geometry: &DisplayGeometry,
    ) -> Result<NativeWindowHandle, SessionError>;

    /// Destroys a previously created native player surface window.
    fn destroy_surface(&self, handle: NativeWindowHandle) -> Result<(), SessionError>;
}

/// Default surface factory utilizing `DesktopWindowProvider`.
pub struct ProviderSurfaceFactory {
    provider: Arc<dyn DesktopWindowProvider>,
}

impl ProviderSurfaceFactory {
    /// Creates a new factory backed by the window provider.
    #[must_use]
    pub fn new(provider: Arc<dyn DesktopWindowProvider>) -> Self {
        Self { provider }
    }
}

impl SurfaceFactory for ProviderSurfaceFactory {
    fn create_surface(
        &self,
        display_id: &DisplayId,
        geometry: &DisplayGeometry,
    ) -> Result<NativeWindowHandle, SessionError> {
        let title = format!("KinetiWall_Surface_{}", display_id);
        let bounds = geometry.monitor_rect;
        self.provider
            .create_test_window(
                &title,
                bounds.left,
                bounds.top,
                bounds.width(),
                bounds.height(),
            )
            .map_err(|e| SessionError::SurfaceCreationFailed(display_id.clone(), e.to_string()))
    }

    fn destroy_surface(&self, handle: NativeWindowHandle) -> Result<(), SessionError> {
        self.provider.destroy_window(handle).map_err(|e| {
            SessionError::SurfaceCreationFailed(DisplayId("unknown".into()), e.to_string())
        })
    }
}

// -------------------------------------------------------------------------------------------------
// Diagnostics & Event Logging
// -------------------------------------------------------------------------------------------------

/// Operational event classification for session lifecycle diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventType {
    Created,
    Activated,
    Retained,
    GeometryUpdated,
    Reconfigured,
    Suspended,
    Resumed,
    Detached,
    Failed,
    Stopped,
}

/// Structured diagnostic log event for session management.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplaySessionDiagnosticEvent {
    /// Session identifier, if associated with a session.
    pub session_id: Option<DisplaySessionId>,

    /// Target display identifier.
    pub display_id: DisplayId,

    /// Timestamp when the event occurred (Unix epoch millis).
    pub timestamp_millis: u64,

    /// Event category.
    pub event_type: SessionEventType,

    /// Descriptive summary of the lifecycle event or failure details.
    pub description: String,
}

// -------------------------------------------------------------------------------------------------
// Display Session Coordinator Implementation
// -------------------------------------------------------------------------------------------------

/// Orchestrates independent per-monitor playback sessions, reconciles state with saved assignments,
/// and isolates runtime failures across displays.
pub struct DisplaySessionCoordinator {
    provider: Arc<dyn DesktopWindowProvider>,
    attachment_coordinator: Arc<Mutex<AttachmentCoordinator>>,
    surface_factory: Arc<dyn SurfaceFactory>,
    sessions: HashMap<DisplayId, DisplaySessionDescriptor>,
    diagnostic_history: Vec<DisplaySessionDiagnosticEvent>,
    coordinator_generation: u64,
    is_shut_down: bool,
}

impl DisplaySessionCoordinator {
    /// Creates a new `DisplaySessionCoordinator` with explicit components.
    #[must_use]
    pub fn new(
        provider: Arc<dyn DesktopWindowProvider>,
        attachment_coordinator: Arc<Mutex<AttachmentCoordinator>>,
        surface_factory: Arc<dyn SurfaceFactory>,
    ) -> Self {
        Self {
            provider,
            attachment_coordinator,
            surface_factory,
            sessions: HashMap::new(),
            diagnostic_history: Vec::new(),
            coordinator_generation: 1,
            is_shut_down: false,
        }
    }

    /// Creates a coordinator with the default provider-backed surface factory.
    #[must_use]
    pub fn with_default_factory(
        provider: Arc<dyn DesktopWindowProvider>,
        attachment_coordinator: Arc<Mutex<AttachmentCoordinator>>,
    ) -> Self {
        let surface_factory = Arc::new(ProviderSurfaceFactory::new(provider.clone()));
        Self::new(provider, attachment_coordinator, surface_factory)
    }

    /// Monotonically increasing coordinator generation number.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.coordinator_generation
    }

    /// Returns true if the coordinator has been shut down.
    #[must_use]
    pub fn is_shut_down(&self) -> bool {
        self.is_shut_down
    }

    /// Retrieves an active session descriptor for a display ID, if present.
    #[must_use]
    pub fn get_session(&self, display_id: &DisplayId) -> Option<&DisplaySessionDescriptor> {
        self.sessions.get(display_id)
    }

    /// Returns a list of all currently tracked sessions.
    #[must_use]
    pub fn list_sessions(&self) -> Vec<DisplaySessionDescriptor> {
        self.sessions.values().cloned().collect()
    }

    /// Returns the active count of attached and running sessions.
    #[must_use]
    pub fn active_session_count(&self) -> usize {
        self.sessions
            .values()
            .filter(|s| s.state.is_active())
            .count()
    }

    /// Generates a declarative delta plan by comparing active sessions with resolved assignments.
    ///
    /// Preserves healthy running sessions without restarting them, handles disconnected monitors
    /// cleanly, and plans new sessions for unassigned or newly connected displays.
    #[must_use]
    pub fn create_plan(
        &self,
        snapshot: &DisplayTopologySnapshot,
        resolved_assignments: &[ResolvedDisplayAssignment],
    ) -> SessionPlan {
        let mut actions = Vec::new();
        let now = Self::current_timestamp_millis();

        // Index resolved assignments by DisplayId
        let resolved_map: HashMap<&DisplayId, &ResolvedDisplayAssignment> = resolved_assignments
            .iter()
            .map(|r| (&r.display_id, r))
            .collect();

        // Evaluate all resolved assignments
        for resolved in resolved_assignments {
            let display_id = &resolved.display_id;
            let current_session_opt = self.sessions.get(display_id);

            if resolved.is_playable {
                let wallpaper_id = resolved
                    .wallpaper_id
                    .expect("playable assignment must have wallpaper_id");
                let geometry = resolved
                    .target_geometry
                    .expect("playable assignment must have target_geometry");

                match current_session_opt {
                    None => {
                        // New session needed
                        actions.push(SessionPlanAction::CreateSession {
                            display_id: display_id.clone(),
                            wallpaper_id,
                            geometry,
                        });
                    }
                    Some(session) => {
                        if session.state.is_terminated() {
                            // Previous session failed or stopped; replace with new session
                            actions.push(SessionPlanAction::CreateSession {
                                display_id: display_id.clone(),
                                wallpaper_id,
                                geometry,
                            });
                        } else if session.wallpaper_id != wallpaper_id {
                            // Wallpaper changed; reconfigure
                            actions.push(SessionPlanAction::ReconfigureSession {
                                session_id: session.session_id,
                                display_id: display_id.clone(),
                                new_wallpaper_id: wallpaper_id,
                                geometry,
                            });
                        } else if session.geometry != geometry {
                            // Geometry changed; update bounds
                            actions.push(SessionPlanAction::UpdateGeometry {
                                session_id: session.session_id,
                                display_id: display_id.clone(),
                                new_geometry: geometry,
                            });
                        } else {
                            // Preserved without mutation
                            actions.push(SessionPlanAction::RetainSession {
                                session_id: session.session_id,
                                display_id: display_id.clone(),
                                wallpaper_id,
                            });
                        }
                    }
                }
            } else {
                // Not playable (disconnected, unassigned, missing, unavailable)
                if let Some(session) = current_session_opt {
                    if !session.state.is_terminated() {
                        let reason = match resolved.outcome {
                            AssignmentResolutionOutcome::DisplayDisconnected => {
                                SessionStopReason::DisplayDisconnected
                            }
                            AssignmentResolutionOutcome::Unassigned => {
                                SessionStopReason::AssignmentRemoved
                            }
                            _ => SessionStopReason::WallpaperUnavailable,
                        };

                        actions.push(SessionPlanAction::StopSession {
                            session_id: session.session_id,
                            display_id: display_id.clone(),
                            reason,
                        });
                    }
                }
            }
        }

        // Also check any active sessions for displays not in resolved_assignments
        for (display_id, session) in &self.sessions {
            if !resolved_map.contains_key(display_id) && !session.state.is_terminated() {
                actions.push(SessionPlanAction::StopSession {
                    session_id: session.session_id,
                    display_id: display_id.clone(),
                    reason: SessionStopReason::DisplayDisconnected,
                });
            }
        }

        SessionPlan {
            actions,
            snapshot_generation: snapshot.generation,
            plan_timestamp_millis: now,
        }
    }

    /// Applies a declarative `SessionPlan`, activating, reconfiguring, updating, and stopping
    /// sessions with strict failure isolation across displays.
    pub fn apply_plan(
        &mut self,
        plan: &SessionPlan,
        host_candidate: &DesktopCandidateWindow,
    ) -> Result<SessionReconciliationReport, SessionError> {
        if self.is_shut_down {
            return Err(SessionError::CoordinatorShutDown);
        }

        self.coordinator_generation = self.coordinator_generation.wrapping_add(1);

        let mut report = SessionReconciliationReport {
            retained_count: 0,
            activated: Vec::new(),
            geometry_updated: Vec::new(),
            reconfigured: Vec::new(),
            stopped: Vec::new(),
            failures: HashMap::new(),
            coordinator_generation: self.coordinator_generation,
        };

        for action in &plan.actions {
            match action {
                SessionPlanAction::RetainSession {
                    session_id,
                    display_id,
                    ..
                } => {
                    report.retained_count += 1;
                    self.record_diagnostic(
                        Some(*session_id),
                        display_id.clone(),
                        SessionEventType::Retained,
                        format!("Session `{}` retained without changes", session_id),
                    );
                }

                SessionPlanAction::CreateSession {
                    display_id,
                    wallpaper_id,
                    geometry,
                } => {
                    match self.activate_session_internal(
                        display_id,
                        *wallpaper_id,
                        *geometry,
                        host_candidate,
                    ) {
                        Ok(desc) => {
                            report.activated.push(display_id.clone());
                            self.record_diagnostic(
                                Some(desc.session_id),
                                display_id.clone(),
                                SessionEventType::Activated,
                                format!(
                                    "Session `{}` activated for display `{}` with wallpaper `{}`",
                                    desc.session_id, display_id, wallpaper_id
                                ),
                            );
                        }
                        Err(err) => {
                            let err_str = err.to_string();
                            report.failures.insert(display_id.clone(), err_str.clone());
                            // Isolate failure: mark session as Failed if it was tracked
                            if let Some(session) = self.sessions.get_mut(display_id) {
                                session.state = DisplaySessionState::Failed {
                                    reason: err_str.clone(),
                                };
                                session.last_updated_millis = Self::current_timestamp_millis();
                            }
                            self.record_diagnostic(
                                None,
                                display_id.clone(),
                                SessionEventType::Failed,
                                format!(
                                    "Failed to activate session for `{}`: {}",
                                    display_id, err_str
                                ),
                            );
                        }
                    }
                }

                SessionPlanAction::UpdateGeometry {
                    session_id,
                    display_id,
                    new_geometry,
                } => match self.update_geometry_internal(display_id, *new_geometry) {
                    Ok(_) => {
                        report.geometry_updated.push(display_id.clone());
                        self.record_diagnostic(
                            Some(*session_id),
                            display_id.clone(),
                            SessionEventType::GeometryUpdated,
                            format!(
                                "Geometry updated for session `{}` on display `{}` to {:?}",
                                session_id, display_id, new_geometry.monitor_rect
                            ),
                        );
                    }
                    Err(err) => {
                        let err_str = err.to_string();
                        report.failures.insert(display_id.clone(), err_str.clone());
                        self.record_diagnostic(
                            Some(*session_id),
                            display_id.clone(),
                            SessionEventType::Failed,
                            format!(
                                "Failed to update geometry for session `{}`: {}",
                                session_id, err_str
                            ),
                        );
                    }
                },

                SessionPlanAction::ReconfigureSession {
                    session_id,
                    display_id,
                    new_wallpaper_id,
                    geometry,
                } => {
                    match self.reconfigure_session_internal(
                        display_id,
                        *new_wallpaper_id,
                        *geometry,
                        host_candidate,
                    ) {
                        Ok(_) => {
                            report.reconfigured.push(display_id.clone());
                            self.record_diagnostic(
                                Some(*session_id),
                                display_id.clone(),
                                SessionEventType::Reconfigured,
                                format!(
                                    "Session `{}` reconfigured with new wallpaper `{}`",
                                    session_id, new_wallpaper_id
                                ),
                            );
                        }
                        Err(err) => {
                            let err_str = err.to_string();
                            report.failures.insert(display_id.clone(), err_str.clone());
                            self.record_diagnostic(
                                Some(*session_id),
                                display_id.clone(),
                                SessionEventType::Failed,
                                format!(
                                    "Failed to reconfigure session `{}`: {}",
                                    session_id, err_str
                                ),
                            );
                        }
                    }
                }

                SessionPlanAction::StopSession {
                    session_id,
                    display_id,
                    reason,
                } => match self.stop_session_internal(display_id, *reason) {
                    Ok(_) => {
                        report.stopped.push(display_id.clone());
                        self.record_diagnostic(
                            Some(*session_id),
                            display_id.clone(),
                            SessionEventType::Stopped,
                            format!("Session `{}` stopped (reason: {})", session_id, reason),
                        );
                    }
                    Err(err) => {
                        let err_str = err.to_string();
                        report.failures.insert(display_id.clone(), err_str.clone());
                        self.record_diagnostic(
                            Some(*session_id),
                            display_id.clone(),
                            SessionEventType::Failed,
                            format!("Failed to stop session `{}`: {}", session_id, err_str),
                        );
                    }
                },
            }
        }

        Ok(report)
    }

    /// Directly activates a new session on a specified display.
    pub fn activate_session(
        &mut self,
        display_id: &DisplayId,
        wallpaper_id: WallpaperId,
        geometry: DisplayGeometry,
        host_candidate: &DesktopCandidateWindow,
    ) -> Result<DisplaySessionDescriptor, SessionError> {
        if self.is_shut_down {
            return Err(SessionError::CoordinatorShutDown);
        }

        if let Some(existing) = self.sessions.get(display_id) {
            if existing.state.is_active() {
                return Err(SessionError::DisplayAlreadyHasSession(
                    display_id.clone(),
                    existing.session_id,
                ));
            }
        }

        self.activate_session_internal(display_id, wallpaper_id, geometry, host_candidate)
    }

    /// Internal implementation of session activation.
    fn activate_session_internal(
        &mut self,
        display_id: &DisplayId,
        wallpaper_id: WallpaperId,
        geometry: DisplayGeometry,
        host_candidate: &DesktopCandidateWindow,
    ) -> Result<DisplaySessionDescriptor, SessionError> {
        let now = Self::current_timestamp_millis();
        let session_id = DisplaySessionId::new();

        // 1. Allocate session record in Starting state
        let mut desc = DisplaySessionDescriptor {
            session_id,
            display_id: display_id.clone(),
            wallpaper_id,
            state: DisplaySessionState::Starting,
            source_handle: None,
            host_handle: Some(host_candidate.handle),
            geometry,
            generation: 1,
            created_at_millis: now,
            last_updated_millis: now,
        };

        // 2. Create native player surface window
        let surface_handle = match self.surface_factory.create_surface(display_id, &geometry) {
            Ok(h) => h,
            Err(e) => {
                desc.state = DisplaySessionState::Failed {
                    reason: e.to_string(),
                };
                self.sessions.insert(display_id.clone(), desc);
                return Err(e);
            }
        };

        desc.source_handle = Some(surface_handle);

        // 3. Attach surface to desktop host candidate using AttachmentCoordinator
        let (_tid, pid) = self.provider.get_window_thread_process_id(surface_handle);
        let attach_req = AttachmentRequest {
            source_handle: surface_handle,
            target_candidate: host_candidate.clone(),
            player_instance_id: session_id.0,
            owning_process_id: pid,
            target_display_bounds: Some(geometry.monitor_rect),
        };

        let attach_res = {
            let mut coord = self.attachment_coordinator.lock().unwrap();
            coord.attach(&attach_req)
        };

        match attach_res {
            Ok(_record) => {
                desc.state = DisplaySessionState::Active;
                desc.last_updated_millis = Self::current_timestamp_millis();
                self.sessions.insert(display_id.clone(), desc.clone());
                Ok(desc)
            }
            Err(err) => {
                // Rollback: destroy created surface
                let _ = self.surface_factory.destroy_surface(surface_handle);
                let err_str = err.to_string();
                desc.state = DisplaySessionState::Failed {
                    reason: err_str.clone(),
                };
                desc.source_handle = None;
                self.sessions.insert(display_id.clone(), desc);
                Err(SessionError::AttachmentFailed(display_id.clone(), err_str))
            }
        }
    }

    /// Internal implementation of updating geometry for an active session.
    fn update_geometry_internal(
        &mut self,
        display_id: &DisplayId,
        new_geometry: DisplayGeometry,
    ) -> Result<(), SessionError> {
        let session = self
            .sessions
            .get_mut(display_id)
            .ok_or_else(|| SessionError::DisplayHasNoSession(display_id.clone()))?;

        let source = session
            .source_handle
            .ok_or_else(|| SessionError::DisplayHasNoSession(display_id.clone()))?;

        let sync_res = {
            let mut coord = self.attachment_coordinator.lock().unwrap();
            coord.synchronize_surface_geometry(source, new_geometry.monitor_rect)
        };

        match sync_res {
            Ok(report) => {
                session.geometry = new_geometry;
                session.generation = session.generation.wrapping_add(1);
                session.last_updated_millis = Self::current_timestamp_millis();
                tracing::debug!(
                    display = %display_id,
                    applied = report.mutation_applied,
                    "Surface geometry updated"
                );
                Ok(())
            }
            Err(err) => {
                let err_str = err.to_string();
                session.state = DisplaySessionState::Failed {
                    reason: err_str.clone(),
                };
                Err(SessionError::SynchronizationFailed(
                    display_id.clone(),
                    err_str,
                ))
            }
        }
    }

    /// Internal implementation of reconfiguring a session with a new wallpaper.
    fn reconfigure_session_internal(
        &mut self,
        display_id: &DisplayId,
        new_wallpaper_id: WallpaperId,
        geometry: DisplayGeometry,
        host_candidate: &DesktopCandidateWindow,
    ) -> Result<(), SessionError> {
        // Stop current session first
        let _ = self.stop_session_internal(display_id, SessionStopReason::Replaced);

        // Activate new session
        self.activate_session_internal(display_id, new_wallpaper_id, geometry, host_candidate)?;
        Ok(())
    }

    /// Orderly stops and detaches an active session on a display.
    pub fn stop_session(
        &mut self,
        display_id: &DisplayId,
        reason: SessionStopReason,
    ) -> Result<DisplaySessionDescriptor, SessionError> {
        self.stop_session_internal(display_id, reason)
    }

    /// Internal implementation of session stoppage.
    fn stop_session_internal(
        &mut self,
        display_id: &DisplayId,
        _reason: SessionStopReason,
    ) -> Result<DisplaySessionDescriptor, SessionError> {
        let session = self
            .sessions
            .get_mut(display_id)
            .ok_or_else(|| SessionError::DisplayHasNoSession(display_id.clone()))?;

        session.state = DisplaySessionState::Stopping;
        let source_opt = session.source_handle;

        if let Some(source) = source_opt {
            // Detach from host window
            let detach_res = {
                let mut coord = self.attachment_coordinator.lock().unwrap();
                coord.detach(source)
            };

            if let Err(e) = detach_res {
                tracing::warn!(
                    source = %source,
                    display = %display_id,
                    error = %e,
                    "Detachment warning during session stop"
                );
            }

            // Destroy surface window
            let _ = self.surface_factory.destroy_surface(source);
        }

        session.source_handle = None;
        session.state = DisplaySessionState::Stopped;
        session.last_updated_millis = Self::current_timestamp_millis();

        let desc = session.clone();
        Ok(desc)
    }

    /// Suspends playback on a display (e.g. power saving).
    pub fn suspend_session(&mut self, display_id: &DisplayId) -> Result<(), SessionError> {
        let session = self
            .sessions
            .get_mut(display_id)
            .ok_or_else(|| SessionError::DisplayHasNoSession(display_id.clone()))?;

        if session.state.is_active() {
            session.state = DisplaySessionState::Suspended;
            session.last_updated_millis = Self::current_timestamp_millis();
            let session_id = session.session_id;
            self.record_diagnostic(
                Some(session_id),
                display_id.clone(),
                SessionEventType::Suspended,
                format!("Session `{}` suspended", session_id),
            );
        }
        Ok(())
    }

    /// Transitions an active display session to static wallpaper fallback presentation.
    pub fn transition_to_static_fallback(
        &mut self,
        display_id: &DisplayId,
        fallback_source: &str,
    ) -> Result<(), SessionError> {
        let session = self
            .sessions
            .get_mut(display_id)
            .ok_or_else(|| SessionError::DisplayHasNoSession(display_id.clone()))?;

        if session.state.is_active() || matches!(session.state, DisplaySessionState::Suspended) {
            session.state = DisplaySessionState::StaticFallback {
                fallback_source: fallback_source.to_string(),
            };
            session.last_updated_millis = Self::current_timestamp_millis();
            let session_id = session.session_id;
            self.record_diagnostic(
                Some(session_id),
                display_id.clone(),
                SessionEventType::Suspended,
                format!(
                    "Session `{}` transitioned to static fallback (source: {})",
                    session_id, fallback_source
                ),
            );
        }
        Ok(())
    }

    /// Resumes playback on a suspended or static fallback display session.
    pub fn resume_session(&mut self, display_id: &DisplayId) -> Result<(), SessionError> {
        let session = self
            .sessions
            .get_mut(display_id)
            .ok_or_else(|| SessionError::DisplayHasNoSession(display_id.clone()))?;

        if matches!(
            session.state,
            DisplaySessionState::Suspended | DisplaySessionState::StaticFallback { .. }
        ) {
            session.state = DisplaySessionState::Active;
            session.last_updated_millis = Self::current_timestamp_millis();
            let session_id = session.session_id;
            self.record_diagnostic(
                Some(session_id),
                display_id.clone(),
                SessionEventType::Resumed,
                format!("Session `{}` resumed", session_id),
            );
        }
        Ok(())
    }

    /// Validates an incoming callback or event generation against the active session.
    pub fn validate_generation(
        &self,
        display_id: &DisplayId,
        generation: u64,
    ) -> Result<(), SessionError> {
        let session = self
            .sessions
            .get(display_id)
            .ok_or_else(|| SessionError::DisplayHasNoSession(display_id.clone()))?;

        if session.generation != generation {
            return Err(SessionError::StaleGeneration {
                display_id: display_id.clone(),
                expected: session.generation,
                actual: generation,
            });
        }
        Ok(())
    }

    /// Orderly shuts down all active sessions, detaching surfaces and cleaning up windows.
    pub fn shutdown(&mut self) -> Result<(), SessionError> {
        if self.is_shut_down {
            return Ok(());
        }

        self.is_shut_down = true;
        let display_ids: Vec<DisplayId> = self.sessions.keys().cloned().collect();

        for id in display_ids {
            let _ = self.stop_session_internal(&id, SessionStopReason::CoordinatorShutdown);
        }

        // Shut down attachment coordinator
        {
            let mut coord = self.attachment_coordinator.lock().unwrap();
            let _ = coord.detach_all_shutdown();
        }

        self.record_diagnostic(
            None,
            DisplayId("all".into()),
            SessionEventType::Stopped,
            "Coordinator orderly shutdown completed".to_string(),
        );

        Ok(())
    }

    /// Returns a copy of the diagnostic event history.
    #[must_use]
    pub fn get_diagnostics(&self) -> Vec<DisplaySessionDiagnosticEvent> {
        self.diagnostic_history.clone()
    }

    /// Clears the diagnostic event history.
    pub fn clear_diagnostics(&mut self) {
        self.diagnostic_history.clear();
    }

    /// Appends a diagnostic event into the bounded ring buffer.
    fn record_diagnostic(
        &mut self,
        session_id: Option<DisplaySessionId>,
        display_id: DisplayId,
        event_type: SessionEventType,
        description: String,
    ) {
        if self.diagnostic_history.len() >= MAX_SESSION_DIAGNOSTIC_EVENTS {
            self.diagnostic_history.remove(0);
        }

        self.diagnostic_history.push(DisplaySessionDiagnosticEvent {
            session_id,
            display_id,
            timestamp_millis: Self::current_timestamp_millis(),
            event_type,
            description,
        });
    }

    fn current_timestamp_millis() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}
