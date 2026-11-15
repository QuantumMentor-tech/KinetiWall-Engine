use std::path::Path;
use thiserror::Error;

use crate::model::WallpaperId;

/// Top-level error type returned by KinetiWall catalog operations.
#[derive(Debug, Error)]
pub enum CatalogError {
    /// Input failed domain validation rules before persistence or use.
    #[error("validation failed: {0}")]
    Validation(#[from] ValidationError),

    /// Asset reference or filesystem path violated path-safety boundaries.
    #[error("path safety violation: {0}")]
    PathSafety(#[from] PathSafetyError),

    /// Attempted to insert a catalog entry with an identifier that already exists.
    #[error("duplicate wallpaper id: {0}")]
    DuplicateId(WallpaperId),

    /// Target catalog record was not found.
    #[error("wallpaper record not found: {0}")]
    NotFound(WallpaperId),

    /// Stored catalog manifest uses an unexpected or unsupported schema version.
    #[error("unsupported catalog schema version {found} (supported: {supported})")]
    UnsupportedSchemaVersion { found: u32, supported: u32 },

    /// Underlying local storage operation failed.
    #[error("persistence failure: {0}")]
    Persistence(#[from] PersistenceError),
}

/// Structured domain-metadata validation failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValidationError {
    #[error("title must not be empty or whitespace-only")]
    BlankTitle,

    #[error("title exceeds maximum length of {max} characters (got {actual})")]
    TitleTooLong { max: usize, actual: usize },

    #[error("title contains disallowed control characters")]
    TitleControlCharacters,

    #[error("description exceeds maximum length of {max} characters (got {actual})")]
    DescriptionTooLong { max: usize, actual: usize },

    #[error("author must not be empty or whitespace-only")]
    BlankAuthor,

    #[error("author exceeds maximum length of {max} characters (got {actual})")]
    AuthorTooLong { max: usize, actual: usize },

    #[error("version `{value}` must follow semantic versioning X.Y.Z")]
    InvalidSemVer { field: &'static str, value: String },

    #[error("license exceeds maximum length of {max} characters (got {actual})")]
    LicenseTooLong { max: usize, actual: usize },

    #[error("tags count {actual} exceeds maximum of {max}")]
    TooManyTags { max: usize, actual: usize },

    #[error("tag at index {index} is invalid: {reason}")]
    InvalidTag { index: usize, reason: String },

    #[error("thumbnail reference `{reason}` is invalid")]
    InvalidThumbnailRef { reason: String },

    #[error("updated_at ({updated_at}) precedes created_at ({created_at})")]
    InvalidLifecycleOrder {
        created_at: String,
        updated_at: String,
    },

    #[error("entrypoint `{entrypoint}` is incompatible with wallpaper type `{wallpaper_type}`")]
    IncompatibleAssetType {
        wallpaper_type: &'static str,
        entrypoint: String,
    },

    #[error("unsupported schema version {found} (expected {expected})")]
    UnsupportedSchemaVersion { found: u32, expected: u32 },
}

/// Path validation and sandbox containment errors.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PathSafetyError {
    #[error("asset entrypoint path must not be empty")]
    EmptyPath,

    #[error("path contains NUL byte")]
    NullByte,

    #[error("path exceeds maximum allowed length of {max} characters (got {actual})")]
    PathTooLong { max: usize, actual: usize },

    #[error("managed package entrypoint must be a relative path")]
    ManagedPathMustBeRelative,

    #[error("external asset path must be an absolute local or UNC path")]
    ExternalPathMustBeAbsolute,

    #[error("path traversal (`..`) is prohibited in catalog asset references")]
    TraversalDetected,

    #[error("path escapes the wallpaper package root boundary")]
    PackageBoundaryEscape,

    #[error("forbidden executable or script extension `{extension}` rejected")]
    ForbiddenExtension { extension: String },

    #[error("malformed path `{detail}`")]
    MalformedPath { detail: String },
}

/// Local storage read/write and deserialization failures.
#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("failed to create catalog directory `{redacted_path}`: {source}")]
    CreateDirectory {
        redacted_path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to read catalog file `{redacted_path}`: {source}")]
    ReadFile {
        redacted_path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to write catalog file `{redacted_path}`: {source}")]
    WriteFile {
        redacted_path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to remove catalog record `{redacted_path}`: {source}")]
    RemoveRecord {
        redacted_path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse manifest JSON in `{redacted_path}`: {source}")]
    JsonParse {
        redacted_path: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("failed to serialize manifest JSON: {source}")]
    JsonSerialize {
        #[source]
        source: serde_json::Error,
    },

    #[error("manifest id `{manifest_id}` does not match package directory id `{directory_id}`")]
    DirectoryIdMismatch {
        directory_id: String,
        manifest_id: WallpaperId,
    },
}

/// Redacts user-identifying path prefixes before emitting diagnostics or error messages.
///
/// Conforms to `KW-OBS-001` / `DEC-DSI-08`: strips drive/home prefixes while retaining the
/// trailing filename or `wallpapers/{id}/...` segment needed for troubleshooting.
pub fn redact_path_for_diagnostics(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let components: Vec<String> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();

    if let Some(pos) = components
        .iter()
        .position(|c| c.eq_ignore_ascii_case("wallpapers"))
    {
        return format!("[CATALOG_ROOT]\\{}", components[pos..].join("\\"));
    }

    // Preserve only the last 2 components when path lies outside catalog root.
    if components.len() > 2 {
        let tail = &components[components.len() - 2..];
        return format!("[REDACTED]\\{}", tail.join("\\"));
    }

    if raw.contains(':') || raw.starts_with("\\\\") {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            return format!("[REDACTED]\\{name}");
        }
        return "[REDACTED]".to_string();
    }

    raw.into_owned()
}
