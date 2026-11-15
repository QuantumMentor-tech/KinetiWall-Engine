//! # `kinetiwall-playback`
//!
//! Playback architecture contracts (Step 31): typed requests, capability model, lifecycle state
//! machine, player-adapter trait, and a single-owner orchestrator. Contains **no** media decoding,
//! web rendering, process spawning, IPC transport, or Windows desktop code.

#![forbid(unsafe_code)]

pub mod adapter;
pub mod config;
pub mod diagnostics;
pub mod health;
pub mod lifecycle;
pub mod mock;
pub mod negotiation;
pub mod orchestrator;
pub mod recovery;
pub mod supervisor;
pub mod types;

pub use adapter::{
    CancelAck, PlayerAdapter, PlayerReport, PrepareProgress, SessionContext, StopAck,
};
pub use config::{
    AudioReactivityOptions, BackendOptions, CommonPlaybackConfig, ConfigUpdateResult,
    ConfigValidationError, EffectivePlaybackConfig, HwAccelPreference, LoopMode, NormalizedVolume,
    PlaybackConfig, RuntimeConfigUpdate, ScalingMode, StartMode, VideoBackendOptions, VideoHwDec,
    WebBackendOptions,
};
pub use diagnostics::{
    BackendPerformanceMetrics, DiagnosticCategory, DiagnosticEvent, DiagnosticsAggregator,
    DiagnosticsSnapshot, LifecycleTimingSummary, LifecycleTimings, ProcessResourceSample,
    ResourceSummary, MAX_RETAINED_EVENTS, MAX_RETAINED_SAMPLES,
};
pub use health::{FailureClassification, HealthReport, PlayerHealthStatus, PlayerInstanceId};
pub use lifecycle::{DesiredState, SessionSnapshot, SessionState};
pub use negotiation::{BackendCapabilities, ConfigAdjustment, NegotiationReport, Negotiator};
pub use orchestrator::{
    CancelOutcome, EventOutcome, PendingPhase, PlaybackOrchestrator, ShutdownReport, StartOutcome,
    StopOutcome,
};
pub use recovery::{RecoveryBudget, RecoveryOutcome, RecoveryPolicy};
pub use supervisor::{AudioCoordinator, PlayerSupervisor, SupervisedTarget};
pub use types::{
    AllowAllPolicy, DisplayTargetId, PlaybackAssetInfo, PlaybackError, PlaybackErrorKind,
    PlaybackPolicy, PlaybackRequest, PlaybackRequestId, PlaybackSessionId, PlayerAvailability,
    PlayerCapabilities, WallpaperPlaybackLookup,
};
