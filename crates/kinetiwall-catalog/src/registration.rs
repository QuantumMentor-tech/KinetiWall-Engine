use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{info, warn};

use crate::error::{redact_path_for_diagnostics, CatalogError, PathSafetyError};
use crate::model::{
    AssetReference, AvailabilityState, CatalogEntrySummary, CatalogLifecycle, WallpaperId,
    WallpaperMetadata, WallpaperRecord, WallpaperSourceKind, WallpaperType, MAX_TITLE_LEN,
};
use crate::path_safety::{
    classify_io_availability_error, normalize_external_comparison_key, validate_asset_reference,
};
use crate::repository::WallpaperCatalogRepository;
use crate::validation::classify_extension_wallpaper_type;

/// Upper bound on items processed in a single user-initiated registration batch.
pub const MAX_REGISTRATION_BATCH_SIZE: usize = 500;

/// Maximum header bytes read during registration-time content validation.
const HEADER_INSPECTION_BYTES: usize = 512;

/// Outcome of a user-initiated asset selection interaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionOutcome {
    /// User chose one or more local filesystem paths.
    Selected(Vec<PathBuf>),
    /// User cancelled or dismissed the selection dialog (normal non-error outcome).
    Cancelled,
}

/// Abstraction over UI/native file selection so selection stays decoupled from validation and storage.
pub trait AssetSelectionSource {
    fn select_assets(&mut self) -> Result<SelectionOutcome, RegistrationError>;
}

/// Errors emitted at the registration workflow boundary (distinct from per-item outcomes).
#[derive(Debug, Error)]
pub enum RegistrationError {
    #[error("a wallpaper registration operation is already in progress")]
    OperationInProgress,

    #[error("selection contains {actual} items, exceeding the maximum batch limit of {max}")]
    BatchLimitExceeded { max: usize, actual: usize },

    #[error("asset selection failed: {detail}")]
    SelectionFailed { detail: String },

    #[error("failed to query existing catalog records: {0}")]
    CatalogQueryFailed(#[from] CatalogError),
}

/// Reason a candidate asset was rejected during registration validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum RegistrationRejectionReason {
    PathSafety(String),
    UnexpectedDirectory,
    UnsupportedExtension,
    EmptyFile,
    InvalidContentSignature {
        expected_type: WallpaperType,
        detail: String,
    },
    ValidationFailed(String),
}

/// Per-item registration outcome returned to callers and UI presentation layers.
///
/// Paths are redacted to avoid leaking unnecessary host directory prefixes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RegistrationItemOutcome {
    Registered {
        id: WallpaperId,
        summary: CatalogEntrySummary,
    },
    AlreadyPresent {
        existing_id: WallpaperId,
        summary: CatalogEntrySummary,
    },
    Rejected {
        redacted_path: String,
        reason: RegistrationRejectionReason,
    },
    InaccessibleOrMissing {
        redacted_path: String,
        availability: AvailabilityState,
    },
    PersistenceFailed {
        redacted_path: String,
        detail: String,
    },
}

/// Aggregate status of a registration batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchOverallStatus {
    Cancelled,
    EmptySelection,
    Succeeded,
    PartialSuccess,
    AllFailed,
}

/// Structured summary of a multi-asset or single-asset registration workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrationBatchReport {
    pub overall_status: BatchOverallStatus,
    pub total_selected: usize,
    pub registered_count: usize,
    pub already_present_count: usize,
    pub rejected_count: usize,
    pub inaccessible_count: usize,
    pub failed_count: usize,
    pub items: Vec<RegistrationItemOutcome>,
}

/// Coordinates user-initiated local wallpaper validation and catalog registration.
///
/// Registers read-only `ExternalDirectFile` references in the Step 25 catalog without
/// copying, moving, renaming, modifying, or deleting original source files.
#[derive(Debug, Default)]
pub struct WallpaperRegistrationService {
    in_flight: Arc<AtomicBool>,
}

impl WallpaperRegistrationService {
    #[must_use]
    pub fn new() -> Self {
        Self {
            in_flight: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Returns `true` if a registration workflow is currently executing.
    #[must_use]
    pub fn is_in_flight(&self) -> bool {
        self.in_flight.load(Ordering::Acquire)
    }

    /// Runs selection via `selector`, then validates and registers selected paths into `repo`.
    pub fn select_and_register<R, S>(
        &self,
        repo: &mut R,
        selector: &mut S,
    ) -> Result<RegistrationBatchReport, RegistrationError>
    where
        R: WallpaperCatalogRepository,
        S: AssetSelectionSource,
    {
        let _guard = self.acquire_in_flight_guard()?;
        let selection = selector.select_assets()?;
        self.process_selection(repo, selection)
    }

    /// Validates and registers a pre-selected [`SelectionOutcome`] into `repo`.
    pub fn register_selection<R>(
        &self,
        repo: &mut R,
        selection: SelectionOutcome,
    ) -> Result<RegistrationBatchReport, RegistrationError>
    where
        R: WallpaperCatalogRepository,
    {
        let _guard = self.acquire_in_flight_guard()?;
        self.process_selection(repo, selection)
    }

    fn acquire_in_flight_guard(&self) -> Result<InFlightGuard, RegistrationError> {
        if self
            .in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(RegistrationError::OperationInProgress);
        }
        Ok(InFlightGuard {
            flag: Arc::clone(&self.in_flight),
        })
    }

    fn process_selection<R: WallpaperCatalogRepository>(
        &self,
        repo: &mut R,
        selection: SelectionOutcome,
    ) -> Result<RegistrationBatchReport, RegistrationError> {
        let paths = match selection {
            SelectionOutcome::Cancelled => {
                info!("wallpaper selection cancelled by user");
                return Ok(RegistrationBatchReport {
                    overall_status: BatchOverallStatus::Cancelled,
                    total_selected: 0,
                    registered_count: 0,
                    already_present_count: 0,
                    rejected_count: 0,
                    inaccessible_count: 0,
                    failed_count: 0,
                    items: Vec::new(),
                });
            }
            SelectionOutcome::Selected(paths) => paths,
        };

        if paths.is_empty() {
            return Ok(RegistrationBatchReport {
                overall_status: BatchOverallStatus::EmptySelection,
                total_selected: 0,
                registered_count: 0,
                already_present_count: 0,
                rejected_count: 0,
                inaccessible_count: 0,
                failed_count: 0,
                items: Vec::new(),
            });
        }

        if paths.len() > MAX_REGISTRATION_BATCH_SIZE {
            return Err(RegistrationError::BatchLimitExceeded {
                max: MAX_REGISTRATION_BATCH_SIZE,
                actual: paths.len(),
            });
        }

        // Build a normalized lookup index of existing ExternalDirectFile entries for O(1) duplicate checks.
        let mut known_external: HashMap<String, WallpaperRecord> = HashMap::new();
        for existing in repo.list()? {
            if existing.asset.source_kind == WallpaperSourceKind::ExternalDirectFile {
                let key = normalize_external_comparison_key(Path::new(&existing.asset.entrypoint));
                known_external.insert(key, existing);
            }
        }

        let mut report = RegistrationBatchReport {
            overall_status: BatchOverallStatus::Succeeded,
            total_selected: paths.len(),
            registered_count: 0,
            already_present_count: 0,
            rejected_count: 0,
            inaccessible_count: 0,
            failed_count: 0,
            items: Vec::with_capacity(paths.len()),
        };

        for raw_path in paths {
            let outcome = self.register_single_path(repo, &mut known_external, &raw_path);
            match &outcome {
                RegistrationItemOutcome::Registered { .. } => report.registered_count += 1,
                RegistrationItemOutcome::AlreadyPresent { .. } => report.already_present_count += 1,
                RegistrationItemOutcome::Rejected { .. } => report.rejected_count += 1,
                RegistrationItemOutcome::InaccessibleOrMissing { .. } => {
                    report.inaccessible_count += 1
                }
                RegistrationItemOutcome::PersistenceFailed { .. } => report.failed_count += 1,
            }
            report.items.push(outcome);
        }

        report.overall_status = classify_batch_status(&report);
        Ok(report)
    }

    fn register_single_path<R: WallpaperCatalogRepository>(
        &self,
        repo: &mut R,
        known_external: &mut HashMap<String, WallpaperRecord>,
        raw_path: &Path,
    ) -> RegistrationItemOutcome {
        let redacted_path = redact_path_for_diagnostics(raw_path);
        let raw_str = raw_path.to_string_lossy().into_owned();
        let asset = AssetReference {
            source_kind: WallpaperSourceKind::ExternalDirectFile,
            entrypoint: raw_str,
        };

        // 1. Validate path form and forbidden executable extensions.
        let normalized_path = match validate_asset_reference(&asset) {
            Ok(p) => p,
            Err(err) => {
                return RegistrationItemOutcome::Rejected {
                    redacted_path,
                    reason: map_path_safety_rejection(err),
                };
            }
        };

        // 2. Check filesystem metadata (distinguish directory, missing, inaccessible).
        let metadata = match fs::metadata(&normalized_path) {
            Ok(m) => m,
            Err(err) => {
                return RegistrationItemOutcome::InaccessibleOrMissing {
                    redacted_path,
                    availability: classify_io_availability_error(&err),
                };
            }
        };

        if metadata.is_dir() {
            return RegistrationItemOutcome::Rejected {
                redacted_path,
                reason: RegistrationRejectionReason::UnexpectedDirectory,
            };
        }

        if !metadata.is_file() {
            return RegistrationItemOutcome::Rejected {
                redacted_path,
                reason: RegistrationRejectionReason::ValidationFailed(
                    "selected path is not a regular file".to_string(),
                ),
            };
        }

        // 3. Determine supported wallpaper type from allowlisted extension, then verify bounded header.
        let Some(wallpaper_type) = classify_extension_wallpaper_type(&normalized_path) else {
            return RegistrationItemOutcome::Rejected {
                redacted_path,
                reason: RegistrationRejectionReason::UnsupportedExtension,
            };
        };

        // 4. Duplicate check against existing catalog records and earlier items in this batch.
        let dedup_key = normalize_external_comparison_key(&normalized_path);
        if let Some(existing) = known_external.get(&dedup_key) {
            return RegistrationItemOutcome::AlreadyPresent {
                existing_id: existing.id,
                summary: existing.to_summary(),
            };
        }

        // 5. Bounded read-only content/header inspection (rechecks accessibility close to registration).
        match verify_bounded_content_signature(&normalized_path, wallpaper_type) {
            Ok(()) => {}
            Err(ContentCheckError::Io(err)) => {
                return RegistrationItemOutcome::InaccessibleOrMissing {
                    redacted_path,
                    availability: classify_io_availability_error(&err),
                };
            }
            Err(ContentCheckError::Rejected(reason)) => {
                return RegistrationItemOutcome::Rejected {
                    redacted_path,
                    reason,
                };
            }
        }

        // 6. Construct validated domain record and persist through Step 25 repository.
        let title = derive_display_title(&normalized_path);
        let id = WallpaperId::new();
        let mut record = WallpaperRecord::new(
            id,
            wallpaper_type,
            AssetReference {
                source_kind: WallpaperSourceKind::ExternalDirectFile,
                entrypoint: normalized_path.to_string_lossy().into_owned(),
            },
            WallpaperMetadata::new(title),
            CatalogLifecycle::now(false),
        );
        record.availability = AvailabilityState::Available;

        match repo.create(record) {
            Ok(saved) => {
                let summary = saved.to_summary();
                known_external.insert(dedup_key, saved);
                RegistrationItemOutcome::Registered { id, summary }
            }
            Err(CatalogError::Validation(err)) => RegistrationItemOutcome::Rejected {
                redacted_path,
                reason: RegistrationRejectionReason::ValidationFailed(err.to_string()),
            },
            Err(CatalogError::PathSafety(err)) => RegistrationItemOutcome::Rejected {
                redacted_path,
                reason: map_path_safety_rejection(err),
            },
            Err(err) => {
                warn!(path = %redacted_path, error = %err, "catalog persistence failed during registration");
                RegistrationItemOutcome::PersistenceFailed {
                    redacted_path,
                    detail: err.to_string(),
                }
            }
        }
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

pub(crate) enum ContentCheckError {
    Io(std::io::Error),
    Rejected(RegistrationRejectionReason),
}

pub(crate) fn verify_bounded_content_signature(
    path: &Path,
    wallpaper_type: WallpaperType,
) -> Result<(), ContentCheckError> {
    let mut file = File::open(path).map_err(ContentCheckError::Io)?;
    let mut buf = [0u8; HEADER_INSPECTION_BYTES];
    let bytes_read = file.read(&mut buf).map_err(ContentCheckError::Io)?;
    if bytes_read == 0 {
        return Err(ContentCheckError::Rejected(
            RegistrationRejectionReason::EmptyFile,
        ));
    }

    let header = &buf[..bytes_read];

    // Reject Windows PE executables disguised with media/web extensions.
    if header.starts_with(b"MZ") {
        return Err(ContentCheckError::Rejected(
            RegistrationRejectionReason::InvalidContentSignature {
                expected_type: wallpaper_type,
                detail: "file begins with executable MZ header".to_string(),
            },
        ));
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();

    match wallpaper_type {
        WallpaperType::Video => verify_video_container_header(header, &ext),
        WallpaperType::Web => verify_web_document_header(header),
    }
}

fn verify_video_container_header(header: &[u8], ext: &str) -> Result<(), ContentCheckError> {
    let valid = match ext {
        "mp4" | "m4v" | "mov" => header.len() >= 8 && &header[4..8] == b"ftyp",
        "webm" | "mkv" => header.len() >= 4 && header[..4] == [0x1A, 0x45, 0xDF, 0xA3],
        "avi" => header.len() >= 12 && &header[0..4] == b"RIFF" && &header[8..12] == b"AVI ",
        _ => false,
    };

    if !valid {
        return Err(ContentCheckError::Rejected(
            RegistrationRejectionReason::InvalidContentSignature {
                expected_type: WallpaperType::Video,
                detail: format!("file header does not match expected `.{ext}` video container"),
            },
        ));
    }
    Ok(())
}

fn verify_web_document_header(header: &[u8]) -> Result<(), ContentCheckError> {
    if header.contains(&0) {
        return Err(ContentCheckError::Rejected(
            RegistrationRejectionReason::InvalidContentSignature {
                expected_type: WallpaperType::Web,
                detail: "binary NUL bytes detected in HTML document".to_string(),
            },
        ));
    }

    // Permit a split UTF-8 codepoint at the end of the 512-byte slice.
    let utf8_ok = match std::str::from_utf8(header) {
        Ok(_) => true,
        Err(err) => err.error_len().is_none(),
    };

    if !utf8_ok {
        return Err(ContentCheckError::Rejected(
            RegistrationRejectionReason::InvalidContentSignature {
                expected_type: WallpaperType::Web,
                detail: "HTML entrypoint is not valid UTF-8 text".to_string(),
            },
        ));
    }

    Ok(())
}

/// Derives a safe, bounded user-facing display title from a filename stem without exposing
/// full directory paths or altering the file on disk.
#[must_use]
pub fn derive_display_title(path: &Path) -> String {
    let raw_stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .trim();

    let sanitized: String = raw_stem
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_TITLE_LEN)
        .collect();

    let trimmed = sanitized.trim();
    if trimmed.is_empty() {
        "Untitled Wallpaper".to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn map_path_safety_rejection(err: PathSafetyError) -> RegistrationRejectionReason {
    match err {
        PathSafetyError::ForbiddenExtension { .. } => {
            RegistrationRejectionReason::UnsupportedExtension
        }
        other => RegistrationRejectionReason::PathSafety(other.to_string()),
    }
}

fn classify_batch_status(report: &RegistrationBatchReport) -> BatchOverallStatus {
    let positive = report.registered_count + report.already_present_count;
    let negative = report.rejected_count + report.inaccessible_count + report.failed_count;

    if positive > 0 && negative == 0 {
        BatchOverallStatus::Succeeded
    } else if positive > 0 && negative > 0 {
        BatchOverallStatus::PartialSuccess
    } else {
        BatchOverallStatus::AllFailed
    }
}
