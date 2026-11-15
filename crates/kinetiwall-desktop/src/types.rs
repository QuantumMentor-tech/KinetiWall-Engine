//! Core types, handles, candidate window records, and shell topology classifications.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Type-safe, platform-independent wrapper for a Win32 window handle (`HWND`).
///
/// Encapsulates the native pointer-sized handle without exposing raw FFI pointers
/// in public APIs. Guaranteed memory-safe and serializable for diagnostic snapshots.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
#[repr(transparent)]
pub struct NativeWindowHandle(pub usize);

impl NativeWindowHandle {
    /// The NULL window handle.
    pub const NULL: Self = Self(0);

    #[must_use]
    pub const fn from_raw(raw: usize) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn as_raw(self) -> usize {
        self.0
    }

    #[must_use]
    pub const fn is_null(self) -> bool {
        self.0 == 0
    }
}

impl fmt::Debug for NativeWindowHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HWND(0x{:08X})", self.0)
    }
}

impl fmt::Display for NativeWindowHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HWND(0x{:08X})", self.0)
    }
}

/// Screen-coordinate bounding rectangle of a desktop window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WindowRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl WindowRect {
    #[must_use]
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    #[must_use]
    pub const fn width(&self) -> i32 {
        self.right - self.left
    }

    #[must_use]
    pub const fn height(&self) -> i32 {
        self.bottom - self.top
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.width() <= 0 || self.height() <= 0
    }

    /// Computes the minimal bounding rectangle enclosing both rectangles.
    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        Self {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }

    /// Checks if this rectangle overlaps or intersects another rectangle.
    #[must_use]
    pub fn intersects(&self, other: &Self) -> bool {
        self.left < other.right
            && self.right > other.left
            && self.top < other.bottom
            && self.bottom > other.top
    }
}

/// Classified architectural role of an observed desktop window in the Windows shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateRole {
    /// The main desktop root window (e.g. `Progman`).
    DesktopRoot,
    /// The top-level window carrying the desktop icon view (can be `Progman` or a `WorkerW`).
    IconHostCarrier,
    /// The intermediate shell icon view container (`SHELLDLL_DefView`).
    IconView,
    /// The actual desktop icons list control (`SysListView32` or modern Explorer list).
    IconList,
    /// The validated secondary `WorkerW` sitting directly behind `SHELLDLL_DefView`,
    /// identified as a candidate for future wallpaper surface hosting.
    WallpaperHostCandidate,
    /// Other `WorkerW` windows used for shell flyouts, taskbar animation, DWM effects, etc.
    AncillaryWorker,
    /// Unclassified or unrecognized window role.
    Unknown,
}

impl fmt::Display for CandidateRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// Comprehensive, validated metadata record for an observed desktop window candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopCandidateWindow {
    /// The native window handle.
    pub handle: NativeWindowHandle,
    /// Registered Win32 window class name (e.g. "Progman", "WorkerW", "SHELLDLL_DefView").
    pub class_name: String,
    /// Window title or caption text.
    pub window_title: String,
    /// OS process ID owning the window (typically `explorer.exe`).
    pub process_id: u32,
    /// OS thread ID owning the window.
    pub thread_id: u32,
    /// Classified role within the desktop shell hierarchy.
    pub role: CandidateRole,
    /// Parent window handle, if any.
    pub parent: Option<NativeWindowHandle>,
    /// Screen-coordinate bounding rectangle.
    pub bounds: Option<WindowRect>,
    /// Whether the window has the `WS_VISIBLE` style.
    pub is_visible: bool,
    /// Whether this window directly contains a `SHELLDLL_DefView` child window.
    pub has_shelldll_defview_child: bool,
    /// Wall-clock timestamp in milliseconds when this candidate was validated.
    pub validated_at_millis: u64,
}

/// Overall classification of the observed Windows desktop shell topology.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopShellTopology {
    /// Classic / Unsplit Shell: `SHELLDLL_DefView` is child of `Progman`. No background
    /// `WorkerW` exists yet. Passive discovery cannot recommend a wallpaper host without
    /// an active split probe.
    ClassicUnsplit,
    /// Standard Split Shell: An icon carrier carries `SHELLDLL_DefView`, and exactly one
    /// sibling `WorkerW` exists immediately behind it without `SHELLDLL_DefView`.
    StandardWorkerWSplit,
    /// Modern Multi-WorkerW Shell: Multiple `WorkerW` windows are active (Windows 11 modern
    /// shell / Spotlight), with exactly one verified behind the icon view.
    ModernMultiWorkerW,
    /// Ambiguous Shell: Multiple plausible `WorkerW` candidates exist, but none can be
    /// conclusively identified as sitting directly behind the desktop icon view.
    Ambiguous,
    /// Desktop Root Unavailable: Neither `Progman` nor any recognized shell root window exists.
    DesktopRootUnavailable,
    /// Shell Unavailable: Windows Explorer process is not running or shell is restarting.
    ShellUnavailable,
    /// Unknown Topology: Windows exist, but hierarchy does not match any known pattern.
    UnknownTopology,
}

impl fmt::Display for DesktopShellTopology {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// The authoritative result of a desktop-host discovery evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopologyClassification {
    /// The classified desktop shell topology.
    pub topology: DesktopShellTopology,
    /// The single recommended wallpaper host candidate window, if conclusively validated.
    pub recommended_candidate: Option<DesktopCandidateWindow>,
    /// All candidate desktop windows observed during discovery.
    pub all_candidates: Vec<DesktopCandidateWindow>,
    /// The window carrying the desktop icons (`Progman` or `WorkerW`), if found.
    pub icon_carrier: Option<DesktopCandidateWindow>,
    /// The `SHELLDLL_DefView` window, if found.
    pub icon_view: Option<DesktopCandidateWindow>,
    /// Monotonic timestamp when this snapshot was captured.
    pub observed_at_millis: u64,
    /// Whether the topology was classified with high certainty and validated suitable
    /// for consideration by the future wallpaper attachment layer.
    pub is_safe_for_attachment_review: bool,
    /// Human-readable explanation of the classification decision and supporting evidence.
    pub reason: String,
}

/// Structured error conditions that can arise during native desktop discovery.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum DiscoveryError {
    #[error("Desktop root window not found: {0}")]
    DesktopRootNotFound(String),

    #[error("Native Win32 API failure in `{function}`: error code {code}, {message}")]
    NativeApiFailure {
        function: String,
        code: u32,
        message: String,
    },

    #[error("Window handle {0} is invalid or has been destroyed")]
    InvalidHandle(NativeWindowHandle),

    #[error("Candidate window {0} vanished during inspection or validation")]
    CandidateVanished(NativeWindowHandle),

    #[error("Active desktop split probe timed out after {0} ms")]
    ActiveProbeTimedOut(u32),

    #[error("Active desktop split probe is disabled by discovery options")]
    ActiveProbeDisabled,

    #[error("Discovery snapshot is stale and requires re-enumeration")]
    StaleSnapshot,
}
