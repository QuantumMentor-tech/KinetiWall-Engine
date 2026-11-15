use std::fs::{self, File};
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use crate::error::PathSafetyError;
use crate::model::{
    AssetReference, AvailabilityState, WallpaperId, WallpaperSourceKind, MAX_PATH_LEN,
};

/// Executable and script extensions prohibited as catalog entrypoints or thumbnail targets (`KW-DAT-007`).
const FORBIDDEN_EXTENSIONS: &[&str] = &[
    "exe", "dll", "sys", "com", "scr", "msi", "drv", "ocx", "bat", "cmd", "ps1", "psm1", "vbs",
    "vbe", "js", "jse", "wsf", "wsh", "reg", "lnk", "pif", "url", "jar", "class", "pyc",
];

/// Normalizes and validates an [`AssetReference`] against security and path-form boundaries.
///
/// This validates path syntax without asserting that the underlying target currently exists on disk.
pub fn validate_asset_reference(asset: &AssetReference) -> Result<PathBuf, PathSafetyError> {
    let raw = asset.entrypoint.as_str();
    check_common_path_guards(raw)?;

    match asset.source_kind {
        WallpaperSourceKind::ManagedPackage => validate_managed_relative_path(raw),
        WallpaperSourceKind::ExternalDirectFile => validate_external_absolute_path(raw),
    }
}

/// Validates an optional relative thumbnail path stored inside a package manifest.
pub fn validate_thumbnail_relative_path(raw: &str) -> Result<PathBuf, PathSafetyError> {
    check_common_path_guards(raw)?;
    validate_managed_relative_path(raw)
}

/// Resolves an [`AssetReference`] to a concrete filesystem path inside `library_root` (for managed
/// packages) or the validated external absolute path (for user-owned media).
///
/// For `ManagedPackage` entries, if both the package root and target exist on disk, canonicalization
/// verifies that symlinks/junctions do not escape the package directory boundary.
pub fn resolve_asset_path(
    library_root: &Path,
    id: WallpaperId,
    asset: &AssetReference,
) -> Result<PathBuf, PathSafetyError> {
    let validated = validate_asset_reference(asset)?;

    match asset.source_kind {
        WallpaperSourceKind::ManagedPackage => {
            let package_root = package_directory(library_root, id);
            let joined = package_root.join(&validated);

            // Prevent symlink / reparse-point boundary escapes when paths exist on disk.
            if package_root.exists() && joined.exists() {
                if let (Ok(canon_root), Ok(canon_target)) =
                    (fs::canonicalize(&package_root), fs::canonicalize(&joined))
                {
                    if !canon_target.starts_with(&canon_root) {
                        return Err(PathSafetyError::PackageBoundaryEscape);
                    }
                }
            }

            Ok(joined)
        }
        WallpaperSourceKind::ExternalDirectFile => Ok(validated),
    }
}

/// Computes the internal KinetiWall metadata/package directory for a wallpaper ID (`wallpapers/{id}`).
#[must_use]
pub fn package_directory(library_root: &Path, id: WallpaperId) -> PathBuf {
    library_root.join(id.to_string())
}

/// Computes the manifest file path for a catalog entry (`wallpapers/{id}/project.json`).
#[must_use]
pub fn manifest_path(library_root: &Path, id: WallpaperId) -> PathBuf {
    package_directory(library_root, id).join("project.json")
}

/// Probes read-only availability for the target asset referenced by `(id, asset)`.
///
/// Never modifies, moves, creates, or deletes the target asset.
#[must_use]
pub fn probe_asset_availability(
    library_root: &Path,
    id: WallpaperId,
    asset: &AssetReference,
) -> AvailabilityState {
    let resolved = match resolve_asset_path(library_root, id, asset) {
        Ok(path) => path,
        Err(err) => {
            return AvailabilityState::InvalidAsset {
                reason: err.to_string(),
            };
        }
    };

    match fs::metadata(&resolved) {
        Ok(meta) => {
            if !meta.is_file() {
                return AvailabilityState::InvalidAsset {
                    reason: "entrypoint must resolve to a regular file, not a directory"
                        .to_string(),
                };
            }
            // Perform a read-only open check to verify ACL readability without modifying the file.
            match File::open(&resolved) {
                Ok(_) => AvailabilityState::Available,
                Err(err) => classify_io_availability_error(&err),
            }
        }
        Err(err) => classify_io_availability_error(&err),
    }
}

pub(crate) fn classify_io_availability_error(err: &std::io::Error) -> AvailabilityState {
    match err.kind() {
        ErrorKind::NotFound => AvailabilityState::Missing,
        ErrorKind::PermissionDenied => AvailabilityState::Inaccessible {
            reason: "permission denied".to_string(),
        },
        _ => {
            // Win32 ERROR_ACCESS_DENIED (5) or ERROR_SHARING_VIOLATION (32)
            if matches!(err.raw_os_error(), Some(5) | Some(32)) {
                AvailabilityState::Inaccessible {
                    reason: format!("os access error ({})", err.kind()),
                }
            } else {
                AvailabilityState::Inaccessible {
                    reason: err.kind().to_string(),
                }
            }
        }
    }
}

fn check_common_path_guards(raw: &str) -> Result<(), PathSafetyError> {
    if raw.trim().is_empty() {
        return Err(PathSafetyError::EmptyPath);
    }
    if raw.contains('\0') {
        return Err(PathSafetyError::NullByte);
    }
    if raw.chars().count() > MAX_PATH_LEN {
        return Err(PathSafetyError::PathTooLong {
            max: MAX_PATH_LEN,
            actual: raw.chars().count(),
        });
    }

    let path = Path::new(raw);
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let lower = ext.to_ascii_lowercase();
        if FORBIDDEN_EXTENSIONS.contains(&lower.as_str()) {
            return Err(PathSafetyError::ForbiddenExtension { extension: lower });
        }
    }

    Ok(())
}

fn validate_managed_relative_path(raw: &str) -> Result<PathBuf, PathSafetyError> {
    if raw.starts_with('/') || raw.starts_with('\\') {
        return Err(PathSafetyError::ManagedPathMustBeRelative);
    }

    let path = Path::new(raw);
    if path.is_absolute() {
        return Err(PathSafetyError::ManagedPathMustBeRelative);
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir => return Err(PathSafetyError::TraversalDetected),
            Component::Prefix(_) | Component::RootDir => {
                return Err(PathSafetyError::ManagedPathMustBeRelative);
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err(PathSafetyError::EmptyPath);
    }

    Ok(normalized)
}

fn validate_external_absolute_path(raw: &str) -> Result<PathBuf, PathSafetyError> {
    let path = Path::new(raw);

    // On Windows: rejects relative paths (`foo\bar.mp4`), root-relative (`\foo.mp4`),
    // and drive-relative (`C:foo.mp4`). Accepts `C:\...` and UNC `\\server\share\...`.
    if !is_strictly_absolute_path(path) {
        return Err(PathSafetyError::ExternalPathMustBeAbsolute);
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir => return Err(PathSafetyError::TraversalDetected),
        }
    }

    if normalized.file_name().is_none() {
        return Err(PathSafetyError::MalformedPath {
            detail: "external asset path must identify a file".to_string(),
        });
    }

    Ok(normalized)
}

fn is_strictly_absolute_path(path: &Path) -> bool {
    if path.is_absolute() {
        return true;
    }

    // Handle Windows-style paths on non-Windows test hosts if ever evaluated cross-platform.
    let s = path.as_os_str().to_string_lossy();
    let bytes = s.as_bytes();
    let is_drive_abs = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/');
    let is_unc = s.starts_with("\\\\") && s.len() > 4;
    is_drive_abs || is_unc
}

/// Computes a platform-appropriate normalized comparison key for external file references.
///
/// Uses `fs::canonicalize` when accessible (resolving drive-letter/casing differences on Windows
/// without altering or moving the underlying file), and falls back to lexical normalization.
#[must_use]
pub fn normalize_external_comparison_key(path: &Path) -> String {
    let resolved = fs::canonicalize(path).unwrap_or_else(|_| {
        let mut normalized = PathBuf::new();
        for component in path.components() {
            if !matches!(component, Component::CurDir) {
                normalized.push(component.as_os_str());
            }
        }
        normalized
    });

    let text = resolved.to_string_lossy();
    let stripped = text.strip_prefix(r"\\?\").unwrap_or(&text);

    if cfg!(windows) {
        stripped.to_lowercase()
    } else {
        stripped.to_string()
    }
}
