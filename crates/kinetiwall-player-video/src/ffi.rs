//! Native `libmpv` dynamic loading via `libloading` (C-API FFI for `mpv-2.dll` / `libmpv-2.dll`).

#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::Path;
use std::sync::Arc;

use libloading::{Library, Symbol};
use tracing::info;

// --- Raw mpv types and constants ---
pub type MpvHandle = *mut c_void;

pub const MPV_EVENT_NONE: c_int = 0;
pub const MPV_EVENT_SHUTDOWN: c_int = 1;
pub const MPV_EVENT_LOG_MESSAGE: c_int = 2;
pub const MPV_EVENT_GET_PROPERTY_REPLY: c_int = 3;
pub const MPV_EVENT_SET_PROPERTY_REPLY: c_int = 4;
pub const MPV_EVENT_COMMAND_REPLY: c_int = 5;
pub const MPV_EVENT_START_FILE: c_int = 6;
pub const MPV_EVENT_END_FILE: c_int = 7;
pub const MPV_EVENT_FILE_LOADED: c_int = 8;
pub const MPV_EVENT_TRACKS_CHANGED: c_int = 9;
pub const MPV_EVENT_TRACK_SWITCHED: c_int = 10;
pub const MPV_EVENT_IDLE: c_int = 11;
pub const MPV_EVENT_PAUSE: c_int = 12;
pub const MPV_EVENT_UNPAUSE: c_int = 13;
pub const MPV_EVENT_TICK: c_int = 14;
pub const MPV_EVENT_SCRIPT_INPUT_DISPATCH: c_int = 15;
pub const MPV_EVENT_CLIENT_MESSAGE: c_int = 16;
pub const MPV_EVENT_VIDEO_RECONFIG: c_int = 17;
pub const MPV_EVENT_AUDIO_RECONFIG: c_int = 18;
pub const MPV_EVENT_METADATA_UPDATE: c_int = 19;
pub const MPV_EVENT_SEEK: c_int = 20;
pub const MPV_EVENT_PLAYBACK_RESTART: c_int = 21;
pub const MPV_EVENT_PROPERTY_CHANGE: c_int = 22;
pub const MPV_EVENT_CHAPTER_CHANGE: c_int = 23;
pub const MPV_EVENT_QUEUE_OVERFLOW: c_int = 24;
pub const MPV_EVENT_HOOK: c_int = 25;

pub const MPV_ERROR_SUCCESS: c_int = 0;
pub const MPV_ERROR_EVENT_QUEUE_FULL: c_int = -1;
pub const MPV_ERROR_NOMEM: c_int = -2;
pub const MPV_ERROR_UNINITIALIZED: c_int = -3;
pub const MPV_ERROR_INVALID_PARAMETER: c_int = -4;
pub const MPV_ERROR_OPTION_NOT_FOUND: c_int = -5;
pub const MPV_ERROR_OPTION_FORMAT: c_int = -6;
pub const MPV_ERROR_OPTION_ERROR: c_int = -7;
pub const MPV_ERROR_PROPERTY_NOT_FOUND: c_int = -8;
pub const MPV_ERROR_PROPERTY_FORMAT: c_int = -9;
pub const MPV_ERROR_PROPERTY_UNAVAILABLE: c_int = -10;
pub const MPV_ERROR_PROPERTY_ERROR: c_int = -11;
pub const MPV_ERROR_COMMAND: c_int = -12;
pub const MPV_ERROR_LOADING_FAILED: c_int = -13;
pub const MPV_ERROR_AO_INIT_FAILED: c_int = -14;
pub const MPV_ERROR_VO_INIT_FAILED: c_int = -15;

#[repr(C)]
pub struct MpvEvent {
    pub event_id: c_int,
    pub error: c_int,
    pub reply_userdata: u64,
    pub data: *mut c_void,
}

pub type FnMpvCreate = unsafe extern "C" fn() -> MpvHandle;
pub type FnMpvInitialize = unsafe extern "C" fn(ctx: MpvHandle) -> c_int;
pub type FnMpvDestroy = unsafe extern "C" fn(ctx: MpvHandle);
pub type FnMpvTerminateDestroy = unsafe extern "C" fn(ctx: MpvHandle);
pub type FnMpvCommand = unsafe extern "C" fn(ctx: MpvHandle, args: *mut *const c_char) -> c_int;
pub type FnMpvSetOptionString =
    unsafe extern "C" fn(ctx: MpvHandle, name: *const c_char, data: *const c_char) -> c_int;
pub type FnMpvSetPropertyString =
    unsafe extern "C" fn(ctx: MpvHandle, name: *const c_char, data: *const c_char) -> c_int;
pub type FnMpvGetPropertyString =
    unsafe extern "C" fn(ctx: MpvHandle, name: *const c_char) -> *mut c_char;
pub type FnMpvFree = unsafe extern "C" fn(data: *mut c_void);
pub type FnMpvWaitEvent = unsafe extern "C" fn(ctx: MpvHandle, timeout: f64) -> *mut MpvEvent;
pub type FnMpvErrorString = unsafe extern "C" fn(error: c_int) -> *const c_char;

/// Dynamic function table loaded from `mpv-2.dll` or `libmpv-2.dll`.
pub struct MpvFunctions {
    _lib: Arc<Library>,
    create: FnMpvCreate,
    initialize: FnMpvInitialize,
    destroy: FnMpvDestroy,
    terminate_destroy: FnMpvTerminateDestroy,
    command: FnMpvCommand,
    set_option_string: FnMpvSetOptionString,
    set_property_string: FnMpvSetPropertyString,
    get_property_string: FnMpvGetPropertyString,
    free: FnMpvFree,
    wait_event: FnMpvWaitEvent,
    error_string: FnMpvErrorString,
}

impl MpvFunctions {
    pub fn load_from_search() -> Result<Self, String> {
        let candidates = ["mpv-2.dll", "libmpv-2.dll", "mpv-1.dll", "libmpv-1.dll"];
        for candidate in candidates {
            if let Ok(loaded) = Self::load(candidate) {
                info!("Successfully dynamically loaded libmpv from `{candidate}`");
                return Ok(loaded);
            }
        }
        Err("Could not find or load mpv-2.dll or libmpv-2.dll in PATH or working directory.".into())
    }

    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let path_ref = path.as_ref();
        let lib = unsafe {
            Library::new(path_ref).map_err(|e| {
                format!(
                    "Failed to load dynamic library `{}`: {e}",
                    path_ref.display()
                )
            })?
        };

        let (
            create,
            initialize,
            destroy,
            terminate_destroy,
            command,
            set_option_string,
            set_property_string,
            get_property_string,
            free,
            wait_event,
            error_string,
        ) = unsafe {
            let c: Symbol<FnMpvCreate> = lib
                .get(b"mpv_create\0")
                .map_err(|e| format!("mpv_create: {e}"))?;
            let init: Symbol<FnMpvInitialize> = lib
                .get(b"mpv_initialize\0")
                .map_err(|e| format!("mpv_initialize: {e}"))?;
            let dest: Symbol<FnMpvDestroy> = lib
                .get(b"mpv_destroy\0")
                .map_err(|e| format!("mpv_destroy: {e}"))?;
            let term: Symbol<FnMpvTerminateDestroy> = lib
                .get(b"mpv_terminate_destroy\0")
                .map_err(|e| format!("mpv_terminate_destroy: {e}"))?;
            let cmd: Symbol<FnMpvCommand> = lib
                .get(b"mpv_command\0")
                .map_err(|e| format!("mpv_command: {e}"))?;
            let opt: Symbol<FnMpvSetOptionString> = lib
                .get(b"mpv_set_option_string\0")
                .map_err(|e| format!("mpv_set_option_string: {e}"))?;
            let setp: Symbol<FnMpvSetPropertyString> = lib
                .get(b"mpv_set_property_string\0")
                .map_err(|e| format!("mpv_set_property_string: {e}"))?;
            let getp: Symbol<FnMpvGetPropertyString> = lib
                .get(b"mpv_get_property_string\0")
                .map_err(|e| format!("mpv_get_property_string: {e}"))?;
            let fr: Symbol<FnMpvFree> = lib
                .get(b"mpv_free\0")
                .map_err(|e| format!("mpv_free: {e}"))?;
            let we: Symbol<FnMpvWaitEvent> = lib
                .get(b"mpv_wait_event\0")
                .map_err(|e| format!("mpv_wait_event: {e}"))?;
            let es: Symbol<FnMpvErrorString> = lib
                .get(b"mpv_error_string\0")
                .map_err(|e| format!("mpv_error_string: {e}"))?;

            (
                *c, *init, *dest, *term, *cmd, *opt, *setp, *getp, *fr, *we, *es,
            )
        };

        Ok(Self {
            _lib: Arc::new(lib),
            create,
            initialize,
            destroy,
            terminate_destroy,
            command,
            set_option_string,
            set_property_string,
            get_property_string,
            free,
            wait_event,
            error_string,
        })
    }

    pub fn mpv_create(&self) -> Result<MpvHandle, String> {
        let handle = unsafe { (self.create)() };
        if handle.is_null() {
            Err("mpv_create returned NULL handle".into())
        } else {
            Ok(handle)
        }
    }

    pub fn mpv_initialize(&self, ctx: MpvHandle) -> Result<(), String> {
        let res = unsafe { (self.initialize)(ctx) };
        if res < 0 {
            Err(self.error_to_string(res))
        } else {
            Ok(())
        }
    }

    pub fn mpv_terminate_destroy(&self, ctx: MpvHandle) {
        unsafe { (self.terminate_destroy)(ctx) };
    }

    pub fn mpv_destroy(&self, ctx: MpvHandle) {
        unsafe { (self.destroy)(ctx) };
    }

    pub fn mpv_set_option_string(
        &self,
        ctx: MpvHandle,
        name: &str,
        val: &str,
    ) -> Result<(), String> {
        let c_name = CString::new(name).map_err(|e| e.to_string())?;
        let c_val = CString::new(val).map_err(|e| e.to_string())?;
        let res = unsafe { (self.set_option_string)(ctx, c_name.as_ptr(), c_val.as_ptr()) };
        if res < 0 {
            Err(format!(
                "Failed to set option `{name}`=`{val}`: {}",
                self.error_to_string(res)
            ))
        } else {
            Ok(())
        }
    }

    pub fn mpv_set_property_string(
        &self,
        ctx: MpvHandle,
        name: &str,
        val: &str,
    ) -> Result<(), String> {
        let c_name = CString::new(name).map_err(|e| e.to_string())?;
        let c_val = CString::new(val).map_err(|e| e.to_string())?;
        let res = unsafe { (self.set_property_string)(ctx, c_name.as_ptr(), c_val.as_ptr()) };
        if res < 0 {
            Err(format!(
                "Failed to set property `{name}`=`{val}`: {}",
                self.error_to_string(res)
            ))
        } else {
            Ok(())
        }
    }

    pub fn mpv_get_property_string(&self, ctx: MpvHandle, name: &str) -> Option<String> {
        let c_name = CString::new(name).ok()?;
        let ptr = unsafe { (self.get_property_string)(ctx, c_name.as_ptr()) };
        if ptr.is_null() {
            None
        } else {
            let s = unsafe { CStr::from_ptr(ptr) }
                .to_string_lossy()
                .into_owned();
            unsafe { (self.free)(ptr as *mut c_void) };
            Some(s)
        }
    }

    pub fn mpv_command(&self, ctx: MpvHandle, args: &[&str]) -> Result<(), String> {
        let c_strings: Vec<CString> = args
            .iter()
            .map(|s| CString::new(*s).map_err(|e| e.to_string()))
            .collect::<Result<_, _>>()?;
        let mut pointers: Vec<*const c_char> = c_strings.iter().map(|cs| cs.as_ptr()).collect();
        pointers.push(std::ptr::null());

        let res = unsafe { (self.command)(ctx, pointers.as_mut_ptr()) };
        if res < 0 {
            Err(format!(
                "mpv_command {:?} failed: {}",
                args,
                self.error_to_string(res)
            ))
        } else {
            Ok(())
        }
    }

    pub fn mpv_wait_event(&self, ctx: MpvHandle, timeout_sec: f64) -> Option<MpvEvent> {
        let ptr = unsafe { (self.wait_event)(ctx, timeout_sec) };
        if ptr.is_null() {
            None
        } else {
            Some(unsafe { std::ptr::read(ptr) })
        }
    }

    pub fn error_to_string(&self, err_code: c_int) -> String {
        let ptr = unsafe { (self.error_string)(err_code) };
        if ptr.is_null() {
            format!("Error code {err_code}")
        } else {
            unsafe { CStr::from_ptr(ptr) }
                .to_string_lossy()
                .into_owned()
        }
    }
}
