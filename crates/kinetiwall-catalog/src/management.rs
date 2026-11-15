use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::warn;

use crate::error::{redact_path_for_diagnostics, CatalogError};
use crate::model::{
    AssetReference, AvailabilityState, CatalogEntrySummary, WallpaperId, WallpaperRecord,
    WallpaperSourceKind, WallpaperType,
};
use crate::path_safety::{normalize_external_comparison_key, validate_asset_reference};
use crate::registration::{
    map_path_safety_rejection, verify_bounded_content_signature, ContentCheckError,
    RegistrationRejectionReason,
};
use crate::repository::WallpaperCatalogRepository;
use crate::validation::{classify_extension_wallpaper_type, validate_metadata};

/// Allowlisted user-editable metadata patch for an existing wallpaper catalog entry (`Step 30`).
///
/// Explicitly excludes protected fields (`id`, `schema_version`, `wallpaper_type`, `source_kind`,
/// `created_at`, `is_builtin`, `availability`, and non-user-editable package manifest fields).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditableMetadataPatch {
    pub title: String,
    pub description: String,
    pub tags: Vec<String>,
    /// Optional optimistic-concurrency guard matching the record's known `updated_at` timestamp.
    pub expected_updated_at: Option<DateTime<Utc>>,
}

impl EditableMetadataPatch {
    /// Constructs a patch for allowlisted user-editable metadata fields.
    #[must_use]
    pub fn new(
        title: impl Into<String>,
        description: impl Into<String>,
        tags: Vec<String>,
    ) -> Self {
        Self {
            title: title.into(),
            description: description.into(),
            tags,
            expected_updated_at: None,
        }
    }

    /// Attaches an optimistic concurrency check against the record's expected `updated_at`.
    #[must_use]
    pub fn with_expected_updated_at(mut self, expected_updated_at: DateTime<Utc>) -> Self {
        self.expected_updated_at = Some(expected_updated_at);
        self
    }
}

/// Outcome of a controlled metadata update request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum MetadataUpdateOutcome {
    /// Metadata changed, passed validation, and was atomically persisted.
    Updated { summary: CatalogEntrySummary },
    /// Submitted metadata matched the existing record; no unnecessary disk write was performed.
    Unchanged { summary: CatalogEntrySummary },
}

impl MetadataUpdateOutcome {
    /// Returns the resulting IPC-safe summary regardless of whether a write was needed.
    #[must_use]
    pub fn summary(&self) -> &CatalogEntrySummary {
        match self {
            Self::Updated { summary } | Self::Unchanged { summary } => summary,
        }
    }
}

/// Result of a user-initiated single-file selection dialog for asset relinking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelinkSelectionOutcome {
    /// User cancelled the file picker without choosing a replacement file.
    Cancelled,
    /// User chose a single candidate replacement file path.
    Selected(PathBuf),
}

/// Abstraction over a native single-file picker used when relinking a missing or moved wallpaper asset.
pub trait SingleAssetSelectionSource {
    /// Selects a replacement file path for `target_id`.
    fn select_replacement_asset(
        &mut self,
        target_id: WallpaperId,
    ) -> Result<RelinkSelectionOutcome, String>;
}

/// Deterministic single-file selection source for automated tests and programmatic relinking.
#[derive(Debug, Clone)]
pub struct StaticRelinkSelectionSource {
    result: Result<RelinkSelectionOutcome, String>,
}

impl StaticRelinkSelectionSource {
    #[must_use]
    pub fn selected(path: PathBuf) -> Self {
        Self {
            result: Ok(RelinkSelectionOutcome::Selected(path)),
        }
    }

    #[must_use]
    pub fn cancelled() -> Self {
        Self {
            result: Ok(RelinkSelectionOutcome::Cancelled),
        }
    }

    #[must_use]
    pub fn failing(reason: impl Into<String>) -> Self {
        Self {
            result: Err(reason.into()),
        }
    }
}

impl SingleAssetSelectionSource for StaticRelinkSelectionSource {
    fn select_replacement_asset(
        &mut self,
        _target_id: WallpaperId,
    ) -> Result<RelinkSelectionOutcome, String> {
        self.result.clone()
    }
}

/// Specific reason why a candidate replacement asset was rejected during relinking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum RelinkRejectionReason {
    /// Candidate asset's wallpaper type conflicts with the existing catalog record's immutable `wallpaper_type`.
    TypeMismatch {
        expected_type: WallpaperType,
        candidate_type: WallpaperType,
    },
    /// Candidate external asset is already registered to another catalog record.
    DuplicateOwnedByOther { existing_id: WallpaperId },
    /// Candidate failed path-safety, extension allowlist, or bounded content-header inspection.
    AssetValidation(RegistrationRejectionReason),
}

/// Outcome of a user-initiated missing-asset recovery / relinking operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RelinkOutcome {
    /// Replacement asset passed all checks and the record's reference and availability were updated.
    Relinked {
        id: WallpaperId,
        summary: CatalogEntrySummary,
    },
    /// Candidate resolved to the exact same already-available asset; no write needed.
    UnchangedSameAsset {
        id: WallpaperId,
        summary: CatalogEntrySummary,
    },
    /// User cancelled the file selection dialog; existing record remained untouched.
    Cancelled { id: WallpaperId },
    /// Candidate replacement path was rejected by validation, type compatibility, or duplicate checks.
    Rejected {
        id: WallpaperId,
        redacted_path: String,
        reason: RelinkRejectionReason,
    },
    /// Candidate replacement path was missing or inaccessible; existing record remained untouched.
    InaccessibleOrMissing {
        id: WallpaperId,
        redacted_path: String,
        availability: AvailabilityState,
    },
}

/// Outcome of a user-confirmed catalog-only removal request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RemovalOutcome {
    /// Catalog record manifest was removed while preserving original media files on disk.
    Removed { id: WallpaperId },
    /// Record was already absent from the catalog (idempotent no-op).
    AlreadyRemoved { id: WallpaperId },
    /// Record is currently active/assigned in runtime state and cannot be removed while in use.
    BlockedActiveAssignment { id: WallpaperId },
    /// Record is marked as a protected built-in entry and cannot be removed via user catalog management.
    BlockedBuiltinProtected { id: WallpaperId },
}

/// Top-level error for catalog management workflows (`Step 30`).
#[derive(Debug, Error)]
pub enum CatalogManagementError {
    #[error("another catalog management operation is already in progress")]
    OperationInProgress,

    #[error("stale metadata update for wallpaper `{id}`: expected updated_at `{expected_updated_at}`, found `{actual_updated_at}`")]
    StaleRecordConflict {
        id: WallpaperId,
        expected_updated_at: DateTime<Utc>,
        actual_updated_at: DateTime<Utc>,
    },

    #[error("replacement file selection failed: {0}")]
    SelectionFailed(String),

    #[error(transparent)]
    Catalog(#[from] CatalogError),
}

/// Coordinates controlled metadata updates, missing-asset recovery (relinking), and safe catalog-only
/// removal on top of `WallpaperCatalogRepository`.
///
/// Guarantees:
/// - Never deletes, moves, renames, or modifies external user media files.
/// - Serializes in-flight management mutations so overlapping UI requests cannot corrupt records.
#[derive(Debug, Default, Clone)]
pub struct CatalogManagementService {
    in_flight: Arc<AtomicBool>,
}

impl CatalogManagementService {
    #[must_use]
    pub fn new() -> Self {
        Self {
            in_flight: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Updates allowlisted user-editable metadata (`title`, `description`, `tags`) for `id`.
    ///
    /// Preserves all protected identity, classification, lifecycle, and non-editable manifest fields.
    pub fn update_editable_metadata<R: WallpaperCatalogRepository>(
        &self,
        repo: &mut R,
        id: WallpaperId,
        patch: EditableMetadataPatch,
    ) -> Result<MetadataUpdateOutcome, CatalogManagementError> {
        let _guard = self.acquire_in_flight()?;

        let existing = repo.get(id)?.ok_or(CatalogError::NotFound(id))?;

        if let Some(expected) = patch.expected_updated_at {
            if existing.lifecycle.updated_at != expected {
                return Err(CatalogManagementError::StaleRecordConflict {
                    id,
                    expected_updated_at: expected,
                    actual_updated_at: existing.lifecycle.updated_at,
                });
            }
        }

        let normalized_title = patch.title.trim().to_string();
        let normalized_description = patch.description.trim().to_string();
        let normalized_tags = normalize_patch_tags(&patch.tags);

        let mut candidate_metadata = existing.metadata.clone();
        candidate_metadata.title = normalized_title;
        candidate_metadata.description = normalized_description;
        candidate_metadata.tags = normalized_tags;

        // Validate candidate metadata before checking unchanged or writing.
        validate_metadata(&candidate_metadata).map_err(CatalogError::Validation)?;

        if candidate_metadata.title == existing.metadata.title
            && candidate_metadata.description == existing.metadata.description
            && candidate_metadata.tags == existing.metadata.tags
        {
            return Ok(MetadataUpdateOutcome::Unchanged {
                summary: existing.to_summary(),
            });
        }

        let updated_at = next_monotonic_timestamp(existing.lifecycle.updated_at);
        let updated = repo.update_metadata(id, candidate_metadata, updated_at)?;

        Ok(MetadataUpdateOutcome::Updated {
            summary: updated.to_summary(),
        })
    }

    /// Runs a user-initiated replacement file selection and relinks `target_id` if valid.
    pub fn select_and_relink_asset<R: WallpaperCatalogRepository, S: SingleAssetSelectionSource>(
        &self,
        repo: &mut R,
        target_id: WallpaperId,
        selector: &mut S,
    ) -> Result<RelinkOutcome, CatalogManagementError> {
        let _guard = self.acquire_in_flight()?;

        // Verify the target record exists before opening or applying selection.
        if repo.get(target_id)?.is_none() {
            return Err(CatalogManagementError::Catalog(CatalogError::NotFound(
                target_id,
            )));
        }

        let outcome = selector
            .select_replacement_asset(target_id)
            .map_err(CatalogManagementError::SelectionFailed)?;

        match outcome {
            RelinkSelectionOutcome::Cancelled => Ok(RelinkOutcome::Cancelled { id: target_id }),
            RelinkSelectionOutcome::Selected(candidate_path) => {
                self.relink_asset_inner(repo, target_id, &candidate_path)
            }
        }
    }

    /// Relinks an existing catalog entry `target_id` to `candidate_path` after full path-safety,
    /// type-compatibility, duplicate-ownership, and bounded header verification.
    pub fn relink_asset_from_path<R: WallpaperCatalogRepository>(
        &self,
        repo: &mut R,
        target_id: WallpaperId,
        candidate_path: &Path,
    ) -> Result<RelinkOutcome, CatalogManagementError> {
        let _guard = self.acquire_in_flight()?;
        self.relink_asset_inner(repo, target_id, candidate_path)
    }

    /// Removes a catalog entry if and only if it is not currently active/assigned or built-in protected.
    ///
    /// Never deletes, moves, or modifies the underlying external media file on disk.
    pub fn remove_catalog_entry<R: WallpaperCatalogRepository>(
        &self,
        repo: &mut R,
        id: WallpaperId,
        active_wallpaper_ids: &BTreeSet<WallpaperId>,
    ) -> Result<RemovalOutcome, CatalogManagementError> {
        let _guard = self.acquire_in_flight()?;

        let Some(existing) = repo.get(id)? else {
            return Ok(RemovalOutcome::AlreadyRemoved { id });
        };

        if existing.lifecycle.is_builtin {
            return Ok(RemovalOutcome::BlockedBuiltinProtected { id });
        }

        if active_wallpaper_ids.contains(&id) {
            return Ok(RemovalOutcome::BlockedActiveAssignment { id });
        }

        let removed = repo.remove_record(id)?;
        if removed {
            Ok(RemovalOutcome::Removed { id })
        } else {
            Ok(RemovalOutcome::AlreadyRemoved { id })
        }
    }

    fn relink_asset_inner<R: WallpaperCatalogRepository>(
        &self,
        repo: &mut R,
        target_id: WallpaperId,
        raw_path: &Path,
    ) -> Result<RelinkOutcome, CatalogManagementError> {
        let existing = repo
            .get(target_id)?
            .ok_or(CatalogError::NotFound(target_id))?;

        let redacted_path = redact_path_for_diagnostics(raw_path);
        let candidate_asset = AssetReference {
            source_kind: WallpaperSourceKind::ExternalDirectFile,
            entrypoint: raw_path.to_string_lossy().into_owned(),
        };

        // 1. Validate path syntax and forbidden executable extensions.
        let normalized_path = match validate_asset_reference(&candidate_asset) {
            Ok(path) => path,
            Err(err) => {
                return Ok(RelinkOutcome::Rejected {
                    id: target_id,
                    redacted_path,
                    reason: RelinkRejectionReason::AssetValidation(map_path_safety_rejection(err)),
                });
            }
        };

        // 2. Check filesystem metadata (distinguish directory, missing, inaccessible).
        let metadata = match fs::metadata(&normalized_path) {
            Ok(m) => m,
            Err(err) => {
                return Ok(RelinkOutcome::InaccessibleOrMissing {
                    id: target_id,
                    redacted_path,
                    availability: classify_io_availability_error(&err),
                });
            }
        };

        if metadata.is_dir() {
            return Ok(RelinkOutcome::Rejected {
                id: target_id,
                redacted_path,
                reason: RelinkRejectionReason::AssetValidation(
                    RegistrationRejectionReason::UnexpectedDirectory,
                ),
            });
        }

        if !metadata.is_file() {
            return Ok(RelinkOutcome::Rejected {
                id: target_id,
                redacted_path,
                reason: RelinkRejectionReason::AssetValidation(
                    RegistrationRejectionReason::ValidationFailed(
                        "selected replacement path is not a regular file".to_string(),
                    ),
                ),
            });
        }

        // 3. Verify supported extension and compatibility with the record's immutable wallpaper_type.
        let Some(candidate_type) = classify_extension_wallpaper_type(&normalized_path) else {
            return Ok(RelinkOutcome::Rejected {
                id: target_id,
                redacted_path,
                reason: RelinkRejectionReason::AssetValidation(
                    RegistrationRejectionReason::UnsupportedExtension,
                ),
            });
        };

        if candidate_type != existing.wallpaper_type {
            return Ok(RelinkOutcome::Rejected {
                id: target_id,
                redacted_path,
                reason: RelinkRejectionReason::TypeMismatch {
                    expected_type: existing.wallpaper_type,
                    candidate_type,
                },
            });
        }

        // 4. Check duplicate ownership and same-asset no-op across existing catalog records.
        let candidate_key = normalize_external_comparison_key(&normalized_path);
        for record in repo.list()? {
            if record.asset.source_kind == WallpaperSourceKind::ExternalDirectFile {
                let record_key =
                    normalize_external_comparison_key(Path::new(&record.asset.entrypoint));
                if record_key == candidate_key && record.id != target_id {
                    return Ok(RelinkOutcome::Rejected {
                        id: target_id,
                        redacted_path,
                        reason: RelinkRejectionReason::DuplicateOwnedByOther {
                            existing_id: record.id,
                        },
                    });
                }
            }
        }

        // 5. Bounded read-only content/header inspection.
        match verify_bounded_content_signature(&normalized_path, existing.wallpaper_type) {
            Ok(()) => {}
            Err(ContentCheckError::Io(err)) => {
                return Ok(RelinkOutcome::InaccessibleOrMissing {
                    id: target_id,
                    redacted_path,
                    availability: classify_io_availability_error(&err),
                });
            }
            Err(ContentCheckError::Rejected(reason)) => {
                return Ok(RelinkOutcome::Rejected {
                    id: target_id,
                    redacted_path,
                    reason: RelinkRejectionReason::AssetValidation(reason),
                });
            }
        }

        // 6. If the record already references this exact external asset and is already Available,
        // return UnchangedSameAsset without rewriting the manifest.
        if existing.asset.source_kind == WallpaperSourceKind::ExternalDirectFile {
            let existing_key =
                normalize_external_comparison_key(Path::new(&existing.asset.entrypoint));
            if existing_key == candidate_key
                && existing.availability == AvailabilityState::Available
            {
                return Ok(RelinkOutcome::UnchangedSameAsset {
                    id: target_id,
                    summary: existing.to_summary(),
                });
            }
        }

        // 7. Persist updated asset reference and Available state atomically.
        let new_asset = AssetReference {
            source_kind: WallpaperSourceKind::ExternalDirectFile,
            entrypoint: normalized_path.to_string_lossy().into_owned(),
        };
        let updated_at = next_monotonic_timestamp(existing.lifecycle.updated_at);

        match repo.relink_asset(
            target_id,
            new_asset,
            AvailabilityState::Available,
            updated_at,
        ) {
            Ok(updated) => Ok(RelinkOutcome::Relinked {
                id: target_id,
                summary: updated.to_summary(),
            }),
            Err(err) => {
                warn!(
                    id = %target_id,
                    path = %redacted_path,
                    error = %err,
                    "catalog persistence failed during asset relinking"
                );
                Err(CatalogManagementError::Catalog(err))
            }
        }
    }

    fn acquire_in_flight(&self) -> Result<InFlightGuard, CatalogManagementError> {
        if self
            .in_flight
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Err(CatalogManagementError::OperationInProgress);
        }
        Ok(InFlightGuard {
            flag: Arc::clone(&self.in_flight),
        })
    }
}

struct InFlightGuard {
    flag: Arc<AtomicBool>,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::Release);
    }
}

/// Normalizes tag list by trimming outer whitespace while preserving empty/control tags so
/// `validate_metadata` rejects invalid tag entries deterministicly, and deduplicating valid
/// case-insensitive duplicates in first-seen order.
fn normalize_patch_tags(raw_tags: &[String]) -> Vec<String> {
    let mut normalized = Vec::with_capacity(raw_tags.len());
    let mut seen_lower = BTreeSet::new();

    for raw in raw_tags {
        let trimmed = raw.trim();
        if trimmed.is_empty() || raw.chars().any(char::is_control) {
            // Preserve invalid tag so `validate_metadata` surfaces `EmptyOrControlTag`.
            normalized.push(raw.clone());
            continue;
        }
        let lower = trimmed.to_lowercase();
        if seen_lower.insert(lower) {
            normalized.push(trimmed.to_string());
        }
    }

    normalized
}

fn next_monotonic_timestamp(previous: DateTime<Utc>) -> DateTime<Utc> {
    let now = Utc::now();
    if now > previous {
        now
    } else {
        previous + Duration::milliseconds(1)
    }
}

fn classify_io_availability_error(err: &std::io::Error) -> AvailabilityState {
    match err.kind() {
        ErrorKind::NotFound => AvailabilityState::Missing,
        ErrorKind::PermissionDenied => AvailabilityState::Inaccessible {
            reason: "permission denied".to_string(),
        },
        other => AvailabilityState::Inaccessible {
            reason: format!("io error ({other:?})"),
        },
    }
}

/// Helper for inspecting whether a catalog record's editable metadata differs from a proposed patch.
#[must_use]
pub fn has_editable_metadata_changes(
    record: &WallpaperRecord,
    patch: &EditableMetadataPatch,
) -> bool {
    let normalized_title = patch.title.trim();
    let normalized_description = patch.description.trim();
    let normalized_tags = normalize_patch_tags(&patch.tags);

    record.metadata.title != normalized_title
        || record.metadata.description != normalized_description
        || record.metadata.tags != normalized_tags
}
