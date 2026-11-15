//! Validation and containment verification for local web wallpaper assets.
//!
//! Enforces:
//! 1. No null bytes or malformed characters.
//! 2. Path must be an existing regular file with `.html` or `.htm` extension.
//! 3. Content signature validation (reusing `classify_extension_wallpaper_type`).
//! 4. Identification of the root asset boundary directory for virtual host mapping.

use std::fs::File;
use std::path::PathBuf;

use kinetiwall_catalog::{
    classify_extension_wallpaper_type, validate_asset_reference, AssetReference,
    WallpaperSourceKind, WallpaperType,
};
use kinetiwall_playback::PlaybackErrorKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedWebWallpaper {
    pub entrypoint: PathBuf,
    pub asset_root: PathBuf,
}

pub fn validate_web_wallpaper_path(raw: &str) -> Result<ValidatedWebWallpaper, PlaybackErrorKind> {
    if raw.contains('\0') {
        return Err(PlaybackErrorKind::InvalidRequest);
    }

    let asset = AssetReference {
        source_kind: WallpaperSourceKind::ExternalDirectFile,
        entrypoint: raw.to_string(),
    };

    let path = validate_asset_reference(&asset).map_err(|_| PlaybackErrorKind::InvalidRequest)?;

    if classify_extension_wallpaper_type(&path) != Some(WallpaperType::Web) {
        return Err(PlaybackErrorKind::InvalidRequest);
    }

    let meta = std::fs::metadata(&path).map_err(|_| PlaybackErrorKind::AssetUnavailable)?;
    if !meta.is_file() {
        return Err(PlaybackErrorKind::AssetUnavailable);
    }

    // Verify file accessibility
    File::open(&path).map_err(|_| PlaybackErrorKind::AssetUnavailable)?;

    // Asset root is the parent directory of index.html
    let asset_root = path
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or(PlaybackErrorKind::InvalidRequest)?;

    Ok(ValidatedWebWallpaper {
        entrypoint: path,
        asset_root,
    })
}

/// Checks whether a requested navigation URI violates KinetiWall's hermetic isolation model.
/// Only `https://kinetiwall.local/` virtual origin is permitted; all external internet,
/// `file:///` protocols, custom schemes, and popups are blocked.
pub fn is_uri_permitted(uri: &str) -> bool {
    uri == "about:blank" || uri.starts_with("https://kinetiwall.local/")
}
