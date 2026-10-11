//! Finding a video's streams, through the TV app's sign-in.
//!
//! The streams come from the same signed-in `player` request the TV app
//! makes ([`InnerTube::player_streams`]), so YouTube treats the lookup as
//! the user watching, without "confirm you're not a bot" checks. Their URLs
//! carry challenges that YouTube's player script solves, which
//! [`crate::solver`] runs. The formats are picked here: the best video up to
//! 1080p (VP9 before H.264 before AV1, which many GPUs can't decode in
//! hardware yet), the best audio in the video's original language, and a
//! video stream per quality to switch to. The response also has the
//! captions and the seek bar's preview pictures.
//!
//! Live streams come as numbered segments of a couple of seconds rather
//! than one file; [`live::LiveReader`] reads them as one stream.
//!
//! Lookups are cached until the stream URLs expire, concurrent lookups of the
//! same video share one result, and [`Resolver::prefetch`] lets the UI look a
//! video up before it is clicked. Clicks jump ahead of queued prefetches.

pub mod chunked;
pub mod live;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use crate::channel::array;
use crate::error::{Error, Result};
use crate::feed::{str_at, text};
use crate::innertube::{InnerTube, TV_DOWNGRADED};
use crate::solver::{Challenges, Solutions, Solver};

const MAX_CACHED: usize = 256;
/// Stream URLs are used until this long before they expire.
const EXPIRY_MARGIN: Duration = Duration::from_secs(300);
/// How often to check whether YouTube serves streams it held back, and for
/// how long past the time it said.
const SERVED_CHECK_EVERY: Duration = Duration::from_millis(400);
const SERVED_GRACE: Duration = Duration::from_secs(3);

/// The streams to play for one video: a video stream and, unless they come
/// combined, a separate audio stream.
#[derive(Clone, Debug, Default)]
pub struct StreamSet {
    pub title: Option<String>,
    pub duration: Option<f64>,
    pub live: bool,
    /// A live stream that just ended, as YouTube serves it until it has
    /// made a plain video of it: of known length, but read in segments as
    /// when it was live (its URLs answer no byte ranges).
    pub recorded: bool,
    /// How far back a live stream can be watched, in seconds.
    pub dvr: Option<f64>,
    pub video: Stream,
    pub audio: Option<Stream>,
    /// Video streams to switch to, one per height and frame rate, best
    /// first. They play with the same audio stream.
    pub qualities: Vec<Stream>,
    pub captions: Vec<Caption>,
    pub storyboard: Option<Storyboard>,
    /// YouTube refuses the streams until about then, the time a pre-roll
    /// ad takes to become skippable (see [`preroll`] and
    /// [`Resolver::until_served`]).
    pub available_at: Option<Instant>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Stream {
    pub url: String,
    /// Headers to send when fetching the stream.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub vcodec: Option<String>,
    #[serde(default)]
    pub acodec: Option<String>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub fps: Option<f64>,
    /// YouTube's name for the quality ("1080p60"), which goes by the
    /// width class too: a 1920×800 video is "1080p".
    #[serde(default)]
    pub label: Option<String>,
    /// A live stream's segment length, in seconds: it is read segment by
    /// segment (see [`live::LiveReader`]).
    #[serde(default)]
    pub segment_duration: Option<f64>,
}

/// Gets a new URL for a video's stream of format `itag`, for a reader whose
/// URL YouTube refused (expired, or held back again): the video looked up
/// again, once YouTube serves it.
pub fn refresher(resolver: Arc<Resolver>, video_id: String, itag: u32) -> chunked::Refresh {
    Arc::new(move || {
        let (resolver, video_id) = (resolver.clone(), video_id.clone());
        Box::pin(async move {
            let fresh = resolver.resolve_fresh(&video_id).await.ok()?;
            resolver.until_served(&fresh).await;
            Some(fresh.by_itag(itag)?.url.clone())
        })
    })
}

impl StreamSet {
    /// The stream of this format (`itag`), among the video, audio and
    /// qualities.
    pub fn by_itag(&self, itag: u32) -> Option<&Stream> {
        std::iter::once(&self.video)
            .chain(&self.audio)
            .chain(&self.qualities)
            .find(|s| s.itag() == Some(itag))
    }
}

impl Stream {
    /// YouTube's number for the stream's format, from its URL.
    pub fn itag(&self) -> Option<u32> {
        url::Url::parse(&self.url)
            .ok()?
            .query_pairs()
            .find(|(key, _)| key == "itag")?
            .1
            .parse()
            .ok()
    }

    /// As YouTube names qualities: "1080p60", "720p".
    pub fn quality_label(&self) -> Option<String> {
        if let Some(label) = &self.label {
            return Some(label.clone());
        }
        let height = self.height?;
        let fps = self.fps.filter(|f| *f > 30.0).map(|f| f.round() as u32);
        Some(match fps {
            Some(fps) => format!("{height}p{fps}"),
            None => format!("{height}p"),
        })
    }
}

/// A subtitle track: uploaded, or YouTube's automatic captions.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Caption {
    /// Language code, e.g. "en" or "pt-BR".
    pub lang: String,
    /// The language's name, e.g. "English".
    pub name: String,
    /// The track in YouTube's json3 format; see [`crate::captions`].
    pub url: String,
    /// Generated by speech recognition.
    #[serde(default)]
    pub auto: bool,
}

impl Caption {
    /// As YouTube's menu shows it: "English (auto-generated)".
    pub fn label(&self) -> String {
        if self.auto {
            format!("{} (auto-generated)", self.name)
        } else {
            self.name.clone()
        }
    }
}

/// The seek bar's preview pictures: each fragment is a picture of
/// `rows × columns` frames of `width × height`, together covering
/// `duration` seconds of the video.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Storyboard {
    pub width: u32,
    pub height: u32,
    pub rows: u32,
    pub columns: u32,
    pub fragments: Vec<StoryboardFragment>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct StoryboardFragment {
    pub url: String,
    pub duration: f64,
}

impl Storyboard {
    /// The fragment holding the frame for `time` seconds, and which frame of
    /// it (row, column).
    pub fn frame_at(&self, time: f64) -> Option<(&StoryboardFragment, u32, u32)> {
        let per_fragment = (self.rows * self.columns).max(1);
        let mut start = 0.0;
        for fragment in &self.fragments {
            let end = start + fragment.duration;
            if time < end || std::ptr::eq(fragment, self.fragments.last()?) {
                let step = fragment.duration / per_fragment as f64;
                let index = if step > 0.0 {
                    ((time - start).max(0.0) / step) as u32
                } else {
                    0
                };
                let index = index.min(per_fragment - 1);
                return Some((
                    fragment,
                    index / self.columns.max(1),
                    index % self.columns.max(1),
                ));
            }
            start = end;
        }
        None
    }
}

#[derive(Clone, Debug)]
pub struct ResolverOptions {
    /// The tallest video picked automatically; the others stay available as
    /// qualities.
    pub max_height: u32,
    /// Video codecs, best first: prefixes of a format's codec name.
    pub video_codecs: Vec<String>,
    pub audio_codecs: Vec<String>,
    /// Lookups at the same time.
    pub concurrency: usize,
    pub timeout: Duration,
}

impl Default for ResolverOptions {
    fn default() -> Self {
        Self {
            max_height: 1080,
            video_codecs: vec!["vp9".into(), "vp09".into(), "avc1".into(), "av01".into()],
            audio_codecs: vec!["opus".into(), "mp4a".into()],
            concurrency: 4,
            timeout: Duration::from_secs(60),
        }
    }
}

type Outcome = std::result::Result<Arc<StreamSet>, Failure>;

/// A failed lookup, for every caller waiting on it (errors don't clone).
#[derive(Clone, Debug)]
enum Failure {
    Unplayable {
        reason: String,
        why: crate::error::Unplayable,
        channel_id: Option<String>,
    },
    Other(String),
}

impl From<Error> for Failure {
    fn from(e: Error) -> Self {
        match e {
            Error::Unplayable {
                reason,
                why,
                channel_id,
            } => Failure::Unplayable {
                reason,
                why,
                channel_id,
            },
            e => Failure::Other(e.to_string()),
        }
    }
}

impl From<Failure> for Error {
    fn from(f: Failure) -> Self {
        match f {
            Failure::Unplayable {
                reason,
                why,
                channel_id,
            } => Error::Unplayable {
                reason,
                why,
                channel_id,
            },
            Failure::Other(message) => Error::Streams(message),
        }
    }
}

struct Cached {
    streams: Arc<StreamSet>,
    expires: Instant,
    added: Instant,
}

#[derive(Default)]
struct State {
    cache: HashMap<String, Cached>,
    waiting: HashMap<String, Vec<oneshot::Sender<Outcome>>>,
    in_progress: HashSet<String>,
}

struct Queues {
    urgent: mpsc::UnboundedReceiver<String>,
    later: mpsc::UnboundedReceiver<String>,
}

/// What every lookup needs.
struct Lookup {
    youtube: Arc<InnerTube>,
    solver: Solver,
    options: ResolverOptions,
    /// Check for a newer player script before the next lookup.
    refresh_player: AtomicBool,
}

pub struct Resolver {
    state: Arc<Mutex<State>>,
    urgent: mpsc::UnboundedSender<String>,
    later: mpsc::UnboundedSender<String>,
    lookup: Arc<Lookup>,
    runtime: tokio::runtime::Handle,
}

impl Resolver {
    /// Starts the lookup workers on `runtime`.
    pub fn new(
        youtube: Arc<InnerTube>,
        options: ResolverOptions,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        let (urgent, urgent_rx) = mpsc::unbounded_channel();
        let (later, later_rx) = mpsc::unbounded_channel();
        let state = Arc::new(Mutex::new(State::default()));
        let queues = Arc::new(tokio::sync::Mutex::new(Queues {
            urgent: urgent_rx,
            later: later_rx,
        }));
        let lookup = Arc::new(Lookup {
            youtube,
            solver: Solver::new(crate::paths::cache_dir().join("solver")),
            options: options.clone(),
            refresh_player: AtomicBool::new(false),
        });
        for _ in 0..options.concurrency.max(1) {
            runtime.spawn(worker(state.clone(), queues.clone(), lookup.clone()));
        }
        Self {
            state,
            urgent,
            later,
            lookup,
            runtime,
        }
    }

    /// Gets the challenge solver ready for the current player script, so
    /// the first lookup doesn't wait for it.
    pub fn warm_up(&self) {
        let lookup = self.lookup.clone();
        self.runtime.spawn(async move {
            if let Err(e) = lookup.prepare().await {
                tracing::warn!("couldn't get the challenge solver ready: {e}");
            }
        });
    }

    /// Looks up the streams for a video, ahead of any queued prefetches.
    pub async fn resolve(&self, video_id: &str) -> Result<Arc<StreamSet>> {
        self.request(video_id, true).await
    }

    /// Looks a video up again, after YouTube refused its streams: with
    /// fresh URLs, and a newer player script if there is one.
    pub async fn resolve_fresh(&self, video_id: &str) -> Result<Arc<StreamSet>> {
        self.forget(video_id);
        self.lookup.refresh_player.store(true, Ordering::Relaxed);
        self.resolve(video_id).await
    }

    /// Waits until YouTube serves the streams. With a pre-roll ad it holds
    /// them back for about as long as the ad takes to become skippable (see
    /// [`preroll`]); this checks every 400 ms instead of waiting all that
    /// time, since YouTube usually lets go a little earlier.
    pub async fn until_served(&self, streams: &StreamSet) {
        let Some(at) = streams.available_at else {
            return;
        };
        let http = self.lookup.youtube.http().clone();
        let video = &streams.video;
        while Instant::now() < at + SERVED_GRACE {
            let mut request = http
                .get(&video.url)
                .header(reqwest::header::RANGE, "bytes=0-0");
            if let Some(agent) = video.headers.get("User-Agent") {
                request = request.header(reqwest::header::USER_AGENT, agent);
            }
            if request.send().await.is_ok_and(|r| r.status().is_success()) {
                return;
            }
            tokio::time::sleep(SERVED_CHECK_EVERY).await;
        }
    }

    /// Drops a video's cached streams.
    pub fn forget(&self, video_id: &str) {
        self.state.lock().unwrap().cache.remove(video_id);
    }

    /// Looks a video up in the background so that playing it later is
    /// instant. Does nothing if it is cached or already being looked up.
    pub fn prefetch(&self, video_id: &str) {
        let rx = self.enqueue(video_id, false);
        if let Some(rx) = rx {
            self.runtime.spawn(async move {
                let _ = rx.await;
            });
        }
    }

    async fn request(&self, video_id: &str, urgent: bool) -> Result<Arc<StreamSet>> {
        if let Some(streams) = self.cached(video_id) {
            return Ok(streams);
        }
        let Some(rx) = self.enqueue(video_id, urgent) else {
            return self
                .cached(video_id)
                .ok_or_else(|| Error::Streams("stream lookup failed".into()));
        };
        rx.await
            .map_err(|_| Error::Streams("stream lookup was cancelled".into()))?
            .map_err(Error::from)
    }

    fn cached(&self, video_id: &str) -> Option<Arc<StreamSet>> {
        let state = self.state.lock().unwrap();
        state
            .cache
            .get(video_id)
            .filter(|c| c.expires > Instant::now())
            .map(|c| c.streams.clone())
    }

    /// Registers interest in a lookup and queues it if needed. Returns
    /// `None` if the streams are already cached.
    fn enqueue(&self, video_id: &str, urgent: bool) -> Option<oneshot::Receiver<Outcome>> {
        let mut state = self.state.lock().unwrap();
        if state
            .cache
            .get(video_id)
            .is_some_and(|c| c.expires > Instant::now())
        {
            return None;
        }
        let (tx, rx) = oneshot::channel();
        let waiters = state.waiting.entry(video_id.to_owned()).or_default();
        let first = waiters.is_empty();
        waiters.push(tx);
        // An urgent request queues again even if a prefetch is waiting, so it
        // doesn't sit behind other prefetches; workers skip the duplicate.
        if (first || urgent) && !state.in_progress.contains(video_id) {
            let queue = if urgent { &self.urgent } else { &self.later };
            let _ = queue.send(video_id.to_owned());
        }
        Some(rx)
    }
}

async fn worker(
    state: Arc<Mutex<State>>,
    queues: Arc<tokio::sync::Mutex<Queues>>,
    lookup: Arc<Lookup>,
) {
    loop {
        let video_id = {
            let mut guard = queues.lock().await;
            let Queues { urgent, later } = &mut *guard;
            tokio::select! {
                biased;
                Some(id) = urgent.recv() => id,
                Some(id) = later.recv() => id,
                else => return,
            }
        };
        {
            let mut s = state.lock().unwrap();
            if !s.waiting.contains_key(&video_id) || s.in_progress.contains(&video_id) {
                continue;
            }
            s.in_progress.insert(video_id.clone());
        }

        let outcome =
            match tokio::time::timeout(lookup.options.timeout, lookup.streams(&video_id)).await {
                Ok(Ok((streams, lifetime))) => Ok((Arc::new(streams), lifetime)),
                Ok(Err(e)) => Err(Failure::from(e)),
                Err(_) => Err(Failure::Other(
                    "finding the streams took too long".to_owned(),
                )),
            };

        let waiters = {
            let mut s = state.lock().unwrap();
            s.in_progress.remove(&video_id);
            if let Ok((streams, lifetime)) = &outcome {
                let now = Instant::now();
                s.cache.insert(
                    video_id.clone(),
                    Cached {
                        streams: streams.clone(),
                        expires: now + *lifetime,
                        added: now,
                    },
                );
                if s.cache.len() > MAX_CACHED {
                    let oldest = s
                        .cache
                        .iter()
                        .min_by_key(|(_, c)| c.added)
                        .map(|(k, _)| k.clone());
                    oldest.map(|k| s.cache.remove(&k));
                }
            }
            s.waiting.remove(&video_id).unwrap_or_default()
        };
        if let Err(e) = &outcome {
            match e {
                Failure::Unplayable { reason, .. } => {
                    tracing::info!(video_id, reason, "YouTube won't play it")
                }
                Failure::Other(message) => {
                    tracing::warn!(video_id, error = %message, "stream lookup failed")
                }
            }
        }
        let outcome = outcome.map(|(streams, _)| streams);
        for waiter in waiters {
            let _ = waiter.send(outcome.clone());
        }
    }
}

impl Lookup {
    /// Loads the current player into the solver.
    async fn prepare(&self) -> Result<()> {
        let script = self.youtube.player_script(false).await?;
        self.solve(&script, Challenges::default()).await.map(|_| ())
    }

    async fn solve(
        &self,
        script: &crate::innertube::PlayerScript,
        challenges: Challenges,
    ) -> Result<Solutions> {
        let code = if self.solver.knows(&script.id) {
            None
        } else {
            Some(self.youtube.player_script_code(script).await?)
        };
        self.solver.solve(&script.id, code, challenges).await
    }

    /// A video's streams, and how long their URLs work.
    async fn streams(&self, video_id: &str) -> Result<(StreamSet, Duration)> {
        if self.refresh_player.swap(false, Ordering::Relaxed) {
            self.youtube.player_script(true).await?;
        }
        let (resp, script) = self.youtube.player_streams(video_id).await?;
        let available_at = preroll(&resp).map(|wait| Instant::now() + wait);
        check_playable(&resp)?;
        let formats = formats(&resp);
        let picked = pick(&formats, &self.options)
            .ok_or_else(|| Error::Streams("YouTube listed no streams for this video".into()))?;
        let solutions = self.solve(&script, picked.challenges()).await?;
        let lifetime = resp
            .pointer("/streamingData/expiresInSeconds")
            .and_then(|e| {
                e.as_str()
                    .and_then(|s| s.parse().ok())
                    .or_else(|| e.as_u64())
            })
            .map_or(Duration::from_secs(3600), Duration::from_secs)
            .saturating_sub(EXPIRY_MARGIN);
        Ok((
            picked.into_streams(&resp, &solutions, available_at)?,
            lifetime,
        ))
    }
}

/// How long the pre-roll ads would play, if the response has any: until
/// their skip button (or their end), YouTube refuses the video's streams.
pub fn preroll(resp: &Value) -> Option<Duration> {
    let mut total = 0.0;
    for placement in array(resp.get("adPlacements")) {
        let renderer = placement.get("adPlacementRenderer");
        let kind = renderer.and_then(|r| str_at(r, "/config/adPlacementConfig/kind"));
        let Some(ad) = renderer.and_then(|r| r.pointer("/renderer/instreamVideoAdRenderer")) else {
            continue;
        };
        if kind != Some("AD_PLACEMENT_KIND_START") {
            continue;
        }
        let skip = ad
            .get("skipOffsetMilliseconds")
            .and_then(Value::as_f64)
            .map(|ms| ms / 1000.0);
        let length = str_at(ad, "/playerVars").and_then(|vars| {
            url::form_urlencoded::parse(vars.as_bytes())
                .find(|(k, _)| k == "length_seconds")
                .and_then(|(_, v)| v.parse::<f64>().ok())
        });
        total += skip.or(length).unwrap_or(0.0);
    }
    (total > 0.0).then(|| Duration::from_secs_f64(total))
}

/// Fails with YouTube's reason when the video can't be played.
/// Why YouTube won't play the video, unless it will. (Signed out, it asks
/// to sign in for age-restricted videos and, often, to confirm the viewer
/// isn't a bot.)
fn check_playable(resp: &Value) -> Result<()> {
    let status = resp
        .pointer("/playabilityStatus/status")
        .and_then(Value::as_str);
    if status == Some("OK") {
        return Ok(());
    }
    let reason = str_at(resp, "/playabilityStatus/reason")
        .map(str::to_owned)
        .or_else(|| {
            let reason = text(
                resp.pointer("/playabilityStatus/errorScreen/playerErrorMessageRenderer/reason"),
            );
            (!reason.is_empty()).then_some(reason)
        })
        .unwrap_or_else(|| {
            format!(
                "YouTube won't play this video ({})",
                status.unwrap_or("no status")
            )
        });
    let subreason =
        text(resp.pointer("/playabilityStatus/errorScreen/playerErrorMessageRenderer/subreason"));
    let upcoming = resp
        .pointer("/videoDetails/isUpcoming")
        .and_then(Value::as_bool)
        == Some(true);
    let members = [&reason, &subreason].iter().any(|r| {
        r.contains("members-only")
            || r.contains("Join this channel")
            || r.contains("channel's members")
    });
    let why = if upcoming {
        crate::error::Unplayable::Upcoming
    } else if status == Some("LOGIN_REQUIRED") {
        crate::error::Unplayable::SignIn
    } else if members {
        crate::error::Unplayable::MembersOnly
    } else {
        crate::error::Unplayable::Other
    };
    Err(Error::Unplayable {
        reason,
        why,
        channel_id: str_at(resp, "/videoDetails/channelId").map(str::to_owned),
    })
}

// Formats

/// A stream as a player response lists it, before its challenges are solved.
#[derive(Clone, Debug, Default, PartialEq)]
struct Format {
    itag: u64,
    url: String,
    /// The `n` parameter to solve.
    n: Option<String>,
    /// A scrambled signature to solve, and the parameter to put it in.
    signature: Option<(String, String)>,
    video_codec: Option<String>,
    audio_codec: Option<String>,
    height: Option<u32>,
    fps: Option<f64>,
    label: Option<String>,
    bitrate: u64,
    hdr: bool,
    /// For a dubbed video's audio: whether this is its original language.
    original: Option<bool>,
    segment_duration: Option<f64>,
    /// A live stream's: how far back it can be watched.
    dvr: Option<f64>,
}

/// Every stream in a player response: separate video and audio streams
/// (`adaptiveFormats`), and ones with both (`formats`).
fn formats(resp: &Value) -> Vec<Format> {
    let data = resp.get("streamingData");
    array(data.and_then(|d| d.get("adaptiveFormats")))
        .chain(array(data.and_then(|d| d.get("formats"))))
        .filter_map(format)
        .collect()
}

fn format(f: &Value) -> Option<Format> {
    let (url, signature) = match str_at(f, "/url") {
        Some(url) => (url.to_owned(), None),
        None => {
            let cipher = str_at(f, "/signatureCipher").or_else(|| str_at(f, "/cipher"))?;
            let fields: HashMap<String, String> = url::form_urlencoded::parse(cipher.as_bytes())
                .into_owned()
                .collect();
            let parameter = fields
                .get("sp")
                .cloned()
                .unwrap_or_else(|| "signature".into());
            (
                fields.get("url")?.clone(),
                Some((fields.get("s")?.clone(), parameter)),
            )
        }
    };
    let n = url::Url::parse(&url)
        .ok()?
        .query_pairs()
        .find(|(k, _)| k == "n")
        .map(|(_, v)| v.into_owned());
    let mime = str_at(f, "/mimeType").unwrap_or_default();
    let (kind, codecs) = mime.split_once(';').unwrap_or((mime, ""));
    let codecs: Vec<&str> = codecs
        .trim()
        .trim_start_matches("codecs=")
        .trim_matches('"')
        .split(',')
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .collect();
    let (video_codec, audio_codec) = match (kind.starts_with("video/"), codecs.as_slice()) {
        (true, [video, audio, ..]) => (Some(*video), Some(*audio)),
        (true, [video]) => (Some(*video), None),
        (false, [audio, ..]) => (None, Some(*audio)),
        _ => (None, None),
    };
    let transfer = str_at(f, "/colorInfo/transferCharacteristics").unwrap_or_default();
    Some(Format {
        itag: f.get("itag").and_then(Value::as_u64).unwrap_or(0),
        url,
        n,
        signature,
        video_codec: video_codec.map(str::to_owned),
        audio_codec: audio_codec.map(str::to_owned),
        height: f.get("height").and_then(Value::as_u64).map(|h| h as u32),
        fps: f.get("fps").and_then(Value::as_f64),
        // Without "HDR" and the like: those streams aren't picked.
        label: str_at(f, "/qualityLabel")
            .and_then(|l| l.split_whitespace().next())
            .map(str::to_owned),
        bitrate: f.get("bitrate").and_then(Value::as_u64).unwrap_or(0),
        hdr: transfer.contains("SMPTEST2084") || transfer.contains("ARIB_STD_B67"),
        original: f.get("audioTrack").map(|track| {
            track.get("audioIsDefault").and_then(Value::as_bool) == Some(true)
                || str_at(track, "/displayName").is_some_and(|n| n.contains("original"))
        }),
        segment_duration: f.get("targetDurationSec").and_then(Value::as_f64),
        dvr: f.get("maxDvrDurationSec").and_then(Value::as_f64),
    })
}

/// The streams picked for a video, before their challenges are solved.
#[derive(Debug)]
struct Picked {
    video: Format,
    audio: Option<Format>,
    qualities: Vec<Format>,
}

/// Picks the best video up to `max_height` (higher frame rates first, then
/// the codecs in order), the best audio in the video's original language,
/// and one video stream per height and frame rate. HDR streams are left
/// out: on most screens they'd look washed out.
fn pick(formats: &[Format], options: &ResolverOptions) -> Option<Picked> {
    let rank = |codec: &Option<String>, order: &[String]| {
        codec
            .as_deref()
            .and_then(|c| order.iter().position(|p| c.starts_with(p.as_str())))
            .unwrap_or(order.len())
    };
    let video_rank = |f: &Format| rank(&f.video_codec, &options.video_codecs);
    let videos: Vec<&Format> = formats
        .iter()
        .filter(|f| f.video_codec.is_some() && f.audio_codec.is_none() && !f.hdr)
        .collect();
    let better = |a: &&Format, b: &&Format| {
        let fps = |f: &Format| f.fps.unwrap_or(30.0).round() as i64;
        (
            a.height,
            fps(a),
            std::cmp::Reverse(video_rank(a)),
            a.bitrate,
        )
            .cmp(&(
                b.height,
                fps(b),
                std::cmp::Reverse(video_rank(b)),
                b.bitrate,
            ))
    };

    // A quality per height and frame rate, in the best codec there is.
    let mut by_quality: BTreeMap<(u32, bool), &Format> = BTreeMap::new();
    for f in &videos {
        let key = (f.height.unwrap_or(0), f.fps.unwrap_or(30.0) > 30.0);
        let replace = by_quality.get(&key).is_none_or(|best| {
            (std::cmp::Reverse(video_rank(f)), f.bitrate)
                > (std::cmp::Reverse(video_rank(best)), best.bitrate)
        });
        if replace {
            by_quality.insert(key, f);
        }
    }
    let qualities: Vec<Format> = by_quality.values().rev().map(|f| (*f).clone()).collect();

    let within: Vec<&Format> = videos
        .iter()
        .copied()
        .filter(|f| f.height.is_some_and(|h| h <= options.max_height))
        .collect();
    // Too tall for the limit only: the smallest.
    let video = within
        .iter()
        .copied()
        .max_by(better)
        .or_else(|| videos.iter().copied().min_by_key(|f| f.height));

    let audios: Vec<&Format> = formats
        .iter()
        .filter(|f| f.audio_codec.is_some() && f.video_codec.is_none())
        .collect();
    let original: Vec<&Format> = audios
        .iter()
        .copied()
        .filter(|f| f.original != Some(false))
        .collect();
    let audio = if original.is_empty() {
        &audios
    } else {
        &original
    }
    .iter()
    .max_by_key(|f| {
        (
            std::cmp::Reverse(rank(&f.audio_codec, &options.audio_codecs)),
            f.bitrate,
        )
    })
    .map(|f| (*f).clone());

    match video {
        Some(video) => Some(Picked {
            video: video.clone(),
            audio,
            qualities,
        }),
        // Only streams with both: take the best of those.
        None => formats
            .iter()
            .filter(|f| f.video_codec.is_some() && f.audio_codec.is_some())
            .max_by_key(|f| f.height)
            .map(|f| Picked {
                video: f.clone(),
                audio: None,
                qualities: Vec::new(),
            }),
    }
}

impl Picked {
    fn all(&self) -> impl Iterator<Item = &Format> {
        std::iter::once(&self.video)
            .chain(&self.audio)
            .chain(&self.qualities)
    }

    fn challenges(&self) -> Challenges {
        let mut challenges = Challenges::default();
        for f in self.all() {
            challenges.n.extend(f.n.clone());
            challenges
                .sig
                .extend(f.signature.as_ref().map(|(s, _)| s.clone()));
        }
        challenges
    }

    fn into_streams(
        self,
        resp: &Value,
        solutions: &Solutions,
        available_at: Option<Instant>,
    ) -> Result<StreamSet> {
        let details = resp.get("videoDetails");
        let live = details
            .and_then(|d| d.get("isLive"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let duration = details
            .and_then(|d| str_at(d, "/lengthSeconds"))
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|d| *d > 0.0 && !live);
        let recorded = !live
            && self.video.segment_duration.is_some()
            && details
                .and_then(|d| d.get("isPostLiveDvr"))
                .and_then(Value::as_bool)
                == Some(true);
        let stream = |f: &Format| solved(f, solutions);
        Ok(StreamSet {
            title: details.and_then(|d| str_at(d, "/title")).map(str::to_owned),
            duration,
            live,
            recorded,
            dvr: self.video.dvr.filter(|_| live),
            video: stream(&self.video)?,
            audio: self.audio.as_ref().map(stream).transpose()?,
            qualities: self.qualities.iter().map(stream).collect::<Result<_>>()?,
            captions: captions(resp),
            storyboard: duration.and_then(|d| storyboard(resp, d)),
            available_at,
        })
    }
}

/// A format's stream with its challenges answered.
fn solved(f: &Format, solutions: &Solutions) -> Result<Stream> {
    let missing = || Error::Streams("the challenge solver didn't answer every challenge".into());
    let mut url =
        url::Url::parse(&f.url).map_err(|e| Error::Streams(format!("bad stream URL: {e}")))?;
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| {
            let v = if k == "n" {
                solutions.n.get(v.as_ref()).cloned().ok_or_else(missing)?
            } else {
                v.into_owned()
            };
            Ok((k.into_owned(), v))
        })
        .collect::<Result<_>>()?;
    {
        let mut query = url.query_pairs_mut();
        query.clear();
        for (k, v) in &pairs {
            query.append_pair(k, v);
        }
        if let Some((scrambled, parameter)) = &f.signature {
            query.append_pair(parameter, solutions.sig.get(scrambled).ok_or_else(missing)?);
        }
    }
    Ok(Stream {
        url: url.into(),
        headers: BTreeMap::from([("User-Agent".to_owned(), TV_DOWNGRADED.user_agent.to_owned())]),
        vcodec: f.video_codec.clone(),
        acodec: f.audio_codec.clone(),
        height: f.height,
        fps: f.fps,
        label: f.label.clone(),
        segment_duration: f.segment_duration,
    })
}

/// Uploaded captions in every language, and the automatic ones.
fn captions(resp: &Value) -> Vec<Caption> {
    array(resp.pointer("/captions/playerCaptionsTracklistRenderer/captionTracks"))
        .filter_map(|track| {
            let base = str_at(track, "/baseUrl")?;
            let mut url = url::Url::parse(base).ok()?;
            let pairs: Vec<(String, String)> = url
                .query_pairs()
                .filter(|(k, _)| k != "fmt")
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            url.query_pairs_mut()
                .clear()
                .extend_pairs(pairs)
                .append_pair("fmt", "json3");
            let auto = str_at(track, "/kind") == Some("asr");
            let name = text(track.get("name"));
            Some(Caption {
                lang: str_at(track, "/languageCode")?.to_owned(),
                name: name.trim_end_matches(" (auto-generated)").to_owned(),
                url: url.into(),
                auto,
            })
        })
        .collect()
}

/// The preview pictures nearest 160 pixels wide, from the storyboard spec:
/// `url|level|level|...`, each level `width#height#count#columns#rows#
/// interval#name#sigh`, and `$L`, `$N` and `$M` in the URL for the level,
/// its name and the picture's number.
fn storyboard(resp: &Value, duration: f64) -> Option<Storyboard> {
    let spec = str_at(resp, "/storyboards/playerStoryboardSpecRenderer/spec")?;
    let mut parts = spec.split('|');
    let base = parts.next()?;
    let mut best: Option<Storyboard> = None;
    for (level, args) in parts.enumerate() {
        let args: Vec<&str> = args.split('#').collect();
        let [width, height, count, columns, rows, _interval, name, sigh] = args[..] else {
            continue;
        };
        let number = |s: &str| s.parse::<u32>().ok().filter(|n| *n > 0);
        let (Some(width), Some(height), Some(count), Some(columns), Some(rows)) = (
            number(width),
            number(height),
            number(count),
            number(columns),
            number(rows),
        ) else {
            continue;
        };
        if best
            .as_ref()
            .is_some_and(|b| b.width.abs_diff(160) <= width.abs_diff(160))
        {
            continue;
        }
        let url = format!(
            "{}&sigh={sigh}",
            base.replace("$L", &level.to_string()).replace("$N", name)
        );
        let pictures = (count as f64 / (rows * columns) as f64).ceil() as u32;
        let per_picture = duration * (rows * columns) as f64 / count as f64;
        let fragments = (0..pictures)
            .map(|i| StoryboardFragment {
                url: url.replace("$M", &i.to_string()),
                duration: per_picture.min(duration - i as f64 * per_picture).max(0.0),
            })
            .collect();
        best = Some(Storyboard {
            width,
            height,
            rows,
            columns,
            fragments,
        });
    }
    best.filter(|b| !b.fragments.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(itag: u64, codec: &str, height: u32, fps: f64, bitrate: u64) -> Format {
        Format {
            itag,
            url: format!("https://r1.googlevideo.com/videoplayback?itag={itag}&n=abc"),
            n: Some("abc".into()),
            video_codec: Some(codec.into()),
            height: Some(height),
            fps: Some(fps),
            bitrate,
            ..Format::default()
        }
    }

    fn audio(itag: u64, codec: &str, bitrate: u64, original: Option<bool>) -> Format {
        Format {
            itag,
            url: format!("https://r1.googlevideo.com/videoplayback?itag={itag}&n=abc"),
            n: Some("abc".into()),
            audio_codec: Some(codec.into()),
            bitrate,
            original,
            ..Format::default()
        }
    }

    #[test]
    fn picks_like_youtube() {
        let formats = vec![
            video(137, "avc1.640028", 1080, 30.0, 4_000_000),
            video(248, "vp9", 1080, 30.0, 2_500_000),
            video(399, "av01.0.08M.08", 1080, 30.0, 2_000_000),
            video(313, "vp9", 2160, 30.0, 15_000_000),
            video(136, "avc1.4D401F", 720, 30.0, 2_000_000),
            video(302, "vp9", 720, 60.0, 3_000_000),
            Format {
                hdr: true,
                ..video(337, "vp09.02.51.10", 2160, 60.0, 30_000_000)
            },
            audio(140, "mp4a.40.2", 130_000, None),
            audio(251, "opus", 140_000, None),
        ];
        let picked = pick(&formats, &ResolverOptions::default()).unwrap();
        assert_eq!(
            picked.video.itag, 248,
            "VP9 at 1080p, the tallest within the limit"
        );
        assert_eq!(picked.audio.unwrap().itag, 251, "Opus");
        let qualities: Vec<u64> = picked.qualities.iter().map(|f| f.itag).collect();
        assert_eq!(
            qualities,
            [313, 248, 302, 136],
            "a codec per height and frame rate, no HDR"
        );
    }

    #[test]
    fn dubbed_videos_play_in_their_original_language() {
        let formats = vec![
            video(248, "vp9", 1080, 30.0, 2_500_000),
            audio(140, "mp4a.40.2", 130_000, Some(false)),
            audio(140, "mp4a.40.2", 129_000, Some(true)),
            audio(140, "mp4a.40.2", 131_000, Some(false)),
        ];
        let picked = pick(&formats, &ResolverOptions::default()).unwrap();
        assert_eq!(picked.audio.unwrap().bitrate, 129_000);
    }

    #[test]
    fn finds_streams_by_format() {
        let stream = |itag: u32| Stream {
            url: format!("https://r1.googlevideo.com/videoplayback?expire=1&itag={itag}&clen=5"),
            ..Stream::default()
        };
        let set = StreamSet {
            video: stream(248),
            audio: Some(stream(251)),
            qualities: vec![stream(248), stream(247), stream(244)],
            ..StreamSet::default()
        };
        assert_eq!(set.video.itag(), Some(248));
        assert_eq!(set.by_itag(251).and_then(Stream::itag), Some(251));
        assert_eq!(set.by_itag(244).and_then(Stream::itag), Some(244));
        assert!(set.by_itag(18).is_none());
        assert_eq!(Stream::default().itag(), None);
    }

    #[test]
    fn solved_urls_carry_the_answers() {
        let mut f = video(248, "vp9", 1080, 30.0, 1);
        f.url = "https://r1.googlevideo.com/videoplayback?expire=1&n=abc&sparams=n%2Cexpire".into();
        f.signature = Some(("scrambled".into(), "sig".into()));
        let solutions = Solutions {
            n: HashMap::from([("abc".into(), "xyz".into())]),
            sig: HashMap::from([("scrambled".into(), "a=b".into())]),
        };
        let stream = solved(&f, &solutions).unwrap();
        assert_eq!(
            stream.url,
            "https://r1.googlevideo.com/videoplayback?expire=1&n=xyz&sparams=n%2Cexpire&sig=a%3Db"
        );
        assert!(
            solved(&f, &Solutions::default()).is_err(),
            "an unanswered challenge"
        );
    }

    #[test]
    fn signature_ciphers() {
        let f = format(&serde_json::json!({
            "itag": 251,
            "mimeType": "audio/webm; codecs=\"opus\"",
            "bitrate": 140000,
            "signatureCipher": "s=AB%3DCD&sp=sig&url=https%3A%2F%2Fr1.googlevideo.com%2Fvideoplayback%3Fn%3Dqq%26itag%3D251",
        }))
        .unwrap();
        assert_eq!(f.signature, Some(("AB=CD".into(), "sig".into())));
        assert_eq!(f.n.as_deref(), Some("qq"));
        assert_eq!(f.audio_codec.as_deref(), Some("opus"));
        assert_eq!(f.video_codec, None);
    }

    #[test]
    fn storyboard_spec() {
        let resp = serde_json::json!({ "storyboards": { "playerStoryboardSpecRenderer": {
            "spec": "https://i.ytimg.com/sb/ID/storyboard3_L$L/$N.jpg?sqp=x|48#27#100#10#10#0#default#rs$A|80#45#61#10#10#2000#M$M#rs$B|160#90#61#5#5#2000#M$M#rs$C",
        } } });
        let board = storyboard(&resp, 120.0).unwrap();
        assert_eq!((board.width, board.rows, board.columns), (160, 5, 5));
        assert_eq!(board.fragments.len(), 3, "61 frames, 25 a picture");
        assert_eq!(
            board.fragments[0].url,
            "https://i.ytimg.com/sb/ID/storyboard3_L2/M0.jpg?sqp=x&sigh=rs$C"
        );
        let total: f64 = board.fragments.iter().map(|f| f.duration).sum();
        assert!((total - 120.0).abs() < 0.01, "{total}");
    }

    #[test]
    fn preroll_wait() {
        let ad = |kind: &str, skip: Option<u64>| {
            let mut ad = serde_json::json!({ "playerVars": "length_seconds=15&video_id=ad" });
            if let Some(skip) = skip {
                ad["skipOffsetMilliseconds"] = skip.into();
            }
            serde_json::json!({ "adPlacementRenderer": {
                "config": { "adPlacementConfig": { "kind": kind } },
                "renderer": { "instreamVideoAdRenderer": ad },
            } })
        };
        let resp = |placements: Vec<Value>| serde_json::json!({ "adPlacements": placements });
        assert_eq!(preroll(&resp(vec![])), None);
        assert_eq!(
            preroll(&resp(vec![ad("AD_PLACEMENT_KIND_START", Some(5000))])),
            Some(Duration::from_secs(5)),
            "until it can be skipped"
        );
        assert_eq!(
            preroll(&resp(vec![ad("AD_PLACEMENT_KIND_START", None)])),
            Some(Duration::from_secs(15))
        );
        assert_eq!(
            preroll(&resp(vec![ad("AD_PLACEMENT_KIND_MILLISECONDS", None)])),
            None,
            "mid-rolls don't hold the start"
        );
    }

    #[test]
    fn playability() {
        let resp = |status: &str, reason: &str| serde_json::json!({ "playabilityStatus": { "status": status, "reason": reason } });
        assert!(check_playable(&resp("OK", "")).is_ok());
        let why = |r: &Value| match check_playable(r) {
            Err(Error::Unplayable { why, reason, .. }) => (why, reason),
            other => panic!("{other:?}"),
        };
        use crate::error::Unplayable as U;
        let age = why(&resp("LOGIN_REQUIRED", "Sign in to confirm your age"));
        assert_eq!(age, (U::SignIn, "Sign in to confirm your age".to_owned()));
        let mut upcoming = resp("UNPLAYABLE", "This live event will begin in 5 minutes.");
        upcoming["videoDetails"] = serde_json::json!({ "isUpcoming": true, "channelId": "UCx" });
        assert_eq!(why(&upcoming).0, U::Upcoming);
        let members = resp(
            "UNPLAYABLE",
            "Join this channel to get access to members-only content like this video, and other exclusive perks.",
        );
        assert_eq!(why(&members).0, U::MembersOnly);
        assert_eq!(why(&resp("ERROR", "This video is unavailable")).0, U::Other);
    }

    #[test]
    fn storyboard_frames() {
        let board = Storyboard {
            width: 160,
            height: 90,
            rows: 2,
            columns: 2,
            fragments: vec![
                StoryboardFragment {
                    url: "a".into(),
                    duration: 40.0,
                },
                StoryboardFragment {
                    url: "b".into(),
                    duration: 20.0,
                },
            ],
        };
        let at = |t| board.frame_at(t).map(|(f, r, c)| (f.url.as_str(), r, c));
        assert_eq!(at(0.0), Some(("a", 0, 0)));
        assert_eq!(at(25.0), Some(("a", 1, 0)), "10 s per frame: the third");
        assert_eq!(at(39.9), Some(("a", 1, 1)));
        assert_eq!(
            at(45.0),
            Some(("b", 0, 1)),
            "5 s per frame in the shorter one"
        );
        assert_eq!(at(500.0), Some(("b", 1, 1)), "past the end: the last frame");
    }

    #[test]
    fn quality_labels() {
        let q = |height, fps| Stream {
            height: Some(height),
            fps: Some(fps),
            ..Stream::default()
        };
        assert_eq!(q(1080, 60.0).quality_label().as_deref(), Some("1080p60"));
        assert_eq!(q(720, 30.0).quality_label().as_deref(), Some("720p"));
        assert_eq!(q(480, 25.0).quality_label().as_deref(), Some("480p"));
        // YouTube's own name wins: this one is 1920×1072.
        let named = Stream {
            label: Some("1080p".into()),
            ..q(1072, 25.0)
        };
        assert_eq!(named.quality_label().as_deref(), Some("1080p"));
    }
}
