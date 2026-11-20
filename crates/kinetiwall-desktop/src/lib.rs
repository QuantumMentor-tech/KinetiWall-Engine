//! # `kinetiwall-desktop`
//!
//! Native Windows desktop host discovery, shell topology classification, candidate window
//! validation, controlled wallpaper window attachment, transactional rollback, surface
//! synchronization, multi-monitor display topology modeling, per-monitor wallpaper assignment,
//! and independent per-monitor playback sessions (Steps 39, 40, 41, 42, 43, & 44).
//!
//! ## Architectural Boundaries
//! - **Discovery & Verification**: Inspects and classifies the Windows desktop shell hierarchy,
//!   validating target windows against desktop icon carriers.
//! - **Controlled Attachment**: Attaches dedicated, application-owned player windows to validated
//!   `WorkerW` hosts with explicit window style updates and non-intrusive visibility.
//! - **Transactional Rollback**: Guarantees deterministic rollback of styles, parentage, and bounds
//!   if any stage of attachment fails.
//! - **Surface Synchronization & Safe Reattachment**: Synchronizes attached surface geometry
//!   across resolution and DPI transitions with anti-loop guards, coalescing, and safe host recovery.
//! - **Multi-Monitor Display Discovery**: Enumerates logical desktop monitors, models active display
//!   topology, calculates virtual desktop geometry, and resolves stable application-level display identities.
//! - **Per-Monitor Wallpaper Assignment**: Persists user wallpaper selections per display, validates
//!   against the catalog, and resolves effective assignments with fallback and disconnect preservation.
//! - **Per-Monitor Playback Sessions**: Coordinates independent runtime playback sessions per display,
//!   reconciles active sessions with declarative plans, and isolates runtime failures across displays.
//! - **Multi-Monitor Layout Modes**: Orchestrates Independent, Span, and Clone presentation modes,
//!   virtual desktop composition geometry, synchronized clone playback, and transactional mode transitions (Step 45).

pub mod adaptive;
pub mod arbitration;
pub mod assignment;
pub mod attachment;
pub mod budget;
pub mod compatibility;
pub mod discovery;
pub mod display;
pub mod display_power;
pub mod fallback;
pub mod inactivity;
pub mod layout;
pub mod policy;
pub mod power;
pub mod provider;
pub mod session;
pub mod settings;
pub mod synchronization;
pub mod types;
pub mod win_session;

pub use adaptive::{
    AdaptiveAction, AdaptiveBackendCapability, AdaptiveDiagnosticEvent,
    AdaptiveDiagnosticEventType, AdaptivePlaybackPolicyEvaluator, AdaptivePolicyDecision,
    AdaptiveQualityLevel, AdaptiveQualitySettings, EffectiveQualityOverride,
    PlaybackResourceObservation, PressureState, ResourceMetricKind, ResourceMetricUnit,
    ResourceObservationScope, ResourceSignalQuality, SustainedPressureDetector,
    DEFAULT_ADJUSTMENT_COOLDOWN_MS, DEFAULT_CPU_PRESSURE_THRESHOLD_PERCENT,
    DEFAULT_CPU_RECOVERY_THRESHOLD_PERCENT, DEFAULT_RECOVERY_DWELL_THRESHOLD_MS,
    DEFAULT_SUSTAINED_PRESSURE_THRESHOLD_MS, DEFAULT_TARGET_FPS_FLOOR, MAX_ADAPTIVE_DIAGNOSTICS,
    MAX_RESOURCE_SAMPLE_HISTORY, MIN_SAFE_FPS_FLOOR,
};
pub use assignment::{
    AssignmentDiagnosticEvent, AssignmentError, AssignmentKind, AssignmentRepository,
    AssignmentResolutionOutcome, AssignmentService, AssignmentStoreData, DisplayAssignment,
    DisplayPresenceStatus, FileAssignmentRepository, MockAssignmentRepository,
    ResolvedDisplayAssignment, CURRENT_ASSIGNMENT_SCHEMA_VERSION, MAX_ASSIGNMENT_DIAGNOSTIC_EVENTS,
};
pub use attachment::{
    constants, AttachmentCoordinator, AttachmentError, AttachmentRecord, AttachmentRequest,
    AttachmentStage, AttachmentState, DetachmentRecord, OriginalWindowState, RollbackOutcome,
};
pub use budget::{
    AggregateBudgetState, AggregateWorkloadSnapshot, ApplicationResourceBudget, ArbitrationAction,
    BudgetDiagnosticEvent, BudgetDiagnosticEventType, MeasurementCoverageStatus, ResourceBudgetArbiter,
    ResourceOwnership, TrackedPlaybackResource, TrackedResourceId,
    DEFAULT_BUDGET_ADJUSTMENT_COOLDOWN_MS, DEFAULT_BUDGET_PRESSURE_CPU_PERCENT,
    DEFAULT_BUDGET_RECOVERY_CPU_PERCENT, DEFAULT_BUDGET_RECOVERY_DWELL_MS,
    DEFAULT_BUDGET_SUSTAINED_PRESSURE_MS, DEFAULT_MAX_SIMULTANEOUS_ADJUSTMENTS,
    MAX_BUDGET_DIAGNOSTICS, MAX_RESOURCE_SAMPLES,
};
pub use compatibility::{CompatibilityPolicy, CompatibilityRejectionReason};
pub use discovery::{DesktopDiscoveryOptions, DesktopHostDiscovery};
pub use display::{
    DesktopDisplayProvider, DisplayChangeDelta, DisplayDescriptor, DisplayDiscoveryError,
    DisplayGeometry, DisplayId, DisplaySnapshotManager, DisplayTopologySnapshot,
    DisplayTopologyType, MockDesktopDisplayProvider, MockDisplayRecord,
    NativeDesktopDisplayProvider, NativeMonitorHandle, RawDisplayConfigSummary,
    RawDisplayDeviceInfo, RawMonitorInfo,
};
pub use fallback::{
    FallbackAsset, FallbackAssetCache, FallbackCapability, FallbackDiagnosticEvent,
    FallbackDiagnosticEventType, FallbackError, FallbackSourceKind, FallbackTransitionStage,
    FrameCaptureProvider, MockFrameCaptureProvider, NativeDesktopFrameCaptureProvider,
    PreservedSessionState, StaticFallbackCoordinator, MAX_FALLBACK_DIAGNOSTICS,
    MAX_FALLBACK_DIMENSION, MAX_FALLBACK_FILE_SIZE_BYTES, MAX_RETAINED_FALLBACK_ASSETS,
};
pub use layout::{
    CloneSourcePolicy, DisplayViewportMapping, LayoutDiagnosticEvent, LayoutError, LayoutEventType,
    LayoutPlanResolver, LayoutPlanValidationOutcome, LayoutTransitionReport,
    MultiMonitorLayoutCoordinator, MultiMonitorLayoutMode, MultiMonitorLayoutPlan,
    PlaybackSyncLevel, SpanCompositionPolicy, SurfaceLayoutPlan, MAX_LAYOUT_DIAGNOSTIC_EVENTS,
    SPAN_VIRTUAL_DESKTOP_DISPLAY_ID,
};
pub use policy::{
    DesktopForegroundObserver, ForegroundObservation, FullscreenClassification,
    FullscreenClassifier, FullscreenPauseMode, FullscreenPlaybackSettings, PauseReason,
    PolicyDiagnosticEvent, PolicyEventType, SmartPlaybackPolicyEvaluator,
    MAX_POLICY_DIAGNOSTIC_EVENTS,
};
pub use power::{
    BatteryAvailability, BatteryChargingState, BatterySaverState, DesktopPowerProvider,
    MockDesktopPowerProvider, NativeDesktopPowerProvider, PowerError, PowerNotificationHandles,
    PowerPlaybackSettings, PowerPolicyEvaluation, PowerPolicyMode, PowerSource, PowerStatusQuality,
    SystemPowerSnapshot,
};
pub use provider::{
    DesktopWindowProvider, MockDesktopWindowProvider, MockWindowRecord, NativeDesktopWindowProvider,
};
pub use session::{
    DisplaySessionCoordinator, DisplaySessionDescriptor, DisplaySessionDiagnosticEvent,
    DisplaySessionId, DisplaySessionState, ProviderSurfaceFactory, SessionError, SessionEventType,
    SessionPlan, SessionPlanAction, SessionReconciliationReport, SessionStopReason, SurfaceFactory,
    MAX_SESSION_DIAGNOSTIC_EVENTS,
};
pub use synchronization::{
    SurfaceChangeEvent, SurfaceSyncError, SurfaceSyncEvent, SurfaceSyncMetadata, SurfaceSyncReport,
    SurfaceSyncState, MAX_SYNC_DIAGNOSTIC_EVENTS,
};
pub use types::{
    CandidateRole, DesktopCandidateWindow, DesktopShellTopology, DiscoveryError,
    NativeWindowHandle, TopologyClassification, WindowRect,
};
pub use win_session::{
    DesktopSessionNotificationProvider, MockDesktopSessionNotificationProvider,
    NativeDesktopSessionNotificationProvider, SessionConnectionState, SessionDiagnosticEvent,
    SessionDiagnosticEventType, SessionNotificationEvent, SessionNotificationEventKind,
    SessionNotificationRegistration, SessionNotificationScope, SessionPlaybackSettings,
    SessionPolicyEvaluation, WinSessionError, WindowsSessionSnapshot, WorkstationLockState,
    MAX_SESSION_POLICY_DIAGNOSTICS,
};
pub use display_power::{
    decode_display_power_value, DesktopDisplayPowerProvider, DisplayDimmedAction, DisplayPowerDiagnosticEvent,
    DisplayPowerDiagnosticEventType, DisplayPowerError, DisplayPowerPlaybackSettings,
    DisplayPowerPolicyEvaluation, DisplayPowerScope, DisplayPowerSnapshot, DisplayPowerState,
    MockDesktopDisplayPowerProvider, NativeDesktopDisplayPowerProvider, MAX_DISPLAY_POWER_DIAGNOSTICS,
};
pub use inactivity::{
    calculate_elapsed_ticks, DesktopUserInactivityProvider, InactivityDiagnosticEvent,
    InactivityDiagnosticEventType, InactivityError, InactivityPlaybackSettings,
    InactivityPolicyEvaluation, InactivitySnapshot, InactivityState,
    MockDesktopUserInactivityProvider, NativeDesktopUserInactivityProvider,
    DEFAULT_INACTIVITY_THRESHOLD_MILLIS, MAX_INACTIVITY_DIAGNOSTICS,
    MAX_INACTIVITY_THRESHOLD_MILLIS, MIN_INACTIVITY_THRESHOLD_MILLIS,
};
pub use arbitration::{
    ArbitrationDiagnosticEvent, ConstraintCategory, DesiredSessionState, EffectivePlaybackDecision,
    PolicyArbitrator, PolicyExplanationCode, SessionArbitrationInput, MAX_ARBITRATION_DIAGNOSTICS,
};
pub use settings::{
    migrate_raw, AudioConfig, BatteryAction, DiagnosticsConfig, FileSettingsRepository,
    FullscreenAction, GeneralConfig, InMemorySettingsRepository, KinetiWallConfig,
    MonitorAssignmentConfig, NotificationPreference, PerformanceConfig, ReducedMotionPreference,
    RemoteDesktopAction, RenderingConfig, SettingsError, SettingsRepository, StretchMode,
    ThemePreference, WindowStateAction, BACKUP_CONFIG_EXTENSION, CURRENT_CONFIG_SCHEMA_VERSION,
    DEFAULT_CONFIG_FILENAME, TEMP_CONFIG_EXTENSION,
};
