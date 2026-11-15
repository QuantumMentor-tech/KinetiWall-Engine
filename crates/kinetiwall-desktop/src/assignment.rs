//! Per-monitor wallpaper assignment model, persistence, validation, and conflict resolution (Step 43).
//!
//! Provides a testable, reliable assignment subsystem that links wallpaper catalog entries
//! to validated application-level display identities (`DisplayId`), maintaining strict separation
//! between saved user intent, catalog availability, display topology state, and native playback/attachment.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use kinetiwall_catalog::{
    AvailabilityState, WallpaperCatalogRepository, WallpaperId, WallpaperType,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::display::{DisplayGeometry, DisplayId, DisplayTopologySnapshot};
use crate::layout::MultiMonitorLayoutMode;

/// Current assignment store schema version.
pub const CURRENT_ASSIGNMENT_SCHEMA_VERSION: u32 = 1;

/// Maximum number of diagnostic events retained in memory.
pub const MAX_ASSIGNMENT_DIAGNOSTIC_EVENTS: usize = 100;

// -------------------------------------------------------------------------------------------------
// Domain Model: Assignment Entities
// -------------------------------------------------------------------------------------------------

/// Explicit attribution of an assignment record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentKind {
    /// Explicit user assignment specifically bound to a particular display.
    Explicit,
    /// Derived from the application's global wallpaper preference.
    GlobalFallback,
}

impl std::fmt::Display for AssignmentKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Explicit => write!(f, "explicit"),
            Self::GlobalFallback => write!(f, "global_fallback"),
        }
    }
}

/// Persisted record representing the user's intent to display a specific wallpaper on a display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayAssignment {
    /// Unique identifier for this assignment record.
    pub assignment_id: Uuid,

    /// Stable application-level display identifier (Step 42).
    pub display_id: DisplayId,

    /// Target wallpaper catalog identifier (UUIDv4).
    pub wallpaper_id: WallpaperId,

    /// Assignment source classification.
    pub kind: AssignmentKind,

    /// Timestamp when this assignment was first created.
    pub created_at: DateTime<Utc>,

    /// Timestamp when this assignment was last modified.
    pub updated_at: DateTime<Utc>,
}

impl DisplayAssignment {
    /// Creates a new explicit assignment.
    #[must_use]
    pub fn new_explicit(display_id: DisplayId, wallpaper_id: WallpaperId) -> Self {
        let now = Utc::now();
        Self {
            assignment_id: Uuid::new_v4(),
            display_id,
            wallpaper_id,
            kind: AssignmentKind::Explicit,
            created_at: now,
            updated_at: now,
        }
    }
}

/// Persistent container representing all saved display assignments and global defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssignmentStoreData {
    /// Schema version for forward/backward compatibility.
    pub schema_version: u32,

    /// Global wallpaper selection for displays without an explicit assignment.
    #[serde(default)]
    pub global_wallpaper_id: Option<WallpaperId>,

    /// Map of `DisplayId` string to explicit `DisplayAssignment` (enforces uniqueness).
    #[serde(default)]
    pub assignments: BTreeMap<String, DisplayAssignment>,

    /// Selected multi-monitor layout mode (Independent, Span, Clone). Defaults to Independent.
    #[serde(default)]
    pub layout_mode: MultiMonitorLayoutMode,

    /// Timestamp when the store was last updated.
    pub updated_at: DateTime<Utc>,
}

impl Default for AssignmentStoreData {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_ASSIGNMENT_SCHEMA_VERSION,
            global_wallpaper_id: None,
            assignments: BTreeMap::new(),
            layout_mode: MultiMonitorLayoutMode::Independent,
            updated_at: Utc::now(),
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Resolution & Status Classification
// -------------------------------------------------------------------------------------------------

/// Operational presence status of a display in the active display snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayPresenceStatus {
    /// Display is enumerated and verified in the active display topology snapshot.
    Connected,
    /// Display is not present in the current snapshot; saved assignment is preserved safely.
    Disconnected,
    /// Multiple physical displays matched or device identity is ambiguous.
    Ambiguous,
    /// Display identifier is session-scoped only and cannot be persisted across sessions.
    SessionScopedOnly,
}

/// Comprehensive outcome of resolving a display's assignment against catalog and display state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentResolutionOutcome {
    /// Valid explicit assignment; display is connected and wallpaper is verified available.
    ValidExplicit,

    /// Valid assignment derived from global fallback; display is connected and wallpaper is available.
    ValidGlobalFallback,

    /// No explicit assignment exists and no global fallback is configured.
    Unassigned,

    /// Stored wallpaper ID does not exist in the wallpaper catalog.
    WallpaperMissing,

    /// Wallpaper exists in catalog, but underlying file is missing or inaccessible on disk.
    WallpaperUnavailable,

    /// Target display is currently disconnected; saved user intent is preserved.
    DisplayDisconnected,

    /// Display identity is ambiguous; user clarification required.
    DisplayAmbiguous,

    /// Wallpaper content type or format is unsupported by the playback engine.
    IncompatibleContentType,

    /// Conflicting assignment rules or contradictory metadata detected.
    Conflicting,
}

impl AssignmentResolutionOutcome {
    /// Returns true only when the assignment represents an immediately playable setup.
    #[must_use]
    pub const fn is_playable(&self) -> bool {
        matches!(self, Self::ValidExplicit | Self::ValidGlobalFallback)
    }
}

/// Full runtime-resolved view of a display and its effective wallpaper assignment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedDisplayAssignment {
    /// Application-level display identifier.
    pub display_id: DisplayId,

    /// Active presence status in the Windows desktop environment.
    pub display_presence: DisplayPresenceStatus,

    /// Display geometry if the display is currently connected.
    pub target_geometry: Option<DisplayGeometry>,

    /// Resolved wallpaper catalog identifier, if assigned.
    pub wallpaper_id: Option<WallpaperId>,

    /// Source attribution of the assignment, if assigned.
    pub assignment_kind: Option<AssignmentKind>,

    /// Resolution outcome classification.
    pub outcome: AssignmentResolutionOutcome,

    /// Whether this display is immediately ready for native playback.
    pub is_playable: bool,

    /// Wallpaper title from catalog metadata, if available.
    pub effective_title: Option<String>,

    /// Wallpaper media type (video/web), if available.
    pub effective_type: Option<WallpaperType>,

    /// Topology snapshot generation against which resolution was evaluated.
    pub snapshot_generation: u64,
}

/// Diagnostic event recorded during assignment mutations or resolution cycles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssignmentDiagnosticEvent {
    pub event_id: Uuid,
    pub timestamp_millis: u64,
    pub display_id: Option<DisplayId>,
    pub wallpaper_id: Option<WallpaperId>,
    pub description: String,
    pub outcome: AssignmentResolutionOutcome,
}

// -------------------------------------------------------------------------------------------------
// Error Taxonomy
// -------------------------------------------------------------------------------------------------

/// Errors produced during assignment operations, validation, and persistence.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum AssignmentError {
    #[error("Persistence I/O failure: {0}")]
    StorageFailure(String),

    #[error("Serialization / Deserialization error: {0}")]
    SerializationFailure(String),

    #[error("Wallpaper `{0}` does not exist in the local catalog")]
    WallpaperNotFound(WallpaperId),

    #[error("Wallpaper asset `{id}` is unavailable on disk: {reason}")]
    WallpaperUnavailable { id: WallpaperId, reason: String },

    #[error("Display identity `{0}` is ambiguous in current topology")]
    DisplayAmbiguous(DisplayId),

    #[error("Display identity `{0}` is session-scoped and cannot be safely persisted")]
    DisplaySessionScoped(DisplayId),

    #[error("Unsupported assignment schema version: expected <= {expected}, found {found}")]
    UnsupportedSchemaVersion { expected: u32, found: u32 },

    #[error("Conflicting assignment detected for display `{display_id}`: already assigned to `{existing_wallpaper}`")]
    ConflictingAssignment {
        display_id: DisplayId,
        existing_wallpaper: WallpaperId,
    },

    #[error("Catalog error during assignment validation: {0}")]
    CatalogError(String),
}

// -------------------------------------------------------------------------------------------------
// Assignment Repository Abstraction & Storage
// -------------------------------------------------------------------------------------------------

/// Repository contract for loading and saving display assignment stores.
pub trait AssignmentRepository: Send + Sync {
    /// Loads the stored assignment data.
    fn load_store(&self) -> Result<AssignmentStoreData, AssignmentError>;

    /// Atomically persists assignment data.
    fn save_store(&self, store: &AssignmentStoreData) -> Result<(), AssignmentError>;
}

/// File-based atomic JSON repository for assignment storage.
#[derive(Debug, Clone)]
pub struct FileAssignmentRepository {
    store_path: PathBuf,
}

impl FileAssignmentRepository {
    /// Creates a repository targeting the given JSON file path.
    #[must_use]
    pub fn new(store_path: impl Into<PathBuf>) -> Self {
        Self {
            store_path: store_path.into(),
        }
    }

    /// Returns the target store path.
    #[must_use]
    pub fn store_path(&self) -> &Path {
        &self.store_path
    }
}

impl AssignmentRepository for FileAssignmentRepository {
    fn load_store(&self) -> Result<AssignmentStoreData, AssignmentError> {
        if !self.store_path.exists() {
            return Ok(AssignmentStoreData::default());
        }

        let contents = fs::read_to_string(&self.store_path)
            .map_err(|e| AssignmentError::StorageFailure(e.to_string()))?;

        let store: AssignmentStoreData = serde_json::from_str(&contents)
            .map_err(|e| AssignmentError::SerializationFailure(e.to_string()))?;

        if store.schema_version > CURRENT_ASSIGNMENT_SCHEMA_VERSION {
            return Err(AssignmentError::UnsupportedSchemaVersion {
                expected: CURRENT_ASSIGNMENT_SCHEMA_VERSION,
                found: store.schema_version,
            });
        }

        Ok(store)
    }

    fn save_store(&self, store: &AssignmentStoreData) -> Result<(), AssignmentError> {
        if let Some(parent) = self.store_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| AssignmentError::StorageFailure(e.to_string()))?;
        }

        let serialized = serde_json::to_string_pretty(store)
            .map_err(|e| AssignmentError::SerializationFailure(e.to_string()))?;

        let temp_path = self.store_path.with_extension("tmp");
        {
            let mut file = File::create(&temp_path)
                .map_err(|e| AssignmentError::StorageFailure(e.to_string()))?;
            file.write_all(serialized.as_bytes())
                .map_err(|e| AssignmentError::StorageFailure(e.to_string()))?;
            file.sync_all()
                .map_err(|e| AssignmentError::StorageFailure(e.to_string()))?;
        }

        fs::rename(&temp_path, &self.store_path)
            .map_err(|e| AssignmentError::StorageFailure(e.to_string()))?;

        Ok(())
    }
}

/// In-memory mock repository for deterministic unit tests.
#[derive(Debug, Default, Clone)]
pub struct MockAssignmentRepository {
    store: Arc<Mutex<AssignmentStoreData>>,
    pub fail_load: Arc<Mutex<bool>>,
    pub fail_save: Arc<Mutex<bool>>,
}

impl MockAssignmentRepository {
    #[must_use]
    pub fn new() -> Self {
        Self {
            store: Arc::new(Mutex::new(AssignmentStoreData::default())),
            fail_load: Arc::new(Mutex::new(false)),
            fail_save: Arc::new(Mutex::new(false)),
        }
    }

    pub fn set_data(&self, data: AssignmentStoreData) {
        *self.store.lock().unwrap() = data;
    }
}

impl AssignmentRepository for MockAssignmentRepository {
    fn load_store(&self) -> Result<AssignmentStoreData, AssignmentError> {
        if *self.fail_load.lock().unwrap() {
            return Err(AssignmentError::StorageFailure(
                "Mock simulated load failure".into(),
            ));
        }
        Ok(self.store.lock().unwrap().clone())
    }

    fn save_store(&self, store: &AssignmentStoreData) -> Result<(), AssignmentError> {
        if *self.fail_save.lock().unwrap() {
            return Err(AssignmentError::StorageFailure(
                "Mock simulated save failure".into(),
            ));
        }
        *self.store.lock().unwrap() = store.clone();
        Ok(())
    }
}

// -------------------------------------------------------------------------------------------------
// Assignment Service & Resolution Engine
// -------------------------------------------------------------------------------------------------

/// Application service managing display wallpaper assignments, atomic persistence,
/// and deterministic resolution against topology snapshots and the wallpaper catalog.
pub struct AssignmentService {
    repository: Arc<dyn AssignmentRepository>,
    store: Arc<RwLock<AssignmentStoreData>>,
    diagnostic_history: Arc<Mutex<Vec<AssignmentDiagnosticEvent>>>,
}

impl AssignmentService {
    /// Creates a new `AssignmentService` backed by the given repository.
    pub fn new(repository: Arc<dyn AssignmentRepository>) -> Result<Self, AssignmentError> {
        let store = repository.load_store()?;
        Ok(Self {
            repository,
            store: Arc::new(RwLock::new(store)),
            diagnostic_history: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// Retrieves the current global fallback wallpaper ID, if set.
    #[must_use]
    pub fn get_global_wallpaper(&self) -> Option<WallpaperId> {
        self.store.read().unwrap().global_wallpaper_id
    }

    /// Sets or clears the global fallback wallpaper selection.
    pub fn set_global_wallpaper(
        &self,
        wallpaper_id: Option<WallpaperId>,
    ) -> Result<(), AssignmentError> {
        let mut store = self.store.write().unwrap();
        if store.global_wallpaper_id == wallpaper_id {
            return Ok(());
        }

        store.global_wallpaper_id = wallpaper_id;
        store.updated_at = Utc::now();
        self.repository.save_store(&store)?;

        self.record_diagnostic(
            None,
            wallpaper_id,
            format!("Global fallback wallpaper updated to: {:?}", wallpaper_id),
            if wallpaper_id.is_some() {
                AssignmentResolutionOutcome::ValidGlobalFallback
            } else {
                AssignmentResolutionOutcome::Unassigned
            },
        );

        Ok(())
    }

    /// Retrieves the saved multi-monitor presentation layout mode.
    #[must_use]
    pub fn get_layout_mode(&self) -> MultiMonitorLayoutMode {
        self.store.read().unwrap().layout_mode
    }

    /// Updates and persists the multi-monitor presentation layout mode.
    pub fn set_layout_mode(&self, mode: MultiMonitorLayoutMode) -> Result<(), AssignmentError> {
        let mut store = self.store.write().unwrap();
        if store.layout_mode == mode {
            return Ok(());
        }

        store.layout_mode = mode;
        store.updated_at = Utc::now();
        self.repository.save_store(&store)?;

        self.record_diagnostic(
            None,
            None,
            format!("Multi-monitor layout mode updated to: {:?}", mode),
            AssignmentResolutionOutcome::ValidExplicit,
        );

        Ok(())
    }

    /// Assigns a wallpaper to a specific display ID explicitly.
    ///
    /// If an assignment already exists for the display, it is atomically replaced.
    /// If the requested assignment is identical to the current one, this is an idempotent no-op.
    pub fn assign_wallpaper(
        &self,
        display_id: &DisplayId,
        wallpaper_id: WallpaperId,
    ) -> Result<DisplayAssignment, AssignmentError> {
        let mut store = self.store.write().unwrap();

        // Idempotency check
        if let Some(existing) = store.assignments.get(&display_id.0) {
            if existing.wallpaper_id == wallpaper_id {
                return Ok(existing.clone());
            }
        }

        let assignment = DisplayAssignment::new_explicit(display_id.clone(), wallpaper_id);
        store
            .assignments
            .insert(display_id.0.clone(), assignment.clone());
        store.updated_at = Utc::now();

        self.repository.save_store(&store)?;

        self.record_diagnostic(
            Some(display_id.clone()),
            Some(wallpaper_id),
            format!(
                "Explicit wallpaper assignment persisted for display `{}`",
                display_id
            ),
            AssignmentResolutionOutcome::ValidExplicit,
        );

        Ok(assignment)
    }

    /// Removes an explicit assignment for a display ID.
    ///
    /// Returns `true` if an assignment was removed, or `false` if none existed.
    pub fn remove_assignment(&self, display_id: &DisplayId) -> Result<bool, AssignmentError> {
        let mut store = self.store.write().unwrap();
        if store.assignments.remove(&display_id.0).is_some() {
            store.updated_at = Utc::now();
            self.repository.save_store(&store)?;

            self.record_diagnostic(
                Some(display_id.clone()),
                None,
                format!("Explicit assignment removed for display `{}`", display_id),
                AssignmentResolutionOutcome::Unassigned,
            );

            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Clears all explicit assignments. Returns the count of removed assignments.
    pub fn clear_all_explicit_assignments(&self) -> Result<usize, AssignmentError> {
        let mut store = self.store.write().unwrap();
        let count = store.assignments.len();
        if count == 0 {
            return Ok(0);
        }

        store.assignments.clear();
        store.updated_at = Utc::now();
        self.repository.save_store(&store)?;

        self.record_diagnostic(
            None,
            None,
            format!("Cleared all {} explicit assignments", count),
            AssignmentResolutionOutcome::Unassigned,
        );

        Ok(count)
    }

    /// Retrieves the saved explicit assignment for a display ID, if present.
    #[must_use]
    pub fn get_assignment(&self, display_id: &DisplayId) -> Option<DisplayAssignment> {
        self.store
            .read()
            .unwrap()
            .assignments
            .get(&display_id.0)
            .cloned()
    }

    /// Lists all currently saved explicit assignments.
    #[must_use]
    pub fn list_assignments(&self) -> Vec<DisplayAssignment> {
        self.store
            .read()
            .unwrap()
            .assignments
            .values()
            .cloned()
            .collect()
    }

    /// Resolves an assignment for a given display against the current topology snapshot and catalog.
    pub fn resolve_display<R: WallpaperCatalogRepository>(
        &self,
        display_id: &DisplayId,
        snapshot: &DisplayTopologySnapshot,
        catalog: &R,
    ) -> ResolvedDisplayAssignment {
        let store = self.store.read().unwrap();
        let display_desc_opt = snapshot.find_by_id(display_id);

        let (presence, geometry) = match display_desc_opt {
            Some(desc) => (DisplayPresenceStatus::Connected, Some(desc.geometry)),
            None => (DisplayPresenceStatus::Disconnected, None),
        };

        // Determine effective assignment: explicit preferred over global fallback
        let (wallpaper_id_opt, kind_opt) =
            if let Some(explicit) = store.assignments.get(&display_id.0) {
                (Some(explicit.wallpaper_id), Some(AssignmentKind::Explicit))
            } else if let Some(global_id) = store.global_wallpaper_id {
                (Some(global_id), Some(AssignmentKind::GlobalFallback))
            } else {
                (None, None)
            };

        // If no assignment exists at all
        let Some(wallpaper_id) = wallpaper_id_opt else {
            return ResolvedDisplayAssignment {
                display_id: display_id.clone(),
                display_presence: presence,
                target_geometry: geometry,
                wallpaper_id: None,
                assignment_kind: None,
                outcome: AssignmentResolutionOutcome::Unassigned,
                is_playable: false,
                effective_title: None,
                effective_type: None,
                snapshot_generation: snapshot.generation,
            };
        };

        // If target display is disconnected, user intent is preserved safely
        if presence == DisplayPresenceStatus::Disconnected {
            return ResolvedDisplayAssignment {
                display_id: display_id.clone(),
                display_presence: presence,
                target_geometry: None,
                wallpaper_id: Some(wallpaper_id),
                assignment_kind: kind_opt,
                outcome: AssignmentResolutionOutcome::DisplayDisconnected,
                is_playable: false,
                effective_title: None,
                effective_type: None,
                snapshot_generation: snapshot.generation,
            };
        }

        // Query catalog for the target wallpaper
        match catalog.get(wallpaper_id) {
            Ok(Some(record)) => {
                let is_available = matches!(record.availability, AvailabilityState::Available);
                let outcome = if is_available {
                    match kind_opt {
                        Some(AssignmentKind::Explicit) => {
                            AssignmentResolutionOutcome::ValidExplicit
                        }
                        _ => AssignmentResolutionOutcome::ValidGlobalFallback,
                    }
                } else {
                    AssignmentResolutionOutcome::WallpaperUnavailable
                };

                ResolvedDisplayAssignment {
                    display_id: display_id.clone(),
                    display_presence: presence,
                    target_geometry: geometry,
                    wallpaper_id: Some(wallpaper_id),
                    assignment_kind: kind_opt,
                    outcome,
                    is_playable: outcome.is_playable(),
                    effective_title: Some(record.metadata.title),
                    effective_type: Some(record.wallpaper_type),
                    snapshot_generation: snapshot.generation,
                }
            }
            Ok(None) => ResolvedDisplayAssignment {
                display_id: display_id.clone(),
                display_presence: presence,
                target_geometry: geometry,
                wallpaper_id: Some(wallpaper_id),
                assignment_kind: kind_opt,
                outcome: AssignmentResolutionOutcome::WallpaperMissing,
                is_playable: false,
                effective_title: None,
                effective_type: None,
                snapshot_generation: snapshot.generation,
            },
            Err(_) => ResolvedDisplayAssignment {
                display_id: display_id.clone(),
                display_presence: presence,
                target_geometry: geometry,
                wallpaper_id: Some(wallpaper_id),
                assignment_kind: kind_opt,
                outcome: AssignmentResolutionOutcome::WallpaperUnavailable,
                is_playable: false,
                effective_title: None,
                effective_type: None,
                snapshot_generation: snapshot.generation,
            },
        }
    }

    /// Resolves assignments for all displays in the active snapshot plus any disconnected saved displays.
    pub fn resolve_all_displays<R: WallpaperCatalogRepository>(
        &self,
        snapshot: &DisplayTopologySnapshot,
        catalog: &R,
    ) -> Vec<ResolvedDisplayAssignment> {
        let mut results = Vec::new();
        let mut seen_ids = std::collections::HashSet::new();

        // 1. Resolve all currently connected displays in the active snapshot
        for display in &snapshot.displays {
            seen_ids.insert(display.id.clone());
            results.push(self.resolve_display(&display.id, snapshot, catalog));
        }

        // 2. Also resolve any saved assignments for displays that are currently disconnected
        let store = self.store.read().unwrap();
        for explicit in store.assignments.values() {
            if !seen_ids.contains(&explicit.display_id) {
                seen_ids.insert(explicit.display_id.clone());
                results.push(self.resolve_display(&explicit.display_id, snapshot, catalog));
            }
        }

        results
    }

    /// Returns recorded diagnostic events.
    #[must_use]
    pub fn get_diagnostic_history(&self) -> Vec<AssignmentDiagnosticEvent> {
        self.diagnostic_history.lock().unwrap().clone()
    }

    /// Clears recorded diagnostic events.
    pub fn clear_diagnostics(&self) {
        self.diagnostic_history.lock().unwrap().clear();
    }

    fn record_diagnostic(
        &self,
        display_id: Option<DisplayId>,
        wallpaper_id: Option<WallpaperId>,
        description: String,
        outcome: AssignmentResolutionOutcome,
    ) {
        let now_millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let mut history = self.diagnostic_history.lock().unwrap();
        if history.len() >= MAX_ASSIGNMENT_DIAGNOSTIC_EVENTS {
            history.remove(0);
        }

        history.push(AssignmentDiagnosticEvent {
            event_id: Uuid::new_v4(),
            timestamp_millis: now_millis,
            display_id,
            wallpaper_id,
            description,
            outcome,
        });
    }
}
