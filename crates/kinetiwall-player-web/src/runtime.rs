//! Dynamic runtime detection for Microsoft Edge WebView2 on Windows.

use std::path::{Path, PathBuf};

pub struct WebView2RuntimeInfo {
    pub version: String,
    pub path: PathBuf,
}

/// Probes for an installed WebView2 runtime on Windows via registry or standard system directories.
pub fn detect_webview2_runtime() -> Option<WebView2RuntimeInfo> {
    // 1. Check standard Program Files (x86) directory
    let candidate = Path::new("C:\\Program Files (x86)\\Microsoft\\EdgeWebView\\Application");
    if candidate.exists() {
        if let Ok(entries) = std::fs::read_dir(candidate) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                // Version folders typically follow major.minor.build.patch format
                if name.chars().next().is_some_and(|c| c.is_ascii_digit()) && name.contains('.') {
                    return Some(WebView2RuntimeInfo {
                        version: name,
                        path: entry.path(),
                    });
                }
            }
        }
    }

    // 2. Check 64-bit Program Files directory
    let candidate64 = Path::new("C:\\Program Files\\Microsoft\\EdgeWebView\\Application");
    if candidate64.exists() {
        if let Ok(entries) = std::fs::read_dir(candidate64) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.chars().next().is_some_and(|c| c.is_ascii_digit()) && name.contains('.') {
                    return Some(WebView2RuntimeInfo {
                        version: name,
                        path: entry.path(),
                    });
                }
            }
        }
    }

    None
}
