//! How mpv reads YouTube's streams: through these callbacks (libmpv's
//! stream_cb API) rather than its own HTTP, which YouTube throttles. A
//! stream plays as a `ytstream://<n>` URL that [`url_for`] registers, read
//! by a [`ChunkedReader`] (in bounded ranges, getting a new URL when the
//! one in use is refused), or for a live stream by a [`LiveReader`] (segment
//! by segment).
//!
//! A reader that gives up says why here, so the player can tell a failed
//! stream from one that ended ([`take_failure`]).

use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use yt_core::streams::Stream;
use yt_core::streams::chunked::{ChunkedReader, Refresh};
use yt_core::streams::live::{LiveReader, SharedHead};

use super::mpv::{Mpv, MpvError, ffi};

const PROTOCOL: &str = "ytstream";
/// Registered streams kept for mpv to open: a video and its audio, and
/// the pair before (still opening while the next loads).
const KEPT: u64 = 4;

/// How a registered stream is read.
pub enum Source {
    Chunked {
        refresh: Option<Refresh>,
    },
    Live {
        /// The segment to start at; `None` for near the newest.
        start: Option<u64>,
        head: SharedHead,
    },
}

struct Registered {
    stream: Stream,
    source: Source,
}

static REGISTERED: Mutex<Option<HashMap<u64, Registered>>> = Mutex::new(None);
static NEXT: AtomicU64 = AtomicU64::new(1);
static HTTP: Mutex<Option<reqwest::Client>> = Mutex::new(None);
/// Why the last stream that gave up did.
static FAILURE: Mutex<Option<String>> = Mutex::new(None);

/// Lets mpv open `ytstream://` URLs, fetching with `http`.
pub fn install(mpv: &Mpv, http: reqwest::Client) -> Result<(), MpvError> {
    *HTTP.lock().unwrap() = Some(http);
    mpv.add_stream_protocol(PROTOCOL, open)
}

/// The URL mpv opens for `stream`. Older ones stop working after a while.
pub fn url_for(stream: &Stream, source: Source) -> String {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let mut registered = REGISTERED.lock().unwrap();
    let registered = registered.get_or_insert_with(HashMap::new);
    registered.retain(|k, _| *k + KEPT > id);
    registered.insert(
        id,
        Registered {
            stream: stream.clone(),
            source,
        },
    );
    format!("{PROTOCOL}://{id}")
}

/// Why a stream gave up since the last call, if one did. Call it when
/// playback stops short: a stream that can't be read ends like one at its
/// end.
pub fn take_failure() -> Option<String> {
    FAILURE.lock().unwrap().take()
}

/// Forgets a failure, when another video loads.
pub fn clear_failure() {
    FAILURE.lock().unwrap().take();
}

enum Reader {
    Chunked(ChunkedReader),
    Live(LiveReader),
}

/// One open stream: mpv's cookie.
struct Open {
    reader: Mutex<Reader>,
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
        let id: u64 = uri.strip_prefix("ytstream://")?.parse().ok()?;
        let http = HTTP.lock().unwrap().clone()?;
        let registered = REGISTERED.lock().unwrap();
        let Registered { stream, source } = registered.as_ref()?.get(&id)?;
        let user_agent = stream.headers.get("User-Agent").cloned();
        let runtime = crate::runtime::handle();
        let reader = match source {
            Source::Chunked { refresh } => Reader::Chunked(ChunkedReader::new(
                http,
                stream.url.clone(),
                user_agent,
                refresh.clone(),
                runtime,
            )),
            Source::Live { start, head } => Reader::Live(LiveReader::new(
                http,
                stream.url.clone(),
                user_agent,
                stream.segment_duration.unwrap_or(2.0),
                *start,
                head.clone(),
                runtime,
            )),
        };
        let cancelled = match &reader {
            Reader::Chunked(r) => r.cancel_flag(),
            Reader::Live(r) => r.cancel_flag(),
        };
        Some((
            Open {
                reader: Mutex::new(reader),
                cancelled,
            },
            matches!(source, Source::Chunked { .. }),
        ))
    });
    let Ok(Some((opened, seekable))) = opened else {
        return ffi::ERROR_LOADING_FAILED;
    };
    // SAFETY: mpv passes a struct for the callbacks to fill in.
    let info = unsafe { &mut *info };
    info.cookie = Box::into_raw(Box::new(opened)).cast();
    info.read_fn = Some(read);
    info.seek_fn = seekable.then_some(seek as _);
    info.size_fn = seekable.then_some(size as _);
    info.close_fn = Some(close);
    info.cancel_fn = Some(cancel);
    0
}

/// Runs `f` on the cookie's reader, and says whether mpv cancelled it.
/// `None` if the reader panicked, now or before.
fn with_reader<T>(cookie: *mut c_void, f: impl FnOnce(&mut Reader) -> T) -> Option<(T, bool)> {
    // SAFETY: the cookie is the `Open` made in `open`, alive until `close`.
    let open = unsafe { &*(cookie as *const Open) };
    let value = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        open.reader.lock().ok().map(|mut reader| f(&mut reader))
    }))
    .ok()
    .flatten()?;
    Some((value, open.cancelled.load(Ordering::Relaxed)))
}

/// Notes why a stream gave up (unless mpv stopped it), and answers mpv
/// with an error.
fn failed(error: std::io::Error, cancelled: bool) -> i64 {
    if !cancelled {
        tracing::warn!("stream: {error}");
        *FAILURE.lock().unwrap() = Some(error.to_string());
    }
    -1
}

unsafe extern "C" fn read(cookie: *mut c_void, buf: *mut c_char, size: u64) -> i64 {
    // SAFETY: mpv's buffer holds `size` bytes.
    let buf = unsafe { std::slice::from_raw_parts_mut(buf.cast::<u8>(), size as usize) };
    let result = with_reader(cookie, |reader| match reader {
        Reader::Chunked(r) => r.read(buf),
        Reader::Live(r) => r.read(buf),
    });
    match result {
        Some((Ok(n), _)) => n as i64,
        Some((Err(e), cancelled)) => failed(e, cancelled),
        None => -1,
    }
}

unsafe extern "C" fn seek(cookie: *mut c_void, offset: i64) -> i64 {
    let result = with_reader(cookie, |reader| match reader {
        Reader::Chunked(r) => r.seek(offset.max(0) as u64),
        Reader::Live(_) => Err(std::io::ErrorKind::Unsupported.into()),
    });
    match result {
        Some((Ok(position), _)) => position as i64,
        _ => i64::from(ffi::ERROR_GENERIC),
    }
}

unsafe extern "C" fn size(cookie: *mut c_void) -> i64 {
    let result = with_reader(cookie, |reader| match reader {
        Reader::Chunked(r) => r.size(),
        Reader::Live(_) => Err(std::io::ErrorKind::Unsupported.into()),
    });
    match result {
        Some((Ok(size), _)) => size as i64,
        _ => i64::from(ffi::ERROR_UNSUPPORTED),
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
