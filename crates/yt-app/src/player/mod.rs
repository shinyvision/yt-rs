//! Video playback with libmpv.
//!
//! [`Player`] owns the mpv instance and lives on the GTK main thread. mpv
//! wakes it up from its own threads; the player then drains mpv's events on
//! the main loop, keeps [`PlayerState`] current and tells its listeners.
//! [`render::RenderContext`] draws the video into textures (`ui::video`).

pub mod gl;
pub mod live;
pub mod mpv;
pub mod render;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gtk::glib;
use serde::Serialize;
use yt_core::streams::live::SharedHead;
use yt_core::streams::{Stream, StreamSet};

use mpv::{EndReason, Event, Format, Mpv, MpvError, PropertyValue, quote};

/// What the player is doing, kept current from mpv's property changes.
#[derive(Clone, Debug, Default, Serialize)]
pub struct PlayerState {
    /// The YouTube video ID that is loaded, if any.
    pub video_id: Option<String>,
    pub position: f64,
    pub duration: f64,
    pub paused: bool,
    /// Waiting for data.
    pub buffering: bool,
    /// How far ahead of the position the stream is downloaded, in seconds.
    pub buffered: f64,
    pub volume: f64,
    pub muted: bool,
    pub ended: bool,
    /// Playback speed, 1 being normal.
    pub speed: f64,
    /// The quality playing, e.g. "1080p60", and whether it was picked
    /// automatically.
    pub quality: Option<String>,
    pub auto_quality: bool,
    /// The language of the captions shown, if any.
    pub captions: Option<String>,
    pub looping: bool,
    pub stable_volume: bool,
    /// For a live stream: the part that can be watched. Its `position` is
    /// in the stream's own time, between `start` and `edge`.
    pub live: Option<LiveWindow>,
}

/// The part of a live stream that can be watched: from `start` (as far
/// back as YouTube keeps) to the live `edge`, in the stream's time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct LiveWindow {
    pub start: f64,
    pub edge: f64,
}

impl LiveWindow {
    /// Within this many seconds of the edge counts as watching live.
    pub const EDGE: f64 = 15.0;

    pub fn at_edge(&self, position: f64) -> bool {
        self.edge - position < Self::EDGE
    }
}

/// A live stream being played: where its newest segment is (kept current
/// by the stream's readers), its segment length, and how far back it goes.
#[derive(Clone)]
struct LiveSession {
    head: SharedHead,
    segment: f64,
    dvr: f64,
}

#[derive(Clone, Debug)]
pub enum PlayerEvent {
    /// A video started loading.
    Loading {
        video_id: String,
    },
    /// The video is ready and about to play.
    Loaded,
    Position(f64),
    Duration(f64),
    Paused(bool),
    Buffering(bool),
    /// The amount buffered ahead changed (see [`PlayerState::buffered`]).
    Buffered,
    Volume {
        volume: f64,
        muted: bool,
    },
    /// A seek started from `from` to `to`.
    Seeking {
        from: f64,
        to: f64,
    },
    /// A seek finished and playback continues from `position`.
    Seeked {
        position: f64,
    },
    /// The video loaded, but its separate audio stream couldn't be opened
    /// (e.g. YouTube refused it), so it plays without sound.
    AudioFailed,
    Ended,
    /// Playback stopped because something went wrong.
    Error(String),
    /// The video was unloaded.
    Stopped,
    Speed(f64),
    /// The quality, captions, looping or stable volume changed; see
    /// [`PlayerState`].
    Settings,
}

type Listener = Rc<dyn Fn(&PlayerEvent, &PlayerState)>;

pub struct Player {
    mpv: Arc<Mpv>,
    state: RefCell<PlayerState>,
    /// The streams of the loaded video.
    streams: RefCell<Option<StreamSet>>,
    /// Captions added to the loaded video: language, title, SRT and mpv's
    /// track ID (none until mpv has loaded them).
    captions: RefCell<Vec<AddedCaptions>>,
    listeners: RefCell<Vec<Listener>>,
    seeking: Cell<bool>,
    /// Whether the loaded video came with a separate audio stream.
    expects_audio: Cell<bool>,
    /// The video stream playing (the picked one or a quality).
    playing: RefCell<Option<Stream>>,
    live: RefCell<Option<LiveSession>>,
}

struct AddedCaptions {
    lang: String,
    title: String,
    srt: String,
    track: Option<i64>,
}

/// The audio filter that evens out loudness ("stable volume").
const STABLE_VOLUME_FILTER: &str = "@stable:lavfi=[dynaudnorm=f=250:g=15:p=0.9]";

// Property observation IDs.
const TIME_POS: u64 = 1;
const DURATION: u64 = 2;
const PAUSE: u64 = 3;
const PAUSED_FOR_CACHE: u64 = 4;
const CACHE_TIME: u64 = 5;
const VOLUME: u64 = 6;
const MUTE: u64 = 7;
const EOF_REACHED: u64 = 8;
const SPEED: u64 = 9;

impl Player {
    /// `http` fetches live streams' segments.
    pub fn new(http: reqwest::Client) -> Result<Rc<Self>, MpvError> {
        let mpv = Mpv::create()?;
        for (name, value) in [
            ("vo", "libmpv"),
            ("hwdec", "auto-safe"),
            // The app finds the streams; mpv's own yt-dlp hook stays off.
            ("ytdl", "no"),
            ("keep-open", "yes"),
            ("idle", "yes"),
            ("osc", "no"),
            ("input-default-bindings", "no"),
            ("input-vo-keyboard", "no"),
            ("terminal", "no"),
            ("audio-client-name", "yt-rs"),
            ("cache", "yes"),
            ("demuxer-max-bytes", "48MiB"),
            ("demuxer-max-back-bytes", "16MiB"),
            ("demuxer-readahead-secs", "30"),
            ("hr-seek", "yes"),
            ("video-timing-offset", "0"),
            // Captions as YouTube shows them: white text on a dark box.
            ("sub-font", "sans-serif"),
            ("sub-font-size", "46"),
            ("sub-color", "#FFFFFFFF"),
            ("sub-back-color", "#BF080808"),
            ("sub-border-style", "background-box"),
            ("sub-outline-size", "4"),
            ("sub-shadow-offset", "0"),
            ("sub-margin-y", "40"),
            ("sub-scale-with-window", "yes"),
        ] {
            mpv.set_option(name, value)?;
        }
        mpv.initialize()?;
        live::install(&mpv, http)?;
        // More of mpv's log for looking into a problem: YT_RS_MPV_LOG=v
        // (or debug, trace), shown with YT_RS_LOG=mpv=debug.
        let level = std::env::var("YT_RS_MPV_LOG").unwrap_or_else(|_| "warn".into());
        mpv.request_log_messages(&level)?;
        for (id, name, format) in [
            (TIME_POS, "time-pos", Format::Double),
            (DURATION, "duration", Format::Double),
            (PAUSE, "pause", Format::Flag),
            (PAUSED_FOR_CACHE, "paused-for-cache", Format::Flag),
            (CACHE_TIME, "demuxer-cache-time", Format::Double),
            (VOLUME, "volume", Format::Double),
            (MUTE, "mute", Format::Flag),
            (EOF_REACHED, "eof-reached", Format::Flag),
            (SPEED, "speed", Format::Double),
        ] {
            mpv.observe(id, name, format)?;
        }

        let player = Rc::new(Self {
            mpv: Arc::new(mpv),
            state: RefCell::new(PlayerState {
                volume: 100.0,
                speed: 1.0,
                ..PlayerState::default()
            }),
            streams: RefCell::new(None),
            captions: RefCell::new(Vec::new()),
            listeners: RefCell::new(Vec::new()),
            seeking: Cell::new(false),
            expects_audio: Cell::new(false),
            playing: RefCell::new(None),
            live: RefCell::new(None),
        });

        // mpv calls the wakeup callback from its own threads; drain the
        // events on the main loop. The flag coalesces bursts of wakeups.
        // (tokio's channels don't need the tokio runtime to be awaited.)
        let pending = Arc::new(AtomicBool::new(false));
        let (wake, mut woken) = tokio::sync::mpsc::unbounded_channel::<()>();
        {
            let pending = pending.clone();
            player.mpv.set_wakeup_callback(move || {
                if !pending.swap(true, Ordering::AcqRel) {
                    let _ = wake.send(());
                }
            });
        }
        let weak = Rc::downgrade(&player);
        glib::spawn_future_local(async move {
            while woken.recv().await.is_some() {
                pending.store(false, Ordering::Release);
                let Some(player) = weak.upgrade() else { break };
                player.drain_events();
            }
        });
        Ok(player)
    }

    pub fn mpv(&self) -> &Arc<Mpv> {
        &self.mpv
    }

    pub fn state(&self) -> PlayerState {
        self.state.borrow().clone()
    }

    pub fn connect(&self, listener: impl Fn(&PlayerEvent, &PlayerState) + 'static) {
        self.listeners.borrow_mut().push(Rc::new(listener));
    }

    /// Plays a video from its resolved streams, starting at `start` seconds.
    pub fn load(&self, video_id: &str, streams: &StreamSet, start: f64) -> Result<(), MpvError> {
        {
            let mut state = self.state.borrow_mut();
            *state = PlayerState {
                video_id: Some(video_id.to_owned()),
                // Where playback will begin, until mpv reports positions.
                position: if streams.live { 0.0 } else { start.max(0.0) },
                volume: state.volume,
                muted: state.muted,
                speed: state.speed,
                stable_volume: state.stable_volume,
                quality: streams.video.quality_label(),
                auto_quality: true,
                ..PlayerState::default()
            };
        }
        *self.streams.borrow_mut() = Some(streams.clone());
        *self.live.borrow_mut() = streams.live.then(|| LiveSession {
            head: SharedHead::default(),
            segment: streams.video.segment_duration.unwrap_or(2.0),
            dvr: streams.dvr.unwrap_or(0.0),
        });
        self.captions.borrow_mut().clear();
        let _ = self.mpv.set_option("loop-file", "no");
        self.emit(PlayerEvent::Loading {
            video_id: video_id.to_owned(),
        });
        self.open(streams, &streams.video, start, false)
    }

    /// Opens `video` with the set's audio, from `start` seconds.
    fn open(
        &self,
        streams: &StreamSet,
        video: &Stream,
        start: f64,
        paused: bool,
    ) -> Result<(), MpvError> {
        self.expects_audio.set(streams.audio.is_some());
        *self.playing.borrow_mut() = Some(video.clone());
        // Live streams are read segment by segment (see `live`), from the
        // segment holding `start` (in the stream's time), or near the edge.
        let session = self.live.borrow().clone();
        let first = session.as_ref().and_then(|l| {
            let head = (*l.head.lock().unwrap())?;
            (start > 0.0).then(|| head.segment_at(start, l.segment))
        });
        let url = |stream: &Stream| match (&session, stream.segment_duration) {
            (Some(l), Some(_)) => live::url_for(stream, first, l.head.clone()),
            _ => stream.url.clone(),
        };
        let mut options = Vec::new();
        if session.is_some() {
            // Positions in the stream's own time, which places them in its
            // window.
            options.push("rebase-start-time=no".to_owned());
        }
        if let Some(audio) = &streams.audio {
            options.push(format!("audio-file={}", quote(&url(audio))));
        }
        if let Some(title) = &streams.title {
            options.push(format!("force-media-title={}", quote(title)));
        }
        if let Some(agent) = video.headers.get("User-Agent") {
            options.push(format!("user-agent={}", quote(agent)));
        }
        if start > 1.0 && !streams.live {
            options.push(format!("start={start:.1}"));
        }
        self.mpv
            .command(&["loadfile", &url(video), "replace", "-1", &options.join(",")])?;
        self.mpv.set_flag("pause", paused)
    }

    /// The streams of the loaded video.
    pub fn streams(&self) -> Option<StreamSet> {
        self.streams.borrow().clone()
    }

    /// Switches to one of the video's qualities (an index into
    /// [`StreamSet::qualities`]), or back to the one picked automatically,
    /// carrying on from the same moment.
    pub fn set_quality(&self, index: Option<usize>) -> Result<(), String> {
        let streams = self.streams().ok_or("no video is playing")?;
        let video = match index {
            Some(i) => streams.qualities.get(i).ok_or("no such quality")?.clone(),
            None => streams.video.clone(),
        };
        let state = self.state();
        self.open(&streams, &video, state.position, state.paused)
            .map_err(|e| e.to_string())?;
        {
            let mut state = self.state.borrow_mut();
            state.quality = video.quality_label();
            state.auto_quality = index.is_none();
        }
        // Loading the stream again drops its captions; they come back once it
        // has loaded.
        for added in self.captions.borrow_mut().iter_mut() {
            added.track = None;
        }
        self.emit(PlayerEvent::Settings);
        Ok(())
    }

    pub fn set_speed(&self, speed: f64) {
        let _ = self.mpv.set_double("speed", speed.clamp(0.25, 4.0));
    }

    /// Shows captions from SRT text, in place of any shown.
    pub fn show_captions(&self, lang: &str, title: &str, srt: String) {
        let mut captions = self.captions.borrow_mut();
        if !captions.iter().any(|c| c.lang == lang) {
            captions.push(AddedCaptions {
                lang: lang.to_owned(),
                title: title.to_owned(),
                srt,
                track: None,
            });
        }
        drop(captions);
        self.select_captions(Some(lang));
    }

    /// Whether captions in this language were loaded for the video.
    pub fn has_captions(&self, lang: &str) -> bool {
        self.captions.borrow().iter().any(|c| c.lang == lang)
    }

    /// Shows loaded captions in this language, or hides captions.
    pub fn select_captions(&self, lang: Option<&str>) {
        match lang {
            None => {
                let _ = self.mpv.set_option("sid", "no");
            }
            Some(lang) => {
                let mut captions = self.captions.borrow_mut();
                let Some(added) = captions.iter_mut().find(|c| c.lang == lang) else {
                    return;
                };
                match added.track {
                    Some(track) => {
                        let _ = self.mpv.set_option("sid", &track.to_string());
                    }
                    None => {
                        // mpv reads the text from memory, so this doesn't wait
                        // for the network.
                        let data = format!("memory://{}", added.srt);
                        if self
                            .mpv
                            .command(&["sub-add", &data, "select", &added.title, &added.lang])
                            .is_ok()
                        {
                            added.track = self.mpv.get_string("sid").and_then(|id| id.parse().ok());
                        }
                    }
                }
            }
        }
        self.state.borrow_mut().captions = lang.map(str::to_owned);
        self.emit(PlayerEvent::Settings);
    }

    pub fn set_looping(&self, looping: bool) {
        let _ = self
            .mpv
            .set_option("loop-file", if looping { "inf" } else { "no" });
        self.state.borrow_mut().looping = looping;
        self.emit(PlayerEvent::Settings);
    }

    /// Evens out loud and quiet parts, like YouTube's "stable volume".
    pub fn set_stable_volume(&self, on: bool) {
        if self.state.borrow().stable_volume == on {
            return;
        }
        let _ = self.mpv.command(&[
            "af",
            if on { "add" } else { "remove" },
            STABLE_VOLUME_FILTER,
        ]);
        self.state.borrow_mut().stable_volume = on;
        self.emit(PlayerEvent::Settings);
    }

    /// One frame forward or back (pausing first).
    pub fn frame_step(&self, forward: bool) {
        let _ = self.mpv.command(&[if forward {
            "frame-step"
        } else {
            "frame-back-step"
        }]);
    }

    /// Details for "stats for nerds", as mpv reports them.
    pub fn stats(&self) -> Vec<(&'static str, String)> {
        let get = |name: &str| self.mpv.get_string(name).unwrap_or_default();
        let number = |name: &str| get(name).parse::<f64>().unwrap_or(0.0);
        let state = self.state();
        vec![
            ("Video ID", state.video_id.clone().unwrap_or_default()),
            (
                "Viewport / Frames",
                format!(
                    "{}×{} / {} dropped",
                    get("osd-width"),
                    get("osd-height"),
                    number("frame-drop-count") as i64 + number("decoder-frame-drop-count") as i64
                ),
            ),
            (
                "Current / Optimal res",
                format!(
                    "{}×{}@{:.0} / {}",
                    get("video-params/w"),
                    get("video-params/h"),
                    number("container-fps"),
                    state.quality.clone().unwrap_or_default()
                ),
            ),
            (
                "Volume / Normalized",
                format!(
                    "{:.0}% / {}",
                    state.volume,
                    if state.stable_volume { "on" } else { "off" }
                ),
            ),
            (
                "Codecs",
                format!(
                    "{} / {}",
                    get("current-tracks/video/codec"),
                    get("current-tracks/audio/codec")
                ),
            ),
            ("Hardware decoding", get("hwdec-current")),
            (
                "Bitrate",
                format!(
                    "{:.0} Kbps video / {:.0} Kbps audio",
                    number("video-bitrate") / 1000.0,
                    number("audio-bitrate") / 1000.0
                ),
            ),
            (
                "Buffer health",
                format!("{:.2} s", number("demuxer-cache-duration")),
            ),
            ("Speed", format!("{}×", state.speed)),
        ]
    }

    pub fn stop(&self) {
        if self.state.borrow().video_id.is_some() {
            let _ = self.mpv.command(&["stop"]);
            // Listeners still see which video stopped.
            self.emit(PlayerEvent::Stopped);
            self.state.borrow_mut().video_id = None;
        }
    }

    pub fn set_paused(&self, paused: bool) {
        let _ = self.mpv.set_flag("pause", paused);
    }

    pub fn toggle_pause(&self) {
        let state = self.state();
        if state.ended {
            self.seek(0.0, true);
            self.set_paused(false);
        } else {
            self.set_paused(!state.paused);
        }
    }

    /// Seeks to `position` seconds. `exact` seeks to that exact frame;
    /// otherwise to the nearest keyframe, which is much faster and suits
    /// dragging the seek bar.
    /// Where a video can be sought to: a live stream's window, or the
    /// whole video.
    pub fn timeline(&self) -> (f64, f64) {
        match self.live_window() {
            Some(w) => (w.start, w.edge),
            None => (0.0, self.state.borrow().duration),
        }
    }

    /// Jumps to a live stream's edge.
    pub fn go_live(&self) {
        self.seek(f64::INFINITY, true);
    }

    fn live_window(&self) -> Option<LiveWindow> {
        let live = self.live.borrow();
        let live = live.as_ref()?;
        let head = (*live.head.lock().unwrap())?;
        let edge = head.edge(live.segment);
        Some(LiveWindow {
            start: (edge - live.dvr).max(head.first(live.segment)).min(edge),
            edge,
        })
    }

    /// A live stream can't be sought in place: it opens again at the
    /// segment holding `position`, or near the edge.
    fn seek_live(&self, position: f64) {
        let (Some(window), Some(streams), Some(video)) = (
            self.live_window(),
            self.streams(),
            self.playing.borrow().clone(),
        ) else {
            return;
        };
        let target = position.clamp(window.start, window.edge);
        let (from, paused) = {
            let state = self.state.borrow();
            (state.position, state.paused)
        };
        let start = if window.at_edge(target) { 0.0 } else { target };
        if self.open(&streams, &video, start, paused).is_ok() {
            self.emit(PlayerEvent::Seeking { from, to: target });
            {
                let mut state = self.state.borrow_mut();
                state.position = target;
                state.live = Some(window);
            }
            self.emit(PlayerEvent::Position(target));
        }
    }

    pub fn seek(&self, position: f64, exact: bool) {
        if self.live.borrow().is_some() {
            self.seek_live(position);
            return;
        }
        let mode = if exact {
            "absolute+exact"
        } else {
            "absolute+keyframes"
        };
        let from = self.state.borrow().position;
        if self
            .mpv
            .command(&["seek", &format!("{position:.3}"), mode])
            .is_ok()
        {
            self.emit(PlayerEvent::Seeking { from, to: position });
            self.seeking.set(true);
            // Show the new position right away instead of waiting for mpv.
            self.state.borrow_mut().position = position;
            self.emit(PlayerEvent::Position(position));
        }
    }

    pub fn seek_by(&self, seconds: f64) {
        let position = self.state.borrow().position;
        let (start, end) = self.timeline();
        self.seek((position + seconds).clamp(start, end.max(start)), true);
    }

    pub fn set_volume(&self, volume: f64) {
        let _ = self.mpv.set_double("volume", volume.clamp(0.0, 130.0));
    }

    pub fn set_muted(&self, muted: bool) {
        let _ = self.mpv.set_flag("mute", muted);
    }

    fn emit(&self, event: PlayerEvent) {
        let state = self.state.borrow().clone();
        let listeners: Vec<Listener> = self.listeners.borrow().clone();
        for listener in listeners {
            listener(&event, &state);
        }
    }

    fn drain_events(&self) {
        while let Some(event) = self.mpv.wait_event(0.0) {
            self.handle(event);
        }
    }

    fn handle(&self, event: Event) {
        match event {
            Event::FileLoaded => {
                // Captions that were showing before the stream was opened
                // again (a quality change) come back.
                let shown = self.state.borrow().captions.clone();
                if shown.is_some() && self.captions.borrow().iter().all(|c| c.track.is_none()) {
                    self.select_captions(shown.as_deref());
                }
                self.emit(PlayerEvent::Loaded);
                // mpv opens external audio files before this event; a
                // refused one leaves the video without an audio track.
                if self.expects_audio.get()
                    && self.mpv.get_string("current-tracks/audio/id").is_none()
                {
                    tracing::warn!("the audio stream couldn't be opened");
                    self.emit(PlayerEvent::AudioFailed);
                }
            }
            Event::PlaybackRestart => {
                if self.seeking.replace(false) {
                    let position = self.state.borrow().position;
                    self.emit(PlayerEvent::Seeked { position });
                }
            }
            Event::EndFile {
                reason: EndReason::Error,
                error,
            } => {
                self.emit(PlayerEvent::Error(
                    error.unwrap_or_else(|| "playback failed".into()),
                ));
            }
            Event::Log {
                level,
                prefix,
                text,
            } => match level.as_str() {
                "fatal" | "error" => tracing::error!(target: "mpv", "[{prefix}] {text}"),
                "warn" => tracing::warn!(target: "mpv", "[{prefix}] {text}"),
                "info" => tracing::info!(target: "mpv", "[{prefix}] {text}"),
                _ => tracing::debug!(target: "mpv", "[{prefix}] {text}"),
            },
            Event::PropertyChange { id, value, .. } => self.property_changed(id, value),
            _ => {}
        }
    }

    fn property_changed(&self, id: u64, value: PropertyValue) {
        let double = |v: &PropertyValue| {
            if let PropertyValue::Double(d) = v {
                Some(*d)
            } else {
                None
            }
        };
        let flag = |v: &PropertyValue| matches!(v, PropertyValue::Flag(true));
        let event = {
            let mut state = self.state.borrow_mut();
            match id {
                TIME_POS => {
                    let Some(position) = double(&value) else {
                        return;
                    };
                    if self.seeking.get() {
                        return;
                    }
                    state.position = position;
                    state.live = self.live_window();
                    PlayerEvent::Position(position)
                }
                DURATION => {
                    state.duration = double(&value).unwrap_or(0.0);
                    PlayerEvent::Duration(state.duration)
                }
                PAUSE => {
                    state.paused = flag(&value);
                    PlayerEvent::Paused(state.paused)
                }
                PAUSED_FOR_CACHE => {
                    state.buffering = flag(&value);
                    PlayerEvent::Buffering(state.buffering)
                }
                CACHE_TIME => {
                    state.buffered = double(&value).unwrap_or(0.0);
                    PlayerEvent::Buffered
                }
                VOLUME => {
                    state.volume = double(&value).unwrap_or(state.volume);
                    PlayerEvent::Volume {
                        volume: state.volume,
                        muted: state.muted,
                    }
                }
                MUTE => {
                    state.muted = flag(&value);
                    PlayerEvent::Volume {
                        volume: state.volume,
                        muted: state.muted,
                    }
                }
                SPEED => {
                    state.speed = double(&value).unwrap_or(1.0);
                    PlayerEvent::Speed(state.speed)
                }
                EOF_REACHED => {
                    let ended = flag(&value);
                    if ended == state.ended {
                        return;
                    }
                    state.ended = ended;
                    if !ended {
                        return;
                    }
                    PlayerEvent::Ended
                }
                _ => return,
            }
        };
        self.emit(event);
    }
}
