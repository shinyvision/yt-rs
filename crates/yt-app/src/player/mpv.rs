//! A minimal safe wrapper around libmpv's client API (mpv/client.h,
//! API version 2). Only what the app needs is bound.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::ptr::NonNull;
use std::sync::Mutex;

#[allow(non_camel_case_types)]
pub(crate) mod ffi {
    use std::ffi::{c_char, c_double, c_int, c_void};

    pub enum mpv_handle {}

    pub const FORMAT_STRING: c_int = 1;
    pub const FORMAT_FLAG: c_int = 3;
    pub const FORMAT_INT64: c_int = 4;
    pub const FORMAT_DOUBLE: c_int = 5;

    pub const EVENT_NONE: c_int = 0;
    pub const EVENT_SHUTDOWN: c_int = 1;
    pub const EVENT_LOG_MESSAGE: c_int = 2;
    pub const EVENT_START_FILE: c_int = 6;
    pub const EVENT_END_FILE: c_int = 7;
    pub const EVENT_FILE_LOADED: c_int = 8;
    pub const EVENT_VIDEO_RECONFIG: c_int = 17;
    pub const EVENT_SEEK: c_int = 20;
    pub const EVENT_PLAYBACK_RESTART: c_int = 21;
    pub const EVENT_PROPERTY_CHANGE: c_int = 22;

    #[repr(C)]
    pub struct mpv_event {
        pub event_id: c_int,
        pub error: c_int,
        pub reply_userdata: u64,
        pub data: *mut c_void,
    }

    #[repr(C)]
    pub struct mpv_event_property {
        pub name: *const c_char,
        pub format: c_int,
        pub data: *mut c_void,
    }

    #[repr(C)]
    pub struct mpv_event_end_file {
        pub reason: c_int,
        pub error: c_int,
        pub playlist_entry_id: i64,
        pub playlist_insert_id: i64,
        pub playlist_insert_num_entries: c_int,
    }

    #[repr(C)]
    pub struct mpv_event_log_message {
        pub prefix: *const c_char,
        pub level: *const c_char,
        pub text: *const c_char,
        pub log_level: c_int,
    }

    /// mpv/stream_cb.h: a stream mpv reads through the app's functions.
    #[repr(C)]
    pub struct mpv_stream_cb_info {
        pub cookie: *mut c_void,
        pub read_fn: Option<unsafe extern "C" fn(*mut c_void, *mut c_char, u64) -> i64>,
        pub seek_fn: Option<unsafe extern "C" fn(*mut c_void, i64) -> i64>,
        pub size_fn: Option<unsafe extern "C" fn(*mut c_void) -> i64>,
        pub close_fn: Option<unsafe extern "C" fn(*mut c_void)>,
        pub cancel_fn: Option<unsafe extern "C" fn(*mut c_void)>,
    }

    pub type StreamOpen =
        unsafe extern "C" fn(*mut c_void, *mut c_char, *mut mpv_stream_cb_info) -> c_int;

    pub const ERROR_LOADING_FAILED: c_int = -13;
    pub const ERROR_UNSUPPORTED: c_int = -18;
    pub const ERROR_GENERIC: c_int = -20;

    /// `LC_NUMERIC` in glibc.
    pub const LC_NUMERIC: c_int = 1;

    unsafe extern "C" {
        pub fn setlocale(category: c_int, locale: *const c_char) -> *mut c_char;
    }

    #[link(name = "mpv")]
    unsafe extern "C" {
        pub fn mpv_client_api_version() -> std::ffi::c_ulong;
        pub fn mpv_error_string(error: c_int) -> *const c_char;
        pub fn mpv_free(data: *mut c_void);
        pub fn mpv_create() -> *mut mpv_handle;
        pub fn mpv_initialize(ctx: *mut mpv_handle) -> c_int;
        pub fn mpv_terminate_destroy(ctx: *mut mpv_handle);
        pub fn mpv_set_option_string(
            ctx: *mut mpv_handle,
            name: *const c_char,
            data: *const c_char,
        ) -> c_int;
        pub fn mpv_command(ctx: *mut mpv_handle, args: *mut *const c_char) -> c_int;
        pub fn mpv_set_property_string(
            ctx: *mut mpv_handle,
            name: *const c_char,
            data: *const c_char,
        ) -> c_int;
        pub fn mpv_set_property(
            ctx: *mut mpv_handle,
            name: *const c_char,
            format: c_int,
            data: *mut c_void,
        ) -> c_int;
        pub fn mpv_get_property_string(ctx: *mut mpv_handle, name: *const c_char) -> *mut c_char;
        pub fn mpv_observe_property(
            ctx: *mut mpv_handle,
            id: u64,
            name: *const c_char,
            format: c_int,
        ) -> c_int;
        pub fn mpv_request_log_messages(ctx: *mut mpv_handle, min_level: *const c_char) -> c_int;
        pub fn mpv_wait_event(ctx: *mut mpv_handle, timeout: c_double) -> *mut mpv_event;
        pub fn mpv_stream_cb_add_ro(
            ctx: *mut mpv_handle,
            protocol: *const c_char,
            user_data: *mut c_void,
            open_fn: Option<StreamOpen>,
        ) -> c_int;
        pub fn mpv_set_wakeup_callback(
            ctx: *mut mpv_handle,
            cb: Option<unsafe extern "C" fn(*mut c_void)>,
            d: *mut c_void,
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MpvError(pub String);

impl std::fmt::Display for MpvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn check(code: c_int, what: &str) -> Result<(), MpvError> {
    if code >= 0 {
        return Ok(());
    }
    // SAFETY: mpv_error_string returns a static string for any code.
    let msg = unsafe { CStr::from_ptr(ffi::mpv_error_string(code)) }.to_string_lossy();
    Err(MpvError(format!("{what}: {msg}")))
}

fn cstring(s: &str) -> CString {
    CString::new(s.replace('\0', "")).expect("NULs removed")
}

/// The format to observe a property in.
#[derive(Clone, Copy, Debug)]
pub enum Format {
    Flag,
    Double,
}

impl Format {
    fn raw(self) -> c_int {
        match self {
            Format::Flag => ffi::FORMAT_FLAG,
            Format::Double => ffi::FORMAT_DOUBLE,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PropertyValue {
    /// The property is unavailable, e.g. `duration` before a file loads.
    None,
    Flag(bool),
    Double(f64),
    Int(i64),
    String(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndReason {
    Eof,
    Stop,
    Quit,
    Error,
    Redirect,
    Unknown,
}

#[derive(Clone, Debug)]
pub enum Event {
    Shutdown,
    StartFile,
    FileLoaded,
    EndFile {
        reason: EndReason,
        error: Option<String>,
    },
    VideoReconfig,
    Seek,
    PlaybackRestart,
    PropertyChange {
        id: u64,
        value: PropertyValue,
    },
    Log {
        level: String,
        prefix: String,
        text: String,
    },
    Other,
}

type Callback = Box<dyn Fn() + Send + Sync>;

/// An mpv instance. libmpv's client API is thread-safe, so this can be
/// shared between threads.
pub struct Mpv {
    handle: NonNull<ffi::mpv_handle>,
    wakeup: Mutex<Option<Box<Callback>>>,
}

// SAFETY: every mpv_* client function may be called from any thread.
unsafe impl Send for Mpv {}
unsafe impl Sync for Mpv {}

impl Mpv {
    /// Creates an uninitialized instance; set options, then call
    /// [`Mpv::initialize`].
    pub fn create() -> Result<Self, MpvError> {
        // mpv parses numbers with "." and refuses to start under a locale
        // that formats them differently. GTK applies the user's locale at
        // startup, so put numbers back to "C".
        // SAFETY: called on the main thread before other threads use locales.
        unsafe { ffi::setlocale(ffi::LC_NUMERIC, c"C".as_ptr()) };
        // SAFETY: plain constructor; null on failure.
        let version = unsafe { ffi::mpv_client_api_version() };
        if version >> 16 != 2 {
            return Err(MpvError(format!(
                "unsupported libmpv API version {}.{}",
                version >> 16,
                version & 0xffff
            )));
        }
        let handle = NonNull::new(unsafe { ffi::mpv_create() })
            .ok_or_else(|| MpvError("mpv_create failed".into()))?;
        Ok(Self {
            handle,
            wakeup: Mutex::new(None),
        })
    }

    pub fn raw(&self) -> *mut ffi::mpv_handle {
        self.handle.as_ptr()
    }

    pub fn set_option(&self, name: &str, value: &str) -> Result<(), MpvError> {
        let (n, v) = (cstring(name), cstring(value));
        check(
            unsafe { ffi::mpv_set_option_string(self.raw(), n.as_ptr(), v.as_ptr()) },
            name,
        )
    }

    /// Lets mpv open `protocol://...` URLs through `open`, which fills in
    /// the stream's read functions.
    pub fn add_stream_protocol(
        &self,
        protocol: &str,
        open: ffi::StreamOpen,
    ) -> Result<(), MpvError> {
        let name = cstring(protocol);
        check(
            // SAFETY: mpv copies the protocol name; `open` is a plain function.
            unsafe {
                ffi::mpv_stream_cb_add_ro(
                    self.raw(),
                    name.as_ptr(),
                    std::ptr::null_mut(),
                    Some(open),
                )
            },
            "stream_cb_add_ro",
        )
    }

    pub fn initialize(&self) -> Result<(), MpvError> {
        check(unsafe { ffi::mpv_initialize(self.raw()) }, "initialize")
    }

    pub fn command(&self, args: &[&str]) -> Result<(), MpvError> {
        let owned: Vec<CString> = args.iter().map(|a| cstring(a)).collect();
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|a| a.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        check(
            unsafe { ffi::mpv_command(self.raw(), ptrs.as_mut_ptr()) },
            args.first().copied().unwrap_or("command"),
        )
    }

    pub fn set_property(&self, name: &str, value: &str) -> Result<(), MpvError> {
        let (n, v) = (cstring(name), cstring(value));
        check(
            unsafe { ffi::mpv_set_property_string(self.raw(), n.as_ptr(), v.as_ptr()) },
            name,
        )
    }

    pub fn set_flag(&self, name: &str, value: bool) -> Result<(), MpvError> {
        let n = cstring(name);
        let mut v: c_int = value.into();
        check(
            unsafe {
                ffi::mpv_set_property(
                    self.raw(),
                    n.as_ptr(),
                    ffi::FORMAT_FLAG,
                    (&raw mut v).cast(),
                )
            },
            name,
        )
    }

    pub fn set_double(&self, name: &str, value: f64) -> Result<(), MpvError> {
        let n = cstring(name);
        let mut v = value;
        check(
            unsafe {
                ffi::mpv_set_property(
                    self.raw(),
                    n.as_ptr(),
                    ffi::FORMAT_DOUBLE,
                    (&raw mut v).cast(),
                )
            },
            name,
        )
    }

    pub fn get_string(&self, name: &str) -> Option<String> {
        let n = cstring(name);
        let ptr = unsafe { ffi::mpv_get_property_string(self.raw(), n.as_ptr()) };
        if ptr.is_null() {
            return None;
        }
        let s = unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned();
        unsafe { ffi::mpv_free(ptr.cast()) };
        Some(s)
    }

    pub fn observe(&self, id: u64, name: &str, format: Format) -> Result<(), MpvError> {
        let n = cstring(name);
        check(
            unsafe { ffi::mpv_observe_property(self.raw(), id, n.as_ptr(), format.raw()) },
            name,
        )
    }

    pub fn request_log_messages(&self, level: &str) -> Result<(), MpvError> {
        let l = cstring(level);
        check(
            unsafe { ffi::mpv_request_log_messages(self.raw(), l.as_ptr()) },
            "request_log_messages",
        )
    }

    /// Calls `callback` (on an mpv thread) whenever events are waiting.
    pub fn set_wakeup_callback(&self, callback: impl Fn() + Send + Sync + 'static) {
        unsafe extern "C" fn trampoline(data: *mut c_void) {
            // SAFETY: `data` points at the boxed callback, which lives until
            // the handle is destroyed.
            let callback = unsafe { &*(data as *const Callback) };
            callback();
        }
        let boxed: Box<Callback> = Box::new(Box::new(callback));
        let data = (&*boxed as *const Callback).cast_mut().cast();
        let mut slot = self.wakeup.lock().unwrap();
        unsafe { ffi::mpv_set_wakeup_callback(self.raw(), Some(trampoline), data) };
        *slot = Some(boxed);
    }

    /// Returns the next event, or `None` if there is none within `timeout`
    /// seconds (0 to only check).
    pub fn wait_event(&self, timeout: f64) -> Option<Event> {
        // SAFETY: the event stays valid until the next mpv_wait_event call,
        // and everything is copied out of it before returning.
        let event = unsafe { &*ffi::mpv_wait_event(self.raw(), timeout) };
        let s = |p: *const c_char| {
            if p.is_null() {
                String::new()
            } else {
                unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
            }
        };
        Some(match event.event_id {
            ffi::EVENT_NONE => return None,
            ffi::EVENT_SHUTDOWN => Event::Shutdown,
            ffi::EVENT_START_FILE => Event::StartFile,
            ffi::EVENT_FILE_LOADED => Event::FileLoaded,
            ffi::EVENT_VIDEO_RECONFIG => Event::VideoReconfig,
            ffi::EVENT_SEEK => Event::Seek,
            ffi::EVENT_PLAYBACK_RESTART => Event::PlaybackRestart,
            ffi::EVENT_END_FILE => {
                let data = unsafe { &*(event.data as *const ffi::mpv_event_end_file) };
                let reason = match data.reason {
                    0 => EndReason::Eof,
                    2 => EndReason::Stop,
                    3 => EndReason::Quit,
                    4 => EndReason::Error,
                    5 => EndReason::Redirect,
                    _ => EndReason::Unknown,
                };
                let error =
                    (data.error < 0).then(|| s(unsafe { ffi::mpv_error_string(data.error) }));
                Event::EndFile { reason, error }
            }
            ffi::EVENT_PROPERTY_CHANGE => {
                let prop = unsafe { &*(event.data as *const ffi::mpv_event_property) };
                let value = unsafe {
                    match prop.format {
                        ffi::FORMAT_FLAG => PropertyValue::Flag(*(prop.data as *const c_int) != 0),
                        ffi::FORMAT_DOUBLE => PropertyValue::Double(*(prop.data as *const f64)),
                        ffi::FORMAT_INT64 => PropertyValue::Int(*(prop.data as *const i64)),
                        ffi::FORMAT_STRING => {
                            PropertyValue::String(s(*(prop.data as *const *const c_char)))
                        }
                        _ => PropertyValue::None,
                    }
                };
                Event::PropertyChange {
                    id: event.reply_userdata,
                    value,
                }
            }
            ffi::EVENT_LOG_MESSAGE => {
                let msg = unsafe { &*(event.data as *const ffi::mpv_event_log_message) };
                Event::Log {
                    level: s(msg.level),
                    prefix: s(msg.prefix),
                    text: s(msg.text).trim_end().to_owned(),
                }
            }
            _ => Event::Other,
        })
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        unsafe {
            ffi::mpv_set_wakeup_callback(self.raw(), None, std::ptr::null_mut());
            ffi::mpv_terminate_destroy(self.raw());
        }
    }
}

/// Quotes a value for mpv's option lists, which split on commas. URLs from
/// YouTube contain commas.
pub fn quote(value: &str) -> String {
    format!("%{}%{value}", value.len())
}
