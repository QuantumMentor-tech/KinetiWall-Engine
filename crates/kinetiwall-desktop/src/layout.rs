//! Multi-monitor layout modes, wallpaper spanning, and synchronized clone playback (Step 45).
//!
//! Provides a unified layout-mode model supporting Independent, Span, and Clone presentations,
//! with validated topology requirements, mode-specific assignment resolution, virtual desktop
//! geometry composition, safe playback coordination, and reversible runtime transitions.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use kinetiwall_catalog::{AvailabilityState, WallpaperCatalogRepository, WallpaperId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::assignment::{
    AssignmentError, AssignmentResolutionOutcome, AssignmentService, ResolvedDisplayAssignment,
};
use crate::display::{DisplayDescriptor, DisplayGeometry, DisplayId, DisplayTopologySnapshot};
use crate::session::{DisplaySessionCoordinator, SessionError, SessionStopReason};
use crate::types::{DesktopCandidateWindow, WindowRect};

/// Maximum number of layout diagnostic events retained in memory.
pub const MAX_LAYOUT_DIAGNOSTIC_EVENTS: usize = 100;

/// Well-known synthetic display identifier for a full-desktop Span session.
pub const SPAN_VIRTUAL_DESKTOP_DISPLAY_ID: &str = "disp_SPAN_VIRTUAL_DESKTOP";

// -------------------------------------------------------------------------------------------------
// Domain Types: Multi-Monitor Presentation Modes
// -------------------------------------------------------------------------------------------------

/// Supported multi-monitor presentation modes for wallpaper display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MultiMonitorLayoutMode {
    /// Each display resolves its own wallpaper assignment and has its own independent session.
    #[default]
    Independent,

    /// A single continuous wallpaper presentation across the union of participating displays
    /// covering the virtual desktop bounds.
    Span,

    /// The exact same wallpaper content presented across multiple participating displays.
    Clone,
}

impl std::fmt::Display for MultiMonitorLayoutMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Independent => write!(f, "independent"),
            Self::Span => write!(f, "span"),
            Self::Clone => write!(f, "clone"),
        }
    }
}

/// Policy for resolving the authoritative wallpaper in Clone mode when displays have differing assignments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CloneSourcePolicy {
    /// Uses the wallpaper assigned to the primary display (origin 0,0).
    #[default]
    PrimaryDisplay,

    /// Uses an explicitly specified wallpaper ID for the clone group.
    ExplicitWallpaper(WallpaperId),

    /// Requires all participating displays to already match assignments; returns conflict if they differ.
    StrictMatch,
}

/// Composition policy for Span mode determining the participating display scope and scaling.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SpanCompositionPolicy {
    /// Spans across the entire virtual desktop bounding box of all connected monitors.
    #[default]
    VirtualDesktopFill,

    /// Spans across all connected monitors, preserving aspect ratio with letterboxing/pillarboxing.
    VirtualDesktopFit,

    /// Spans across a specific designated subset of displays.
    ExplicitDisplays(Vec<DisplayId>),
}

/// Advertising honest playback synchronization capabilities and requirements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackSyncLevel {
    /// Matching content, independent rendering timelines (no synchronization).
    #[default]
    None,

    /// Started concurrently with initial seek/play command; minor clock drift tolerated.
    BestEffort,

    /// Shared clock / periodic timeline seek alignment supported by the backend.
    SharedTimeline,
}

impl std::fmt::Display for PlaybackSyncLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::BestEffort => write!(f, "best_effort"),
            Self::SharedTimeline => write!(f, "shared_timeline"),
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Layout Validation & Geometry Mapping
// -------------------------------------------------------------------------------------------------

/// Validation outcome of evaluating a requested layout mode against the current topology.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum LayoutPlanValidationOutcome {
    /// Plan is internally consistent, topology is valid, and mode is ready for activation.
    Valid,

    /// Only a single display is connected; Span or Clone simplifies to a single-monitor session.
    SingleDisplayOnly,

    /// Conflicting assignments detected in Clone mode with `StrictMatch` policy.
    ConflictingCloneAssignments {
        conflicts: Vec<(DisplayId, WallpaperId)>,
    },

    /// Virtual desktop bounds for Span mode are empty or non-positive.
    InvalidSpanGeometry { reason: String },

    /// Display topology does not meet the requirements for the requested mode.
    TopologyIncompatible { reason: String },

    /// The active player backend cannot support the requested multi-surface or spanning mode.
    BackendCapabilityUnsupported { reason: String },

    /// Required wallpaper asset is missing from catalog or inaccessible on disk.
    MissingWallpaperAsset { wallpaper_id: WallpaperId },

    /// Snapshot generation used to calculate the plan is stale.
    StaleSnapshotGeneration { expected: u64, actual: u64 },
}

impl LayoutPlanValidationOutcome {
    /// Returns true only if the plan can be safely activated.
    #[must_use]
    pub const fn is_activatable(&self) -> bool {
        matches!(self, Self::Valid | Self::SingleDisplayOnly)
    }
}

/// Mapping of a physical/logical display monitor into the virtual desktop composition coordinate space.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayViewportMapping {
    /// Display identity.
    pub display_id: DisplayId,

    /// Monitor rectangle in virtual screen coordinates (preserving negative offsets).
    pub monitor_rect: WindowRect,

    /// Viewport rectangle normalized relative to the composition origin `(0, 0)`.
    pub composition_rect: WindowRect,

    /// Whether this display is the Windows primary display.
    pub is_primary: bool,
}

/// Specification for a native player surface window required by a layout plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceLayoutPlan {
    /// Unique identifier for this planned surface.
    pub surface_id: Uuid,

    /// Displays covered by this surface (1 display for Independent/Clone, multiple for Span).
    pub target_display_ids: Vec<DisplayId>,

    /// Target surface bounds in desktop coordinate space (relative to desktop host).
    pub target_bounds: WindowRect,

    /// Resolved wallpaper catalog identifier.
    pub wallpaper_id: WallpaperId,

    /// Synchronization requirement for this surface.
    pub sync_level: PlaybackSyncLevel,
}

/// Comprehensive, declarative layout plan representing the intended multi-monitor presentation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultiMonitorLayoutPlan {
    /// Mode requested by user or configuration.
    pub selected_mode: MultiMonitorLayoutMode,

    /// Effective mode permitted by the current topology (e.g. single monitor simplifies to Independent).
    pub effective_mode: MultiMonitorLayoutMode,

    /// List of participating display identifiers.
    pub participating_displays: Vec<DisplayId>,

    /// Overall bounding rectangle of participating displays in virtual desktop coordinates.
    pub virtual_desktop_bounds: WindowRect,

    /// Viewport mapping of each participating display relative to the composition.
    pub viewport_mappings: Vec<DisplayViewportMapping>,

    /// Native surfaces required to implement this layout plan.
    pub surface_plans: Vec<SurfaceLayoutPlan>,

    /// Authoritative wallpaper resolved for Span or Clone mode, if applicable.
    pub authoritative_wallpaper_id: Option<WallpaperId>,

    /// Validation outcome explaining whether and why this plan is activatable.
    pub validation_outcome: LayoutPlanValidationOutcome,

    /// Snapshot generation this plan was derived from.
    pub snapshot_generation: u64,

    /// Timestamp when this plan was created.
    pub plan_timestamp_millis: u64,

    /// Whether this plan is safe to activate.
    pub is_activatable: bool,
}

// -------------------------------------------------------------------------------------------------
// Layout Errors & Diagnostics
// -------------------------------------------------------------------------------------------------

/// Strongly typed errors for multi-monitor layout operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum LayoutError {
    #[error("Layout plan validation failed: {0:?}")]
    PlanValidationFailed(LayoutPlanValidationOutcome),

    #[error("No active layout plan exists to activate or rollback")]
    NoActivePlan,

    #[error("Transition failed during stage `{stage}`: {reason}")]
    TransitionFailed { stage: String, reason: String },

    #[error("Rollback failed: {0}")]
    RollbackFailed(String),

    #[error("Coordinator has been shut down")]
    CoordinatorShutDown,

    #[error("Session error: {0}")]
    SessionError(String),

    #[error("Assignment error: {0}")]
    AssignmentError(String),
}

impl From<SessionError> for LayoutError {
    fn from(err: SessionError) -> Self {
        Self::SessionError(err.to_string())
    }
}

impl From<AssignmentError> for LayoutError {
    fn from(err: AssignmentError) -> Self {
        Self::AssignmentError(err.to_string())
    }
}

/// Operational events for layout lifecycle diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayoutEventType {
    PlanEvaluated,
    PlanRejected,
    TransitionStarted,
    TransitionCommitted,
    TransitionRolledBack,
    ModeReconfigured,
    ParticipantRemoved,
    CoordinatorShutdown,
}

/// Structured diagnostic event record for layout transitions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutDiagnosticEvent {
    pub timestamp_millis: u64,
    pub event_type: LayoutEventType,
    pub requested_mode: MultiMonitorLayoutMode,
    pub effective_mode: MultiMonitorLayoutMode,
    pub description: String,
    pub is_success: bool,
}

/// Result report produced upon applying a layout transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutTransitionReport {
    pub previous_mode: Option<MultiMonitorLayoutMode>,
    pub active_mode: MultiMonitorLayoutMode,
    pub active_plan: MultiMonitorLayoutPlan,
    pub activated_surfaces_count: usize,
    pub transition_generation: u64,
    pub duration_millis: u64,
}

// -------------------------------------------------------------------------------------------------
// Layout Plan Resolver Engine
// -------------------------------------------------------------------------------------------------

/// Purely observational resolver that generates deterministic `MultiMonitorLayoutPlan` records
/// from topology snapshots and assignment data without mutating any OS windows.
pub struct LayoutPlanResolver;

impl LayoutPlanResolver {
    /// Resolves a complete `MultiMonitorLayoutPlan` for the requested mode.
    #[must_use]
    pub fn resolve_plan<R: WallpaperCatalogRepository>(
        requested_mode: MultiMonitorLayoutMode,
        snapshot: &DisplayTopologySnapshot,
        resolved_assignments: &[ResolvedDisplayAssignment],
        catalog: &R,
        clone_policy: CloneSourcePolicy,
        span_policy: &SpanCompositionPolicy,
    ) -> MultiMonitorLayoutPlan {
        let now = current_timestamp_millis();

        // 1. Identify participating displays
        let connected_displays: Vec<&DisplayDescriptor> = snapshot
            .displays
            .iter()
            .filter(|d| d.is_attached && d.geometry.is_valid())
            .collect();

        if connected_displays.is_empty() {
            return MultiMonitorLayoutPlan {
                selected_mode: requested_mode,
                effective_mode: MultiMonitorLayoutMode::Independent,
                participating_displays: Vec::new(),
                virtual_desktop_bounds: WindowRect::default(),
                viewport_mappings: Vec::new(),
                surface_plans: Vec::new(),
                authoritative_wallpaper_id: None,
                validation_outcome: LayoutPlanValidationOutcome::TopologyIncompatible {
                    reason: "No valid connected displays in topology snapshot".into(),
                },
                snapshot_generation: snapshot.generation,
                plan_timestamp_millis: now,
                is_activatable: false,
            };
        }

        // Filter by explicit span participants if requested
        let participating_descriptors: Vec<&DisplayDescriptor> = match (requested_mode, span_policy)
        {
            (
                MultiMonitorLayoutMode::Span,
                SpanCompositionPolicy::ExplicitDisplays(explicit_ids),
            ) => connected_displays
                .into_iter()
                .filter(|d| explicit_ids.contains(&d.id))
                .collect(),
            _ => connected_displays,
        };

        if participating_descriptors.is_empty() {
            return MultiMonitorLayoutPlan {
                selected_mode: requested_mode,
                effective_mode: MultiMonitorLayoutMode::Independent,
                participating_displays: Vec::new(),
                virtual_desktop_bounds: WindowRect::default(),
                viewport_mappings: Vec::new(),
                surface_plans: Vec::new(),
                authoritative_wallpaper_id: None,
                validation_outcome: LayoutPlanValidationOutcome::TopologyIncompatible {
                    reason: "No participating displays matched the span policy".into(),
                },
                snapshot_generation: snapshot.generation,
                plan_timestamp_millis: now,
                is_activatable: false,
            };
        }

        let participating_ids: Vec<DisplayId> = participating_descriptors
            .iter()
            .map(|d| d.id.clone())
            .collect();

        // 2. Compute bounding rectangle of participating displays in virtual desktop space
        let mut virtual_bounds = participating_descriptors[0].geometry.monitor_rect;
        for desc in &participating_descriptors[1..] {
            virtual_bounds = virtual_bounds.union(&desc.geometry.monitor_rect);
        }

        if virtual_bounds.is_empty() || virtual_bounds.width() <= 0 || virtual_bounds.height() <= 0
        {
            return MultiMonitorLayoutPlan {
                selected_mode: requested_mode,
                effective_mode: MultiMonitorLayoutMode::Independent,
                participating_displays: participating_ids,
                virtual_desktop_bounds: virtual_bounds,
                viewport_mappings: Vec::new(),
                surface_plans: Vec::new(),
                authoritative_wallpaper_id: None,
                validation_outcome: LayoutPlanValidationOutcome::InvalidSpanGeometry {
                    reason: format!("Virtual desktop bounds are invalid: {:?}", virtual_bounds),
                },
                snapshot_generation: snapshot.generation,
                plan_timestamp_millis: now,
                is_activatable: false,
            };
        }

        // 3. Compute viewport mappings relative to composition space (0,0 based)
        let viewport_mappings: Vec<DisplayViewportMapping> = participating_descriptors
            .iter()
            .map(|d| {
                let mon = d.geometry.monitor_rect;
                let comp_rect = WindowRect::new(
                    mon.left - virtual_bounds.left,
                    mon.top - virtual_bounds.top,
                    mon.right - virtual_bounds.left,
                    mon.bottom - virtual_bounds.top,
                );
                DisplayViewportMapping {
                    display_id: d.id.clone(),
                    monitor_rect: mon,
                    composition_rect: comp_rect,
                    is_primary: d.is_primary,
                }
            })
            .collect();

        // 4. Handle Single Display simplification
        if participating_descriptors.len() == 1 {
            let desc = participating_descriptors[0];
            let assigned_wallpaper_id = resolved_assignments
                .iter()
                .find(|r| r.display_id == desc.id && r.is_playable)
                .and_then(|r| r.wallpaper_id);

            let Some(wall_id) = assigned_wallpaper_id else {
                return MultiMonitorLayoutPlan {
                    selected_mode: requested_mode,
                    effective_mode: MultiMonitorLayoutMode::Independent,
                    participating_displays: participating_ids,
                    virtual_desktop_bounds: virtual_bounds,
                    viewport_mappings,
                    surface_plans: Vec::new(),
                    authoritative_wallpaper_id: None,
                    validation_outcome: LayoutPlanValidationOutcome::MissingWallpaperAsset {
                        wallpaper_id: WallpaperId::new(),
                    },
                    snapshot_generation: snapshot.generation,
                    plan_timestamp_millis: now,
                    is_activatable: false,
                };
            };

            let single_plan = SurfaceLayoutPlan {
                surface_id: Uuid::new_v4(),
                target_display_ids: vec![desc.id.clone()],
                target_bounds: desc.geometry.monitor_rect,
                wallpaper_id: wall_id,
                sync_level: PlaybackSyncLevel::None,
            };

            return MultiMonitorLayoutPlan {
                selected_mode: requested_mode,
                effective_mode: MultiMonitorLayoutMode::Independent,
                participating_displays: participating_ids,
                virtual_desktop_bounds: virtual_bounds,
                viewport_mappings,
                surface_plans: vec![single_plan],
                authoritative_wallpaper_id: Some(wall_id),
                validation_outcome: LayoutPlanValidationOutcome::SingleDisplayOnly,
                snapshot_generation: snapshot.generation,
                plan_timestamp_millis: now,
                is_activatable: true,
            };
        }

        // 5. Multi-display layout resolution
        match requested_mode {
            MultiMonitorLayoutMode::Independent => Self::resolve_independent_plan(
                participating_descriptors,
                resolved_assignments,
                virtual_bounds,
                viewport_mappings,
                snapshot.generation,
                now,
            ),
            MultiMonitorLayoutMode::Span => Self::resolve_span_plan(
                participating_descriptors,
                resolved_assignments,
                catalog,
                virtual_bounds,
                viewport_mappings,
                snapshot.generation,
                now,
            ),
            MultiMonitorLayoutMode::Clone => Self::resolve_clone_plan(
                participating_descriptors,
                resolved_assignments,
                catalog,
                clone_policy,
                virtual_bounds,
                viewport_mappings,
                snapshot.generation,
                now,
            ),
        }
    }

    /// Resolves an Independent layout plan where each display receives its own surface and assignment.
    fn resolve_independent_plan(
        participating: Vec<&DisplayDescriptor>,
        resolved_assignments: &[ResolvedDisplayAssignment],
        virtual_bounds: WindowRect,
        viewport_mappings: Vec<DisplayViewportMapping>,
        generation: u64,
        now: u64,
    ) -> MultiMonitorLayoutPlan {
        let participating_ids: Vec<DisplayId> =
            participating.iter().map(|d| d.id.clone()).collect();
        let mut surface_plans = Vec::new();

        for desc in &participating {
            if let Some(resolved) = resolved_assignments
                .iter()
                .find(|r| r.display_id == desc.id && r.is_playable)
            {
                if let Some(wall_id) = resolved.wallpaper_id {
                    surface_plans.push(SurfaceLayoutPlan {
                        surface_id: Uuid::new_v4(),
                        target_display_ids: vec![desc.id.clone()],
                        target_bounds: desc.geometry.monitor_rect,
                        wallpaper_id: wall_id,
                        sync_level: PlaybackSyncLevel::None,
                    });
                }
            }
        }

        let (validation_outcome, is_activatable) = if surface_plans.is_empty() {
            let missing_wall = resolved_assignments
                .iter()
                .find(|r| {
                    matches!(
                        r.outcome,
                        AssignmentResolutionOutcome::WallpaperMissing
                            | AssignmentResolutionOutcome::WallpaperUnavailable
                    )
                })
                .and_then(|r| r.wallpaper_id)
                .unwrap_or_else(WallpaperId::new);

            (
                LayoutPlanValidationOutcome::MissingWallpaperAsset {
                    wallpaper_id: missing_wall,
                },
                false,
            )
        } else {
            (LayoutPlanValidationOutcome::Valid, true)
        };

        MultiMonitorLayoutPlan {
            selected_mode: MultiMonitorLayoutMode::Independent,
            effective_mode: MultiMonitorLayoutMode::Independent,
            participating_displays: participating_ids,
            virtual_desktop_bounds: virtual_bounds,
            viewport_mappings,
            surface_plans,
            authoritative_wallpaper_id: None,
            validation_outcome,
            snapshot_generation: generation,
            plan_timestamp_millis: now,
            is_activatable,
        }
    }

    /// Resolves a Span layout plan where a single surface covers the virtual desktop bounds.
    fn resolve_span_plan<R: WallpaperCatalogRepository>(
        participating: Vec<&DisplayDescriptor>,
        resolved_assignments: &[ResolvedDisplayAssignment],
        catalog: &R,
        virtual_bounds: WindowRect,
        viewport_mappings: Vec<DisplayViewportMapping>,
        generation: u64,
        now: u64,
    ) -> MultiMonitorLayoutPlan {
        let participating_ids: Vec<DisplayId> =
            participating.iter().map(|d| d.id.clone()).collect();

        // Authoritative wallpaper selection for Span:
        // Prefers primary display assignment, then first playable assignment
        let primary_desc = participating.iter().find(|d| d.is_primary).copied();
        let primary_id = primary_desc.map(|d| &d.id);

        let authoritative_id_opt = primary_id
            .and_then(|pid| {
                resolved_assignments
                    .iter()
                    .find(|r| r.display_id == *pid && r.is_playable)
                    .and_then(|r| r.wallpaper_id)
            })
            .or_else(|| {
                resolved_assignments
                    .iter()
                    .find(|r| r.is_playable)
                    .and_then(|r| r.wallpaper_id)
            });

        let Some(auth_id) = authoritative_id_opt else {
            return MultiMonitorLayoutPlan {
                selected_mode: MultiMonitorLayoutMode::Span,
                effective_mode: MultiMonitorLayoutMode::Span,
                participating_displays: participating_ids,
                virtual_desktop_bounds: virtual_bounds,
                viewport_mappings,
                surface_plans: Vec::new(),
                authoritative_wallpaper_id: None,
                validation_outcome: LayoutPlanValidationOutcome::MissingWallpaperAsset {
                    wallpaper_id: WallpaperId::new(),
                },
                snapshot_generation: generation,
                plan_timestamp_millis: now,
                is_activatable: false,
            };
        };

        // Check catalog availability
        match catalog.get(auth_id) {
            Ok(Some(rec)) if matches!(rec.availability, AvailabilityState::Available) => {}
            _ => {
                return MultiMonitorLayoutPlan {
                    selected_mode: MultiMonitorLayoutMode::Span,
                    effective_mode: MultiMonitorLayoutMode::Span,
                    participating_displays: participating_ids,
                    virtual_desktop_bounds: virtual_bounds,
                    viewport_mappings,
                    surface_plans: Vec::new(),
                    authoritative_wallpaper_id: Some(auth_id),
                    validation_outcome: LayoutPlanValidationOutcome::MissingWallpaperAsset {
                        wallpaper_id: auth_id,
                    },
                    snapshot_generation: generation,
                    plan_timestamp_millis: now,
                    is_activatable: false,
                };
            }
        }

        // Single surface covering the entire virtual desktop bounds
        let span_surface = SurfaceLayoutPlan {
            surface_id: Uuid::new_v4(),
            target_display_ids: participating_ids.clone(),
            target_bounds: virtual_bounds,
            wallpaper_id: auth_id,
            sync_level: PlaybackSyncLevel::None,
        };

        MultiMonitorLayoutPlan {
            selected_mode: MultiMonitorLayoutMode::Span,
            effective_mode: MultiMonitorLayoutMode::Span,
            participating_displays: participating_ids,
            virtual_desktop_bounds: virtual_bounds,
            viewport_mappings,
            surface_plans: vec![span_surface],
            authoritative_wallpaper_id: Some(auth_id),
            validation_outcome: LayoutPlanValidationOutcome::Valid,
            snapshot_generation: generation,
            plan_timestamp_millis: now,
            is_activatable: true,
        }
    }

    /// Resolves a Clone layout plan where each participating display shows the same wallpaper.
    #[allow(clippy::too_many_arguments)]
    fn resolve_clone_plan<R: WallpaperCatalogRepository>(
        participating: Vec<&DisplayDescriptor>,
        resolved_assignments: &[ResolvedDisplayAssignment],
        catalog: &R,
        clone_policy: CloneSourcePolicy,
        virtual_bounds: WindowRect,
        viewport_mappings: Vec<DisplayViewportMapping>,
        generation: u64,
        now: u64,
    ) -> MultiMonitorLayoutPlan {
        let participating_ids: Vec<DisplayId> =
            participating.iter().map(|d| d.id.clone()).collect();

        // Resolve authoritative wallpaper based on policy
        let authoritative_id = match clone_policy {
            CloneSourcePolicy::ExplicitWallpaper(id) => Some(id),
            CloneSourcePolicy::PrimaryDisplay => {
                let primary_desc = participating.iter().find(|d| d.is_primary).copied();
                let primary_id = primary_desc.map(|d| &d.id);
                primary_id
                    .and_then(|pid| {
                        resolved_assignments
                            .iter()
                            .find(|r| r.display_id == *pid && r.is_playable)
                            .and_then(|r| r.wallpaper_id)
                    })
                    .or_else(|| {
                        resolved_assignments
                            .iter()
                            .find(|r| r.is_playable)
                            .and_then(|r| r.wallpaper_id)
                    })
            }
            CloneSourcePolicy::StrictMatch => {
                let mut distinct_wallpapers: HashMap<WallpaperId, Vec<DisplayId>> = HashMap::new();
                for desc in &participating {
                    if let Some(r) = resolved_assignments
                        .iter()
                        .find(|res| res.display_id == desc.id && res.is_playable)
                    {
                        if let Some(wid) = r.wallpaper_id {
                            distinct_wallpapers
                                .entry(wid)
                                .or_default()
                                .push(desc.id.clone());
                        }
                    }
                }

                if distinct_wallpapers.len() > 1 {
                    let conflicts: Vec<(DisplayId, WallpaperId)> = distinct_wallpapers
                        .into_iter()
                        .flat_map(|(wid, dids)| dids.into_iter().map(move |did| (did, wid)))
                        .collect();

                    return MultiMonitorLayoutPlan {
                        selected_mode: MultiMonitorLayoutMode::Clone,
                        effective_mode: MultiMonitorLayoutMode::Clone,
                        participating_displays: participating_ids,
                        virtual_desktop_bounds: virtual_bounds,
                        viewport_mappings,
                        surface_plans: Vec::new(),
                        authoritative_wallpaper_id: None,
                        validation_outcome:
                            LayoutPlanValidationOutcome::ConflictingCloneAssignments { conflicts },
                        snapshot_generation: generation,
                        plan_timestamp_millis: now,
                        is_activatable: false,
                    };
                }

                distinct_wallpapers.keys().next().copied()
            }
        };

        let Some(auth_id) = authoritative_id else {
            return MultiMonitorLayoutPlan {
                selected_mode: MultiMonitorLayoutMode::Clone,
                effective_mode: MultiMonitorLayoutMode::Clone,
                participating_displays: participating_ids,
                virtual_desktop_bounds: virtual_bounds,
                viewport_mappings,
                surface_plans: Vec::new(),
                authoritative_wallpaper_id: None,
                validation_outcome: LayoutPlanValidationOutcome::MissingWallpaperAsset {
                    wallpaper_id: WallpaperId::new(),
                },
                snapshot_generation: generation,
                plan_timestamp_millis: now,
                is_activatable: false,
            };
        };

        // Check catalog availability
        match catalog.get(auth_id) {
            Ok(Some(rec)) if matches!(rec.availability, AvailabilityState::Available) => {}
            _ => {
                return MultiMonitorLayoutPlan {
                    selected_mode: MultiMonitorLayoutMode::Clone,
                    effective_mode: MultiMonitorLayoutMode::Clone,
                    participating_displays: participating_ids,
                    virtual_desktop_bounds: virtual_bounds,
                    viewport_mappings,
                    surface_plans: Vec::new(),
                    authoritative_wallpaper_id: Some(auth_id),
                    validation_outcome: LayoutPlanValidationOutcome::MissingWallpaperAsset {
                        wallpaper_id: auth_id,
                    },
                    snapshot_generation: generation,
                    plan_timestamp_millis: now,
                    is_activatable: false,
                };
            }
        }

        // Each participating display gets a surface positioned to its bounds showing the same wallpaper
        let surface_plans: Vec<SurfaceLayoutPlan> = participating
            .iter()
            .map(|desc| SurfaceLayoutPlan {
                surface_id: Uuid::new_v4(),
                target_display_ids: vec![desc.id.clone()],
                target_bounds: desc.geometry.monitor_rect,
                wallpaper_id: auth_id,
                sync_level: PlaybackSyncLevel::BestEffort,
            })
            .collect();

        MultiMonitorLayoutPlan {
            selected_mode: MultiMonitorLayoutMode::Clone,
            effective_mode: MultiMonitorLayoutMode::Clone,
            participating_displays: participating_ids,
            virtual_desktop_bounds: virtual_bounds,
            viewport_mappings,
            surface_plans,
            authoritative_wallpaper_id: Some(auth_id),
            validation_outcome: LayoutPlanValidationOutcome::Valid,
            snapshot_generation: generation,
            plan_timestamp_millis: now,
            is_activatable: true,
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Layout Mode Coordinator
// -------------------------------------------------------------------------------------------------

/// Orchestrates multi-monitor layout transitions between Independent, Span, and Clone modes,
/// ensuring transactional activation, geometry synchronization, and reliable rollback.
pub struct MultiMonitorLayoutCoordinator {
    session_coordinator: Arc<Mutex<DisplaySessionCoordinator>>,
    assignment_service: Arc<AssignmentService>,
    saved_mode: MultiMonitorLayoutMode,
    active_mode: Option<MultiMonitorLayoutMode>,
    active_plan: Option<MultiMonitorLayoutPlan>,
    clone_policy: CloneSourcePolicy,
    span_policy: SpanCompositionPolicy,
    transition_generation: u64,
    diagnostic_history: Vec<LayoutDiagnosticEvent>,
    is_shut_down: bool,
}

impl MultiMonitorLayoutCoordinator {
    /// Creates a new `MultiMonitorLayoutCoordinator`.
    #[must_use]
    pub fn new(
        session_coordinator: Arc<Mutex<DisplaySessionCoordinator>>,
        assignment_service: Arc<AssignmentService>,
    ) -> Self {
        let saved_mode = assignment_service.get_layout_mode();
        Self {
            session_coordinator,
            assignment_service,
            saved_mode,
            active_mode: None,
            active_plan: None,
            clone_policy: CloneSourcePolicy::default(),
            span_policy: SpanCompositionPolicy::default(),
            transition_generation: 1,
            diagnostic_history: Vec::new(),
            is_shut_down: false,
        }
    }

    /// Retrieves the currently persisted layout mode preference.
    #[must_use]
    pub fn saved_mode(&self) -> MultiMonitorLayoutMode {
        self.saved_mode
    }

    /// Sets and persists the layout mode preference without activating native window changes.
    pub fn set_saved_mode(&mut self, mode: MultiMonitorLayoutMode) -> Result<(), LayoutError> {
        self.saved_mode = mode;
        self.assignment_service.set_layout_mode(mode)?;
        Ok(())
    }

    /// Retrieves the currently active runtime layout mode, if activated.
    #[must_use]
    pub fn active_mode(&self) -> Option<MultiMonitorLayoutMode> {
        self.active_mode
    }

    /// Retrieves the active layout plan, if one has been successfully activated.
    #[must_use]
    pub fn active_plan(&self) -> Option<&MultiMonitorLayoutPlan> {
        self.active_plan.as_ref()
    }

    /// Updates the clone authoritative source policy.
    pub fn set_clone_policy(&mut self, policy: CloneSourcePolicy) {
        self.clone_policy = policy;
    }

    /// Updates the span composition policy.
    pub fn set_span_policy(&mut self, policy: SpanCompositionPolicy) {
        self.span_policy = policy;
    }

    /// Evaluates and generates a deterministic `MultiMonitorLayoutPlan` without altering desktop windows.
    pub fn evaluate_plan<R: WallpaperCatalogRepository>(
        &self,
        snapshot: &DisplayTopologySnapshot,
        catalog: &R,
    ) -> MultiMonitorLayoutPlan {
        let resolved_assignments = self
            .assignment_service
            .resolve_all_displays(snapshot, catalog);

        LayoutPlanResolver::resolve_plan(
            self.saved_mode,
            snapshot,
            &resolved_assignments,
            catalog,
            self.clone_policy,
            &self.span_policy,
        )
    }

    /// Applies a layout mode transition to the target plan with transactional recovery.
    pub fn apply_layout<R: WallpaperCatalogRepository>(
        &mut self,
        snapshot: &DisplayTopologySnapshot,
        catalog: &R,
        host_candidate: &DesktopCandidateWindow,
    ) -> Result<LayoutTransitionReport, LayoutError> {
        if self.is_shut_down {
            return Err(LayoutError::CoordinatorShutDown);
        }

        let start_time = current_timestamp_millis();
        let plan = self.evaluate_plan(snapshot, catalog);

        if !plan.is_activatable {
            self.record_diagnostic(
                LayoutEventType::PlanRejected,
                plan.selected_mode,
                plan.effective_mode,
                format!("Layout plan rejected: {:?}", plan.validation_outcome),
                false,
            );
            return Err(LayoutError::PlanValidationFailed(plan.validation_outcome));
        }

        self.transition_generation = self.transition_generation.wrapping_add(1);
        let current_transition_gen = self.transition_generation;
        let prev_mode = self.active_mode;

        self.record_diagnostic(
            LayoutEventType::TransitionStarted,
            plan.selected_mode,
            plan.effective_mode,
            format!(
                "Transitioning layout mode from {:?} to {:?}",
                prev_mode, plan.effective_mode
            ),
            true,
        );

        // Execute transition according to effective mode
        let activation_result = match plan.effective_mode {
            MultiMonitorLayoutMode::Independent => {
                self.activate_independent_layout(&plan, snapshot, catalog, host_candidate)
            }
            MultiMonitorLayoutMode::Span => self.activate_span_layout(&plan, host_candidate),
            MultiMonitorLayoutMode::Clone => {
                self.activate_clone_layout(&plan, snapshot, host_candidate)
            }
        };

        match activation_result {
            Ok(surfaces_count) => {
                self.active_mode = Some(plan.effective_mode);
                self.active_plan = Some(plan.clone());

                let duration = current_timestamp_millis().saturating_sub(start_time);
                self.record_diagnostic(
                    LayoutEventType::TransitionCommitted,
                    plan.selected_mode,
                    plan.effective_mode,
                    format!(
                        "Layout mode {:?} successfully committed ({} surfaces)",
                        plan.effective_mode, surfaces_count
                    ),
                    true,
                );

                Ok(LayoutTransitionReport {
                    previous_mode: prev_mode,
                    active_mode: plan.effective_mode,
                    active_plan: plan,
                    activated_surfaces_count: surfaces_count,
                    transition_generation: current_transition_gen,
                    duration_millis: duration,
                })
            }
            Err(err) => {
                self.record_diagnostic(
                    LayoutEventType::TransitionRolledBack,
                    plan.selected_mode,
                    plan.effective_mode,
                    format!("Layout transition failed: {}", err),
                    false,
                );
                // Attempt rollback to previous mode if possible
                let _ = self.rollback_internal(snapshot, catalog, host_candidate, prev_mode);
                Err(err)
            }
        }
    }

    /// Internal execution of Independent layout activation.
    fn activate_independent_layout<R: WallpaperCatalogRepository>(
        &mut self,
        _plan: &MultiMonitorLayoutPlan,
        snapshot: &DisplayTopologySnapshot,
        catalog: &R,
        host_candidate: &DesktopCandidateWindow,
    ) -> Result<usize, LayoutError> {
        let mut session_coord = self.session_coordinator.lock().unwrap();

        // If span desktop was active, cleanly stop it
        let span_id = DisplayId(SPAN_VIRTUAL_DESKTOP_DISPLAY_ID.into());
        let _ = session_coord.stop_session(&span_id, SessionStopReason::Replaced);

        // Resolve normal assignments
        let resolved = self
            .assignment_service
            .resolve_all_displays(snapshot, catalog);
        let session_plan = session_coord.create_plan(snapshot, &resolved);
        let report = session_coord.apply_plan(&session_plan, host_candidate)?;

        Ok(report.activated.len() + report.retained_count)
    }

    /// Internal execution of Span layout activation.
    fn activate_span_layout(
        &mut self,
        plan: &MultiMonitorLayoutPlan,
        host_candidate: &DesktopCandidateWindow,
    ) -> Result<usize, LayoutError> {
        let mut session_coord = self.session_coordinator.lock().unwrap();

        // 1. Orderly stop all individual monitor sessions
        for did in &plan.participating_displays {
            let _ = session_coord.stop_session(did, SessionStopReason::Replaced);
        }

        // 2. Activate single virtual desktop spanning surface
        let auth_id =
            plan.authoritative_wallpaper_id
                .ok_or_else(|| LayoutError::TransitionFailed {
                    stage: "SpanPreparation".into(),
                    reason: "Span plan lacks authoritative wallpaper ID".into(),
                })?;

        let span_display_id = DisplayId(SPAN_VIRTUAL_DESKTOP_DISPLAY_ID.into());
        let span_geometry = DisplayGeometry {
            monitor_rect: plan.virtual_desktop_bounds,
            work_area_rect: plan.virtual_desktop_bounds,
            width: plan.virtual_desktop_bounds.width(),
            height: plan.virtual_desktop_bounds.height(),
            scale_factor_percent: Some(100),
        };

        // Stop existing span session if present
        let _ = session_coord.stop_session(&span_display_id, SessionStopReason::Replaced);

        session_coord.activate_session(&span_display_id, auth_id, span_geometry, host_candidate)?;

        Ok(1)
    }

    /// Internal execution of Clone layout activation.
    fn activate_clone_layout(
        &mut self,
        plan: &MultiMonitorLayoutPlan,
        snapshot: &DisplayTopologySnapshot,
        host_candidate: &DesktopCandidateWindow,
    ) -> Result<usize, LayoutError> {
        let mut session_coord = self.session_coordinator.lock().unwrap();

        // Stop span session if active
        let span_id = DisplayId(SPAN_VIRTUAL_DESKTOP_DISPLAY_ID.into());
        let _ = session_coord.stop_session(&span_id, SessionStopReason::Replaced);

        let auth_id =
            plan.authoritative_wallpaper_id
                .ok_or_else(|| LayoutError::TransitionFailed {
                    stage: "ClonePreparation".into(),
                    reason: "Clone plan lacks authoritative wallpaper ID".into(),
                })?;

        let mut activated_count = 0;

        for surface_plan in &plan.surface_plans {
            for did in &surface_plan.target_display_ids {
                let desc_opt = snapshot.find_by_id(did);
                if let Some(desc) = desc_opt {
                    let existing_opt = session_coord.get_session(did);
                    if let Some(existing) = existing_opt {
                        if existing.state.is_active() && existing.wallpaper_id == auth_id {
                            // Retain existing matching session
                            activated_count += 1;
                            continue;
                        }
                    }

                    // Replace/activate session with clone authoritative wallpaper
                    let _ = session_coord.stop_session(did, SessionStopReason::Replaced);
                    session_coord.activate_session(did, auth_id, desc.geometry, host_candidate)?;
                    activated_count += 1;
                }
            }
        }

        Ok(activated_count)
    }

    /// Internal rollback to restore a previous layout mode.
    fn rollback_internal<R: WallpaperCatalogRepository>(
        &mut self,
        snapshot: &DisplayTopologySnapshot,
        catalog: &R,
        host_candidate: &DesktopCandidateWindow,
        previous_mode_opt: Option<MultiMonitorLayoutMode>,
    ) -> Result<(), LayoutError> {
        let fallback_mode = previous_mode_opt.unwrap_or(MultiMonitorLayoutMode::Independent);
        let resolved = self
            .assignment_service
            .resolve_all_displays(snapshot, catalog);

        let mut session_coord = self.session_coordinator.lock().unwrap();

        // Clean up span if present
        let span_id = DisplayId(SPAN_VIRTUAL_DESKTOP_DISPLAY_ID.into());
        let _ = session_coord.stop_session(&span_id, SessionStopReason::Replaced);

        let session_plan = session_coord.create_plan(snapshot, &resolved);
        let _ = session_coord.apply_plan(&session_plan, host_candidate);

        self.active_mode = Some(fallback_mode);
        Ok(())
    }

    /// Shuts down the layout coordinator, stopping all active sessions.
    pub fn shutdown(&mut self) -> Result<(), LayoutError> {
        if self.is_shut_down {
            return Ok(());
        }

        self.is_shut_down = true;
        {
            let mut session_coord = self.session_coordinator.lock().unwrap();
            session_coord.shutdown()?;
        }

        self.record_diagnostic(
            LayoutEventType::CoordinatorShutdown,
            self.saved_mode,
            self.saved_mode,
            "Layout coordinator shutdown completed".to_string(),
            true,
        );

        Ok(())
    }

    /// Returns a copy of the diagnostic event history.
    #[must_use]
    pub fn get_diagnostics(&self) -> Vec<LayoutDiagnosticEvent> {
        self.diagnostic_history.clone()
    }

    /// Records an event in the bounded diagnostic ring buffer.
    fn record_diagnostic(
        &mut self,
        event_type: LayoutEventType,
        requested_mode: MultiMonitorLayoutMode,
        effective_mode: MultiMonitorLayoutMode,
        description: String,
        is_success: bool,
    ) {
        if self.diagnostic_history.len() >= MAX_LAYOUT_DIAGNOSTIC_EVENTS {
            self.diagnostic_history.remove(0);
        }

        self.diagnostic_history.push(LayoutDiagnosticEvent {
            timestamp_millis: current_timestamp_millis(),
            event_type,
            requested_mode,
            effective_mode,
            description,
            is_success,
        });
    }
}

fn current_timestamp_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
