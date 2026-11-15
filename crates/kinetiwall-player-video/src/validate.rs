//! Defense-in-depth validation of a media path inside the host. The controller resolves the path
//! from the trusted catalog; the host still refuses anything that is not an absolute, readable,
//! regular file with an allowlisted video extension. A matching extension is **not** proof the
//! file decodes: libmpv decides that.

use std::fs::File;
use std::path::PathBuf;

use kinetiwall_catalog::{
    classify_extension_wallpaper_type, validate_asset_reference, AssetReference,
    WallpaperSourceKind, WallpaperType,
};
use kinetiwall_playback::PlaybackErrorKind;

pub fn validate_media_path(raw: &str) -> Result<PathBuf, PlaybackErrorKind> {
    if raw.contains('\0') {
        return Err(PlaybackErrorKind::InvalidRequest);
    }
    let asset = AssetReference {
        source_kind: WallpaperSourceKind::ExternalDirectFile,
        entrypoint: raw.to_string(),
    };
    // Absolute path, no traversal, forbidden executable/script extensions rejected.
    let path = validate_asset_reference(&asset).map_err(|_| PlaybackErrorKind::InvalidRequest)?;
    if classify_extension_wallpaper_type(&path) != Some(WallpaperType::Video) {
        return Err(PlaybackErrorKind::InvalidRequest);
    }
    let meta = std::fs::metadata(&path).map_err(|_| PlaybackErrorKind::AssetUnavailable)?;
    if !meta.is_file() {
        return Err(PlaybackErrorKind::AssetUnavailable);
    }
    File::open(&path).map_err(|_| PlaybackErrorKind::AssetUnavailable)?;
    Ok(path)
}
