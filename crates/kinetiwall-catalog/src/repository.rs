use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::error::{redact_path_for_diagnostics, CatalogError, PersistenceError};
use crate::model::{
    AssetReference, AvailabilityState, CatalogEntrySummary, StoredManifestV1, WallpaperId,
    WallpaperMetadata, WallpaperRecord, CURRENT_CATALOG_SCHEMA_VERSION,
};
use crate::path_safety::{manifest_path, package_directory, probe_asset_availability};
use crate::validation::{validate_lifecycle, validate_metadata, validate_record};

/// Minimal repository contract for KinetiWall's local wallpaper metadata catalog.
pub trait WallpaperCatalogRepository {
    /// Validates and persists a new wallpaper catalog record.
    fn create(&mut self, record: WallpaperRecord) -> Result<WallpaperRecord, CatalogError>;

    /// Retrieves a catalog record by its stable identifier without mutating state or probing files.
    fn get(&self, id: WallpaperId) -> Result<Option<WallpaperRecord>, CatalogError>;

    /// Lists all valid catalog records deterministically without scanning external directories.
    fn list(&self) -> Result<Vec<WallpaperRecord>, CatalogError>;

    /// Lists IPC-safe summary DTOs for all catalog records without touching underlying media files.
    fn list_summaries(&self) -> Result<Vec<CatalogEntrySummary>, CatalogError> {
        Ok(self
            .list()?
            .iter()
            .map(WallpaperRecord::to_summary)
            .collect())
    }

    /// Probes availability for each catalog record and returns updated IPC-safe summary DTOs.
    fn probe_all_and_list_summaries(&mut self) -> Result<Vec<CatalogEntrySummary>, CatalogError> {
        let ids: Vec<WallpaperId> = self.list()?.into_iter().map(|r| r.id).collect();
        for id in ids {
            let _ = self.check_availability(id)?;
        }
        self.list_summaries()
    }

    /// Validates and atomically updates descriptive metadata for an existing catalog record.
    fn update_metadata(
        &mut self,
        id: WallpaperId,
        metadata: WallpaperMetadata,
        updated_at: DateTime<Utc>,
    ) -> Result<WallpaperRecord, CatalogError>;

    /// Validates and atomically relinks an existing catalog record to a validated replacement asset reference.
    fn relink_asset(
        &mut self,
        id: WallpaperId,
        new_asset: AssetReference,
        availability: AvailabilityState,
        updated_at: DateTime<Utc>,
    ) -> Result<WallpaperRecord, CatalogError>;

    /// Removes only the catalog record metadata without modifying or deleting underlying media assets.
    fn remove_record(&mut self, id: WallpaperId) -> Result<bool, CatalogError>;

    /// Explicitly probes the underlying asset on disk and updates the record's runtime availability.
    fn check_availability(&mut self, id: WallpaperId) -> Result<AvailabilityState, CatalogError>;
}

/// Diagnostic record describing a corrupt, malformed, or unsupported manifest encountered at startup.
///
/// Corrupted files are preserved on disk untouched so evidence is retained for recovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogLoadIssue {
    pub directory_name: String,
    pub redacted_path: String,
    pub reason: String,
}

/// Summary of catalog integrity without mutating entries or deleting missing assets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogIntegrityReport {
    pub valid_record_count: usize,
    pub available_count: usize,
    pub missing_count: usize,
    pub inaccessible_count: usize,
    pub invalid_asset_count: usize,
    pub unchecked_count: usize,
    pub load_issue_count: usize,
}

/// Authoritative IPC-safe snapshot of the wallpaper catalog for library browsing (`Step 27`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogLibrarySnapshot {
    pub entries: Vec<CatalogEntrySummary>,
    pub integrity: CatalogIntegrityReport,
    pub load_issues: Vec<CatalogLoadIssue>,
}

/// Local Hybrid JSON implementation (`KW-DAT-003`) storing each catalog entry manifest at
/// `{library_root}/{wallpaper_id}/project.json` with an in-memory index.
#[derive(Debug)]
pub struct LocalJsonCatalogRepository {
    library_root: PathBuf,
    records: BTreeMap<WallpaperId, WallpaperRecord>,
    load_issues: Vec<CatalogLoadIssue>,
}

impl LocalJsonCatalogRepository {
    /// Opens or initializes the catalog at `library_root` and indexes immediate package directories.
    ///
    /// Never performs unbounded recursive directory traversal and never auto-deletes corrupt manifests.
    pub fn open(library_root: impl Into<PathBuf>) -> Result<Self, CatalogError> {
        let library_root = library_root.into();
        fs::create_dir_all(&library_root).map_err(|source| PersistenceError::CreateDirectory {
            redacted_path: redact_path_for_diagnostics(&library_root),
            source,
        })?;

        let mut repo = Self {
            library_root,
            records: BTreeMap::new(),
            load_issues: Vec::new(),
        };
        repo.load_index()?;
        Ok(repo)
    }

    /// Returns the root directory managed by this catalog repository.
    #[must_use]
    pub fn library_root(&self) -> &Path {
        &self.library_root
    }

    /// Returns non-fatal manifest load errors captured while scanning immediate catalog entries.
    #[must_use]
    pub fn load_issues(&self) -> &[CatalogLoadIssue] {
        &self.load_issues
    }

    /// Produces an integrity snapshot over loaded catalog records and load issues.
    #[must_use]
    pub fn integrity_report(&self) -> CatalogIntegrityReport {
        let mut report = CatalogIntegrityReport {
            valid_record_count: self.records.len(),
            available_count: 0,
            missing_count: 0,
            inaccessible_count: 0,
            invalid_asset_count: 0,
            unchecked_count: 0,
            load_issue_count: self.load_issues.len(),
        };

        for record in self.records.values() {
            match record.availability {
                AvailabilityState::Unchecked => report.unchecked_count += 1,
                AvailabilityState::Available => report.available_count += 1,
                AvailabilityState::Missing => report.missing_count += 1,
                AvailabilityState::Inaccessible { .. } => report.inaccessible_count += 1,
                AvailabilityState::InvalidAsset { .. } => report.invalid_asset_count += 1,
            }
        }

        report
    }

    /// Produces an authoritative, IPC-safe [`CatalogLibrarySnapshot`] for the Wallpaper Library UI (`Step 27`).
    ///
    /// When `probe_availability` is `true`, each catalog record's underlying asset is verified once
    /// before producing the summary list and integrity counts.
    pub fn library_snapshot(
        &mut self,
        probe_availability: bool,
    ) -> Result<CatalogLibrarySnapshot, CatalogError> {
        let entries = if probe_availability {
            self.probe_all_and_list_summaries()?
        } else {
            self.list_summaries()?
        };
        let integrity = self.integrity_report();
        let load_issues = self.load_issues.clone();
        Ok(CatalogLibrarySnapshot {
            entries,
            integrity,
            load_issues,
        })
    }

    fn load_index(&mut self) -> Result<(), CatalogError> {
        let entries =
            fs::read_dir(&self.library_root).map_err(|source| PersistenceError::ReadFile {
                redacted_path: redact_path_for_diagnostics(&self.library_root),
                source,
            })?;

        for entry_res in entries {
            let entry = match entry_res {
                Ok(e) => e,
                Err(err) => {
                    warn!(error = %err, "skipping unreadable entry inside catalog root");
                    continue;
                }
            };

            let entry_path = entry.path();
            if !entry_path.is_dir() {
                continue;
            }

            let dir_name = entry.file_name().to_string_lossy().into_owned();
            // Skip pending deletion markers or non-UUID directories without failing catalog boot.
            let Ok(dir_id) = WallpaperId::parse_str(&dir_name) else {
                debug!(directory = %dir_name, "ignoring non-UUID directory in wallpapers root");
                continue;
            };

            let manifest_file = entry_path.join("project.json");
            if !manifest_file.exists() {
                self.record_issue(
                    &dir_name,
                    &manifest_file,
                    "missing project.json manifest in package directory".to_string(),
                );
                continue;
            }

            match read_and_validate_manifest(&manifest_file, dir_id) {
                Ok(record) => {
                    if self.records.contains_key(&record.id) {
                        self.record_issue(
                            &dir_name,
                            &manifest_file,
                            format!("duplicate wallpaper id `{}`", record.id),
                        );
                        continue;
                    }
                    self.records.insert(record.id, record);
                }
                Err(err) => {
                    self.record_issue(&dir_name, &manifest_file, err.to_string());
                }
            }
        }

        info!(
            loaded_records = self.records.len(),
            load_issues = self.load_issues.len(),
            "initialized local wallpaper catalog index"
        );
        Ok(())
    }

    fn record_issue(&mut self, dir_name: &str, path: &Path, reason: String) {
        let redacted_path = redact_path_for_diagnostics(path);
        warn!(
            directory = %dir_name,
            manifest = %redacted_path,
            reason = %reason,
            "preserving invalid or unsupported catalog manifest for diagnosis"
        );
        self.load_issues.push(CatalogLoadIssue {
            directory_name: dir_name.to_string(),
            redacted_path,
            reason,
        });
    }
}

impl WallpaperCatalogRepository for LocalJsonCatalogRepository {
    fn create(&mut self, record: WallpaperRecord) -> Result<WallpaperRecord, CatalogError> {
        validate_record(&record)?;

        if self.records.contains_key(&record.id) {
            return Err(CatalogError::DuplicateId(record.id));
        }

        let pkg_dir = package_directory(&self.library_root, record.id);
        let manifest_file = manifest_path(&self.library_root, record.id);

        if manifest_file.exists() {
            return Err(CatalogError::DuplicateId(record.id));
        }

        fs::create_dir_all(&pkg_dir).map_err(|source| PersistenceError::CreateDirectory {
            redacted_path: redact_path_for_diagnostics(&pkg_dir),
            source,
        })?;

        let stored = StoredManifestV1::from(&record);
        atomic_write_json(&manifest_file, &stored)?;

        self.records.insert(record.id, record.clone());
        Ok(record)
    }

    fn get(&self, id: WallpaperId) -> Result<Option<WallpaperRecord>, CatalogError> {
        Ok(self.records.get(&id).cloned())
    }

    fn list(&self) -> Result<Vec<WallpaperRecord>, CatalogError> {
        let mut items: Vec<WallpaperRecord> = self.records.values().cloned().collect();
        items.sort_by(|a, b| {
            a.lifecycle
                .created_at
                .cmp(&b.lifecycle.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(items)
    }

    fn update_metadata(
        &mut self,
        id: WallpaperId,
        metadata: WallpaperMetadata,
        updated_at: DateTime<Utc>,
    ) -> Result<WallpaperRecord, CatalogError> {
        validate_metadata(&metadata)?;

        let existing = self
            .records
            .get(&id)
            .ok_or(CatalogError::NotFound(id))?
            .clone();

        let mut updated = existing;
        updated.metadata = metadata;
        updated.lifecycle.updated_at = updated_at;
        validate_lifecycle(&updated.lifecycle)?;
        validate_record(&updated)?;

        let manifest_file = manifest_path(&self.library_root, id);
        let stored = StoredManifestV1::from(&updated);
        atomic_write_json(&manifest_file, &stored)?;

        self.records.insert(id, updated.clone());
        Ok(updated)
    }

    fn relink_asset(
        &mut self,
        id: WallpaperId,
        new_asset: AssetReference,
        availability: AvailabilityState,
        updated_at: DateTime<Utc>,
    ) -> Result<WallpaperRecord, CatalogError> {
        let existing = self
            .records
            .get(&id)
            .ok_or(CatalogError::NotFound(id))?
            .clone();

        let mut updated = existing;
        updated.asset = new_asset;
        updated.availability = availability;
        updated.lifecycle.updated_at = updated_at;
        validate_lifecycle(&updated.lifecycle)?;
        validate_record(&updated)?;

        let manifest_file = manifest_path(&self.library_root, id);
        let stored = StoredManifestV1::from(&updated);
        atomic_write_json(&manifest_file, &stored)?;

        self.records.insert(id, updated.clone());
        Ok(updated)
    }

    fn remove_record(&mut self, id: WallpaperId) -> Result<bool, CatalogError> {
        if !self.records.contains_key(&id) {
            return Ok(false);
        }

        let manifest_file = manifest_path(&self.library_root, id);
        if manifest_file.exists() {
            fs::remove_file(&manifest_file).map_err(|source| PersistenceError::RemoveRecord {
                redacted_path: redact_path_for_diagnostics(&manifest_file),
                source,
            })?;
        }

        // Remove the package metadata directory ONLY if completely empty.
        // Never use recursive deletion here: preserves any user assets or package payloads
        // until an explicit user-authorized package uninstall workflow runs.
        let pkg_dir = package_directory(&self.library_root, id);
        if pkg_dir.exists() {
            let _ = fs::remove_dir(&pkg_dir);
        }

        self.records.remove(&id);
        Ok(true)
    }

    fn check_availability(&mut self, id: WallpaperId) -> Result<AvailabilityState, CatalogError> {
        let record = self
            .records
            .get_mut(&id)
            .ok_or(CatalogError::NotFound(id))?;

        let state = probe_asset_availability(&self.library_root, id, &record.asset);
        record.availability = state.clone();
        Ok(state)
    }
}

fn read_and_validate_manifest(
    manifest_file: &Path,
    expected_dir_id: WallpaperId,
) -> Result<WallpaperRecord, CatalogError> {
    let bytes = fs::read(manifest_file).map_err(|source| PersistenceError::ReadFile {
        redacted_path: redact_path_for_diagnostics(manifest_file),
        source,
    })?;

    // First inspect schema version via untyped JSON so future or corrupted version numbers
    // fail explicitly before typed deserialization.
    let raw_value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|source| PersistenceError::JsonParse {
            redacted_path: redact_path_for_diagnostics(manifest_file),
            source,
        })?;

    let version_u64 = raw_value
        .get("manifest_version")
        .and_then(serde_json::Value::as_u64)
        .ok_or(CatalogError::UnsupportedSchemaVersion {
            found: 0,
            supported: CURRENT_CATALOG_SCHEMA_VERSION,
        })?;

    let version =
        u32::try_from(version_u64).map_err(|_| CatalogError::UnsupportedSchemaVersion {
            found: u32::MAX,
            supported: CURRENT_CATALOG_SCHEMA_VERSION,
        })?;

    if version != CURRENT_CATALOG_SCHEMA_VERSION {
        return Err(CatalogError::UnsupportedSchemaVersion {
            found: version,
            supported: CURRENT_CATALOG_SCHEMA_VERSION,
        });
    }

    let manifest: StoredManifestV1 =
        serde_json::from_value(raw_value).map_err(|source| PersistenceError::JsonParse {
            redacted_path: redact_path_for_diagnostics(manifest_file),
            source,
        })?;

    if manifest.id != expected_dir_id {
        return Err(CatalogError::Persistence(
            PersistenceError::DirectoryIdMismatch {
                directory_id: expected_dir_id.to_string(),
                manifest_id: manifest.id,
            },
        ));
    }

    let record = WallpaperRecord::from(manifest);
    validate_record(&record)?;
    Ok(record)
}

/// Writes a JSON payload using a temporary sibling file, OS flush (`sync_all`), and atomic rename
/// (`std::fs::rename`, backed by `MoveFileExW` on Windows).
fn atomic_write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), PersistenceError> {
    let serialized = serde_json::to_vec_pretty(value)
        .map_err(|source| PersistenceError::JsonSerialize { source })?;

    let tmp_path = path.with_extension("json.tmp");
    let write_result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp_path)?;
        file.write_all(&serialized)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp_path, path)?;
        Ok(())
    })();

    if let Err(source) = write_result {
        let _ = fs::remove_file(&tmp_path);
        return Err(PersistenceError::WriteFile {
            redacted_path: redact_path_for_diagnostics(path),
            source,
        });
    }

    Ok(())
}
