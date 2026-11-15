//! Multi-monitor enumeration, stable display identity, and display topology modeling (Step 42).
//!
//! Provides a testable, read-only Windows display discovery subsystem that enumerates
//! active desktop displays, records their geometry (including virtual-screen and negative coordinates),
//! analyzes active topology, generates stable application-level display identifiers, and publishes
//! versioned display snapshots.

use std::collections::HashSet;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::types::WindowRect;

// -------------------------------------------------------------------------------------------------
// Native Handle & Identity
// -------------------------------------------------------------------------------------------------

/// Type-safe wrapper for a Win32 monitor handle (`HMONITOR`).
///
/// NOTE: A `NativeMonitorHandle` is transient and valid ONLY within the lifetime of
/// the snapshot generation in which it was queried. It must NEVER be stored or used
/// as a permanent or cross-session display identifier.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
#[repr(transparent)]
pub struct NativeMonitorHandle(pub usize);

impl NativeMonitorHandle {
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

impl fmt::Debug for NativeMonitorHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HMONITOR(0x{:08X})", self.0)
    }
}

impl fmt::Display for NativeMonitorHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HMONITOR(0x{:08X})", self.0)
    }
}

/// Stable application-level display identifier.
///
/// Unlike native `HMONITOR` handles which vary between runs or after display changes,
/// a `DisplayId` provides a deterministic identity derived from verified device identity
/// (such as hardware PnP ID and OS display device name).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DisplayId(pub String);

impl DisplayId {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for DisplayId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DisplayId(\"{}\")", self.0)
    }
}

impl fmt::Display for DisplayId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// -------------------------------------------------------------------------------------------------
// Display Geometry & Descriptors
// -------------------------------------------------------------------------------------------------

/// Validated display geometry in virtual-screen coordinate space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayGeometry {
    /// Full display rectangle in virtual-screen coordinates.
    ///
    /// Can have negative left/top coordinates when secondary displays are positioned
    /// to the left of or above the primary monitor.
    pub monitor_rect: WindowRect,

    /// Work area rectangle (excluding taskbars, docked sidebars, and appbars) in virtual screen space.
    pub work_area_rect: WindowRect,

    /// Pixel width calculated safely from `monitor_rect`.
    pub width: i32,

    /// Pixel height calculated safely from `monitor_rect`.
    pub height: i32,

    /// Optional scale factor percentage (e.g. 100, 125, 150, 200) if determined.
    pub scale_factor_percent: Option<u32>,
}

impl DisplayGeometry {
    /// Validates whether the rectangles are non-empty and mathematically consistent.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.width > 0
            && self.height > 0
            && !self.monitor_rect.is_empty()
            && !self.work_area_rect.is_empty()
            && self.work_area_rect.left >= self.monitor_rect.left
            && self.work_area_rect.right <= self.monitor_rect.right
            && self.work_area_rect.top >= self.monitor_rect.top
            && self.work_area_rect.bottom <= self.monitor_rect.bottom
    }
}

/// Comprehensive, validated descriptor for an active desktop monitor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayDescriptor {
    /// Stable application-level display identity.
    pub id: DisplayId,

    /// Transient Win32 monitor handle valid ONLY for the current snapshot generation.
    pub native_handle: NativeMonitorHandle,

    /// OS display device name, e.g. `"\\\\.\\DISPLAY1"`.
    pub device_name: String,

    /// Human-readable friendly monitor name (e.g. `"Dell U2720Q"` or `"Generic PnP Monitor"`).
    pub friendly_name: Option<String>,

    /// Hardware PnP identifier if available from device enumeration.
    pub hardware_id: Option<String>,

    /// Whether this display is the Windows primary monitor (origin `(0, 0)`).
    pub is_primary: bool,

    /// Validated coordinate geometry in virtual-screen space.
    pub geometry: DisplayGeometry,

    /// Whether the display is actively attached to the desktop.
    pub is_attached: bool,

    /// Whether this is a pseudo/mirroring driver device.
    pub is_mirroring: bool,

    /// Snapshot generation in which this descriptor was captured.
    pub snapshot_generation: u64,
}

// -------------------------------------------------------------------------------------------------
// Display Topology Classification
// -------------------------------------------------------------------------------------------------

/// Structural arrangement of displays in the Windows desktop environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayTopologyType {
    /// Exactly one active desktop monitor.
    SingleDisplay,

    /// Two or more distinct monitors arranged in an extended virtual desktop.
    ExtendedDesktop,

    /// Multiple physical monitors duplicating/cloning the same source display area.
    CloneOrMirrored,

    /// Heterogeneous or complex arrangement (e.g. combination of cloned and extended).
    ComplexOrMixed,

    /// Inconsistent, transitional, or unsupported display topology.
    UnknownOrUnsupported,
}

impl fmt::Display for DisplayTopologyType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

// -------------------------------------------------------------------------------------------------
// Snapshot & Change Delta
// -------------------------------------------------------------------------------------------------

/// Coherent, point-in-time snapshot of the Windows display environment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayTopologySnapshot {
    /// Monotonically increasing snapshot generation.
    pub generation: u64,

    /// Capture timestamp in epoch milliseconds.
    pub captured_at_millis: u64,

    /// All active logical desktop monitor descriptors in this snapshot.
    pub displays: Vec<DisplayDescriptor>,

    /// Stable ID of the primary display, if resolved.
    pub primary_display_id: Option<DisplayId>,

    /// Combined bounding rectangle enclosing all active displays in virtual-screen coordinates.
    pub virtual_desktop_bounds: WindowRect,

    /// Classified display topology.
    pub topology_type: DisplayTopologyType,

    /// Whether the snapshot completed with full validation.
    pub is_valid: bool,

    /// Non-fatal diagnostic warnings observed during enumeration.
    pub diagnostic_warnings: Vec<String>,
}

impl DisplayTopologySnapshot {
    /// Returns an empty, uninitialized snapshot.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            generation: 0,
            captured_at_millis: 0,
            displays: Vec::new(),
            primary_display_id: None,
            virtual_desktop_bounds: WindowRect::default(),
            topology_type: DisplayTopologyType::UnknownOrUnsupported,
            is_valid: false,
            diagnostic_warnings: Vec::new(),
        }
    }

    /// Finds a display descriptor by its stable `DisplayId`.
    #[must_use]
    pub fn find_by_id(&self, id: &DisplayId) -> Option<&DisplayDescriptor> {
        self.displays.iter().find(|d| d.id == *id)
    }

    /// Finds a display descriptor by its transient `NativeMonitorHandle`.
    #[must_use]
    pub fn find_by_handle(&self, handle: NativeMonitorHandle) -> Option<&DisplayDescriptor> {
        self.displays.iter().find(|d| d.native_handle == handle)
    }

    /// Retrieves the primary display descriptor if available.
    #[must_use]
    pub fn get_primary(&self) -> Option<&DisplayDescriptor> {
        if let Some(ref pid) = self.primary_display_id {
            self.find_by_id(pid)
        } else {
            self.displays.iter().find(|d| d.is_primary)
        }
    }

    /// Detects meaningful architectural differences between this snapshot and a new snapshot.
    #[must_use]
    pub fn detect_change(&self, new_snap: &DisplayTopologySnapshot) -> DisplayChangeDelta {
        let old_ids: HashSet<DisplayId> = self.displays.iter().map(|d| d.id.clone()).collect();
        let new_ids: HashSet<DisplayId> = new_snap.displays.iter().map(|d| d.id.clone()).collect();

        let displays_added: Vec<DisplayId> = new_ids.difference(&old_ids).cloned().collect();
        let displays_removed: Vec<DisplayId> = old_ids.difference(&new_ids).cloned().collect();

        let mut displays_modified = Vec::new();
        for new_disp in &new_snap.displays {
            if let Some(old_disp) = self.find_by_id(&new_disp.id) {
                if old_disp.geometry != new_disp.geometry
                    || old_disp.is_primary != new_disp.is_primary
                    || old_disp.is_attached != new_disp.is_attached
                {
                    displays_modified.push(new_disp.id.clone());
                }
            }
        }

        let primary_changed = self.primary_display_id != new_snap.primary_display_id;
        let geometry_changed = self.virtual_desktop_bounds != new_snap.virtual_desktop_bounds
            || !displays_modified.is_empty();
        let topology_changed = self.topology_type != new_snap.topology_type;

        let is_meaningful_change = !displays_added.is_empty()
            || !displays_removed.is_empty()
            || !displays_modified.is_empty()
            || primary_changed
            || geometry_changed
            || topology_changed;

        DisplayChangeDelta {
            previous_generation: self.generation,
            current_generation: new_snap.generation,
            displays_added,
            displays_removed,
            displays_modified,
            primary_changed,
            geometry_changed,
            topology_changed,
            is_meaningful_change,
        }
    }
}

/// Meaningful differences detected between two consecutive display snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayChangeDelta {
    pub previous_generation: u64,
    pub current_generation: u64,
    pub displays_added: Vec<DisplayId>,
    pub displays_removed: Vec<DisplayId>,
    pub displays_modified: Vec<DisplayId>,
    pub primary_changed: bool,
    pub geometry_changed: bool,
    pub topology_changed: bool,
    pub is_meaningful_change: bool,
}

// -------------------------------------------------------------------------------------------------
// Errors
// -------------------------------------------------------------------------------------------------

/// Strongly typed errors produced by display discovery and topology analysis.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum DisplayDiscoveryError {
    #[error("Native Win32 display API failure in `{function}` (code {code}): {message}")]
    NativeApiFailure {
        function: String,
        code: u32,
        message: String,
    },

    #[error("No active desktop displays were enumerated by the operating system")]
    NoActiveDisplays,

    #[error("Invalid display geometry for monitor `{device_name}`: {bounds:?} ({reason})")]
    InvalidGeometry {
        device_name: String,
        bounds: WindowRect,
        reason: String,
    },

    #[error("Inconsistent display topology: {0}")]
    InconsistentTopology(String),

    #[error("Ambiguous display identity resolution: {0}")]
    AmbiguousDisplayIdentity(String),

    #[error("Stale snapshot generation: expected {expected}, actual {actual}")]
    StaleSnapshotGeneration { expected: u64, actual: u64 },

    #[error("Display discovery provider unavailable: {0}")]
    ProviderUnavailable(String),
}

// -------------------------------------------------------------------------------------------------
// Display Provider Abstractions
// -------------------------------------------------------------------------------------------------

/// Raw monitor record retrieved directly from `GetMonitorInfoW`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawMonitorInfo {
    pub handle: NativeMonitorHandle,
    pub monitor_rect: WindowRect,
    pub work_area_rect: WindowRect,
    pub is_primary: bool,
    pub device_name: String,
}

/// Raw display device record retrieved from `EnumDisplayDevicesW`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawDisplayDeviceInfo {
    pub device_string: String,
    pub device_id: String,
    pub is_attached: bool,
    pub is_mirroring: bool,
}

/// Summary of active display paths from `QueryDisplayConfig`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawDisplayConfigSummary {
    pub active_paths_count: usize,
    pub unique_sources_count: usize,
    pub unique_targets_count: usize,
    pub is_clone: bool,
}

/// Platform abstraction for Windows display queries.
pub trait DesktopDisplayProvider: Send + Sync {
    /// Enumerates active monitor handles via `EnumDisplayMonitors`.
    fn enum_display_monitors(&self) -> Result<Vec<NativeMonitorHandle>, DisplayDiscoveryError>;

    /// Queries monitor bounds and attributes via `GetMonitorInfoW`.
    fn get_monitor_info(
        &self,
        monitor: NativeMonitorHandle,
    ) -> Result<RawMonitorInfo, DisplayDiscoveryError>;

    /// Queries device metadata via `EnumDisplayDevicesW`.
    fn get_display_device_info(
        &self,
        device_name: &str,
    ) -> Result<Option<RawDisplayDeviceInfo>, DisplayDiscoveryError>;

    /// Queries active display path configuration where available.
    fn query_display_config(
        &self,
    ) -> Result<Option<RawDisplayConfigSummary>, DisplayDiscoveryError>;
}

// -------------------------------------------------------------------------------------------------
// Native Win32 Display Provider Implementation
// -------------------------------------------------------------------------------------------------

#[cfg(windows)]
#[allow(non_snake_case, non_camel_case_types, clippy::upper_case_acronyms)]
mod sys {
    use std::ffi::c_void;

    pub type BOOL = i32;
    pub type DWORD = u32;
    pub type LPARAM = isize;
    pub type UINT = u32;
    pub type HDC = *mut c_void;
    pub type HMONITOR = *mut c_void;

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    pub struct RECT {
        pub left: i32,
        pub top: i32,
        pub right: i32,
        pub bottom: i32,
    }

    pub type MONITORENUMPROC = unsafe extern "system" fn(HMONITOR, HDC, *mut RECT, LPARAM) -> BOOL;

    #[repr(C)]
    pub struct MONITORINFOEXW {
        pub cbSize: DWORD,
        pub rcMonitor: RECT,
        pub rcWork: RECT,
        pub dwFlags: DWORD,
        pub szDevice: [u16; 32],
    }

    #[repr(C)]
    pub struct DISPLAY_DEVICEW {
        pub cb: DWORD,
        pub DeviceName: [u16; 32],
        pub DeviceString: [u16; 128],
        pub StateFlags: DWORD,
        pub DeviceID: [u16; 128],
        pub DeviceKey: [u16; 128],
    }

    pub const MONITORINFOF_PRIMARY: DWORD = 0x00000001;
    pub const DISPLAY_DEVICE_ATTACHED_TO_DESKTOP: DWORD = 0x00000001;
    pub const DISPLAY_DEVICE_MIRRORING_DRIVER: DWORD = 0x00000008;

    pub const QDC_ONLY_ACTIVE_PATHS: UINT = 0x00000002;
    pub const ERROR_SUCCESS: i32 = 0;

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    pub struct LUID {
        pub LowPart: DWORD,
        pub HighPart: i32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct DISPLAYCONFIG_PATH_SOURCE_INFO {
        pub adapterId: LUID,
        pub id: UINT,
        pub modeInfoIdx: UINT,
        pub statusFlags: UINT,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct DISPLAYCONFIG_PATH_TARGET_INFO {
        pub adapterId: LUID,
        pub id: UINT,
        pub modeInfoIdx: UINT,
        pub outputTechnology: UINT,
        pub rotation: UINT,
        pub scaling: UINT,
        pub scanLineOrdering: UINT,
        pub targetAvailable: BOOL,
        pub statusFlags: UINT,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct DISPLAYCONFIG_PATH_INFO {
        pub sourceInfo: DISPLAYCONFIG_PATH_SOURCE_INFO,
        pub targetInfo: DISPLAYCONFIG_PATH_TARGET_INFO,
        pub flags: UINT,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct DISPLAYCONFIG_MODE_INFO {
        pub infoType: UINT,
        pub id: UINT,
        pub adapterId: LUID,
        pub dummy: [u8; 64],
    }

    extern "system" {
        pub fn EnumDisplayMonitors(
            hdc: HDC,
            lprcClip: *const RECT,
            lpfnEnum: MONITORENUMPROC,
            dwData: LPARAM,
        ) -> BOOL;

        pub fn GetMonitorInfoW(hMonitor: HMONITOR, lpmi: *mut MONITORINFOEXW) -> BOOL;

        pub fn EnumDisplayDevicesW(
            lpDevice: *const u16,
            iDevNum: DWORD,
            lpDisplayDevice: *mut DISPLAY_DEVICEW,
            dwFlags: DWORD,
        ) -> BOOL;

        pub fn GetDisplayConfigBufferSizes(
            flags: UINT,
            numPathArrayElements: *mut UINT,
            numModeInfoArrayElements: *mut UINT,
        ) -> i32;

        pub fn QueryDisplayConfig(
            flags: UINT,
            numPathArrayElements: *mut UINT,
            pathArray: *mut DISPLAYCONFIG_PATH_INFO,
            numModeInfoArrayElements: *mut UINT,
            modeInfoArray: *mut DISPLAYCONFIG_MODE_INFO,
            currentTopologyId: *mut UINT,
        ) -> i32;

        pub fn GetLastError() -> DWORD;
    }
}

/// Real Windows API implementation of `DesktopDisplayProvider`.
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeDesktopDisplayProvider;

impl NativeDesktopDisplayProvider {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[cfg(windows)]
unsafe extern "system" fn enum_monitors_callback(
    hmonitor: sys::HMONITOR,
    _hdc: sys::HDC,
    _rect: *mut sys::RECT,
    lparam: sys::LPARAM,
) -> sys::BOOL {
    let list = &mut *(lparam as *mut Vec<NativeMonitorHandle>);
    list.push(NativeMonitorHandle(hmonitor as usize));
    1 // Continue
}

#[cfg(windows)]
impl DesktopDisplayProvider for NativeDesktopDisplayProvider {
    fn enum_display_monitors(&self) -> Result<Vec<NativeMonitorHandle>, DisplayDiscoveryError> {
        let mut list: Vec<NativeMonitorHandle> = Vec::new();
        let res = unsafe {
            sys::EnumDisplayMonitors(
                std::ptr::null_mut(),
                std::ptr::null(),
                enum_monitors_callback,
                &mut list as *mut _ as sys::LPARAM,
            )
        };

        if res == 0 {
            let err = unsafe { sys::GetLastError() };
            Err(DisplayDiscoveryError::NativeApiFailure {
                function: "EnumDisplayMonitors".into(),
                code: err,
                message: "Failed to enumerate desktop display monitors".into(),
            })
        } else {
            Ok(list)
        }
    }

    fn get_monitor_info(
        &self,
        monitor: NativeMonitorHandle,
    ) -> Result<RawMonitorInfo, DisplayDiscoveryError> {
        let mut info = sys::MONITORINFOEXW {
            cbSize: std::mem::size_of::<sys::MONITORINFOEXW>() as sys::DWORD,
            rcMonitor: sys::RECT::default(),
            rcWork: sys::RECT::default(),
            dwFlags: 0,
            szDevice: [0; 32],
        };

        let res = unsafe { sys::GetMonitorInfoW(monitor.0 as sys::HMONITOR, &mut info) };
        if res == 0 {
            let err = unsafe { sys::GetLastError() };
            return Err(DisplayDiscoveryError::NativeApiFailure {
                function: "GetMonitorInfoW".into(),
                code: err,
                message: format!("GetMonitorInfoW failed for {monitor}"),
            });
        }

        let device_name = {
            let len = info
                .szDevice
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(info.szDevice.len());
            String::from_utf16_lossy(&info.szDevice[..len])
        };

        let monitor_rect = WindowRect::new(
            info.rcMonitor.left,
            info.rcMonitor.top,
            info.rcMonitor.right,
            info.rcMonitor.bottom,
        );

        let work_area_rect = WindowRect::new(
            info.rcWork.left,
            info.rcWork.top,
            info.rcWork.right,
            info.rcWork.bottom,
        );

        let is_primary = (info.dwFlags & sys::MONITORINFOF_PRIMARY) != 0;

        Ok(RawMonitorInfo {
            handle: monitor,
            monitor_rect,
            work_area_rect,
            is_primary,
            device_name,
        })
    }

    fn get_display_device_info(
        &self,
        device_name: &str,
    ) -> Result<Option<RawDisplayDeviceInfo>, DisplayDiscoveryError> {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;

        let name_wide: Vec<u16> = OsStr::new(device_name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let mut dev = sys::DISPLAY_DEVICEW {
            cb: std::mem::size_of::<sys::DISPLAY_DEVICEW>() as sys::DWORD,
            DeviceName: [0; 32],
            DeviceString: [0; 128],
            StateFlags: 0,
            DeviceID: [0; 128],
            DeviceKey: [0; 128],
        };

        // Query monitor device under the adapter name (iDevNum = 0)
        let res = unsafe { sys::EnumDisplayDevicesW(name_wide.as_ptr(), 0, &mut dev, 0) };
        if res == 0 {
            // Some virtual or generic displays may not return a monitor sub-device; query adapter directly
            let mut adapter_dev = sys::DISPLAY_DEVICEW {
                cb: std::mem::size_of::<sys::DISPLAY_DEVICEW>() as sys::DWORD,
                DeviceName: [0; 32],
                DeviceString: [0; 128],
                StateFlags: 0,
                DeviceID: [0; 128],
                DeviceKey: [0; 128],
            };
            let res2 =
                unsafe { sys::EnumDisplayDevicesW(name_wide.as_ptr(), 0, &mut adapter_dev, 0) };
            if res2 == 0 {
                return Ok(None);
            }
            dev = adapter_dev;
        }

        let device_string = {
            let len = dev
                .DeviceString
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(dev.DeviceString.len());
            String::from_utf16_lossy(&dev.DeviceString[..len])
                .trim()
                .to_string()
        };

        let device_id = {
            let len = dev
                .DeviceID
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(dev.DeviceID.len());
            String::from_utf16_lossy(&dev.DeviceID[..len])
                .trim()
                .to_string()
        };

        let is_attached = (dev.StateFlags & sys::DISPLAY_DEVICE_ATTACHED_TO_DESKTOP) != 0;
        let is_mirroring = (dev.StateFlags & sys::DISPLAY_DEVICE_MIRRORING_DRIVER) != 0;

        Ok(Some(RawDisplayDeviceInfo {
            device_string,
            device_id,
            is_attached,
            is_mirroring,
        }))
    }

    fn query_display_config(
        &self,
    ) -> Result<Option<RawDisplayConfigSummary>, DisplayDiscoveryError> {
        let mut path_count: sys::UINT = 0;
        let mut mode_count: sys::UINT = 0;

        let res = unsafe {
            sys::GetDisplayConfigBufferSizes(
                sys::QDC_ONLY_ACTIVE_PATHS,
                &mut path_count,
                &mut mode_count,
            )
        };

        if res != sys::ERROR_SUCCESS || path_count == 0 {
            return Ok(None);
        }

        // Bounded allocation
        let path_len = (path_count as usize).min(64);
        let mode_len = (mode_count as usize).min(128);

        let mut paths: Vec<sys::DISPLAYCONFIG_PATH_INFO> = vec![
            sys::DISPLAYCONFIG_PATH_INFO {
                sourceInfo: sys::DISPLAYCONFIG_PATH_SOURCE_INFO {
                    adapterId: sys::LUID::default(),
                    id: 0,
                    modeInfoIdx: 0,
                    statusFlags: 0,
                },
                targetInfo: sys::DISPLAYCONFIG_PATH_TARGET_INFO {
                    adapterId: sys::LUID::default(),
                    id: 0,
                    modeInfoIdx: 0,
                    outputTechnology: 0,
                    rotation: 0,
                    scaling: 0,
                    scanLineOrdering: 0,
                    targetAvailable: 0,
                    statusFlags: 0,
                },
                flags: 0,
            };
            path_len
        ];

        let mut modes: Vec<sys::DISPLAYCONFIG_MODE_INFO> = vec![
            sys::DISPLAYCONFIG_MODE_INFO {
                infoType: 0,
                id: 0,
                adapterId: sys::LUID::default(),
                dummy: [0; 64],
            };
            mode_len
        ];

        let mut current_topology: sys::UINT = 0;
        let mut actual_paths = path_len as sys::UINT;
        let mut actual_modes = mode_len as sys::UINT;

        let query_res = unsafe {
            sys::QueryDisplayConfig(
                sys::QDC_ONLY_ACTIVE_PATHS,
                &mut actual_paths,
                paths.as_mut_ptr(),
                &mut actual_modes,
                modes.as_mut_ptr(),
                &mut current_topology,
            )
        };

        if query_res != sys::ERROR_SUCCESS {
            return Ok(None);
        }

        paths.truncate(actual_paths as usize);

        let mut sources: HashSet<(i32, u32)> = HashSet::new();
        let mut targets: HashSet<(i32, u32)> = HashSet::new();

        for p in &paths {
            sources.insert((p.sourceInfo.adapterId.HighPart, p.sourceInfo.id));
            targets.insert((p.targetInfo.adapterId.HighPart, p.targetInfo.id));
        }

        let is_clone = sources.len() < targets.len() && targets.len() > 1;

        Ok(Some(RawDisplayConfigSummary {
            active_paths_count: paths.len(),
            unique_sources_count: sources.len(),
            unique_targets_count: targets.len(),
            is_clone,
        }))
    }
}

#[cfg(not(windows))]
impl DesktopDisplayProvider for NativeDesktopDisplayProvider {
    fn enum_display_monitors(&self) -> Result<Vec<NativeMonitorHandle>, DisplayDiscoveryError> {
        Ok(Vec::new())
    }

    fn get_monitor_info(
        &self,
        _monitor: NativeMonitorHandle,
    ) -> Result<RawMonitorInfo, DisplayDiscoveryError> {
        Err(DisplayDiscoveryError::ProviderUnavailable(
            "Windows display APIs are not supported on non-Windows platforms".into(),
        ))
    }

    fn get_display_device_info(
        &self,
        _device_name: &str,
    ) -> Result<Option<RawDisplayDeviceInfo>, DisplayDiscoveryError> {
        Ok(None)
    }

    fn query_display_config(
        &self,
    ) -> Result<Option<RawDisplayConfigSummary>, DisplayDiscoveryError> {
        Ok(None)
    }
}

// -------------------------------------------------------------------------------------------------
// Mock Provider for Unit Testing
// -------------------------------------------------------------------------------------------------

/// Configurable in-memory display record for unit tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockDisplayRecord {
    pub handle: NativeMonitorHandle,
    pub device_name: String,
    pub friendly_name: Option<String>,
    pub hardware_id: Option<String>,
    pub monitor_rect: WindowRect,
    pub work_area_rect: WindowRect,
    pub is_primary: bool,
    pub is_attached: bool,
    pub is_mirroring: bool,
}

/// Fully deterministic in-memory provider for display topology tests.
#[derive(Debug, Default, Clone)]
pub struct MockDesktopDisplayProvider {
    displays: Arc<Mutex<Vec<MockDisplayRecord>>>,
    pub fail_enum_monitors: Arc<Mutex<bool>>,
    pub fail_get_monitor_info: Arc<Mutex<bool>>,
    path_summary: Arc<Mutex<Option<RawDisplayConfigSummary>>>,
}

impl MockDesktopDisplayProvider {
    #[must_use]
    pub fn new() -> Self {
        Self {
            displays: Arc::new(Mutex::new(Vec::new())),
            fail_enum_monitors: Arc::new(Mutex::new(false)),
            fail_get_monitor_info: Arc::new(Mutex::new(false)),
            path_summary: Arc::new(Mutex::new(None)),
        }
    }

    pub fn add_display(&self, record: MockDisplayRecord) {
        self.displays.lock().unwrap().push(record);
    }

    pub fn clear_displays(&self) {
        self.displays.lock().unwrap().clear();
    }

    pub fn set_path_summary(&self, summary: Option<RawDisplayConfigSummary>) {
        *self.path_summary.lock().unwrap() = summary;
    }

    pub fn setup_single_display(&self, width: i32, height: i32) {
        self.clear_displays();
        self.add_display(MockDisplayRecord {
            handle: NativeMonitorHandle(0x1000),
            device_name: "\\\\.\\DISPLAY1".to_string(),
            friendly_name: Some("Primary Monitor".to_string()),
            hardware_id: Some("MONITOR\\PRI0001\\1".to_string()),
            monitor_rect: WindowRect::new(0, 0, width, height),
            work_area_rect: WindowRect::new(0, 0, width, height - 40),
            is_primary: true,
            is_attached: true,
            is_mirroring: false,
        });
    }

    pub fn setup_dual_extended(&self, mon1: WindowRect, mon2: WindowRect) {
        self.clear_displays();
        self.add_display(MockDisplayRecord {
            handle: NativeMonitorHandle(0x1000),
            device_name: "\\\\.\\DISPLAY1".to_string(),
            friendly_name: Some("Primary Display".to_string()),
            hardware_id: Some("MONITOR\\PRI0001\\1".to_string()),
            monitor_rect: mon1,
            work_area_rect: WindowRect::new(mon1.left, mon1.top, mon1.right, mon1.bottom - 40),
            is_primary: true,
            is_attached: true,
            is_mirroring: false,
        });

        self.add_display(MockDisplayRecord {
            handle: NativeMonitorHandle(0x2000),
            device_name: "\\\\.\\DISPLAY2".to_string(),
            friendly_name: Some("Secondary Display".to_string()),
            hardware_id: Some("MONITOR\\SEC0002\\2".to_string()),
            monitor_rect: mon2,
            work_area_rect: WindowRect::new(mon2.left, mon2.top, mon2.right, mon2.bottom - 40),
            is_primary: false,
            is_attached: true,
            is_mirroring: false,
        });
    }

    pub fn setup_cloned_displays(&self, width: i32, height: i32) {
        self.clear_displays();
        let bounds = WindowRect::new(0, 0, width, height);
        self.add_display(MockDisplayRecord {
            handle: NativeMonitorHandle(0x1000),
            device_name: "\\\\.\\DISPLAY1".to_string(),
            friendly_name: Some("Main Display".to_string()),
            hardware_id: Some("MONITOR\\PRI0001\\1".to_string()),
            monitor_rect: bounds,
            work_area_rect: bounds,
            is_primary: true,
            is_attached: true,
            is_mirroring: false,
        });

        self.add_display(MockDisplayRecord {
            handle: NativeMonitorHandle(0x2000),
            device_name: "\\\\.\\DISPLAY2".to_string(),
            friendly_name: Some("Cloned Projector".to_string()),
            hardware_id: Some("MONITOR\\PROJ001\\2".to_string()),
            monitor_rect: bounds,
            work_area_rect: bounds,
            is_primary: false,
            is_attached: true,
            is_mirroring: false,
        });

        self.set_path_summary(Some(RawDisplayConfigSummary {
            active_paths_count: 2,
            unique_sources_count: 1,
            unique_targets_count: 2,
            is_clone: true,
        }));
    }
}

impl DesktopDisplayProvider for MockDesktopDisplayProvider {
    fn enum_display_monitors(&self) -> Result<Vec<NativeMonitorHandle>, DisplayDiscoveryError> {
        if *self.fail_enum_monitors.lock().unwrap() {
            return Err(DisplayDiscoveryError::NativeApiFailure {
                function: "EnumDisplayMonitors".into(),
                code: 5,
                message: "Mock simulated EnumDisplayMonitors failure".into(),
            });
        }
        let list = self
            .displays
            .lock()
            .unwrap()
            .iter()
            .map(|d| d.handle)
            .collect();
        Ok(list)
    }

    fn get_monitor_info(
        &self,
        monitor: NativeMonitorHandle,
    ) -> Result<RawMonitorInfo, DisplayDiscoveryError> {
        if *self.fail_get_monitor_info.lock().unwrap() {
            return Err(DisplayDiscoveryError::NativeApiFailure {
                function: "GetMonitorInfoW".into(),
                code: 5,
                message: "Mock simulated GetMonitorInfoW failure".into(),
            });
        }

        let map = self.displays.lock().unwrap();
        let record = map.iter().find(|d| d.handle == monitor).ok_or_else(|| {
            DisplayDiscoveryError::NativeApiFailure {
                function: "GetMonitorInfoW".into(),
                code: 87, // ERROR_INVALID_PARAMETER
                message: format!("Unknown monitor handle: {monitor}"),
            }
        })?;

        Ok(RawMonitorInfo {
            handle: record.handle,
            monitor_rect: record.monitor_rect,
            work_area_rect: record.work_area_rect,
            is_primary: record.is_primary,
            device_name: record.device_name.clone(),
        })
    }

    fn get_display_device_info(
        &self,
        device_name: &str,
    ) -> Result<Option<RawDisplayDeviceInfo>, DisplayDiscoveryError> {
        let map = self.displays.lock().unwrap();
        let record = map.iter().find(|d| d.device_name == device_name);
        match record {
            Some(r) => Ok(Some(RawDisplayDeviceInfo {
                device_string: r.friendly_name.clone().unwrap_or_default(),
                device_id: r.hardware_id.clone().unwrap_or_default(),
                is_attached: r.is_attached,
                is_mirroring: r.is_mirroring,
            })),
            None => Ok(None),
        }
    }

    fn query_display_config(
        &self,
    ) -> Result<Option<RawDisplayConfigSummary>, DisplayDiscoveryError> {
        Ok(self.path_summary.lock().unwrap().clone())
    }
}

// -------------------------------------------------------------------------------------------------
// Display Topology Snapshot Manager
// -------------------------------------------------------------------------------------------------

/// Coordinates display enumeration, stable identity resolution, topology modeling,
/// and snapshot publishing with anti-thrashing and change detection.
pub struct DisplaySnapshotManager {
    provider: Arc<dyn DesktopDisplayProvider>,
    current_snapshot: Arc<DisplayTopologySnapshot>,
    known_identities: Vec<(String, DisplayId)>,
    last_refresh_timestamp: u64,
}

impl DisplaySnapshotManager {
    /// Creates a new `DisplaySnapshotManager` backed by the given display provider.
    #[must_use]
    pub fn new(provider: Arc<dyn DesktopDisplayProvider>) -> Self {
        Self {
            provider,
            current_snapshot: Arc::new(DisplayTopologySnapshot::empty()),
            known_identities: Vec::new(),
            last_refresh_timestamp: 0,
        }
    }

    /// Returns the currently published display topology snapshot.
    #[must_use]
    pub fn current_snapshot(&self) -> Arc<DisplayTopologySnapshot> {
        self.current_snapshot.clone()
    }

    /// Refreshes the display inventory, produces a new snapshot, and calculates the change delta.
    pub fn refresh(
        &mut self,
    ) -> Result<(Arc<DisplayTopologySnapshot>, DisplayChangeDelta), DisplayDiscoveryError> {
        let now_millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let next_generation = self.current_snapshot.generation + 1;
        let mut warnings = Vec::new();

        // 1. Enumerate active logical desktop monitors
        let monitor_handles = self.provider.enum_display_monitors()?;
        if monitor_handles.is_empty() {
            return Err(DisplayDiscoveryError::NoActiveDisplays);
        }

        let mut descriptors: Vec<DisplayDescriptor> = Vec::new();
        let mut primary_id_opt: Option<DisplayId> = None;
        let mut virtual_bounds = WindowRect::default();

        let path_summary = self.provider.query_display_config().unwrap_or_else(|e| {
            warnings.push(format!("QueryDisplayConfig unavailable: {e}"));
            None
        });

        // 2. Query each monitor
        for handle in monitor_handles {
            let raw_monitor = self.provider.get_monitor_info(handle)?;

            // Validate dimensions
            let width = raw_monitor.monitor_rect.width();
            let height = raw_monitor.monitor_rect.height();
            if width <= 0 || height <= 0 {
                return Err(DisplayDiscoveryError::InvalidGeometry {
                    device_name: raw_monitor.device_name,
                    bounds: raw_monitor.monitor_rect,
                    reason: "Width and height must be strictly positive".to_string(),
                });
            }

            let geometry = DisplayGeometry {
                monitor_rect: raw_monitor.monitor_rect,
                work_area_rect: raw_monitor.work_area_rect,
                width,
                height,
                scale_factor_percent: None,
            };

            if !geometry.is_valid() {
                warnings.push(format!(
                    "Monitor `{}` has irregular work-area bounds {:?}",
                    raw_monitor.device_name, raw_monitor.work_area_rect
                ));
            }

            // Query device info
            let dev_info = self
                .provider
                .get_display_device_info(&raw_monitor.device_name)?
                .unwrap_or(RawDisplayDeviceInfo {
                    device_string: String::new(),
                    device_id: String::new(),
                    is_attached: true,
                    is_mirroring: false,
                });

            let friendly_name = if !dev_info.device_string.is_empty() {
                Some(dev_info.device_string)
            } else {
                None
            };

            let hardware_id = if !dev_info.device_id.is_empty() {
                Some(dev_info.device_id)
            } else {
                None
            };

            // 3. Resolve stable application-level display identity
            let display_id = self.resolve_display_id(
                &raw_monitor.device_name,
                hardware_id.as_deref(),
                raw_monitor.is_primary,
            );

            if raw_monitor.is_primary {
                primary_id_opt = Some(display_id.clone());
            }

            // Accumulate virtual desktop bounds
            virtual_bounds = virtual_bounds.union(&raw_monitor.monitor_rect);

            descriptors.push(DisplayDescriptor {
                id: display_id,
                native_handle: handle,
                device_name: raw_monitor.device_name,
                friendly_name,
                hardware_id,
                is_primary: raw_monitor.is_primary,
                geometry,
                is_attached: dev_info.is_attached,
                is_mirroring: dev_info.is_mirroring,
                snapshot_generation: next_generation,
            });
        }

        // Fallback primary identification: if no monitor reported is_primary, default to monitor at (0, 0)
        if primary_id_opt.is_none() {
            if let Some(origin_mon) = descriptors
                .iter_mut()
                .find(|d| d.geometry.monitor_rect.left == 0 && d.geometry.monitor_rect.top == 0)
            {
                origin_mon.is_primary = true;
                primary_id_opt = Some(origin_mon.id.clone());
                warnings.push("Primary display inferred from (0, 0) origin coordinate".to_string());
            } else if let Some(first) = descriptors.first_mut() {
                first.is_primary = true;
                primary_id_opt = Some(first.id.clone());
                warnings.push("Primary display defaulted to first enumerated monitor".to_string());
            }
        }

        // 4. Classify topology arrangement
        let topology_type = Self::classify_topology(&descriptors, path_summary.as_ref());

        let new_snapshot = Arc::new(DisplayTopologySnapshot {
            generation: next_generation,
            captured_at_millis: now_millis,
            displays: descriptors,
            primary_display_id: primary_id_opt,
            virtual_desktop_bounds: virtual_bounds,
            topology_type,
            is_valid: true,
            diagnostic_warnings: warnings,
        });

        let delta = self.current_snapshot.detect_change(&new_snapshot);

        self.current_snapshot = new_snapshot.clone();
        self.last_refresh_timestamp = now_millis;

        Ok((new_snapshot, delta))
    }

    /// Resolves or matches a stable application-level `DisplayId`.
    fn resolve_display_id(
        &mut self,
        device_name: &str,
        hardware_id: Option<&str>,
        _is_primary: bool,
    ) -> DisplayId {
        let key = if let Some(hw) = hardware_id {
            let sanitized = hw.replace(['\\', '#'], "_");
            format!("{}_{}", device_name, sanitized)
        } else {
            device_name.to_string()
        };

        if let Some((_, id)) = self.known_identities.iter().find(|(k, _)| k == &key) {
            return id.clone();
        }

        // Create a new deterministic, sanitized identity
        let slug = key
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
            .collect::<String>();
        let display_id = DisplayId(format!("disp_{slug}"));

        self.known_identities.push((key, display_id.clone()));
        display_id
    }

    /// Classifies the topology of the enumerated displays.
    fn classify_topology(
        displays: &[DisplayDescriptor],
        path_summary: Option<&RawDisplayConfigSummary>,
    ) -> DisplayTopologyType {
        if displays.is_empty() {
            return DisplayTopologyType::UnknownOrUnsupported;
        }

        if displays.len() == 1 {
            return DisplayTopologyType::SingleDisplay;
        }

        // Check path summary if available
        if let Some(summary) = path_summary {
            if summary.is_clone {
                return DisplayTopologyType::CloneOrMirrored;
            }
        }

        // Check if all displays share the exact same coordinates (mirroring)
        let first_rect = displays[0].geometry.monitor_rect;
        let all_identical = displays[1..]
            .iter()
            .all(|d| d.geometry.monitor_rect == first_rect);
        if all_identical {
            return DisplayTopologyType::CloneOrMirrored;
        }

        // Check for non-overlapping extended desktop
        let mut has_overlap = false;
        for i in 0..displays.len() {
            for j in (i + 1)..displays.len() {
                if displays[i]
                    .geometry
                    .monitor_rect
                    .intersects(&displays[j].geometry.monitor_rect)
                {
                    has_overlap = true;
                    break;
                }
            }
            if has_overlap {
                break;
            }
        }

        if !has_overlap {
            DisplayTopologyType::ExtendedDesktop
        } else {
            DisplayTopologyType::ComplexOrMixed
        }
    }
}
