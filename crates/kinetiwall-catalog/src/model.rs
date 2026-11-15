use std::collections::BTreeMap;
use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Current catalog manifest schema version supported by this implementation (`KW-DAT-004`).
pub const CURRENT_CATALOG_SCHEMA_VERSION: u32 = 1;

/// Maximum lengths and bounds enforced on catalog metadata fields.
pub const MAX_TITLE_LEN: usize = 128;
pub const MAX_DESCRIPTION_LEN: usize = 4096;
pub const MAX_AUTHOR_LEN: usize = 64;
pub const MAX_LICENSE_LEN: usize = 64;
pub const MAX_TAG_COUNT: usize = 32;
pub const MAX_TAG_LEN: usize = 32;
pub const MAX_PATH_LEN: usize = 260;

/// Stable, location-independent catalog identity backed by an RFC 4122 UUIDv4 (`DEC-DATA-02`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WallpaperId(Uuid);

impl WallpaperId {
    /// Generates a new random RFC 4122 UUIDv4 identifier.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Wraps an existing UUID without altering its value.
    #[must_use]
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Parses a UUID string into a [`WallpaperId`].
    pub fn parse_str(input: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(input).map(Self)
    }

    /// Returns the underlying UUID value.
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for WallpaperId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for WallpaperId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Explicitly supported wallpaper media/runtime types (`KW-DAT-004`, `KW-F-010`, `KW-F-016`).
///
/// Unknown or unsupported string variants fail deserialization rather than coercing to a valid type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WallpaperType {
    /// Video media decoded out-of-process (`libmpv`).
    Video,
    /// Interactive HTML5/CSS/JS/WebGL bundle rendered out-of-process (`WebView2`).
    Web,
}

impl WallpaperType {
    /// Returns the canonical wire/schema name of this wallpaper type.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Web => "web",
        }
    }
}

impl fmt::Display for WallpaperType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Ownership and storage model for a cataloged wallpaper asset (`KW-DAT-005`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WallpaperSourceKind {
    /// Assets reside inside KinetiWall's managed `wallpapers/{id}/` directory.
    ManagedPackage,
    /// Asset resides at a user-owned external filesystem path referenced read-only.
    ExternalDirectFile,
}

impl WallpaperSourceKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ManagedPackage => "managed_package",
            Self::ExternalDirectFile => "external_direct_file",
        }
    }
}

/// Separates catalog identity/metadata from the underlying media location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetReference {
    /// Determines path resolution rules and deletion safety boundaries.
    pub source_kind: WallpaperSourceKind,
    /// Relative path inside package directory (for `ManagedPackage`),
    /// or validated local/UNC path (for `ExternalDirectFile`).
    pub entrypoint: String,
}

/// Runtime availability classification for a cataloged asset.
///
/// Stored metadata does not treat an existing catalog record as proof of playability;
/// availability is checked explicitly against the filesystem when needed.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AvailabilityState {
    /// Asset existence has not been probed in the current session.
    #[default]
    Unchecked,
    /// Target asset exists and is readable at its expected path.
    Available,
    /// Target asset does not exist on disk (missing or moved).
    Missing,
    /// Target asset exists or parent path cannot be opened due to permissions/locks.
    Inaccessible { reason: String },
    /// Target path exists but fails basic asset validity (e.g., points to a directory
    /// where a media file is required, or fails safety rules).
    InvalidAsset { reason: String },
}

impl AvailabilityState {
    /// Returns `true` only when the asset was explicitly verified as present and readable.
    #[must_use]
    pub const fn is_playable_candidate(&self) -> bool {
        matches!(self, Self::Available)
    }
}

/// Descriptive and compatibility metadata for a cataloged wallpaper (`KW-DAT-004`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WallpaperMetadata {
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_author")]
    pub author: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default = "default_version")]
    pub min_engine_version: String,
    #[serde(default = "default_license")]
    pub license: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub thumbnail_ref: Option<String>,
}

impl WallpaperMetadata {
    /// Constructs a minimal validated-input candidate with canonical defaults.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            description: String::new(),
            author: default_author(),
            version: default_version(),
            min_engine_version: default_version(),
            license: default_license(),
            tags: Vec::new(),
            thumbnail_ref: None,
        }
    }
}

fn default_author() -> String {
    "Unknown".to_string()
}

fn default_version() -> String {
    "1.0.0".to_string()
}

fn default_license() -> String {
    "Custom".to_string()
}

/// Lifecycle timestamps and origin marker owned by the catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogLifecycle {
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub is_builtin: bool,
}

impl CatalogLifecycle {
    /// Creates a lifecycle record stamped at `now`.
    #[must_use]
    pub fn now(is_builtin: bool) -> Self {
        let ts = Utc::now();
        Self {
            created_at: ts,
            updated_at: ts,
            is_builtin,
        }
    }
}

/// Core domain record representing a wallpaper entry in the local catalog.
///
/// Playback state, monitor assignments, and transient UI filters intentionally live outside
/// this domain struct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WallpaperRecord {
    pub id: WallpaperId,
    pub schema_version: u32,
    pub wallpaper_type: WallpaperType,
    pub asset: AssetReference,
    pub metadata: WallpaperMetadata,
    pub lifecycle: CatalogLifecycle,
    pub availability: AvailabilityState,
    /// Unknown forward-compatible fields preserved from `project.json` (`KW-DAT-004`).
    pub preserved_extra: BTreeMap<String, serde_json::Value>,
}

impl WallpaperRecord {
    /// Creates a new catalog record with `CURRENT_CATALOG_SCHEMA_VERSION` and `Unchecked` availability.
    #[must_use]
    pub fn new(
        id: WallpaperId,
        wallpaper_type: WallpaperType,
        asset: AssetReference,
        metadata: WallpaperMetadata,
        lifecycle: CatalogLifecycle,
    ) -> Self {
        Self {
            id,
            schema_version: CURRENT_CATALOG_SCHEMA_VERSION,
            wallpaper_type,
            asset,
            metadata,
            lifecycle,
            availability: AvailabilityState::Unchecked,
            preserved_extra: BTreeMap::new(),
        }
    }

    /// Converts this internal domain entity into an IPC-safe summary DTO without leaking raw OS paths.
    #[must_use]
    pub fn to_summary(&self) -> CatalogEntrySummary {
        CatalogEntrySummary {
            id: self.id,
            schema_version: self.schema_version,
            title: self.metadata.title.clone(),
            description: self.metadata.description.clone(),
            wallpaper_type: self.wallpaper_type,
            source_kind: self.asset.source_kind,
            author: self.metadata.author.clone(),
            version: self.metadata.version.clone(),
            min_engine_version: self.metadata.min_engine_version.clone(),
            license: self.metadata.license.clone(),
            tags: self.metadata.tags.clone(),
            availability: self.availability.clone(),
            created_at: self.lifecycle.created_at,
            updated_at: self.lifecycle.updated_at,
            is_builtin: self.lifecycle.is_builtin,
            has_thumbnail_ref: self.metadata.thumbnail_ref.is_some(),
            thumbnail_ref: self.metadata.thumbnail_ref.clone(),
        }
    }
}

/// Persistent `project.json` schema representation (`Manifest V1` per `KW-DAT-004`).
///
/// Separated from [`WallpaperRecord`] and [`CatalogEntrySummary`] so disk schema evolution
/// does not implicitly break runtime or IPC contracts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredManifestV1 {
    pub manifest_version: u32,
    pub id: WallpaperId,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_author")]
    pub author: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default = "default_version")]
    pub min_engine_version: String,
    #[serde(default = "default_license")]
    pub license: String,
    #[serde(rename = "type")]
    pub wallpaper_type: WallpaperType,
    #[serde(default = "default_source_kind")]
    pub source_kind: WallpaperSourceKind,
    pub entrypoint: String,
    #[serde(default)]
    pub thumbnail: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub is_builtin: bool,
    /// Preserves forward-compatible fields (e.g. `permissions`, `properties`) untouched.
    #[serde(flatten)]
    pub extra_fields: BTreeMap<String, serde_json::Value>,
}

const fn default_source_kind() -> WallpaperSourceKind {
    WallpaperSourceKind::ManagedPackage
}

impl From<&WallpaperRecord> for StoredManifestV1 {
    fn from(record: &WallpaperRecord) -> Self {
        Self {
            manifest_version: record.schema_version,
            id: record.id,
            title: record.metadata.title.clone(),
            description: record.metadata.description.clone(),
            author: record.metadata.author.clone(),
            version: record.metadata.version.clone(),
            min_engine_version: record.metadata.min_engine_version.clone(),
            license: record.metadata.license.clone(),
            wallpaper_type: record.wallpaper_type,
            source_kind: record.asset.source_kind,
            entrypoint: record.asset.entrypoint.clone(),
            thumbnail: record.metadata.thumbnail_ref.clone(),
            tags: record.metadata.tags.clone(),
            created_at: record.lifecycle.created_at,
            updated_at: record.lifecycle.updated_at,
            is_builtin: record.lifecycle.is_builtin,
            extra_fields: record.preserved_extra.clone(),
        }
    }
}

impl From<StoredManifestV1> for WallpaperRecord {
    fn from(manifest: StoredManifestV1) -> Self {
        Self {
            id: manifest.id,
            schema_version: manifest.manifest_version,
            wallpaper_type: manifest.wallpaper_type,
            asset: AssetReference {
                source_kind: manifest.source_kind,
                entrypoint: manifest.entrypoint,
            },
            metadata: WallpaperMetadata {
                title: manifest.title,
                description: manifest.description,
                author: manifest.author,
                version: manifest.version,
                min_engine_version: manifest.min_engine_version,
                license: manifest.license,
                tags: manifest.tags,
                thumbnail_ref: manifest.thumbnail,
            },
            lifecycle: CatalogLifecycle {
                created_at: manifest.created_at,
                updated_at: manifest.updated_at,
                is_builtin: manifest.is_builtin,
            },
            availability: AvailabilityState::Unchecked,
            preserved_extra: manifest.extra_fields,
        }
    }
}

const fn default_schema_version() -> u32 {
    CURRENT_CATALOG_SCHEMA_VERSION
}

/// IPC / service summary DTO safe for frontend consumption without exposing raw host OS paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogEntrySummary {
    pub id: WallpaperId,
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub wallpaper_type: WallpaperType,
    pub source_kind: WallpaperSourceKind,
    pub author: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default = "default_version")]
    pub min_engine_version: String,
    #[serde(default = "default_license")]
    pub license: String,
    pub tags: Vec<String>,
    pub availability: AvailabilityState,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub is_builtin: bool,
    pub has_thumbnail_ref: bool,
    /// Validated package-relative thumbnail reference when present; never contains host OS paths.
    #[serde(default)]
    pub thumbnail_ref: Option<String>,
}
