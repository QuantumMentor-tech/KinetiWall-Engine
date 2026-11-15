//! Windows native desktop window provider and synthetic mock provider for deterministic testing.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::types::{DiscoveryError, NativeWindowHandle, WindowRect};

/// Abstraction for querying Win32 window hierarchy, classes, and ownership.
///
/// Decouples the discovery logic from direct OS FFI, enabling unit testing of all
/// shell topology permutations, transient states, and failure modes on any platform.
pub trait DesktopWindowProvider: Send + Sync {
    /// Finds a top-level window by class name and/or window name.
    fn find_window(
        &self,
        class_name: Option<&str>,
        window_name: Option<&str>,
    ) -> Option<NativeWindowHandle>;

    /// Finds a child window matching the given criteria.
    fn find_window_ex(
        &self,
        parent: NativeWindowHandle,
        child_after: Option<NativeWindowHandle>,
        class_name: Option<&str>,
        window_name: Option<&str>,
    ) -> Option<NativeWindowHandle>;

    /// Enumerates all top-level windows in Z-order (top to bottom).
    fn enum_windows(&self) -> Vec<NativeWindowHandle>;

    /// Enumerates all child windows belonging to the given parent.
    fn enum_child_windows(&self, parent: NativeWindowHandle) -> Vec<NativeWindowHandle>;

    /// Retrieves the registered window class name for a window.
    fn get_class_name(&self, hwnd: NativeWindowHandle) -> Result<String, DiscoveryError>;

    /// Retrieves the window text / caption for a window.
    fn get_window_text(&self, hwnd: NativeWindowHandle) -> Result<String, DiscoveryError>;

    /// Retrieves the thread ID and process ID owning the window: `(thread_id, process_id)`.
    fn get_window_thread_process_id(&self, hwnd: NativeWindowHandle) -> (u32, u32);

    /// Determines whether the window handle designates an existing window.
    fn is_window(&self, hwnd: NativeWindowHandle) -> bool;

    /// Determines the visibility state of the window.
    fn is_window_visible(&self, hwnd: NativeWindowHandle) -> bool;

    /// Retrieves the screen-coordinate bounding rectangle of the window.
    fn get_window_rect(&self, hwnd: NativeWindowHandle) -> Result<WindowRect, DiscoveryError>;

    /// Sends a window message with a bounded timeout (`SendMessageTimeoutW`).
    fn send_message_timeout(
        &self,
        hwnd: NativeWindowHandle,
        msg: u32,
        wparam: usize,
        lparam: isize,
        timeout_ms: u32,
    ) -> Result<usize, DiscoveryError>;

    /// Retrieves the parent window handle, if any.
    fn get_parent(&self, hwnd: NativeWindowHandle) -> Option<NativeWindowHandle>;

    /// Sets the parent window handle. Returns the previous parent on success.
    fn set_parent(
        &self,
        hwnd: NativeWindowHandle,
        new_parent: Option<NativeWindowHandle>,
    ) -> Result<Option<NativeWindowHandle>, DiscoveryError>;

    /// Retrieves window long integer (GWL_STYLE, GWL_EXSTYLE, etc.).
    fn get_window_long(
        &self,
        hwnd: NativeWindowHandle,
        index: i32,
    ) -> Result<isize, DiscoveryError>;

    /// Sets window long integer (GWL_STYLE, GWL_EXSTYLE, etc.). Returns the previous value.
    fn set_window_long(
        &self,
        hwnd: NativeWindowHandle,
        index: i32,
        new_value: isize,
    ) -> Result<isize, DiscoveryError>;

    /// Positions, sizes, and changes Z-order/flags for a window (`SetWindowPos`).
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    fn set_window_pos(
        &self,
        hwnd: NativeWindowHandle,
        insert_after: Option<NativeWindowHandle>,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        flags: u32,
    ) -> Result<(), DiscoveryError>;

    /// Queries the DPI awareness context of a window, if supported by the OS.
    fn get_window_dpi_awareness_context(&self, hwnd: NativeWindowHandle) -> Option<isize>;

    /// Determines whether two DPI awareness contexts are equivalent.
    fn are_dpi_awareness_contexts_equal(&self, ctx1: isize, ctx2: isize) -> bool;

    /// Creates a dedicated test window for verifying attachment and rollback.
    fn create_test_window(
        &self,
        title: &str,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> Result<NativeWindowHandle, DiscoveryError>;

    /// Destroys a window created by the current process.
    fn destroy_window(&self, hwnd: NativeWindowHandle) -> Result<(), DiscoveryError>;

    /// Retrieves the current foreground window handle, if any.
    fn get_foreground_window(&self) -> Option<NativeWindowHandle>;

    /// Determines whether the window is currently minimized (iconic).
    fn is_iconic(&self, hwnd: NativeWindowHandle) -> bool;

    /// Determines whether the window is cloaked by DWM.
    fn is_window_cloaked(&self, hwnd: NativeWindowHandle) -> bool;

    /// Retrieves the extended frame bounds from DWM, excluding invisible drop-shadow resize padding.
    fn get_extended_frame_bounds(
        &self,
        hwnd: NativeWindowHandle,
    ) -> Result<WindowRect, DiscoveryError>;
}

// -------------------------------------------------------------------------------------------------
// Native Win32 Provider Implementation
// -------------------------------------------------------------------------------------------------

#[cfg(windows)]
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
mod sys {
    use std::ffi::c_void;

    pub type HWND = *mut c_void;
    pub type BOOL = i32;
    pub type DWORD = u32;
    pub type LRESULT = isize;
    pub type LPARAM = isize;
    pub type WPARAM = usize;
    pub type UINT = u32;

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    pub struct RECT {
        pub left: i32,
        pub top: i32,
        pub right: i32,
        pub bottom: i32,
    }

    pub type WNDENUMPROC = unsafe extern "system" fn(HWND, LPARAM) -> BOOL;

    pub const SMTO_NORMAL: UINT = 0x0000;
    pub const SMTO_ABORTIFHUNG: UINT = 0x0002;

    #[link(name = "kernel32")]
    extern "system" {
        pub fn GetLastError() -> DWORD;
        pub fn SetLastError(dwErrCode: DWORD);
        pub fn GetModuleHandleA(lpModuleName: *const u8) -> *mut c_void;
        pub fn GetProcAddress(hModule: *mut c_void, lpProcName: *const u8) -> *mut c_void;
    }

    #[link(name = "user32")]
    extern "system" {
        pub fn FindWindowW(lpClassName: *const u16, lpWindowName: *const u16) -> HWND;
        pub fn FindWindowExW(
            hWndParent: HWND,
            hWndChildAfter: HWND,
            lpszClass: *const u16,
            lpszWindow: *const u16,
        ) -> HWND;
        pub fn EnumWindows(lpEnumFunc: WNDENUMPROC, lParam: LPARAM) -> BOOL;
        pub fn EnumChildWindows(hWndParent: HWND, lpEnumFunc: WNDENUMPROC, lParam: LPARAM) -> BOOL;
        pub fn GetClassNameW(hWnd: HWND, lpClassName: *mut u16, nMaxCount: i32) -> i32;
        pub fn GetWindowTextW(hWnd: HWND, lpString: *mut u16, nMaxCount: i32) -> i32;
        pub fn GetWindowThreadProcessId(hWnd: HWND, lpdwProcessId: *mut DWORD) -> DWORD;
        pub fn IsWindow(hWnd: HWND) -> BOOL;
        pub fn IsWindowVisible(hWnd: HWND) -> BOOL;
        pub fn GetWindowRect(hWnd: HWND, lpRect: *mut RECT) -> BOOL;
        pub fn SendMessageTimeoutW(
            hWnd: HWND,
            Msg: UINT,
            wParam: WPARAM,
            lParam: LPARAM,
            fuFlags: UINT,
            uTimeout: UINT,
            lpdwResult: *mut usize,
        ) -> LRESULT;
        pub fn GetParent(hWnd: HWND) -> HWND;
        pub fn SetParent(hWndChild: HWND, hWndNewParent: HWND) -> HWND;
        pub fn SetWindowPos(
            hWnd: HWND,
            hWndInsertAfter: HWND,
            X: i32,
            Y: i32,
            cx: i32,
            cy: i32,
            uFlags: UINT,
        ) -> BOOL;
        #[cfg(target_pointer_width = "64")]
        pub fn GetWindowLongPtrW(hWnd: HWND, nIndex: i32) -> isize;
        #[cfg(target_pointer_width = "64")]
        pub fn SetWindowLongPtrW(hWnd: HWND, nIndex: i32, dwNewLong: isize) -> isize;
        #[cfg(target_pointer_width = "32")]
        pub fn GetWindowLongW(hWnd: HWND, nIndex: i32) -> i32;
        #[cfg(target_pointer_width = "32")]
        pub fn SetWindowLongW(hWnd: HWND, nIndex: i32, dwNewLong: i32) -> i32;
        pub fn CreateWindowExW(
            dwExStyle: DWORD,
            lpClassName: *const u16,
            lpWindowName: *const u16,
            dwStyle: DWORD,
            X: i32,
            Y: i32,
            nWidth: i32,
            nHeight: i32,
            hWndParent: HWND,
            hMenu: *mut c_void,
            hInstance: *mut c_void,
            lpParam: *mut c_void,
        ) -> HWND;
        pub fn DestroyWindow(hWnd: HWND) -> BOOL;
        pub fn GetForegroundWindow() -> HWND;
        pub fn IsIconic(hWnd: HWND) -> BOOL;
    }

    #[link(name = "dwmapi")]
    extern "system" {
        pub fn DwmGetWindowAttribute(
            hWnd: HWND,
            dwAttribute: DWORD,
            pvAttribute: *mut c_void,
            cbAttribute: DWORD,
        ) -> i32;
    }

    pub const DWMWA_EXTENDED_FRAME_BOUNDS: DWORD = 9;
    pub const DWMWA_CLOAKED: DWORD = 14;
}

/// Real Windows API implementation of `DesktopWindowProvider`.
pub struct NativeDesktopWindowProvider;

impl NativeDesktopWindowProvider {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for NativeDesktopWindowProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(windows)]
fn to_wide_null(s: Option<&str>) -> Option<Vec<u16>> {
    s.map(|val| val.encode_utf16().chain(std::iter::once(0)).collect())
}

#[cfg(windows)]
unsafe extern "system" fn enum_windows_callback(hwnd: sys::HWND, lparam: sys::LPARAM) -> sys::BOOL {
    let list = &mut *(lparam as *mut Vec<NativeWindowHandle>);
    list.push(NativeWindowHandle(hwnd as usize));
    1 // TRUE to continue
}

#[cfg(windows)]
impl DesktopWindowProvider for NativeDesktopWindowProvider {
    fn find_window(
        &self,
        class_name: Option<&str>,
        window_name: Option<&str>,
    ) -> Option<NativeWindowHandle> {
        let class_w = to_wide_null(class_name);
        let win_w = to_wide_null(window_name);

        let class_ptr = class_w.as_ref().map_or(std::ptr::null(), |v| v.as_ptr());
        let win_ptr = win_w.as_ref().map_or(std::ptr::null(), |v| v.as_ptr());

        let res = unsafe { sys::FindWindowW(class_ptr, win_ptr) };
        if res.is_null() {
            None
        } else {
            Some(NativeWindowHandle(res as usize))
        }
    }

    fn find_window_ex(
        &self,
        parent: NativeWindowHandle,
        child_after: Option<NativeWindowHandle>,
        class_name: Option<&str>,
        window_name: Option<&str>,
    ) -> Option<NativeWindowHandle> {
        let class_w = to_wide_null(class_name);
        let win_w = to_wide_null(window_name);

        let class_ptr = class_w.as_ref().map_or(std::ptr::null(), |v| v.as_ptr());
        let win_ptr = win_w.as_ref().map_or(std::ptr::null(), |v| v.as_ptr());
        let parent_ptr = parent.0 as sys::HWND;
        let after_ptr = child_after.map_or(std::ptr::null_mut(), |h| h.0 as sys::HWND);

        let res = unsafe { sys::FindWindowExW(parent_ptr, after_ptr, class_ptr, win_ptr) };
        if res.is_null() {
            None
        } else {
            Some(NativeWindowHandle(res as usize))
        }
    }

    fn enum_windows(&self) -> Vec<NativeWindowHandle> {
        let mut windows = Vec::new();
        let ptr = &mut windows as *mut Vec<NativeWindowHandle> as sys::LPARAM;
        unsafe {
            sys::EnumWindows(enum_windows_callback, ptr);
        }
        windows
    }

    fn enum_child_windows(&self, parent: NativeWindowHandle) -> Vec<NativeWindowHandle> {
        let mut windows = Vec::new();
        let ptr = &mut windows as *mut Vec<NativeWindowHandle> as sys::LPARAM;
        unsafe {
            sys::EnumChildWindows(parent.0 as sys::HWND, enum_windows_callback, ptr);
        }
        windows
    }

    fn get_class_name(&self, hwnd: NativeWindowHandle) -> Result<String, DiscoveryError> {
        if hwnd.is_null() {
            return Err(DiscoveryError::InvalidHandle(hwnd));
        }
        let mut buf = [0u16; 256];
        let len =
            unsafe { sys::GetClassNameW(hwnd.0 as sys::HWND, buf.as_mut_ptr(), buf.len() as i32) };
        if len > 0 {
            Ok(String::from_utf16_lossy(&buf[..len as usize]))
        } else {
            let code = unsafe { sys::GetLastError() };
            Err(DiscoveryError::NativeApiFailure {
                function: "GetClassNameW".into(),
                code,
                message: format!("Failed to get class name for {hwnd}"),
            })
        }
    }

    fn get_window_text(&self, hwnd: NativeWindowHandle) -> Result<String, DiscoveryError> {
        if hwnd.is_null() {
            return Err(DiscoveryError::InvalidHandle(hwnd));
        }
        let mut buf = [0u16; 256];
        let len =
            unsafe { sys::GetWindowTextW(hwnd.0 as sys::HWND, buf.as_mut_ptr(), buf.len() as i32) };
        if len >= 0 {
            Ok(String::from_utf16_lossy(&buf[..len as usize]))
        } else {
            let code = unsafe { sys::GetLastError() };
            Err(DiscoveryError::NativeApiFailure {
                function: "GetWindowTextW".into(),
                code,
                message: format!("Failed to get window text for {hwnd}"),
            })
        }
    }

    fn get_window_thread_process_id(&self, hwnd: NativeWindowHandle) -> (u32, u32) {
        if hwnd.is_null() {
            return (0, 0);
        }
        let mut pid: sys::DWORD = 0;
        let tid = unsafe { sys::GetWindowThreadProcessId(hwnd.0 as sys::HWND, &mut pid) };
        (tid, pid)
    }

    fn is_window(&self, hwnd: NativeWindowHandle) -> bool {
        if hwnd.is_null() {
            return false;
        }
        unsafe { sys::IsWindow(hwnd.0 as sys::HWND) != 0 }
    }

    fn is_window_visible(&self, hwnd: NativeWindowHandle) -> bool {
        if hwnd.is_null() {
            return false;
        }
        unsafe { sys::IsWindowVisible(hwnd.0 as sys::HWND) != 0 }
    }

    fn get_window_rect(&self, hwnd: NativeWindowHandle) -> Result<WindowRect, DiscoveryError> {
        if hwnd.is_null() {
            return Err(DiscoveryError::InvalidHandle(hwnd));
        }
        let mut rect = sys::RECT::default();
        let ok = unsafe { sys::GetWindowRect(hwnd.0 as sys::HWND, &mut rect) != 0 };
        if ok {
            Ok(WindowRect::new(
                rect.left,
                rect.top,
                rect.right,
                rect.bottom,
            ))
        } else {
            let code = unsafe { sys::GetLastError() };
            Err(DiscoveryError::NativeApiFailure {
                function: "GetWindowRect".into(),
                code,
                message: format!("Failed to get window rect for {hwnd}"),
            })
        }
    }

    fn send_message_timeout(
        &self,
        hwnd: NativeWindowHandle,
        msg: u32,
        wparam: usize,
        lparam: isize,
        timeout_ms: u32,
    ) -> Result<usize, DiscoveryError> {
        if hwnd.is_null() {
            return Err(DiscoveryError::InvalidHandle(hwnd));
        }
        let mut result: usize = 0;
        let flags = sys::SMTO_NORMAL | sys::SMTO_ABORTIFHUNG;
        let res = unsafe {
            sys::SendMessageTimeoutW(
                hwnd.0 as sys::HWND,
                msg,
                wparam,
                lparam,
                flags,
                timeout_ms,
                &mut result,
            )
        };
        if res != 0 {
            Ok(result)
        } else {
            Err(DiscoveryError::ActiveProbeTimedOut(timeout_ms))
        }
    }

    fn get_parent(&self, hwnd: NativeWindowHandle) -> Option<NativeWindowHandle> {
        if hwnd.is_null() {
            return None;
        }
        let raw = hwnd.0 as sys::HWND;
        let parent = unsafe { sys::GetParent(raw) };
        if parent.is_null() {
            None
        } else {
            Some(NativeWindowHandle(parent as usize))
        }
    }

    fn set_parent(
        &self,
        hwnd: NativeWindowHandle,
        new_parent: Option<NativeWindowHandle>,
    ) -> Result<Option<NativeWindowHandle>, DiscoveryError> {
        if hwnd.is_null() {
            return Err(DiscoveryError::InvalidHandle(hwnd));
        }
        let raw_child = hwnd.0 as sys::HWND;
        let raw_parent = new_parent
            .map(|p| p.0 as sys::HWND)
            .unwrap_or(std::ptr::null_mut());
        unsafe {
            sys::SetLastError(0);
            let prev = sys::SetParent(raw_child, raw_parent);
            if prev.is_null() {
                let err = sys::GetLastError();
                if err != 0 {
                    return Err(DiscoveryError::NativeApiFailure {
                        function: "SetParent".into(),
                        code: err,
                        message: format!(
                            "SetParent failed for child {hwnd} to parent {new_parent:?}"
                        ),
                    });
                }
                Ok(None)
            } else {
                Ok(Some(NativeWindowHandle(prev as usize)))
            }
        }
    }

    fn get_window_long(
        &self,
        hwnd: NativeWindowHandle,
        index: i32,
    ) -> Result<isize, DiscoveryError> {
        if hwnd.is_null() {
            return Err(DiscoveryError::InvalidHandle(hwnd));
        }
        let raw = hwnd.0 as sys::HWND;
        unsafe {
            sys::SetLastError(0);
            #[cfg(target_pointer_width = "64")]
            let res = sys::GetWindowLongPtrW(raw, index);
            #[cfg(target_pointer_width = "32")]
            let res = sys::GetWindowLongW(raw, index) as isize;

            if res == 0 {
                let err = sys::GetLastError();
                if err != 0 {
                    return Err(DiscoveryError::NativeApiFailure {
                        function: "GetWindowLongPtrW".into(),
                        code: err,
                        message: format!("GetWindowLongPtrW failed for {hwnd} at index {index}"),
                    });
                }
            }
            Ok(res)
        }
    }

    fn set_window_long(
        &self,
        hwnd: NativeWindowHandle,
        index: i32,
        new_value: isize,
    ) -> Result<isize, DiscoveryError> {
        if hwnd.is_null() {
            return Err(DiscoveryError::InvalidHandle(hwnd));
        }
        let raw = hwnd.0 as sys::HWND;
        unsafe {
            sys::SetLastError(0);
            #[cfg(target_pointer_width = "64")]
            let old = sys::SetWindowLongPtrW(raw, index, new_value);
            #[cfg(target_pointer_width = "32")]
            let old = sys::SetWindowLongW(raw, index, new_value as i32) as isize;

            if old == 0 {
                let err = sys::GetLastError();
                if err != 0 {
                    return Err(DiscoveryError::NativeApiFailure {
                        function: "SetWindowLongPtrW".into(),
                        code: err,
                        message: format!("SetWindowLongPtrW failed for {hwnd} at index {index}"),
                    });
                }
            }
            Ok(old)
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    fn set_window_pos(
        &self,
        hwnd: NativeWindowHandle,
        insert_after: Option<NativeWindowHandle>,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        flags: u32,
    ) -> Result<(), DiscoveryError> {
        if hwnd.is_null() {
            return Err(DiscoveryError::InvalidHandle(hwnd));
        }
        let raw_hwnd = hwnd.0 as sys::HWND;
        let raw_insert = insert_after
            .map(|h| h.0 as sys::HWND)
            .unwrap_or(std::ptr::null_mut());
        let res = unsafe { sys::SetWindowPos(raw_hwnd, raw_insert, x, y, cx, cy, flags) };
        if res == 0 {
            let err = unsafe { sys::GetLastError() };
            return Err(DiscoveryError::NativeApiFailure {
                function: "SetWindowPos".into(),
                code: err,
                message: format!("SetWindowPos failed for {hwnd}"),
            });
        }
        Ok(())
    }

    fn get_window_dpi_awareness_context(&self, hwnd: NativeWindowHandle) -> Option<isize> {
        if hwnd.is_null() {
            return None;
        }
        unsafe {
            let user32 = sys::GetModuleHandleA(c"user32.dll".as_ptr() as *const u8);
            if user32.is_null() {
                return None;
            }
            let proc = sys::GetProcAddress(
                user32,
                c"GetWindowDpiAwarenessContext".as_ptr() as *const u8,
            );
            if proc.is_null() {
                return None;
            }
            type Pfn = unsafe extern "system" fn(sys::HWND) -> *mut std::ffi::c_void;
            let func: Pfn = std::mem::transmute(proc);
            let ctx = func(hwnd.0 as sys::HWND);
            if ctx.is_null() {
                None
            } else {
                Some(ctx as isize)
            }
        }
    }

    fn are_dpi_awareness_contexts_equal(&self, ctx1: isize, ctx2: isize) -> bool {
        if ctx1 == ctx2 {
            return true;
        }
        unsafe {
            let user32 = sys::GetModuleHandleA(c"user32.dll".as_ptr() as *const u8);
            if user32.is_null() {
                return false;
            }
            let proc = sys::GetProcAddress(
                user32,
                c"AreDpiAwarenessContextsEqual".as_ptr() as *const u8,
            );
            if proc.is_null() {
                return false;
            }
            type Pfn = unsafe extern "system" fn(
                *mut std::ffi::c_void,
                *mut std::ffi::c_void,
            ) -> sys::BOOL;
            let func: Pfn = std::mem::transmute(proc);
            func(ctx1 as *mut std::ffi::c_void, ctx2 as *mut std::ffi::c_void) != 0
        }
    }

    fn create_test_window(
        &self,
        title: &str,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> Result<NativeWindowHandle, DiscoveryError> {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;

        let class_wide: Vec<u16> = OsStr::new("STATIC")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let title_wide: Vec<u16> = OsStr::new(title)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // WS_POPUP | WS_VISIBLE
        let style: u32 = 0x80000000 | 0x10000000;
        let hwnd = unsafe {
            sys::CreateWindowExW(
                0,
                class_wide.as_ptr(),
                title_wide.as_ptr(),
                style,
                x,
                y,
                width,
                height,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if hwnd.is_null() {
            let err = unsafe { sys::GetLastError() };
            Err(DiscoveryError::NativeApiFailure {
                function: "CreateWindowExW".into(),
                code: err,
                message: "Failed to create test player window".into(),
            })
        } else {
            Ok(NativeWindowHandle(hwnd as usize))
        }
    }

    fn destroy_window(&self, hwnd: NativeWindowHandle) -> Result<(), DiscoveryError> {
        if hwnd.is_null() {
            return Ok(());
        }
        let res = unsafe { sys::DestroyWindow(hwnd.0 as sys::HWND) };
        if res == 0 {
            let err = unsafe { sys::GetLastError() };
            Err(DiscoveryError::NativeApiFailure {
                function: "DestroyWindow".into(),
                code: err,
                message: format!("DestroyWindow failed for {hwnd}"),
            })
        } else {
            Ok(())
        }
    }

    fn get_foreground_window(&self) -> Option<NativeWindowHandle> {
        let hwnd = unsafe { sys::GetForegroundWindow() };
        if hwnd.is_null() {
            None
        } else {
            Some(NativeWindowHandle(hwnd as usize))
        }
    }

    fn is_iconic(&self, hwnd: NativeWindowHandle) -> bool {
        if hwnd.is_null() {
            return false;
        }
        unsafe { sys::IsIconic(hwnd.0 as sys::HWND) != 0 }
    }

    fn is_window_cloaked(&self, hwnd: NativeWindowHandle) -> bool {
        if hwnd.is_null() {
            return false;
        }
        let mut cloaked: sys::DWORD = 0;
        let res = unsafe {
            sys::DwmGetWindowAttribute(
                hwnd.0 as sys::HWND,
                sys::DWMWA_CLOAKED,
                &mut cloaked as *mut _ as *mut std::ffi::c_void,
                std::mem::size_of::<sys::DWORD>() as sys::DWORD,
            )
        };
        res == 0 && cloaked != 0
    }

    fn get_extended_frame_bounds(
        &self,
        hwnd: NativeWindowHandle,
    ) -> Result<WindowRect, DiscoveryError> {
        if hwnd.is_null() {
            return Err(DiscoveryError::InvalidHandle(hwnd));
        }
        let mut rect = sys::RECT::default();
        let res = unsafe {
            sys::DwmGetWindowAttribute(
                hwnd.0 as sys::HWND,
                sys::DWMWA_EXTENDED_FRAME_BOUNDS,
                &mut rect as *mut _ as *mut std::ffi::c_void,
                std::mem::size_of::<sys::RECT>() as sys::DWORD,
            )
        };
        if res == 0 {
            Ok(WindowRect::new(rect.left, rect.top, rect.right, rect.bottom))
        } else {
            self.get_window_rect(hwnd)
        }
    }
}

#[cfg(not(windows))]
impl DesktopWindowProvider for NativeDesktopWindowProvider {
    fn find_window(
        &self,
        _class_name: Option<&str>,
        _window_name: Option<&str>,
    ) -> Option<NativeWindowHandle> {
        None
    }
    fn find_window_ex(
        &self,
        _parent: NativeWindowHandle,
        _child_after: Option<NativeWindowHandle>,
        _class_name: Option<&str>,
        _window_name: Option<&str>,
    ) -> Option<NativeWindowHandle> {
        None
    }
    fn enum_windows(&self) -> Vec<NativeWindowHandle> {
        Vec::new()
    }
    fn enum_child_windows(&self, _parent: NativeWindowHandle) -> Vec<NativeWindowHandle> {
        Vec::new()
    }
    fn get_class_name(&self, hwnd: NativeWindowHandle) -> Result<String, DiscoveryError> {
        Err(DiscoveryError::InvalidHandle(hwnd))
    }
    fn get_window_text(&self, hwnd: NativeWindowHandle) -> Result<String, DiscoveryError> {
        Err(DiscoveryError::InvalidHandle(hwnd))
    }
    fn get_window_thread_process_id(&self, _hwnd: NativeWindowHandle) -> (u32, u32) {
        (0, 0)
    }
    fn is_window(&self, _hwnd: NativeWindowHandle) -> bool {
        false
    }
    fn is_window_visible(&self, _hwnd: NativeWindowHandle) -> bool {
        false
    }
    fn get_window_rect(&self, hwnd: NativeWindowHandle) -> Result<WindowRect, DiscoveryError> {
        Err(DiscoveryError::InvalidHandle(hwnd))
    }
    fn send_message_timeout(
        &self,
        _hwnd: NativeWindowHandle,
        _msg: u32,
        _wparam: usize,
        _lparam: isize,
        timeout_ms: u32,
    ) -> Result<usize, DiscoveryError> {
        Err(DiscoveryError::ActiveProbeTimedOut(timeout_ms))
    }
    fn get_parent(&self, _hwnd: NativeWindowHandle) -> Option<NativeWindowHandle> {
        None
    }
    fn set_parent(
        &self,
        _hwnd: NativeWindowHandle,
        _new_parent: Option<NativeWindowHandle>,
    ) -> Result<Option<NativeWindowHandle>, DiscoveryError> {
        Err(DiscoveryError::NativeApiFailure {
            function: "SetParent".into(),
            code: 0,
            message: "Platform unsupported".into(),
        })
    }
    fn get_window_long(
        &self,
        _hwnd: NativeWindowHandle,
        _index: i32,
    ) -> Result<isize, DiscoveryError> {
        Err(DiscoveryError::NativeApiFailure {
            function: "GetWindowLongPtrW".into(),
            code: 0,
            message: "Platform unsupported".into(),
        })
    }
    fn set_window_long(
        &self,
        _hwnd: NativeWindowHandle,
        _index: i32,
        _new_value: isize,
    ) -> Result<isize, DiscoveryError> {
        Err(DiscoveryError::NativeApiFailure {
            function: "SetWindowLongPtrW".into(),
            code: 0,
            message: "Platform unsupported".into(),
        })
    }
    #[allow(clippy::too_many_arguments)]
    fn set_window_pos(
        &self,
        _hwnd: NativeWindowHandle,
        _insert_after: Option<NativeWindowHandle>,
        _x: i32,
        _y: i32,
        _cx: i32,
        _cy: i32,
        _flags: u32,
    ) -> Result<(), DiscoveryError> {
        Err(DiscoveryError::NativeApiFailure {
            function: "SetWindowPos".into(),
            code: 0,
            message: "Platform unsupported".into(),
        })
    }
    fn get_window_dpi_awareness_context(&self, _hwnd: NativeWindowHandle) -> Option<isize> {
        None
    }
    fn are_dpi_awareness_contexts_equal(&self, _ctx1: isize, _ctx2: isize) -> bool {
        false
    }
    fn create_test_window(
        &self,
        _title: &str,
        _x: i32,
        _y: i32,
        _width: i32,
        _height: i32,
    ) -> Result<NativeWindowHandle, DiscoveryError> {
        Err(DiscoveryError::NativeApiFailure {
            function: "CreateWindowExW".into(),
            code: 0,
            message: "Platform unsupported".into(),
        })
    }
    fn destroy_window(&self, _hwnd: NativeWindowHandle) -> Result<(), DiscoveryError> {
        Ok(())
    }
    fn get_foreground_window(&self) -> Option<NativeWindowHandle> {
        None
    }
    fn is_iconic(&self, _hwnd: NativeWindowHandle) -> bool {
        false
    }
    fn is_window_cloaked(&self, _hwnd: NativeWindowHandle) -> bool {
        false
    }
    fn get_extended_frame_bounds(
        &self,
        hwnd: NativeWindowHandle,
    ) -> Result<WindowRect, DiscoveryError> {
        self.get_window_rect(hwnd)
    }
}

// -------------------------------------------------------------------------------------------------
// Mock Provider for Unit & Component Testing
// -------------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct MockWindowRecord {
    pub handle: NativeWindowHandle,
    pub class_name: String,
    pub window_title: String,
    pub process_id: u32,
    pub thread_id: u32,
    pub parent: Option<NativeWindowHandle>,
    pub is_visible: bool,
    pub bounds: WindowRect,
    pub children: Vec<NativeWindowHandle>,
    pub style: isize,
    pub ex_style: isize,
    pub dpi_context: Option<isize>,
}

/// Mock probe response callback type for testing active probe behaviors.
pub type MockProbeCallback =
    Box<dyn Fn(NativeWindowHandle, u32) -> Result<usize, DiscoveryError> + Send + Sync>;

/// Fully deterministic in-memory window provider for testing all topology variations.
#[derive(Clone, Default)]
pub struct MockDesktopWindowProvider {
    windows: Arc<Mutex<HashMap<NativeWindowHandle, MockWindowRecord>>>,
    top_level_z_order: Arc<Mutex<Vec<NativeWindowHandle>>>,
    probe_response: Arc<Mutex<Option<MockProbeCallback>>>,
    pub fail_set_parent: Arc<Mutex<bool>>,
    pub fail_set_window_long: Arc<Mutex<bool>>,
    pub fail_set_window_pos: Arc<Mutex<bool>>,
    pub fail_verification: Arc<Mutex<bool>>,
    next_handle: Arc<Mutex<usize>>,
    foreground_window: Arc<Mutex<Option<NativeWindowHandle>>>,
    minimized_windows: Arc<Mutex<std::collections::HashSet<NativeWindowHandle>>>,
    cloaked_windows: Arc<Mutex<std::collections::HashSet<NativeWindowHandle>>>,
    extended_frame_bounds: Arc<Mutex<HashMap<NativeWindowHandle, WindowRect>>>,
}

impl std::fmt::Debug for MockDesktopWindowProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockDesktopWindowProvider")
            .field(
                "windows_count",
                &self.windows.lock().map(|m| m.len()).unwrap_or(0),
            )
            .field(
                "top_level_count",
                &self.top_level_z_order.lock().map(|z| z.len()).unwrap_or(0),
            )
            .finish()
    }
}

impl MockDesktopWindowProvider {
    #[must_use]
    pub fn new() -> Self {
        Self {
            windows: Arc::new(Mutex::new(HashMap::new())),
            top_level_z_order: Arc::new(Mutex::new(Vec::new())),
            probe_response: Arc::new(Mutex::new(None)),
            fail_set_parent: Arc::new(Mutex::new(false)),
            fail_set_window_long: Arc::new(Mutex::new(false)),
            fail_set_window_pos: Arc::new(Mutex::new(false)),
            fail_verification: Arc::new(Mutex::new(false)),
            next_handle: Arc::new(Mutex::new(0x2000)),
            foreground_window: Arc::new(Mutex::new(None)),
            minimized_windows: Arc::new(Mutex::new(std::collections::HashSet::new())),
            cloaked_windows: Arc::new(Mutex::new(std::collections::HashSet::new())),
            extended_frame_bounds: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Sets the current mock foreground window.
    pub fn set_foreground_window(&self, hwnd: Option<NativeWindowHandle>) {
        *self.foreground_window.lock().unwrap() = hwnd;
    }

    /// Sets the mock minimized (iconic) state for a window.
    pub fn set_minimized(&self, hwnd: NativeWindowHandle, minimized: bool) {
        let mut set = self.minimized_windows.lock().unwrap();
        if minimized {
            set.insert(hwnd);
        } else {
            set.remove(&hwnd);
        }
    }

    /// Sets the mock cloaked state for a window.
    pub fn set_cloaked(&self, hwnd: NativeWindowHandle, cloaked: bool) {
        let mut set = self.cloaked_windows.lock().unwrap();
        if cloaked {
            set.insert(hwnd);
        } else {
            set.remove(&hwnd);
        }
    }

    /// Sets mock extended frame bounds for a window.
    pub fn set_extended_frame_bounds(&self, hwnd: NativeWindowHandle, bounds: WindowRect) {
        self.extended_frame_bounds.lock().unwrap().insert(hwnd, bounds);
    }

    /// Registers a window into the mock hierarchy.
    pub fn add_window(&self, record: MockWindowRecord) {
        let parent = record.parent;
        let handle = record.handle;

        let mut win_map = self.windows.lock().unwrap();
        if let Some(p) = parent {
            if let Some(parent_rec) = win_map.get_mut(&p) {
                if !parent_rec.children.contains(&handle) {
                    parent_rec.children.push(handle);
                }
            }
        } else {
            let mut z_order = self.top_level_z_order.lock().unwrap();
            if !z_order.contains(&handle) {
                z_order.push(handle);
            }
        }
        win_map.insert(handle, record);
    }

    /// Configures custom behavior for `send_message_timeout` (active probe).
    pub fn set_probe_handler<F>(&self, handler: F)
    where
        F: Fn(NativeWindowHandle, u32) -> Result<usize, DiscoveryError> + Send + Sync + 'static,
    {
        *self.probe_response.lock().unwrap() = Some(Box::new(handler));
    }

    pub fn set_fail_set_parent(&self, fail: bool) {
        *self.fail_set_parent.lock().unwrap() = fail;
    }

    pub fn set_fail_set_window_long(&self, fail: bool) {
        *self.fail_set_window_long.lock().unwrap() = fail;
    }

    pub fn set_fail_set_window_pos(&self, fail: bool) {
        *self.fail_set_window_pos.lock().unwrap() = fail;
    }

    pub fn set_fail_verification(&self, fail: bool) {
        *self.fail_verification.lock().unwrap() = fail;
    }
}

impl DesktopWindowProvider for MockDesktopWindowProvider {
    fn find_window(
        &self,
        class_name: Option<&str>,
        window_name: Option<&str>,
    ) -> Option<NativeWindowHandle> {
        let z_order = self.top_level_z_order.lock().unwrap();
        let map = self.windows.lock().unwrap();

        for &hwnd in z_order.iter() {
            if let Some(win) = map.get(&hwnd) {
                let class_match = class_name.is_none_or(|c| win.class_name.eq_ignore_ascii_case(c));
                let name_match =
                    window_name.is_none_or(|n| win.window_title.eq_ignore_ascii_case(n));
                if class_match && name_match {
                    return Some(hwnd);
                }
            }
        }
        None
    }

    fn find_window_ex(
        &self,
        parent: NativeWindowHandle,
        child_after: Option<NativeWindowHandle>,
        class_name: Option<&str>,
        window_name: Option<&str>,
    ) -> Option<NativeWindowHandle> {
        let map = self.windows.lock().unwrap();
        let parent_rec = map.get(&parent)?;

        let mut start = false;
        if child_after.is_none() {
            start = true;
        }

        for &child in &parent_rec.children {
            if !start {
                if Some(child) == child_after {
                    start = true;
                }
                continue;
            }

            if let Some(child_win) = map.get(&child) {
                let class_match =
                    class_name.is_none_or(|c| child_win.class_name.eq_ignore_ascii_case(c));
                let name_match =
                    window_name.is_none_or(|n| child_win.window_title.eq_ignore_ascii_case(n));
                if class_match && name_match {
                    return Some(child);
                }
            }
        }
        None
    }

    fn enum_windows(&self) -> Vec<NativeWindowHandle> {
        self.top_level_z_order.lock().unwrap().clone()
    }

    fn enum_child_windows(&self, parent: NativeWindowHandle) -> Vec<NativeWindowHandle> {
        let map = self.windows.lock().unwrap();
        map.get(&parent)
            .map_or_else(Vec::new, |rec| rec.children.clone())
    }

    fn get_class_name(&self, hwnd: NativeWindowHandle) -> Result<String, DiscoveryError> {
        let map = self.windows.lock().unwrap();
        map.get(&hwnd)
            .map(|w| w.class_name.clone())
            .ok_or(DiscoveryError::InvalidHandle(hwnd))
    }

    fn get_window_text(&self, hwnd: NativeWindowHandle) -> Result<String, DiscoveryError> {
        let map = self.windows.lock().unwrap();
        map.get(&hwnd)
            .map(|w| w.window_title.clone())
            .ok_or(DiscoveryError::InvalidHandle(hwnd))
    }

    fn get_window_thread_process_id(&self, hwnd: NativeWindowHandle) -> (u32, u32) {
        let map = self.windows.lock().unwrap();
        map.get(&hwnd)
            .map_or((0, 0), |w| (w.thread_id, w.process_id))
    }

    fn is_window(&self, hwnd: NativeWindowHandle) -> bool {
        let map = self.windows.lock().unwrap();
        map.contains_key(&hwnd)
    }

    fn is_window_visible(&self, hwnd: NativeWindowHandle) -> bool {
        let map = self.windows.lock().unwrap();
        map.get(&hwnd).is_some_and(|w| w.is_visible)
    }

    fn get_window_rect(&self, hwnd: NativeWindowHandle) -> Result<WindowRect, DiscoveryError> {
        let map = self.windows.lock().unwrap();
        map.get(&hwnd)
            .map(|w| w.bounds)
            .ok_or(DiscoveryError::InvalidHandle(hwnd))
    }

    fn send_message_timeout(
        &self,
        hwnd: NativeWindowHandle,
        msg: u32,
        _wparam: usize,
        _lparam: isize,
        _timeout_ms: u32,
    ) -> Result<usize, DiscoveryError> {
        let handler_guard = self.probe_response.lock().unwrap();
        if let Some(handler) = &*handler_guard {
            handler(hwnd, msg)
        } else {
            Ok(0)
        }
    }

    fn get_parent(&self, hwnd: NativeWindowHandle) -> Option<NativeWindowHandle> {
        let map = self.windows.lock().unwrap();
        map.get(&hwnd).and_then(|w| w.parent)
    }

    fn set_parent(
        &self,
        hwnd: NativeWindowHandle,
        new_parent: Option<NativeWindowHandle>,
    ) -> Result<Option<NativeWindowHandle>, DiscoveryError> {
        if *self.fail_set_parent.lock().unwrap() {
            return Err(DiscoveryError::NativeApiFailure {
                function: "SetParent".into(),
                code: 5,
                message: "Mock simulated SetParent failure".into(),
            });
        }
        let mut map = self.windows.lock().unwrap();
        let old_parent = {
            let win = map
                .get_mut(&hwnd)
                .ok_or(DiscoveryError::InvalidHandle(hwnd))?;
            let old = win.parent;
            win.parent = new_parent;
            old
        };

        if let Some(old_p) = old_parent {
            if let Some(p_rec) = map.get_mut(&old_p) {
                p_rec.children.retain(|&c| c != hwnd);
            }
        }
        if let Some(new_p) = new_parent {
            if let Some(p_rec) = map.get_mut(&new_p) {
                if !p_rec.children.contains(&hwnd) {
                    p_rec.children.push(hwnd);
                }
            }
        }
        Ok(old_parent)
    }

    fn get_window_long(
        &self,
        hwnd: NativeWindowHandle,
        index: i32,
    ) -> Result<isize, DiscoveryError> {
        let map = self.windows.lock().unwrap();
        let win = map.get(&hwnd).ok_or(DiscoveryError::InvalidHandle(hwnd))?;
        match index {
            -16 => Ok(win.style),    // GWL_STYLE
            -20 => Ok(win.ex_style), // GWL_EXSTYLE
            _ => Ok(0),
        }
    }

    fn set_window_long(
        &self,
        hwnd: NativeWindowHandle,
        index: i32,
        new_value: isize,
    ) -> Result<isize, DiscoveryError> {
        if *self.fail_set_window_long.lock().unwrap() {
            return Err(DiscoveryError::NativeApiFailure {
                function: "SetWindowLongPtrW".into(),
                code: 5,
                message: "Mock simulated SetWindowLongPtrW failure".into(),
            });
        }
        let mut map = self.windows.lock().unwrap();
        let win = map
            .get_mut(&hwnd)
            .ok_or(DiscoveryError::InvalidHandle(hwnd))?;
        match index {
            -16 => {
                let old = win.style;
                win.style = new_value;
                Ok(old)
            }
            -20 => {
                let old = win.ex_style;
                win.ex_style = new_value;
                Ok(old)
            }
            _ => Ok(0),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn set_window_pos(
        &self,
        hwnd: NativeWindowHandle,
        _insert_after: Option<NativeWindowHandle>,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        flags: u32,
    ) -> Result<(), DiscoveryError> {
        if *self.fail_set_window_pos.lock().unwrap() {
            return Err(DiscoveryError::NativeApiFailure {
                function: "SetWindowPos".into(),
                code: 5,
                message: "Mock simulated SetWindowPos failure".into(),
            });
        }
        let mut map = self.windows.lock().unwrap();
        let win = map
            .get_mut(&hwnd)
            .ok_or(DiscoveryError::InvalidHandle(hwnd))?;
        let nosize = (flags & 0x0001) != 0;
        let nomove = (flags & 0x0002) != 0;
        let show = (flags & 0x0040) != 0;
        let hide = (flags & 0x0080) != 0;

        if !nomove {
            let w = win.bounds.width();
            let h = win.bounds.height();
            win.bounds.left = x;
            win.bounds.top = y;
            win.bounds.right = x + w;
            win.bounds.bottom = y + h;
        }
        if !nosize {
            win.bounds.right = win.bounds.left + cx;
            win.bounds.bottom = win.bounds.top + cy;
        }
        if show {
            win.is_visible = true;
        }
        if hide {
            win.is_visible = false;
        }
        Ok(())
    }

    fn get_window_dpi_awareness_context(&self, hwnd: NativeWindowHandle) -> Option<isize> {
        let map = self.windows.lock().unwrap();
        map.get(&hwnd).and_then(|w| w.dpi_context)
    }

    fn are_dpi_awareness_contexts_equal(&self, ctx1: isize, ctx2: isize) -> bool {
        ctx1 == ctx2
    }

    fn create_test_window(
        &self,
        title: &str,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> Result<NativeWindowHandle, DiscoveryError> {
        let mut next = self.next_handle.lock().unwrap();
        let handle = NativeWindowHandle(*next);
        *next += 1;

        let record = MockWindowRecord {
            handle,
            class_name: "KinetiWallPlayerTest".to_string(),
            window_title: title.to_string(),
            process_id: 1000,
            thread_id: 1001,
            parent: None,
            is_visible: true,
            bounds: WindowRect::new(x, y, x + width, y + height),
            children: Vec::new(),
            style: 0x80000000_u32 as isize | 0x10000000, // WS_POPUP | WS_VISIBLE
            ex_style: 0,
            dpi_context: Some(2), // DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2
        };
        self.add_window(record);
        Ok(handle)
    }

    fn destroy_window(&self, hwnd: NativeWindowHandle) -> Result<(), DiscoveryError> {
        let mut map = self.windows.lock().unwrap();
        if let Some(rec) = map.remove(&hwnd) {
            if let Some(p) = rec.parent {
                if let Some(p_rec) = map.get_mut(&p) {
                    p_rec.children.retain(|&c| c != hwnd);
                }
            }
        }
        let mut z = self.top_level_z_order.lock().unwrap();
        z.retain(|&h| h != hwnd);
        Ok(())
    }

    fn get_foreground_window(&self) -> Option<NativeWindowHandle> {
        *self.foreground_window.lock().unwrap()
    }

    fn is_iconic(&self, hwnd: NativeWindowHandle) -> bool {
        self.minimized_windows.lock().unwrap().contains(&hwnd)
    }

    fn is_window_cloaked(&self, hwnd: NativeWindowHandle) -> bool {
        self.cloaked_windows.lock().unwrap().contains(&hwnd)
    }

    fn get_extended_frame_bounds(
        &self,
        hwnd: NativeWindowHandle,
    ) -> Result<WindowRect, DiscoveryError> {
        if let Some(bounds) = self.extended_frame_bounds.lock().unwrap().get(&hwnd) {
            Ok(*bounds)
        } else {
            self.get_window_rect(hwnd)
        }
    }
}
