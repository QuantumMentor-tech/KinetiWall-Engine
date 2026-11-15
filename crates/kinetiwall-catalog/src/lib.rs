//! # `kinetiwall-catalog`
//!
//! Local-first wallpaper domain model, validation rules, path-safety boundaries,
//! and Hybrid JSON catalog persistence foundation for KinetiWall (`Step 25`).
//!
//! ## Architectural Boundaries
//! - Owns wallpaper catalog metadata, stable identity (`WallpaperId`), explicit asset type
//!   classification (`WallpaperType`), and read-only availability state (`AvailabilityState`).
//! - Never invokes media players (`libmpv`, `WebView2`), spawns child processes, touches Win32
//!   desktop windows (`WorkerW` / `Progman`), or connects to cloud services.
//! - Treats user-owned external media (`WallpaperSourceKind::ExternalDirectFile`) as strictly
//!   read-only: catalog updates and record removals never delete, rename, or rewrite source files.

#![forbid(unsafe_code)]

pub mod error;
pub mod management;
pub mod model;
pub mod path_safety;
pub mod registration;
pub mod repository;
pub mod validation;

pub use error::{
    redact_path_for_diagnostics, CatalogError, PathSafetyError, PersistenceError, ValidationError,
};
pub use management::{
    has_editable_metadata_changes, CatalogManagementError, CatalogManagementService,
    EditableMetadataPatch, MetadataUpdateOutcome, RelinkOutcome, RelinkRejectionReason,
    RelinkSelectionOutcome, RemovalOutcome, SingleAssetSelectionSource,
    StaticRelinkSelectionSource,
};
pub use model::{
    AssetReference, AvailabilityState, CatalogEntrySummary, CatalogLifecycle, StoredManifestV1,
    WallpaperId, WallpaperMetadata, WallpaperRecord, WallpaperSourceKind, WallpaperType,
    CURRENT_CATALOG_SCHEMA_VERSION,
};
pub use path_safety::{
    manifest_path, normalize_external_comparison_key, package_directory, probe_asset_availability,
    resolve_asset_path, validate_asset_reference, validate_thumbnail_relative_path,
};
pub use registration::{
    derive_display_title, AssetSelectionSource, BatchOverallStatus, RegistrationBatchReport,
    RegistrationError, RegistrationItemOutcome, RegistrationRejectionReason, SelectionOutcome,
    WallpaperRegistrationService, MAX_REGISTRATION_BATCH_SIZE,
};
pub use repository::{
    CatalogIntegrityReport, CatalogLibrarySnapshot, CatalogLoadIssue, LocalJsonCatalogRepository,
    WallpaperCatalogRepository,
};
pub use validation::{
    classify_extension_wallpaper_type, validate_asset_and_type, validate_lifecycle,
    validate_metadata, validate_record,
};
