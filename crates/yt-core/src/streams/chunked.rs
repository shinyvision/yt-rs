//! Reads a stream from googlevideo in bounded ranges. YouTube throttles an
//! open-ended request (`Range: bytes=N-`) to about twice the stream's
//! bitrate, and holds back the first bytes of a very low-bitrate one for
//! longer than anyone waits, while a bounded range comes at full speed
//! (10 MB in a quarter of a second). So a stream is read [`CHUNK`] bytes at
//! a time, as yt-dlp does. Bytes are handed on as they arrive: the chunk
//! size doesn't delay the first ones.
//!
//! A reader gets over a lost connection by asking again from where it was,
//! and over a refused URL (expired, or held back again) by asking its
//! `refresh` for a new one.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use reqwest::StatusCode;
use reqwest::header::{CONTENT_RANGE, RANGE, USER_AGENT};

/// Bytes asked for at once.
pub const CHUNK: u64 = 10 << 20;
/// No bytes for this long: ask again.
const STALLED_AFTER: Duration = Duration::from_secs(15);
/// Failures in a row (with no bytes in between) before giving up, waiting
/// longer after each: about 15 s in all.
const ATTEMPTS: u32 = 7;
/// A chunk may take this long in all (the client's own limit is for API
/// calls); a stalled one is noticed sooner (`STALLED_AFTER`).
const CHUNK_TIMEOUT: Duration = Duration::from_secs(600);
/// How often a wait checks whether it was cancelled.
const POLL: Duration = Duration::from_millis(100);

/// A new URL for the same stream, when YouTube refuses the one in use.
pub type Refresh =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Option<String>> + Send>> + Send + Sync>;

pub struct ChunkedReader {
    http: reqwest::Client,
    url: String,
    user_agent: Option<String>,
    refresh: Option<Refresh>,
    runtime: tokio::runtime::Handle,
    /// The stream's length, once known.
    size: Option<u64>,
    position: u64,
    body: Option<Body>,
    cancelled: Arc<AtomicBool>,
}

/// The response for the range being read.
struct Body {
    response: reqwest::Response,
    /// Arrived and not read yet.
    pending: Bytes,
    /// Where the range ends (exclusive).
    end: u64,
}

impl ChunkedReader {
    pub fn new(
        http: reqwest::Client,
        url: String,
        user_agent: Option<String>,
        refresh: Option<Refresh>,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        Self {
            size: content_length(&url),
            http,
            url,
            user_agent,
            refresh,
            runtime,
            position: 0,
            body: None,
            cancelled: Arc::default(),
        }
    }

    /// Set to stop a read that is waiting, from another thread. Reads end
    /// from then on.
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancelled.clone()
    }

    /// The stream's length in bytes: from the URL (`clen`), or asked for.
    pub fn size(&mut self) -> io::Result<u64> {
        if let Some(size) = self.size {
            return Ok(size);
        }
        self.open(0, 1)?;
        self.body = None;
        self.size
            .ok_or_else(|| io::Error::other("YouTube didn't say how long the stream is"))
    }

    /// Reads from `position` on. Nothing is fetched until the next read.
    pub fn seek(&mut self, position: u64) -> io::Result<u64> {
        if position != self.position {
            self.body = None;
            self.position = position;
        }
        Ok(position)
    }

    /// Fills `buf` with the next bytes; 0 at the end (or once cancelled).
    pub fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut failures = 0;
        loop {
            if buf.is_empty() || self.is_cancelled() {
                return Ok(0);
            }
            if self.size.is_some_and(|size| self.position >= size) {
                return Ok(0);
            }
            let Some(mut body) = self.body.take() else {
                let end = self.size.map_or(self.position + CHUNK, |size| {
                    size.min(self.position + CHUNK)
                });
                match self.open(self.position, end - self.position) {
                    Ok(true) => {}
                    // Past the end.
                    Ok(false) => return Ok(0),
                    Err(e) => self.failed(e, &mut failures)?,
                }
                continue;
            };
            if !body.pending.is_empty() {
                let n = buf.len().min(body.pending.len());
                buf[..n].copy_from_slice(&body.pending.split_to(n));
                self.position += n as u64;
                self.body = Some(body);
                return Ok(n);
            }
            if self.position >= body.end {
                continue;
            }
            match self.wait(|| body.response.chunk(), STALLED_AFTER) {
                Ok(Ok(Some(bytes))) => {
                    failures = 0;
                    body.pending = bytes;
                    self.body = Some(body);
                }
                // The connection closed before the range was all there.
                Ok(Ok(None)) => self.failed(
                    io::Error::new(io::ErrorKind::UnexpectedEof, "the connection closed"),
                    &mut failures,
                )?,
                Ok(Err(e)) => self.failed(io::Error::other(e), &mut failures)?,
                Err(e) => self.failed(e, &mut failures)?,
            }
        }
    }

    /// Counts a failure: waits before the next attempt, or gives up.
    fn failed(&self, error: io::Error, failures: &mut u32) -> io::Result<()> {
        *failures += 1;
        if *failures >= ATTEMPTS || self.is_cancelled() {
            return Err(error);
        }
        tracing::debug!("stream: {error}; asking again");
        let backoff = Duration::from_millis(250 << (*failures - 1).min(4));
        let _ = self.wait(|| tokio::time::sleep(backoff), backoff + POLL);
        Ok(())
    }

    /// Asks for `length` bytes from `start`; false if that is past the end.
    fn open(&mut self, start: u64, length: u64) -> io::Result<bool> {
        let mut refreshed = false;
        loop {
            let mut request = self
                .http
                .get(&self.url)
                .header(RANGE, format!("bytes={start}-{}", start + length - 1))
                .timeout(CHUNK_TIMEOUT);
            if let Some(agent) = &self.user_agent {
                request = request.header(USER_AGENT, agent);
            }
            let response = self
                .wait(|| request.send(), STALLED_AFTER)?
                .map_err(io::Error::other)?;
            let status = response.status();
            match status {
                StatusCode::PARTIAL_CONTENT => {
                    let total = response
                        .headers()
                        .get(CONTENT_RANGE)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.rsplit_once('/'))
                        .and_then(|(_, total)| total.parse::<u64>().ok());
                    if total.is_some() {
                        self.size = total;
                    }
                    self.body = Some(Body {
                        response,
                        pending: Bytes::new(),
                        end: start + length,
                    });
                    return Ok(true);
                }
                // The whole stream, which is what was asked for only from
                // the start.
                StatusCode::OK if start == 0 => {
                    self.size = self.size.or(response.content_length());
                    self.body = Some(Body {
                        response,
                        pending: Bytes::new(),
                        end: u64::MAX,
                    });
                    return Ok(true);
                }
                StatusCode::RANGE_NOT_SATISFIABLE => return Ok(false),
                StatusCode::FORBIDDEN | StatusCode::GONE if !refreshed => {
                    let Some(refresh) = self.refresh.clone() else {
                        return Err(refused(status));
                    };
                    tracing::info!("stream: YouTube answered {status}; getting a new URL");
                    match self.wait(|| refresh(), Duration::from_secs(90))? {
                        Some(url) => {
                            self.size = self.size.or(content_length(&url));
                            self.url = url;
                            refreshed = true;
                        }
                        None => return Err(refused(status)),
                    }
                }
                status => return Err(refused(status)),
            }
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// Runs the future `start` makes to its end, unless it takes longer
    /// than `limit` or the reader is cancelled. It's made on the runtime,
    /// which some futures need from the start.
    fn wait<F: Future>(&self, start: impl FnOnce() -> F, limit: Duration) -> io::Result<F::Output> {
        let cancelled = self.cancelled.clone();
        self.runtime.block_on(async move {
            let mut future = std::pin::pin!(start());
            let deadline = Instant::now() + limit;
            loop {
                if let Ok(value) = tokio::time::timeout(POLL, &mut future).await {
                    return Ok(value);
                }
                if cancelled.load(Ordering::Relaxed) {
                    return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
                }
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "YouTube stopped sending",
                    ));
                }
            }
        })
    }
}

fn refused(status: StatusCode) -> io::Error {
    io::Error::other(format!("YouTube answered {status} for the stream"))
}

/// The length googlevideo URLs carry (`clen`).
pub fn content_length(url: &str) -> Option<u64> {
    url::Url::parse(url)
        .ok()?
        .query_pairs()
        .find(|(key, _)| key == "clen")?
        .1
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::Mutex;

    /// The stream's bytes: its position, mod 251.
    fn byte(at: u64) -> u8 {
        (at % 251) as u8
    }

    /// What the test server saw and does.
    #[derive(Default)]
    struct Server {
        ranges: Vec<String>,
        /// Answer 403 to this many requests first.
        refuse: usize,
        /// Close the connection after this many bytes, once.
        drop_after: Option<usize>,
        /// Ignore requests without a range end (as YouTube throttles them).
        honor_open_ranges: bool,
    }

    /// A googlevideo stand-in serving `size` bytes with ranges, on its own
    /// thread. Returns its URL.
    fn serve(size: u64, server: Arc<Mutex<Server>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let server = server.clone();
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut range = None;
                    let mut line = String::new();
                    while reader.read_line(&mut line).unwrap_or(0) > 2 {
                        if let Some(value) = line.to_ascii_lowercase().strip_prefix("range: bytes=")
                        {
                            range = Some(value.trim().to_owned());
                        }
                        line.clear();
                    }
                    let mut s = server.lock().unwrap();
                    s.ranges.push(range.clone().unwrap_or_default());
                    if s.refuse > 0 {
                        s.refuse -= 1;
                        let _ = stream.write_all(
                            b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        );
                        return;
                    }
                    let (start, end) = match range.as_deref().and_then(|r| r.split_once('-')) {
                        Some((a, "")) if !s.honor_open_ranges => {
                            // Hold back, like YouTube.
                            drop(s);
                            std::thread::sleep(Duration::from_secs(30));
                            let _ = a;
                            return;
                        }
                        Some((a, b)) => (
                            a.parse::<u64>().unwrap(),
                            b.parse::<u64>().map_or(size - 1, |b| b.min(size - 1)),
                        ),
                        None => (0, size - 1),
                    };
                    if start >= size {
                        let _ = stream.write_all(b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                        return;
                    }
                    let body: Vec<u8> = (start..=end).map(byte).collect();
                    let cut = s.drop_after.take();
                    drop(s);
                    let head = format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{size}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(head.as_bytes());
                    let _ = stream.write_all(&body[..cut.unwrap_or(body.len()).min(body.len())]);
                });
            }
        });
        format!("http://{address}/videoplayback?itag=251")
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap()
    }

    fn read_all(reader: &mut ChunkedReader, limit: usize) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        let mut buf = vec![0; 64 << 10];
        while out.len() < limit {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
        Ok(out)
    }

    fn reader(
        url: String,
        refresh: Option<Refresh>,
        rt: &tokio::runtime::Runtime,
    ) -> ChunkedReader {
        // The app's client: its timeouts need the runtime when a request
        // is made, which the reader's thread isn't on.
        ChunkedReader::new(
            crate::http_client(),
            url,
            None,
            refresh,
            rt.handle().clone(),
        )
    }

    #[test]
    fn reads_in_bounded_chunks() {
        let rt = runtime();
        let size = CHUNK * 2 + 12_345;
        let server = Arc::new(Mutex::new(Server::default()));
        let mut r = reader(serve(size, server.clone()), None, &rt);
        assert_eq!(r.size().unwrap(), size);
        let data = read_all(&mut r, usize::MAX).unwrap();
        assert_eq!(data.len() as u64, size);
        assert!(data.iter().enumerate().all(|(i, b)| *b == byte(i as u64)));
        let ranges = server.lock().unwrap().ranges.clone();
        // The size, then three chunks, the last up to the end; never an
        // open-ended range.
        assert_eq!(
            ranges,
            vec![
                "0-0".to_owned(),
                format!("0-{}", CHUNK - 1),
                format!("{CHUNK}-{}", 2 * CHUNK - 1),
                format!("{}-{}", 2 * CHUNK, size - 1),
            ]
        );
    }

    #[test]
    fn knows_the_size_from_the_url() {
        let rt = runtime();
        let server = Arc::new(Mutex::new(Server::default()));
        let url = serve(1000, server.clone()) + "&clen=1000";
        let mut r = reader(url, None, &rt);
        assert_eq!(r.size().unwrap(), 1000);
        assert!(server.lock().unwrap().ranges.is_empty());
        assert_eq!(read_all(&mut r, usize::MAX).unwrap().len(), 1000);
        assert_eq!(server.lock().unwrap().ranges, vec!["0-999".to_owned()]);
    }

    #[test]
    fn seeks() {
        let rt = runtime();
        let size = 5_000_000;
        let server = Arc::new(Mutex::new(Server::default()));
        let mut r = reader(serve(size, server.clone()), None, &rt);
        read_all(&mut r, 1000).unwrap();
        assert_eq!(r.seek(4_000_000).unwrap(), 4_000_000);
        let data = read_all(&mut r, usize::MAX).unwrap();
        assert_eq!(data.len(), 1_000_000);
        assert_eq!(data[0], byte(4_000_000));
        // Past the end: nothing, and no error.
        r.seek(size + 10).unwrap();
        assert_eq!(read_all(&mut r, usize::MAX).unwrap().len(), 0);
    }

    #[test]
    fn carries_on_after_a_lost_connection() {
        let rt = runtime();
        let size = 3_000_000;
        let server = Arc::new(Mutex::new(Server {
            drop_after: Some(1_000_000),
            ..Server::default()
        }));
        let mut r = reader(serve(size, server.clone()), None, &rt);
        let data = read_all(&mut r, usize::MAX).unwrap();
        assert_eq!(data.len() as u64, size);
        assert!(data.iter().enumerate().all(|(i, b)| *b == byte(i as u64)));
        let ranges = server.lock().unwrap().ranges.clone();
        // Asked again from where it broke off.
        assert_eq!(ranges.last().unwrap(), &format!("1000000-{}", size - 1));
    }

    #[test]
    fn gets_a_new_url_when_refused() {
        let rt = runtime();
        let server = Arc::new(Mutex::new(Server {
            refuse: 1,
            ..Server::default()
        }));
        let url = serve(2000, server.clone());
        let fresh = url.clone() + "&fresh=1";
        let asked = Arc::new(AtomicBool::new(false));
        let flag = asked.clone();
        let refresh: Refresh = Arc::new(move || {
            flag.store(true, Ordering::Relaxed);
            let fresh = fresh.clone();
            Box::pin(async move { Some(fresh) })
        });
        let mut r = reader(url, Some(refresh), &rt);
        assert_eq!(read_all(&mut r, usize::MAX).unwrap().len(), 2000);
        assert!(asked.load(Ordering::Relaxed));
    }

    #[test]
    fn gives_up_when_refused_again() {
        let rt = runtime();
        let server = Arc::new(Mutex::new(Server {
            refuse: 100,
            ..Server::default()
        }));
        let url = serve(2000, server.clone());
        let again = url.clone();
        let refresh: Refresh = Arc::new(move || {
            let again = again.clone();
            Box::pin(async move { Some(again) })
        });
        let mut r = reader(url, Some(refresh), &rt);
        let started = Instant::now();
        let error = read_all(&mut r, usize::MAX).unwrap_err();
        assert!(error.to_string().contains("403"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(30));
    }

    #[test]
    fn a_download_stops_and_carries_on() {
        let rt = runtime();
        let size = 3_000_000;
        let server = Arc::new(Mutex::new(Server::default()));
        let url = serve(size, server);
        let dir = std::env::temp_dir().join(format!("yt-rs-test-{}", std::process::id()));
        let path = dir.join("video.webm");
        let _ = std::fs::remove_file(&path);
        // Cancelled after a megabyte or so.
        let stopped = crate::downloads::fetch(reader(url.clone(), None, &rt), &path, |saved, _| {
            saved < 1_000_000
        });
        assert!(stopped.is_err());
        let partial = std::fs::metadata(&path).unwrap().len();
        assert!(partial >= 1_000_000 && partial < size, "{partial}");
        // Then from there to the end.
        let mut calls = 0;
        let saved = crate::downloads::fetch(reader(url, None, &rt), &path, |_, total| {
            calls += 1;
            assert_eq!(total, size);
            true
        })
        .unwrap();
        assert_eq!(saved, size);
        assert!(calls > 1);
        let data = std::fs::read(&path).unwrap();
        assert_eq!(data.len() as u64, size);
        assert!(data.iter().enumerate().all(|(i, b)| *b == byte(i as u64)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_cancelled_read_ends() {
        let rt = runtime();
        // The server holds back open-ended ranges; the reader never asks
        // for one, so make it wait on a refusal that never resolves.
        let server = Arc::new(Mutex::new(Server {
            refuse: 1,
            ..Server::default()
        }));
        let url = serve(2000, server);
        let refresh: Refresh =
            Arc::new(|| Box::pin(async { std::future::pending::<Option<String>>().await }));
        let mut r = reader(url, Some(refresh), &rt);
        let cancel = r.cancel_flag();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            cancel.store(true, Ordering::Relaxed);
        });
        let started = Instant::now();
        let mut buf = [0; 16];
        let result = r.read(&mut buf);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(matches!(result, Ok(0)) || result.is_err());
        // And it stays ended.
        assert_eq!(r.read(&mut buf).unwrap(), 0);
    }
}
