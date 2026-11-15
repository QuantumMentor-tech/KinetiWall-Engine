//! Request, identity, capability, and error contracts for the playback subsystem.

use std::fmt;

use kinetiwall_catalog::{AvailabilityState, WallpaperId, WallpaperType};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::PlaybackConfig;

/// Caller-generated identifier of one playback operation request (idempotency key).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlaybackRequestId(Uuid);

impl PlaybackRequestId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
    #[must_use]
    pub fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }
    #[must_use]
    pub fn is_nil(&self) -> bool {
        self.0.is_nil()
    }
}

impl Default for PlaybackRequestId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for PlaybackRequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Orchestrator-generated identifier of one playback session. A new session (even for the same
/// wallpaper and target) always receives a new ID, so late events from an earlier player instance
/// can never be mistaken for the current one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlaybackSessionId(Uuid);

impl PlaybackSessionId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
    #[must_use]
    pub fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }
}

impl Default for PlaybackSessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for PlaybackSessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Opaque display identifier matching the approved `display_id: u32` contract (`CTR-02`).
/// Display enumeration and validation belong to the future display-management boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DisplayTargetId(pub u32);

/// Playback request. Carries identifiers only: never file paths or media bytes. Asset references
/// are resolved by the trusted catalog/adapter side.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaybackRequest {
    pub request_id: PlaybackRequestId,
    pub wallpaper_id: WallpaperId,
    /// Wallpaper type the caller believes it is requesting; must match the catalog record.
    pub wallpaper_type: WallpaperType,
    pub target: DisplayTargetId,
    #[serde(default)]
    pub config: Option<PlaybackConfig>,
}

impl PlaybackRequest {
    #[must_use]
    pub fn new(
        wallpaper_id: WallpaperId,
        wallpaper_type: WallpaperType,
        target: DisplayTargetId,
    ) -> Self {
        Self {
            request_id: PlaybackRequestId::new(),
            wallpaper_id,
            wallpaper_type,
            target,
            config: None,
        }
    }

    #[must_use]
    pub fn with_config(mut self, config: PlaybackConfig) -> Self {
        self.config = Some(config);
        self
    }
}

/// What a player backend can do *in principle*. Says nothing about runtime availability or policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerCapabilities {
    pub wallpaper_type: WallpaperType,
    /// Backend can abandon an in-flight `prepare`/`start` via `PlayerAdapter::cancel`.
    pub supports_cancellation: bool,
    /// Backend reports confirmed state through `PlayerReport` events (always required for
    /// `Playing`; recorded so unsupported backends are rejected rather than assumed).
    pub reports_observed_state: bool,
}

/// Whether a backend is currently usable in the running environment (runtime installed, etc.).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PlayerAvailability {
    Available,
    Unavailable { reason: String },
}

/// Catalog facts the orchestrator needs; resolved from the trusted catalog, never from the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackAssetInfo {
    pub wallpaper_type: WallpaperType,
    /// Last-known availability. A live probe remains the catalog's responsibility.
    pub availability: AvailabilityState,
}

/// Trusted lookup of catalog facts for a wallpaper ID.
pub trait WallpaperPlaybackLookup {
    fn playback_info(&self, id: WallpaperId) -> Option<PlaybackAssetInfo>;
}

impl<T: kinetiwall_catalog::WallpaperCatalogRepository> WallpaperPlaybackLookup for T {
    fn playback_info(&self, id: WallpaperId) -> Option<PlaybackAssetInfo> {
        self.get(id).ok().flatten().map(|r| PlaybackAssetInfo {
            wallpaper_type: r.wallpaper_type,
            availability: r.availability,
        })
    }
}

/// Application policy deciding whether a request is permitted (authorization, distinct from
/// capability and availability).
pub trait PlaybackPolicy: Send {
    /// `Err(reason)` denies the request; the reason is diagnostic only.
    fn authorize(&self, request: &PlaybackRequest) -> Result<(), String>;
}

/// Permits every structurally valid request. Real policy arrives with later steps.
#[derive(Debug, Default, Clone, Copy)]
pub struct AllowAllPolicy;

impl PlaybackPolicy for AllowAllPolicy {
    fn authorize(&self, _request: &PlaybackRequest) -> Result<(), String> {
        Ok(())
    }
}

/// Failure categories relevant to orchestration. Cancellation is a state/outcome, not an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackErrorKind {
    InvalidRequest,
    UnsupportedCapability,
    PolicyDenied,
    AssetUnavailable,
    BackendUnavailable,
    TargetBusy,
    ShuttingDown,
    PreparationFailed,
    StartupFailed,
    StopFailed,
    OperationRejected,
    Timeout,
    BackendExited,
    Internal,
}

impl PlaybackErrorKind {
    /// Stable, user-safe message. Never contains OS error text or paths.
    #[must_use]
    pub fn user_message(self) -> &'static str {
        match self {
            Self::InvalidRequest => "The playback request was not valid.",
            Self::UnsupportedCapability => "No player supports this wallpaper type.",
            Self::PolicyDenied => "Playback of this wallpaper is not permitted.",
            Self::AssetUnavailable => "The wallpaper's media is missing or unavailable.",
            Self::BackendUnavailable => "The player for this wallpaper type is not available.",
            Self::TargetBusy => "This display already has a playback session. Stop it first.",
            Self::ShuttingDown => "KinetiWall is shutting down.",
            Self::PreparationFailed => "The player could not prepare this wallpaper.",
            Self::StartupFailed => "The player could not start this wallpaper.",
            Self::StopFailed => "The player did not stop cleanly.",
            Self::OperationRejected => "The player rejected the operation.",
            Self::Timeout => "The player did not respond in time.",
            Self::BackendExited => "The player stopped unexpectedly.",
            Self::Internal => "An internal playback error occurred.",
        }
    }
}

/// Structured error: serializable user-safe part plus a local-only diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaybackError {
    pub kind: PlaybackErrorKind,
    pub user_message: String,
    /// Internal detail for logs only; never serialized to the UI.
    #[serde(skip)]
    pub diagnostic: String,
}

impl PlaybackError {
    #[must_use]
    pub fn new(kind: PlaybackErrorKind, diagnostic: impl Into<String>) -> Self {
        Self {
            kind,
            user_message: kind.user_message().to_string(),
            diagnostic: diagnostic.into(),
        }
    }
}

impl fmt::Display for PlaybackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.user_message)
    }
}

impl std::error::Error for PlaybackError {}
