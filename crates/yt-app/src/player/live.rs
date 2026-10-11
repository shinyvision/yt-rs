//! Live streams in mpv. YouTube serves them as segments (see
//! `yt_core::streams::live`), so mpv opens a live stream as a `ytlive://`
//! URL, which these callbacks read with a [`LiveReader`]. [`url_for`]
//! registers the stream about to play, from which segment, and where its
//! readers report the newest segment, and gives its URL.

use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use yt_core::streams::Stream;
use yt_core::streams::live::{LiveReader, SharedHead};

use super::mpv::{Mpv, MpvError, ffi};

const PROTOCOL: &str = "ytlive";

struct Source {
    url: String,
    user_agent: Option<String>,
    segment: f64,
    start: Option<u64>,
    head: SharedHead,
}

static SOURCES: Mutex<Option<HashMap<u64, Source>>> = Mutex::new(None);
static NEXT: AtomicU64 = AtomicU64::new(1);
static HTTP: Mutex<Option<reqwest::Client>> = Mutex::new(None);

/// Lets mpv open `ytlive://` URLs.
pub fn install(mpv: &Mpv, http: reqwest::Client) -> Result<(), MpvError> {
    *HTTP.lock().unwrap() = Some(http);
    mpv.add_stream_protocol(PROTOCOL, open)
}

/// The URL mpv opens for a live stream, read from segment `start` (or near
/// the newest). Earlier ones stop working.
pub fn url_for(stream: &Stream, start: Option<u64>, head: SharedHead) -> String {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let mut sources = SOURCES.lock().unwrap();
    let sources = sources.get_or_insert_with(HashMap::new);
    // A live video has a video and an audio stream; older ones are done.
    sources.retain(|k, _| *k + 2 > id);
    sources.insert(
        id,
        Source {
            url: stream.url.clone(),
            user_agent: stream.headers.get("User-Agent").cloned(),
            segment: stream.segment_duration.unwrap_or(2.0),
            start,
            head,
        },
    );
    format!("{PROTOCOL}://{id}")
}

/// One open stream: mpv's cookie.
struct Open {
    reader: Mutex<LiveReader>,
    cancelled: Arc<AtomicBool>,
}

unsafe extern "C" fn open(
    _: *mut c_void,
    uri: *mut c_char,
    info: *mut ffi::mpv_stream_cb_info,
) -> c_int {
    let opened = std::panic::catch_unwind(|| {
        // SAFETY: mpv passes a NUL-terminated URI.
        let uri = unsafe { CStr::from_ptr(uri) }.to_string_lossy();
        let id: u64 = uri.strip_prefix("ytlive://")?.parse().ok()?;
        let sources = SOURCES.lock().unwrap();
        let source = sources.as_ref()?.get(&id)?;
        let http = HTTP.lock().unwrap().clone()?;
        let reader = LiveReader::new(
            http,
            source.url.clone(),
            source.user_agent.clone(),
            source.segment,
            source.start,
            source.head.clone(),
            crate::runtime::handle(),
        );
        Some(Open {
            cancelled: reader.cancel_flag(),
            reader: Mutex::new(reader),
        })
    });
    let Ok(Some(opened)) = opened else {
        return ffi::ERROR_LOADING_FAILED;
    };
    // SAFETY: mpv passes a struct for the callbacks to fill in.
    let info = unsafe { &mut *info };
    info.cookie = Box::into_raw(Box::new(opened)).cast();
    info.read_fn = Some(read);
    info.seek_fn = None;
    info.size_fn = None;
    info.close_fn = Some(close);
    info.cancel_fn = Some(cancel);
    0
}

unsafe extern "C" fn read(cookie: *mut c_void, buf: *mut c_char, size: u64) -> i64 {
    // SAFETY: the cookie is the `Open` made in `open`, alive until `close`;
    // `buf` holds `size` bytes.
    let (open, buf) = unsafe {
        (
            &*(cookie as *const Open),
            std::slice::from_raw_parts_mut(buf.cast::<u8>(), size as usize),
        )
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        open.reader.lock().unwrap().read(buf)
    }));
    match result {
        Ok(Ok(n)) => n as i64,
        Ok(Err(e)) => {
            tracing::warn!("live stream: {e}");
            -1
        }
        Err(_) => -1,
    }
}

unsafe extern "C" fn close(cookie: *mut c_void) {
    // SAFETY: made by `Box::into_raw` in `open`; mpv closes once.
    drop(unsafe { Box::from_raw(cookie as *mut Open) });
}

unsafe extern "C" fn cancel(cookie: *mut c_void) {
    // SAFETY: as in `read`; only the flag is touched, from mpv's other thread.
    let open = unsafe { &*(cookie as *const Open) };
    open.cancelled.store(true, Ordering::Relaxed);
}
