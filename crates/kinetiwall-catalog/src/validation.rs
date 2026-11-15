use std::path::Path;

use crate::error::{CatalogError, ValidationError};
use crate::model::{
    AssetReference, CatalogLifecycle, WallpaperMetadata, WallpaperRecord, WallpaperType,
    CURRENT_CATALOG_SCHEMA_VERSION, MAX_AUTHOR_LEN, MAX_DESCRIPTION_LEN, MAX_LICENSE_LEN,
    MAX_TAG_COUNT, MAX_TAG_LEN, MAX_TITLE_LEN,
};
use crate::path_safety::{validate_asset_reference, validate_thumbnail_relative_path};

const VIDEO_EXTENSIONS: &[&str] = &["mp4", "webm", "avi", "mkv", "mov", "m4v"];
const WEB_EXTENSIONS: &[&str] = &["html", "htm"];

/// Performs complete domain, schema-version, metadata, lifecycle, and path-safety validation
/// on a [`WallpaperRecord`] without modifying user-supplied values.
pub fn validate_record(record: &WallpaperRecord) -> Result<(), CatalogError> {
    if record.schema_version != CURRENT_CATALOG_SCHEMA_VERSION {
        return Err(CatalogError::UnsupportedSchemaVersion {
            found: record.schema_version,
            supported: CURRENT_CATALOG_SCHEMA_VERSION,
        });
    }

    validate_metadata(&record.metadata)?;
    validate_lifecycle(&record.lifecycle)?;
    validate_asset_and_type(record.wallpaper_type, &record.asset)?;

    Ok(())
}

/// Validates descriptive metadata fields independently (e.g. before applying a metadata update).
pub fn validate_metadata(metadata: &WallpaperMetadata) -> Result<(), ValidationError> {
    let trimmed_title = metadata.title.trim();
    if trimmed_title.is_empty() {
        return Err(ValidationError::BlankTitle);
    }
    let title_len = metadata.title.chars().count();
    if title_len > MAX_TITLE_LEN {
        return Err(ValidationError::TitleTooLong {
            max: MAX_TITLE_LEN,
            actual: title_len,
        });
    }
    if metadata.title.chars().any(char::is_control) {
        return Err(ValidationError::TitleControlCharacters);
    }

    let desc_len = metadata.description.chars().count();
    if desc_len > MAX_DESCRIPTION_LEN {
        return Err(ValidationError::DescriptionTooLong {
            max: MAX_DESCRIPTION_LEN,
            actual: desc_len,
        });
    }

    if metadata.author.trim().is_empty() {
        return Err(ValidationError::BlankAuthor);
    }
    let author_len = metadata.author.chars().count();
    if author_len > MAX_AUTHOR_LEN {
        return Err(ValidationError::AuthorTooLong {
            max: MAX_AUTHOR_LEN,
            actual: author_len,
        });
    }

    validate_semver("version", &metadata.version)?;
    validate_semver("min_engine_version", &metadata.min_engine_version)?;

    let license_len = metadata.license.chars().count();
    if license_len > MAX_LICENSE_LEN {
        return Err(ValidationError::LicenseTooLong {
            max: MAX_LICENSE_LEN,
            actual: license_len,
        });
    }

    if metadata.tags.len() > MAX_TAG_COUNT {
        return Err(ValidationError::TooManyTags {
            max: MAX_TAG_COUNT,
            actual: metadata.tags.len(),
        });
    }

    for (index, tag) in metadata.tags.iter().enumerate() {
        if tag.trim().is_empty() {
            return Err(ValidationError::InvalidTag {
                index,
                reason: "tag must not be blank".to_string(),
            });
        }
        let len = tag.chars().count();
        if len > MAX_TAG_LEN {
            return Err(ValidationError::InvalidTag {
                index,
                reason: format!("exceeds max length of {MAX_TAG_LEN} characters"),
            });
        }
        if tag.chars().any(char::is_control) {
            return Err(ValidationError::InvalidTag {
                index,
                reason: "contains control characters".to_string(),
            });
        }
    }

    if let Some(ref thumb) = metadata.thumbnail_ref {
        validate_thumbnail_relative_path(thumb).map_err(|err| {
            ValidationError::InvalidThumbnailRef {
                reason: err.to_string(),
            }
        })?;
    }

    Ok(())
}

/// Validates lifecycle timestamp ordering.
pub fn validate_lifecycle(lifecycle: &CatalogLifecycle) -> Result<(), ValidationError> {
    if lifecycle.updated_at < lifecycle.created_at {
        return Err(ValidationError::InvalidLifecycleOrder {
            created_at: lifecycle.created_at.to_rfc3339(),
            updated_at: lifecycle.updated_at.to_rfc3339(),
        });
    }
    Ok(())
}

/// Maps a file's extension to an approved [`WallpaperType`] candidate.
///
/// Extension alone does not prove file validity; content/header inspection is still required
/// during registration.
#[must_use]
pub fn classify_extension_wallpaper_type(path: &Path) -> Option<WallpaperType> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)?;

    if VIDEO_EXTENSIONS.contains(&ext.as_str()) {
        Some(WallpaperType::Video)
    } else if WEB_EXTENSIONS.contains(&ext.as_str()) {
        Some(WallpaperType::Web)
    } else {
        None
    }
}

/// Validates path-safety boundaries and consistency between `wallpaper_type` and `asset.entrypoint`.
pub fn validate_asset_and_type(
    wallpaper_type: WallpaperType,
    asset: &AssetReference,
) -> Result<(), CatalogError> {
    let normalized = validate_asset_reference(asset)?;
    let is_compatible =
        classify_extension_wallpaper_type(Path::new(&normalized)) == Some(wallpaper_type);

    if !is_compatible {
        return Err(CatalogError::Validation(
            ValidationError::IncompatibleAssetType {
                wallpaper_type: wallpaper_type.as_str(),
                entrypoint: asset.entrypoint.clone(),
            },
        ));
    }

    Ok(())
}

fn validate_semver(field: &'static str, value: &str) -> Result<(), ValidationError> {
    let mut parts = value.split('.');
    let major = parts.next();
    let minor = parts.next();
    let patch = parts.next();
    let extra = parts.next();

    let valid = match (major, minor, patch, extra) {
        (Some(a), Some(b), Some(c), None) => {
            is_numeric_part(a) && is_numeric_part(b) && is_numeric_part(c)
        }
        _ => false,
    };

    if !valid {
        return Err(ValidationError::InvalidSemVer {
            field,
            value: value.to_string(),
        });
    }

    Ok(())
}

fn is_numeric_part(part: &str) -> bool {
    !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())
}
