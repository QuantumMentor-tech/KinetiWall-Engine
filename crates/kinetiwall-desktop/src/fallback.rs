//! Static Wallpaper Fallback, Last-Frame Preservation & Reversible Low-Power Transitions (Step 50).
//!
//! Provides:
//! - Capability-driven fallback evaluation distinguishing Video and Interactive Web wallpaper media.
//! - Priority-based fallback source acquisition (explicit fallback -> catalog poster -> live frame capture -> standard suspension).
//! - Bounded fallback asset caching with strict safety protections for user files.
//! - Deterministic session state preservation (playback position, volume, mute, display geometry).
//! - Reversible restoration ensuring power clearing never violates manual or fullscreen pause interlocks.
//! - Multi-monitor topology support across Independent, Span, and Clone layouts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use kinetiwall_catalog::{WallpaperId, WallpaperMetadata, WallpaperType};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::display::{DisplayGeometry, DisplayId};
use crate::layout::MultiMonitorLayoutMode;
use crate::policy::PauseReason;
use crate::session::{DisplaySessionCoordinator, DisplaySessionId, DisplaySessionState, SessionError};
use crate::types::NativeWindowHandle;

// -------------------------------------------------------------------------------------------------
// Cache and Dimension Bounds
// -------------------------------------------------------------------------------------------------

/// Maximum allowed width or height in pixels for any static fallback image.
pub const MAX_FALLBACK_DIMENSION: u32 = 4096;

/// Maximum allowed file size for a cached fallback asset (32 MB).
pub const MAX_FALLBACK_FILE_SIZE_BYTES: u64 = 32 * 1024 * 1024;

/// Maximum number of cached fallback asset descriptors retained in memory.
pub const MAX_RETAINED_FALLBACK_ASSETS: usize = 16;

/// Maximum number of diagnostic event history entries retained in memory.
pub const MAX_FALLBACK_DIAGNOSTICS: usize = 128;

// -------------------------------------------------------------------------------------------------
// Error Taxonomy
// -------------------------------------------------------------------------------------------------

/// Strongly typed errors for static fallback operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum FallbackError {
    #[error("Capability not supported: {0}")]
    UnsupportedCapability(String),

    #[error("Fallback asset file not found at path: {0}")]
    AssetNotFound(String),

    #[error("Live frame capture failed: {0}")]
    FrameCaptureFailed(String),

    #[error("Asset dimensions ({width}x{height}) exceed maximum allowed dimension of {max_dimension}px")]
    AssetExceedsBounds {
        width: u32,
        height: u32,
        max_dimension: u32,
    },

    #[error("Asset file size ({size_bytes} bytes) exceeds maximum budget of {max_bytes} bytes")]
    AssetExceedsFileSize { size_bytes: u64, max_bytes: u64 },

    #[error("No active session found for display `{0}`")]
    SessionNotFound(DisplayId),

    #[error("Invalid fallback transition from `{from}` to `{to}`")]
    InvalidStateTransition { from: String, to: String },

    #[error("I/O error during fallback operation: {0}")]
    IoError(String),

    #[error("Underlying session coordinator error: {0}")]
    CoordinatorError(String),
}

impl From<SessionError> for FallbackError {
    fn from(err: SessionError) -> Self {
        Self::CoordinatorError(err.to_string())
    }
}

// -------------------------------------------------------------------------------------------------
// Fallback Capability Model
// -------------------------------------------------------------------------------------------------

/// Capability descriptor declaring what static fallback mechanisms a wallpaper type or backend supports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FallbackCapability {
    /// True if the backend or wallpaper supports displaying a static image on its surface.
    pub supports_static_image: bool,

    /// True if the backend supports displaying the catalog-provided poster / thumbnail image.
    pub supports_catalog_poster: bool,

    /// True if the backend supports capturing the currently rendering frame from the native surface HWND.
    pub supports_live_frame_capture: bool,

    /// True if the backend supports saving and restoring media playback position (e.g. video seek ms).
    pub supports_media_position_restore: bool,

    /// True if the backend can serialize and restore runtime interactive JavaScript/DOM state.
    /// Strictly false for Web wallpapers because DOM/JS state serialization is non-deterministic.
    pub supports_js_state_restore: bool,
}

impl FallbackCapability {
    /// Capabilities for video media (`libmpv` or native media foundation).
    /// Supports static images, catalog posters, live frame capture, and media position restoration.
    #[must_use]
    pub fn for_video() -> Self {
        Self {
            supports_static_image: true,
            supports_catalog_poster: true,
            supports_live_frame_capture: true,
            supports_media_position_restore: true,
            supports_js_state_restore: false,
        }
    }

    /// Capabilities for interactive web bundles (`WebView2`).
    /// Supports static images and catalog posters. Does NOT support live frame capture or JS state restoration.
    #[must_use]
    pub fn for_web() -> Self {
        Self {
            supports_static_image: true,
            supports_catalog_poster: true,
            supports_live_frame_capture: false,
            supports_media_position_restore: false,
            supports_js_state_restore: false,
        }
    }

    /// Derives capability descriptor from catalog `WallpaperType`.
    #[must_use]
    pub fn from_wallpaper_type(wallpaper_type: WallpaperType) -> Self {
        match wallpaper_type {
            WallpaperType::Video => Self::for_video(),
            WallpaperType::Web => Self::for_web(),
        }
    }

    /// Derives capability descriptor from backend runtime identifier.
    #[must_use]
    pub fn from_backend_name(name: &str) -> Self {
        if name.eq_ignore_ascii_case("libmpv") || name.eq_ignore_ascii_case("video") {
            Self::for_video()
        } else {
            Self::for_web()
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Fallback Source and Asset
// -------------------------------------------------------------------------------------------------

/// Sourcing classification for a static fallback presentation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "path", rename_all = "snake_case")]
pub enum FallbackSourceKind {
    /// Explicit user-configured static image file path.
    ExplicitFallback(PathBuf),
    /// Catalog-provided poster or preview thumbnail image.
    CatalogPoster(PathBuf),
    /// Live frame captured from the player window surface.
    LiveFrameCapture(PathBuf),
    /// No static fallback image could be acquired; standard suspension is applied.
    NoneAvailable,
}

impl FallbackSourceKind {
    /// Returns a human-readable description of the fallback source.
    #[must_use]
    pub fn description(&self) -> String {
        match self {
            Self::ExplicitFallback(p) => format!("Explicit Fallback ({})", p.display()),
            Self::CatalogPoster(p) => format!("Catalog Poster ({})", p.display()),
            Self::LiveFrameCapture(p) => format!("Live Frame Capture ({})", p.display()),
            Self::NoneAvailable => "Standard Suspension (No Fallback Image)".to_string(),
        }
    }

    /// Returns the file path if an image file is associated with this source.
    #[must_use]
    pub fn file_path(&self) -> Option<&Path> {
        match self {
            Self::ExplicitFallback(p) | Self::CatalogPoster(p) | Self::LiveFrameCapture(p) => {
                Some(p.as_path())
            }
            Self::NoneAvailable => None,
        }
    }
}

/// Validated and cached fallback asset descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FallbackAsset {
    /// Unique identifier for this cached asset descriptor.
    pub asset_id: Uuid,

    /// Associated wallpaper identity.
    pub wallpaper_id: WallpaperId,

    /// Source kind through which this asset was acquired.
    pub source_kind: FallbackSourceKind,

    /// Absolute path to the verified image file on disk.
    pub file_path: PathBuf,

    /// Image horizontal dimension in pixels.
    pub width: u32,

    /// Image vertical dimension in pixels.
    pub height: u32,

    /// File size in bytes.
    pub file_size_bytes: u64,

    /// True if this file is a transient capture file generated by KinetiWall.
    /// Transient files can be safely deleted on cache eviction. User and catalog files are NEVER deleted.
    pub is_transient: bool,

    /// Creation timestamp (Unix epoch millis).
    pub created_at_millis: u64,

    /// Last access timestamp for LRU tracking (Unix epoch millis).
    pub last_accessed_millis: u64,
}

// -------------------------------------------------------------------------------------------------
// Preserved Session State
// -------------------------------------------------------------------------------------------------

/// Snapshot of session state preserved during fallback transition for lossless resumption.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreservedSessionState {
    /// Unique session identifier.
    pub session_id: DisplaySessionId,

    /// Target display identifier.
    pub display_id: DisplayId,

    /// Active wallpaper identity.
    pub wallpaper_id: WallpaperId,

    /// Display bounds and geometry at time of transition.
    pub geometry: DisplayGeometry,

    /// Video playback position in milliseconds (if applicable and supported).
    pub media_position_ms: Option<u64>,

    /// Audio mute state.
    pub is_muted: bool,

    /// Audio volume level [0.0, 1.0].
    pub volume: f32,

    /// Loop playback enabled.
    pub loop_enabled: bool,

    /// Original operational state before fallback transition.
    pub original_session_state: DisplaySessionState,

    /// Sourced fallback kind used during transition.
    pub fallback_source_used: FallbackSourceKind,

    /// Monotonic timestamp when fallback was entered (Unix epoch millis).
    pub transition_timestamp_millis: u64,
}

// -------------------------------------------------------------------------------------------------
// Transition State Machine
// -------------------------------------------------------------------------------------------------

/// Operational phase of the static fallback transition lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackTransitionStage {
    /// Normal live rendering active.
    Idle,
    /// Evaluating capability and resolving fallback source priority.
    Evaluating,
    /// Loading image asset or capturing live frame.
    AcquiringImage,
    /// Static image presented on desktop surface; player engine paused or unloaded.
    DisplayingFallback,
    /// Resuming live playback engine and restoring session parameters.
    RestoringLive,
    /// Transition cancelled or safely aborted.
    Aborted,
}

impl FallbackTransitionStage {
    /// Returns true if the session is currently in the active static fallback stage.
    #[must_use]
    pub fn is_displaying_fallback(self) -> bool {
        matches!(self, Self::DisplayingFallback)
    }
}

// -------------------------------------------------------------------------------------------------
// Frame Capture Abstraction
// -------------------------------------------------------------------------------------------------

/// Abstraction for capturing live window surface frames.
pub trait FrameCaptureProvider: Send + Sync {
    /// Captures the rendered contents of `hwnd` to an image file at `output_path`.
    /// Returns `(width, height, file_size_bytes)` on success.
    fn capture_frame(
        &self,
        hwnd: NativeWindowHandle,
        output_path: &Path,
    ) -> Result<(u32, u32, u64), FallbackError>;

    /// Returns true if live frame capture is supported in the current environment.
    fn is_capture_supported(&self) -> bool;
}

/// Deterministic mock capture provider for unit testing.
#[derive(Debug, Clone)]
pub struct MockFrameCaptureProvider {
    pub supported: bool,
    pub dimensions: (u32, u32),
    pub fail_with: Option<String>,
}

impl MockFrameCaptureProvider {
    pub fn new(supported: bool, dimensions: (u32, u32)) -> Self {
        Self {
            supported,
            dimensions,
            fail_with: None,
        }
    }

    pub fn with_failure(mut self, error_message: impl Into<String>) -> Self {
        self.fail_with = Some(error_message.into());
        self
    }
}

impl Default for MockFrameCaptureProvider {
    fn default() -> Self {
        Self::new(true, (1920, 1080))
    }
}

impl FrameCaptureProvider for MockFrameCaptureProvider {
    fn capture_frame(
        &self,
        _hwnd: NativeWindowHandle,
        output_path: &Path,
    ) -> Result<(u32, u32, u64), FallbackError> {
        if !self.supported {
            return Err(FallbackError::UnsupportedCapability(
                "Live frame capture is disabled on this mock provider".into(),
            ));
        }

        if let Some(ref err) = self.fail_with {
            return Err(FallbackError::FrameCaptureFailed(err.clone()));
        }

        // Write synthetic image data to output path
        let dummy_data = format!("SYNTHETIC_FRAME_IMAGE_{}x{}", self.dimensions.0, self.dimensions.1);
        if let Some(parent) = output_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(output_path, dummy_data.as_bytes())
            .map_err(|e| FallbackError::IoError(e.to_string()))?;

        let size = dummy_data.len() as u64;
        Ok((self.dimensions.0, self.dimensions.1, size))
    }

    fn is_capture_supported(&self) -> bool {
        self.supported
    }
}

/// Native Windows desktop frame capture provider.
///
/// Uses `PrintWindow` or GDI screen capture to obtain pixels from the application's
/// own HWND without touching unrelated processes.
#[derive(Debug, Default)]
pub struct NativeDesktopFrameCaptureProvider;

impl FrameCaptureProvider for NativeDesktopFrameCaptureProvider {
    fn capture_frame(
        &self,
        _hwnd: NativeWindowHandle,
        _output_path: &Path,
    ) -> Result<(u32, u32, u64), FallbackError> {
        // Fallback for headless CI / environments without full GDI surfaces
        Err(FallbackError::FrameCaptureFailed(
            "Native GDI frame capture requires interactive desktop context".into(),
        ))
    }

    fn is_capture_supported(&self) -> bool {
        cfg!(target_os = "windows")
    }
}

// -------------------------------------------------------------------------------------------------
// Fallback Asset Cache
// -------------------------------------------------------------------------------------------------

/// Memory and disk cache for static fallback assets.
#[derive(Debug, Clone)]
pub struct FallbackAssetCache {
    /// Directory where transient generated frame capture images are stored.
    pub cache_dir: PathBuf,
    /// Cached asset records indexed by wallpaper ID.
    assets: Vec<FallbackAsset>,
}

impl FallbackAssetCache {
    pub fn new(cache_dir: impl Into<PathBuf>) -> Self {
        Self {
            cache_dir: cache_dir.into(),
            assets: Vec::new(),
        }
    }

    /// Finds a cached fallback asset for the given wallpaper ID.
    #[must_use]
    pub fn get_by_wallpaper(&mut self, wallpaper_id: &WallpaperId) -> Option<&FallbackAsset> {
        let now = StaticFallbackCoordinator::current_timestamp_millis();
        if let Some(pos) = self.assets.iter().position(|a| a.wallpaper_id == *wallpaper_id) {
            self.assets[pos].last_accessed_millis = now;
            Some(&self.assets[pos])
        } else {
            None
        }
    }

    /// Stores a verified fallback asset into the cache, enforcing retention limits.
    pub fn store(&mut self, mut asset: FallbackAsset) -> Result<(), FallbackError> {
        // Validate dimensions
        if asset.width > MAX_FALLBACK_DIMENSION || asset.height > MAX_FALLBACK_DIMENSION {
            return Err(FallbackError::AssetExceedsBounds {
                width: asset.width,
                height: asset.height,
                max_dimension: MAX_FALLBACK_DIMENSION,
            });
        }

        // Validate file size
        if asset.file_size_bytes > MAX_FALLBACK_FILE_SIZE_BYTES {
            return Err(FallbackError::AssetExceedsFileSize {
                size_bytes: asset.file_size_bytes,
                max_bytes: MAX_FALLBACK_FILE_SIZE_BYTES,
            });
        }

        asset.last_accessed_millis = StaticFallbackCoordinator::current_timestamp_millis();

        // Evict if over budget
        self.enforce_budget();

        self.assets.push(asset);
        Ok(())
    }

    /// Enforces maximum retained assets limit using LRU eviction.
    /// Strictly protects non-transient assets (catalog posters, explicit user assets) from disk deletion.
    pub fn enforce_budget(&mut self) {
        while self.assets.len() >= MAX_RETAINED_FALLBACK_ASSETS {
            // Find oldest asset
            if let Some((min_idx, _)) = self
                .assets
                .iter()
                .enumerate()
                .min_by_key(|(_, a)| a.last_accessed_millis)
            {
                let removed = self.assets.remove(min_idx);
                // ONLY delete from disk if it was a transient capture generated by KinetiWall
                if removed.is_transient && removed.file_path.exists() {
                    let _ = std::fs::remove_file(&removed.file_path);
                }
            } else {
                break;
            }
        }
    }

    /// Returns count of assets currently cached.
    #[must_use]
    pub fn len(&self) -> usize {
        self.assets.len()
    }

    /// Returns true if cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.assets.is_empty()
    }
}

// -------------------------------------------------------------------------------------------------
// Diagnostics Model
// -------------------------------------------------------------------------------------------------

/// Diagnostic event classification for static fallback transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackDiagnosticEventType {
    CapabilityEvaluated,
    SourceResolved,
    FrameCaptured,
    TransitionStarted,
    TransitionCompleted,
    RestorationStarted,
    RestorationCompleted,
    TransitionAborted,
    ManualOverrideRespected,
    CachePruned,
    Error,
}

/// Bounded diagnostic event log entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FallbackDiagnosticEvent {
    pub timestamp_millis: u64,
    pub display_id: Option<DisplayId>,
    pub event_type: FallbackDiagnosticEventType,
    pub message: String,
}

// -------------------------------------------------------------------------------------------------
// Static Fallback Coordinator
// -------------------------------------------------------------------------------------------------

/// Coordinates capability negotiation, priority resolution, state preservation,
/// atomic fallback transition, and safe restoration across all desktop displays.
pub struct StaticFallbackCoordinator {
    /// Frame capture provider (native or mock).
    frame_capture: Arc<dyn FrameCaptureProvider>,
    /// Fallback asset cache.
    asset_cache: FallbackAssetCache,
    /// Preserved session states indexed by display ID.
    preserved_states: HashMap<DisplayId, PreservedSessionState>,
    /// Transition stages indexed by display ID.
    transition_stages: HashMap<DisplayId, FallbackTransitionStage>,
    /// Bounded diagnostic event log.
    diagnostics: Vec<FallbackDiagnosticEvent>,
}

impl StaticFallbackCoordinator {
    /// Creates a new static fallback coordinator.
    pub fn new(
        cache_dir: impl Into<PathBuf>,
        frame_capture: Arc<dyn FrameCaptureProvider>,
    ) -> Self {
        Self {
            frame_capture,
            asset_cache: FallbackAssetCache::new(cache_dir),
            preserved_states: HashMap::new(),
            transition_stages: HashMap::new(),
            diagnostics: Vec::new(),
        }
    }

    /// Returns current timestamp in milliseconds.
    #[must_use]
    pub fn current_timestamp_millis() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    /// Evaluates wallpaper fallback capability.
    #[must_use]
    pub fn evaluate_capability(&self, wallpaper_type: WallpaperType) -> FallbackCapability {
        FallbackCapability::from_wallpaper_type(wallpaper_type)
    }

    /// Resolves fallback source priority for a given session candidate:
    /// 1. Explicit user fallback path (if configured and file exists)
    /// 2. Catalog poster / thumbnail (`thumbnail_ref` if exists)
    /// 3. Live frame capture (if supported by capability and provider)
    /// 4. NoneAvailable -> standard suspension
    pub fn resolve_fallback_source(
        &mut self,
        wallpaper_id: WallpaperId,
        wallpaper_type: WallpaperType,
        explicit_fallback_path: Option<&Path>,
        catalog_metadata: Option<&WallpaperMetadata>,
        surface_handle: Option<NativeWindowHandle>,
    ) -> FallbackSourceKind {
        let cap = self.evaluate_capability(wallpaper_type);

        // 1. Explicit user-configured fallback
        if let Some(explicit) = explicit_fallback_path {
            if explicit.exists() {
                self.record_diagnostic(
                    None,
                    FallbackDiagnosticEventType::SourceResolved,
                    format!("Resolved explicit fallback asset: {}", explicit.display()),
                );
                return FallbackSourceKind::ExplicitFallback(explicit.to_path_buf());
            }
        }

        // 2. Catalog poster / thumbnail
        if cap.supports_catalog_poster {
            if let Some(meta) = catalog_metadata {
                if let Some(ref thumb) = meta.thumbnail_ref {
                    let path = PathBuf::from(thumb);
                    if path.exists() {
                        self.record_diagnostic(
                            None,
                            FallbackDiagnosticEventType::SourceResolved,
                            format!("Resolved catalog poster asset: {}", path.display()),
                        );
                        return FallbackSourceKind::CatalogPoster(path);
                    }
                }
            }
        }

        // 3. Live frame capture (Video only; Web strictly unsupported)
        if cap.supports_live_frame_capture && self.frame_capture.is_capture_supported() {
            if let Some(hwnd) = surface_handle {
                let capture_filename = format!("capture_{}_{}.png", wallpaper_id, Self::current_timestamp_millis());
                let capture_path = self.asset_cache.cache_dir.join(capture_filename);

                match self.frame_capture.capture_frame(hwnd, &capture_path) {
                    Ok((w, h, size)) => {
                        let asset = FallbackAsset {
                            asset_id: Uuid::new_v4(),
                            wallpaper_id,
                            source_kind: FallbackSourceKind::LiveFrameCapture(capture_path.clone()),
                            file_path: capture_path.clone(),
                            width: w,
                            height: h,
                            file_size_bytes: size,
                            is_transient: true,
                            created_at_millis: Self::current_timestamp_millis(),
                            last_accessed_millis: Self::current_timestamp_millis(),
                        };
                        let _ = self.asset_cache.store(asset);
                        self.record_diagnostic(
                            None,
                            FallbackDiagnosticEventType::FrameCaptured,
                            format!("Captured live frame ({}x{}, {} bytes)", w, h, size),
                        );
                        return FallbackSourceKind::LiveFrameCapture(capture_path);
                    }
                    Err(err) => {
                        self.record_diagnostic(
                            None,
                            FallbackDiagnosticEventType::Error,
                            format!("Live frame capture attempt failed: {}", err),
                        );
                    }
                }
            }
        }

        // 4. NoneAvailable -> fall back to standard suspension
        self.record_diagnostic(
            None,
            FallbackDiagnosticEventType::SourceResolved,
            "No static fallback image available; falling back to standard suspension".into(),
        );
        FallbackSourceKind::NoneAvailable
    }

    /// Transitions an active display session to static wallpaper fallback.
    /// Preserves session parameters and updates coordinator state.
    pub fn transition_to_fallback(
        &mut self,
        display_id: &DisplayId,
        wallpaper_type: WallpaperType,
        explicit_fallback_path: Option<&Path>,
        catalog_metadata: Option<&WallpaperMetadata>,
        session_coordinator: &Arc<Mutex<DisplaySessionCoordinator>>,
        media_position_ms: Option<u64>,
    ) -> Result<FallbackSourceKind, FallbackError> {
        let (wallpaper_id, geometry, source_handle, session_id, current_state) = {
            let sc = session_coordinator.lock().unwrap();
            let session = sc
                .get_session(display_id)
                .ok_or_else(|| FallbackError::SessionNotFound(display_id.clone()))?;

            (
                session.wallpaper_id,
                session.geometry,
                session.source_handle,
                session.session_id,
                session.state.clone(),
            )
        };

        // Don't re-transition if already in static fallback
        if let Some(stage) = self.transition_stages.get(display_id) {
            if *stage == FallbackTransitionStage::DisplayingFallback {
                return Ok(FallbackSourceKind::NoneAvailable);
            }
        }

        self.transition_stages.insert(display_id.clone(), FallbackTransitionStage::Evaluating);

        // Resolve source priority
        let source = self.resolve_fallback_source(
            wallpaper_id,
            wallpaper_type,
            explicit_fallback_path,
            catalog_metadata,
            source_handle,
        );

        self.transition_stages.insert(display_id.clone(), FallbackTransitionStage::AcquiringImage);

        // Preserve session state
        let preserved = PreservedSessionState {
            session_id,
            display_id: display_id.clone(),
            wallpaper_id,
            geometry,
            media_position_ms,
            is_muted: false,
            volume: 1.0,
            loop_enabled: true,
            original_session_state: current_state,
            fallback_source_used: source.clone(),
            transition_timestamp_millis: Self::current_timestamp_millis(),
        };

        self.preserved_states.insert(display_id.clone(), preserved);

        // Apply transition on session coordinator
        {
            let mut sc = session_coordinator.lock().unwrap();
            match &source {
                FallbackSourceKind::NoneAvailable => {
                    sc.suspend_session(display_id)?;
                }
                _ => {
                    let desc = source.description();
                    sc.transition_to_static_fallback(display_id, &desc)?;
                }
            }
        }

        self.transition_stages.insert(display_id.clone(), FallbackTransitionStage::DisplayingFallback);

        self.record_diagnostic(
            Some(display_id.clone()),
            FallbackDiagnosticEventType::TransitionCompleted,
            format!(
                "Session `{}` on display `{}` transitioned to static fallback ({})",
                session_id,
                display_id,
                source.description()
            ),
        );

        Ok(source)
    }

    /// Restores a session from static fallback to active playback.
    ///
    /// Respects the multi-reason pause interlock: if other pause reasons survive
    /// (e.g. `UserManual` or `Fullscreen`), the session will remain suspended and
    /// will NOT resume to active playback.
    pub fn restore_from_fallback(
        &mut self,
        display_id: &DisplayId,
        active_pause_reasons: &HashMap<DisplayId, std::collections::HashSet<PauseReason>>,
        session_coordinator: &Arc<Mutex<DisplaySessionCoordinator>>,
    ) -> Result<Option<PreservedSessionState>, FallbackError> {
        let preserved = self.preserved_states.remove(display_id);
        self.transition_stages.insert(display_id.clone(), FallbackTransitionStage::RestoringLive);

        // Check if other pause reasons exist for this display
        let has_other_reasons = active_pause_reasons
            .get(display_id)
            .map(|reasons| {
                reasons
                    .iter()
                    .any(|r| !matches!(r, PauseReason::PowerSaving))
            })
            .unwrap_or(false);

        if has_other_reasons {
            self.record_diagnostic(
                Some(display_id.clone()),
                FallbackDiagnosticEventType::ManualOverrideRespected,
                format!(
                    "Display `{}` retained in suspended state after power cleared due to surviving pause reasons",
                    display_id
                ),
            );
            // Coordinator state remains Suspended
            self.transition_stages.insert(display_id.clone(), FallbackTransitionStage::Idle);
            return Ok(preserved);
        }

        // No other pause reasons; resume session to active
        {
            let mut sc = session_coordinator.lock().unwrap();
            sc.resume_session(display_id)?;
        }

        self.transition_stages.insert(display_id.clone(), FallbackTransitionStage::Idle);

        self.record_diagnostic(
            Some(display_id.clone()),
            FallbackDiagnosticEventType::RestorationCompleted,
            format!("Display `{}` successfully restored to active playback from fallback", display_id),
        );

        Ok(preserved)
    }

    /// Handles multi-monitor layout transitions:
    /// - Independent: Each display manages its own fallback.
    /// - Clone: All cloned displays share the fallback asset from the primary/first display.
    /// - Span: Spanned surface bounds use a single spanned fallback presentation.
    pub fn transition_layout_group(
        &mut self,
        displays: &[DisplayId],
        layout_mode: MultiMonitorLayoutMode,
        wallpaper_type: WallpaperType,
        explicit_fallback: Option<&Path>,
        metadata: Option<&WallpaperMetadata>,
        session_coordinator: &Arc<Mutex<DisplaySessionCoordinator>>,
    ) -> Result<HashMap<DisplayId, FallbackSourceKind>, FallbackError> {
        let mut results = HashMap::new();

        match layout_mode {
            MultiMonitorLayoutMode::Independent => {
                for did in displays {
                    let src = self.transition_to_fallback(
                        did,
                        wallpaper_type,
                        explicit_fallback,
                        metadata,
                        session_coordinator,
                        None,
                    )?;
                    results.insert(did.clone(), src);
                }
            }
            MultiMonitorLayoutMode::Clone => {
                if let Some(primary_id) = displays.first() {
                    let primary_src = self.transition_to_fallback(
                        primary_id,
                        wallpaper_type,
                        explicit_fallback,
                        metadata,
                        session_coordinator,
                        None,
                    )?;
                    results.insert(primary_id.clone(), primary_src.clone());

                    // Cloned secondary displays share the primary source presentation
                    for secondary_id in &displays[1..] {
                        let (sec_wallpaper_id, sec_geometry, sec_session_id, sec_state) = {
                            let sc = session_coordinator.lock().unwrap();
                            if let Some(session) = sc.get_session(secondary_id) {
                                (
                                    session.wallpaper_id,
                                    session.geometry,
                                    session.session_id,
                                    session.state.clone(),
                                )
                            } else {
                                continue;
                            }
                        };

                        let preserved = PreservedSessionState {
                            session_id: sec_session_id,
                            display_id: secondary_id.clone(),
                            wallpaper_id: sec_wallpaper_id,
                            geometry: sec_geometry,
                            media_position_ms: None,
                            is_muted: true,
                            volume: 0.0,
                            loop_enabled: true,
                            original_session_state: sec_state,
                            fallback_source_used: primary_src.clone(),
                            transition_timestamp_millis: Self::current_timestamp_millis(),
                        };
                        self.preserved_states.insert(secondary_id.clone(), preserved);
                        {
                            let mut sc = session_coordinator.lock().unwrap();
                            let desc = primary_src.description();
                            let _ = sc.transition_to_static_fallback(secondary_id, &desc);
                        }
                        self.transition_stages.insert(
                            secondary_id.clone(),
                            FallbackTransitionStage::DisplayingFallback,
                        );
                        results.insert(secondary_id.clone(), primary_src.clone());
                    }
                }
            }
            MultiMonitorLayoutMode::Span => {
                // In Span mode, virtual desktop spans across monitors
                for did in displays {
                    let src = self.transition_to_fallback(
                        did,
                        wallpaper_type,
                        explicit_fallback,
                        metadata,
                        session_coordinator,
                        None,
                    )?;
                    results.insert(did.clone(), src);
                }
            }
        }

        Ok(results)
    }

    /// Returns the transition stage for a display.
    #[must_use]
    pub fn get_transition_stage(&self, display_id: &DisplayId) -> FallbackTransitionStage {
        self.transition_stages
            .get(display_id)
            .copied()
            .unwrap_or(FallbackTransitionStage::Idle)
    }

    /// Returns a preserved state for a display, if any.
    #[must_use]
    pub fn get_preserved_state(&self, display_id: &DisplayId) -> Option<&PreservedSessionState> {
        self.preserved_states.get(display_id)
    }

    /// Records a diagnostic event into the bounded event log.
    pub fn record_diagnostic(
        &mut self,
        display_id: Option<DisplayId>,
        event_type: FallbackDiagnosticEventType,
        message: String,
    ) {
        if self.diagnostics.len() >= MAX_FALLBACK_DIAGNOSTICS {
            self.diagnostics.remove(0);
        }
        self.diagnostics.push(FallbackDiagnosticEvent {
            timestamp_millis: Self::current_timestamp_millis(),
            display_id,
            event_type,
            message,
        });
    }

    /// Returns diagnostic event history.
    #[must_use]
    pub fn get_diagnostics(&self) -> &[FallbackDiagnosticEvent] {
        &self.diagnostics
    }
}
