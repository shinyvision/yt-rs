//! Reading a live stream.
//!
//! YouTube serves a live stream as numbered segments of a couple of seconds
//! each (`&sq=N` on the stream URL). Each segment is a complete little file
//! (fragmented MP4 or WebM) with continuous timestamps, so the first one,
//! followed by the media of the others without their repeated headers, is
//! one continuous file. A [`LiveReader`] starts at a segment (or a few
//! behind the newest), fetches them one after another, waits for the next
//! while it isn't out yet, and hands out the bytes in order. It blocks, for
//! players that read on their own threads (mpv's stream callbacks).
//!
//! Every answer says which segment is the newest and where it starts in the
//! stream's time ([`LiveHead`]), which places the live edge, and turns a
//! time to seek to into a segment to start at ([`LiveHead::segment_at`]).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How many segments behind the newest to start: a few seconds of buffer.
const START_BEHIND: u64 = 3;
/// After this long without a new segment, the stream has ended.
const ENDED_AFTER: Duration = Duration::from_secs(45);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// The newest segment of a live stream, when it was last heard of.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiveHead {
    pub sequence: u64,
    /// Where it starts, in the stream's time (seconds).
    pub time: f64,
    pub seen: Instant,
}

impl LiveHead {
    /// The live edge now, in the stream's time, for segments of `segment`
    /// seconds: the end of the newest segment then, plus the time since.
    pub fn edge(&self, segment: f64) -> f64 {
        self.time + segment + self.seen.elapsed().as_secs_f64()
    }

    /// The segment that holds `time`, counted back from the newest (the
    /// stream's time drifts from `number × length` over hours).
    pub fn segment_at(&self, time: f64, segment: f64) -> u64 {
        let behind = ((self.time - time) / segment.max(0.1)).ceil().max(0.0) as u64;
        self.sequence.saturating_sub(behind)
    }

    /// Where the stream's first segment starts.
    pub fn first(&self, segment: f64) -> f64 {
        (self.time - self.sequence as f64 * segment).max(0.0)
    }
}

/// The newest segment as the readers of a stream last heard: shared with
/// whoever shows the stream.
pub type SharedHead = Arc<Mutex<Option<LiveHead>>>;

pub struct LiveReader {
    http: reqwest::Client,
    url: String,
    user_agent: Option<String>,
    segment: Duration,
    runtime: tokio::runtime::Handle,
    /// The segment to start at; `None` for a few behind the newest.
    start: Option<u64>,
    head: SharedHead,
    next: Option<u64>,
    buffer: Vec<u8>,
    position: usize,
    cancelled: Arc<AtomicBool>,
    /// A stream that has ended (see [`LiveReader::recorded`]).
    recorded: bool,
}

impl LiveReader {
    /// Reads the live stream at `url`, whose segments are `segment` seconds,
    /// from segment `start` (or near the newest), keeping `head` current.
    pub fn new(
        http: reqwest::Client,
        url: String,
        user_agent: Option<String>,
        segment: f64,
        start: Option<u64>,
        head: SharedHead,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        Self {
            http,
            url,
            user_agent,
            segment: Duration::from_secs_f64(segment.clamp(0.5, 30.0)),
            runtime,
            start,
            head,
            next: None,
            buffer: Vec::new(),
            position: 0,
            cancelled: Arc::default(),
            recorded: false,
        }
    }

    /// For a stream that has ended (a live stream's recording, still in
    /// segments): `None` as the start means the first segment, and it ends
    /// after the last instead of waiting for more.
    pub fn recorded(mut self) -> Self {
        self.recorded = true;
        self
    }

    /// Set to stop a read that is waiting, from another thread.
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancelled.clone()
    }

    /// Fills `buf` with the stream's next bytes, waiting for the next
    /// segment if needed. 0 means the stream has ended (or was cancelled).
    pub fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.position >= self.buffer.len() {
            if !self.fetch_next()? {
                return Ok(0);
            }
        }
        let n = buf.len().min(self.buffer.len() - self.position);
        buf[..n].copy_from_slice(&self.buffer[self.position..self.position + n]);
        self.position += n;
        Ok(n)
    }

    /// Loads the next segment into the buffer; false at the end.
    fn fetch_next(&mut self) -> std::io::Result<bool> {
        let next = match (self.next, self.start) {
            (Some(next), _) => next,
            (None, Some(start)) => start,
            (None, None) if self.recorded => 0,
            (None, None) => {
                let (_, head) = self.get(None)?;
                let head = head.ok_or_else(|| {
                    std::io::Error::other("YouTube didn't say where the live stream is")
                })?;
                head.saturating_sub(START_BEHIND)
            }
        };
        let waiting_since = Instant::now();
        loop {
            if self.cancelled.load(Ordering::Relaxed) {
                return Ok(false);
            }
            let (segment, head) = self.get(Some(next))?;
            if let Some(body) = segment {
                // The first segment's header describes the whole stream.
                self.position = if self.next.is_some() {
                    media_start(&body)
                } else {
                    0
                };
                self.buffer = body;
                self.next = Some(next + 1);
                return Ok(true);
            }
            // A recording ends with its last segment.
            if self.recorded && head.is_some_and(|head| next > head) {
                return Ok(false);
            }
            // Fell too far behind (paused for long): catch up.
            if let Some(head) = head
                && head > next + 1000
                && !self.recorded
            {
                self.next = Some(head.saturating_sub(START_BEHIND));
                return self.fetch_next();
            }
            if waiting_since.elapsed() > ENDED_AFTER {
                return Ok(false);
            }
            self.sleep(self.segment / 2);
        }
    }

    /// Fetches a segment (or the newest, with `None`): its bytes if it is
    /// out, and the newest segment's number.
    fn get(&self, sq: Option<u64>) -> std::io::Result<(Option<Vec<u8>>, Option<u64>)> {
        let url = match sq {
            Some(sq) => format!("{}&sq={sq}", self.url),
            None => self.url.clone(),
        };
        let mut request = self.http.get(url).timeout(REQUEST_TIMEOUT);
        if let Some(agent) = &self.user_agent {
            request = request.header(reqwest::header::USER_AGENT, agent);
        }
        let shared = self.head.clone();
        self.runtime.block_on(async move {
            let resp = request.send().await.map_err(std::io::Error::other)?;
            let header = |name: &str| {
                resp.headers()
                    .get(name)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
            };
            let head = header("x-head-seqnum");
            if let (Some(sequence), Some(millis)) = (head, header("x-head-time-millis")) {
                *shared.lock().unwrap() = Some(LiveHead {
                    sequence,
                    time: millis as f64 / 1000.0,
                    seen: Instant::now(),
                });
            }
            let status = resp.status();
            if status.is_success() && status != reqwest::StatusCode::NO_CONTENT {
                let body = resp.bytes().await.map_err(std::io::Error::other)?;
                return Ok((Some(body.to_vec()), head));
            }
            // Not out yet.
            if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::NO_CONTENT
            {
                return Ok((None, head));
            }
            Err(std::io::Error::other(format!(
                "YouTube answered {status} for a live segment"
            )))
        })
    }

    fn sleep(&self, total: Duration) {
        let until = Instant::now() + total;
        while Instant::now() < until && !self.cancelled.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(50).min(until - Instant::now()));
        }
    }
}

/// Where a segment's media starts, after its file header: the first `moof`
/// box of a fragmented MP4, or the first Cluster of a WebM. 0 if it can't
/// tell.
fn media_start(segment: &[u8]) -> usize {
    if segment.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        webm_media_start(segment)
    } else {
        mp4_media_start(segment)
    }
    .unwrap_or(0)
}

fn mp4_media_start(data: &[u8]) -> Option<usize> {
    let mut at = 0;
    while at + 8 <= data.len() {
        let size = u32::from_be_bytes(data[at..at + 4].try_into().ok()?) as usize;
        if &data[at + 4..at + 8] == b"moof" {
            return Some(at);
        }
        let size = match size {
            // A 64-bit size follows the type.
            1 => u64::from_be_bytes(data.get(at + 8..at + 16)?.try_into().ok()?) as usize,
            0 => return None,
            size => size,
        };
        at = at.checked_add(size)?;
    }
    None
}

/// The EBML header, then the Segment's children up to its first Cluster.
fn webm_media_start(data: &[u8]) -> Option<usize> {
    const SEGMENT: u64 = 0x1853_8067;
    const CLUSTER: u64 = 0x1F43_B675;
    let mut at = 0;
    while at < data.len() {
        let (id, id_len) = vint(&data[at..], true)?;
        let id = id?;
        if id == CLUSTER {
            return Some(at);
        }
        let (size, size_len) = vint(&data[at + id_len..], false)?;
        at += id_len + size_len;
        // Into the Segment (its size is often "unknown"); past anything else.
        if id != SEGMENT {
            at = at.checked_add(usize::try_from(size?).ok()?)?;
        }
    }
    None
}

/// An EBML variable-length number: its value (`None` for "unknown") and
/// length. IDs keep their length marker.
fn vint(data: &[u8], keep_marker: bool) -> Option<(Option<u64>, usize)> {
    let first = *data.first()?;
    let len = first.leading_zeros() as usize + 1;
    if len > 8 || data.len() < len {
        return None;
    }
    let marker_mask = (0xFF_u16 >> len) as u8;
    let mut value = u64::from(if keep_marker {
        first
    } else {
        first & marker_mask
    });
    for byte in &data[1..len] {
        value = value << 8 | u64::from(*byte);
    }
    let unknown = !keep_marker && value == (1u64 << (7 * len)) - 1;
    Some((if unknown { None } else { Some(value) }, len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeking_counts_back_from_the_newest_segment() {
        let head = LiveHead {
            sequence: 5887,
            time: 11773.5,
            seen: Instant::now(),
        };
        assert_eq!(head.segment_at(11773.5, 2.0), 5887);
        assert_eq!(
            head.segment_at(11770.0, 2.0),
            5885,
            "1.75 segments back: the one holding it"
        );
        assert_eq!(head.segment_at(0.0, 2.0), 0);
        assert_eq!(
            head.segment_at(99999.0, 2.0),
            5887,
            "past the edge: the newest"
        );
        assert!((head.first(2.0) - 0.0).abs() < 1e-9);
        assert!(head.edge(2.0) >= 11775.5);
    }

    fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut b = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(kind);
        b.extend_from_slice(body);
        b
    }

    #[test]
    fn mp4_segments_lose_their_header() {
        let mut segment = mp4_box(b"ftyp", b"dash");
        segment.extend(mp4_box(b"moov", &[0; 40]));
        let media = segment.len();
        segment.extend(mp4_box(b"moof", &[1; 16]));
        segment.extend(mp4_box(b"mdat", &[2; 32]));
        assert_eq!(media_start(&segment), media);
        assert_eq!(media_start(b"no boxes here"), 0);
    }

    #[test]
    fn webm_segments_lose_their_header() {
        // EBML header (size 3), Segment (unknown size), Info (size 2), then
        // a Cluster.
        let mut segment = vec![0x1A, 0x45, 0xDF, 0xA3, 0x83, 1, 2, 3];
        segment.extend([
            0x18, 0x53, 0x80, 0x67, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        ]);
        segment.extend([0x15, 0x49, 0xA9, 0x66, 0x82, 9, 9]);
        let media = segment.len();
        segment.extend([0x1F, 0x43, 0xB6, 0x75, 0x81, 0]);
        assert_eq!(media_start(&segment), media);
    }

    #[test]
    fn ebml_numbers() {
        assert_eq!(vint(&[0x81], false), Some((Some(1), 1)));
        assert_eq!(vint(&[0x40, 0x02], false), Some((Some(2), 2)));
        assert_eq!(vint(&[0xFF], false), Some((None, 1)), "unknown size");
        assert_eq!(
            vint(&[0x1A, 0x45, 0xDF, 0xA3], true),
            Some((Some(0x1A45_DFA3), 4))
        );
    }
}
