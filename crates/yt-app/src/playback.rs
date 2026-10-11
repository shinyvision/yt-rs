//! Starting and stopping videos. Playing a video opens the watch page for
//! it, and the watch page showing a video is what starts it: so going back
//! or forward to a video plays that video again. Starting looks up the
//! streams, the saved resume position and the watch details (title,
//! channel, comments) at the same time, then hands the streams to the
//! player.
//!
//! If YouTube refuses a stream (HTTP 403), playback looks the streams up
//! again once (fresh URLs, and a newer player script if YouTube has one)
//! and carries on at the same position, instead of failing or playing
//! without sound.

use std::cell::{Cell, RefCell};

use serde_json::{Value, json};
use yt_core::feed::Video;
use yt_core::streams::StreamSet;

use crate::app::app;
use crate::player::{PlayerEvent, PlayerState};
use crate::{runtime, social};
use gtk::glib;

thread_local! {
    /// Bumped on every play, so a slow lookup for a video the user has
    /// already left doesn't start playing it.
    static GENERATION: Cell<u64> = const { Cell::new(0) };
    /// The video whose streams were already looked up again, so it only
    /// happens once.
    static RECOVERED: RefCell<Option<String>> = const { RefCell::new(None) };
    /// The video started for the watch page (playing, loading or ended).
    static SHOWN: RefCell<Option<String>> = const { RefCell::new(None) };
    /// And what the card said about it, for the mini player's way back.
    static CURRENT: RefCell<Option<Video>> = const { RefCell::new(None) };
    /// Where `play` asked a video to start: (video ID, seconds).
    static START: RefCell<Option<(String, f64)>> = const { RefCell::new(None) };
    /// The videos YouTube suggests after the one shown, for Next and
    /// autoplay.
    static UP_NEXT: RefCell<Vec<Video>> = const { RefCell::new(Vec::new()) };
    /// The countdown to playing the next video.
    static COUNTDOWN: RefCell<Option<glib::SourceId>> = const { RefCell::new(None) };
    /// The sleep timer: when it fires, or `None` for the end of the video.
    static SLEEP: RefCell<Option<Sleep>> = const { RefCell::new(None) };
}

/// Seconds the "Up next" countdown lasts.
const UP_NEXT_DELAY: u32 = 8;
/// How often a live stream or premiere that hasn't started is checked
/// again; it plays once it starts.
const UPCOMING_RECHECK: std::time::Duration = std::time::Duration::from_secs(30);

struct Sleep {
    timer: Option<glib::SourceId>,
    /// Seconds since the epoch it fires at; `None` for the end of the video.
    at: Option<u64>,
}

/// Follows the player to recover from refused streams, to turn captions on
/// and to play the next video; and the pages, to start the video the watch
/// page shows.
pub fn init() {
    let app = app();
    let settings = app.settings.get();
    app.player.set_volume(settings.volume);
    app.player.set_muted(settings.muted);
    app.player.set_stable_volume(settings.stable_volume);
    app.player.connect(|event, state| match event {
        PlayerEvent::AudioFailed => recover(state, "the audio"),
        PlayerEvent::Error(_) => recover(state, "the video"),
        PlayerEvent::Loaded => restore_captions(state),
        PlayerEvent::Ended => ended(state),
        PlayerEvent::Volume { volume, muted } => {
            let (volume, muted) = (*volume, *muted);
            crate::app::app().settings.update(|s| {
                s.volume = volume;
                s.muted = muted;
            });
        }
        PlayerEvent::Settings => {
            let stable = state.stable_volume;
            crate::app::app()
                .settings
                .update(|s| s.stable_volume = stable);
        }
        _ => {}
    });
    app.ext.events.on("watch.loaded", |payload| {
        let page: Option<yt_core::watch::WatchPage> = payload
            .get("page")
            .and_then(|p| serde_json::from_value(p.clone()).ok());
        if let Some(page) = page {
            let next: Vec<Video> = page
                .related
                .into_iter()
                .flat_map(|shelf| shelf.videos)
                .filter(|v| !v.short && v.id != page.video_id)
                .collect();
            UP_NEXT.with(|u| *u.borrow_mut() = next);
        }
    });
    app.ext.events.on("page.changed", |payload| {
        if payload.get("page").and_then(Value::as_str) == Some("watch")
            && let Some(video) = payload
                .pointer("/args/video")
                .and_then(|v| serde_json::from_value::<Video>(v.clone()).ok())
                .filter(|v| !v.id.is_empty())
        {
            start(video);
        }
    });
}

fn recover(state: &PlayerState, what: &str) {
    let Some(video_id) = state.video_id.clone() else {
        return;
    };
    let already = RECOVERED.with(|r| r.borrow().as_deref() == Some(video_id.as_str()));
    if already {
        // The fresh streams failed too.
        let app = app();
        if what == "the audio" {
            app.toast("YouTube refused the audio for this video");
        }
        return;
    }
    RECOVERED.with(|r| *r.borrow_mut() = Some(video_id.clone()));
    tracing::warn!(
        video_id,
        "YouTube refused {what} stream; looking it up again"
    );
    let (position, generation) = (state.position, GENERATION.with(Cell::get));
    app().ext.events.emit(
        "player.fallback",
        &json!({ "video_id": video_id, "position": position }),
    );
    runtime::spawn_local(async move {
        let app = app();
        let streams = app.streams.clone();
        let id = video_id.clone();
        let lookup = runtime::background(async move { streams.resolve_fresh(&id).await }).await;
        if GENERATION.with(Cell::get) != generation {
            return;
        }
        match lookup {
            Ok(streams) => load(&video_id, &streams, position, generation).await,
            Err(e) => failed(&e.to_string(), None),
        }
    });
}

/// Plays a video, from `start` seconds if given, otherwise from where the
/// user left off: opens the watch page for it, which starts it. It starts
/// over even if it is the video already shown.
pub fn play(video: Video, start: Option<f64>) {
    SHOWN.with(|s| s.borrow_mut().take());
    START.with(|s| *s.borrow_mut() = start.map(|t| (video.id.clone(), t)));
    app().navigate("watch", json!({ "video": video }), true);
}

/// Starts the video the watch page now shows, unless it already did.
fn start(video: Video) {
    if SHOWN.with(|s| s.borrow().as_deref() == Some(video.id.as_str())) {
        return;
    }
    SHOWN.with(|s| *s.borrow_mut() = Some(video.id.clone()));
    cancel_up_next();
    UP_NEXT.with(|u| u.borrow_mut().clear());
    let start = START
        .with(|s| s.borrow_mut().take())
        .filter(|(id, _)| *id == video.id)
        .map(|(_, t)| t);
    let app = app();
    let generation = GENERATION.with(|g| {
        g.set(g.get() + 1);
        g.get()
    });
    RECOVERED.with(|r| r.borrow_mut().take());
    app.player.stop();
    CURRENT.with(|c| *c.borrow_mut() = Some(video.clone()));
    app.ext
        .events
        .emit("video.opened", &json!({ "video": video }));

    let id = video.id.clone();
    runtime::spawn_local(async move {
        if let Err(e) = social::load_watch(&id).await {
            tracing::warn!(video_id = id, "couldn't load the watch details: {e}");
            crate::app::app()
                .ext
                .events
                .emit("watch.failed", &json!({ "video_id": id, "error": e }));
        }
    });

    look_up(video, start, generation);
}

/// Looks the streams up, and plays them; or says why YouTube won't.
fn look_up(video: Video, start: Option<f64>, generation: u64) {
    runtime::spawn_local(async move {
        let app = crate::app::app();
        let streams = app.streams.clone();
        let id = video.id.clone();
        let (resume, lookup) = tokio::join!(
            app.tracker.prepare(&video.id),
            runtime::background(async move { streams.resolve(&id).await })
        );
        if GENERATION.with(Cell::get) != generation {
            return;
        }
        match lookup {
            Ok(streams) => {
                let start = start
                    .or(resume)
                    .or_else(|| fallback_resume(&video))
                    .unwrap_or(0.0);
                let mut streams = (*streams).clone();
                if streams.title.is_none() && !video.title.is_empty() {
                    streams.title = Some(video.title.clone());
                }
                load(&video.id, &streams, start, generation).await;
            }
            Err(e) => unplayable(video, start, e, generation),
        }
    });
}

/// Says why YouTube won't play a video, with what to do about it: sign
/// in, join the channel, or wait for the stream to start (it then plays).
fn unplayable(video: Video, start: Option<f64>, error: yt_core::Error, generation: u64) {
    use yt_core::error::{Error, Unplayable};
    let Error::Unplayable {
        reason,
        why,
        channel_id,
    } = error
    else {
        return failed(&error.to_string(), None);
    };
    let action = |label: &str, command: &str, args: Value| {
        Some(json!({ "label": label, "command": command, "args": args }))
    };
    let id = video.id.clone();
    match why {
        Unplayable::Upcoming => {
            tracing::info!(video_id = id, "{reason}");
            app().ext.events.emit(
                "player.upcoming",
                &json!({ "video_id": id, "message": reason }),
            );
            glib::timeout_add_local_once(UPCOMING_RECHECK, move || {
                if GENERATION.with(Cell::get) == generation {
                    look_up(video, start, generation);
                }
            });
        }
        Unplayable::SignIn if !runtime::block_on(app().session.is_signed_in()) => failed(
            &reason,
            action("Sign in", "app.navigate", json!({ "page": "signin" })),
        ),
        // Signed in, but YouTube wants to see the account's age confirmed.
        Unplayable::SignIn => failed(
            &reason,
            action(
                "Open on YouTube",
                "video.open_in_browser",
                json!({ "video": { "id": id } }),
            ),
        ),
        Unplayable::MembersOnly => {
            let url = match channel_id {
                Some(channel) => format!("https://www.youtube.com/channel/{channel}/join"),
                None => format!("https://www.youtube.com/watch?v={id}"),
            };
            failed(
                &reason,
                action("Join", "app.open_url", json!({ "url": url })),
            )
        }
        Unplayable::Other => failed(&reason, None),
    }
}

/// Hands the streams to the player once YouTube serves them: with a
/// pre-roll ad it holds them back for a few seconds (see
/// `yt_core::streams::preroll`), which `player.waiting` announces.
async fn load(video_id: &str, streams: &StreamSet, start: f64, generation: u64) {
    if let Some(at) = streams.available_at {
        let left = at.saturating_duration_since(std::time::Instant::now());
        if !left.is_zero() {
            app().ext.events.emit(
                "player.waiting",
                &json!({ "video_id": video_id, "seconds": left.as_secs_f64().ceil() }),
            );
        }
        let (resolver, set) = (app().streams.clone(), streams.clone());
        runtime::background(async move { resolver.until_served(&set).await }).await;
        if GENERATION.with(Cell::get) != generation {
            return;
        }
    }
    if let Err(e) = app().player.load(video_id, streams, start) {
        failed(&format!("mpv could not play the video: {e}"), None);
    }
}

pub fn stop() {
    cancel_up_next();
    GENERATION.with(|g| g.set(g.get() + 1));
    SHOWN.with(|s| s.borrow_mut().take());
    app().player.stop();
    runtime::release_memory_soon();
}

/// The video playing (or loaded), as the watch page was opened for it.
pub fn current() -> Option<Video> {
    CURRENT
        .with(|c| c.borrow().clone())
        .filter(|_| app().player.state().video_id.is_some())
}

/// Back to the watch page of the video playing in the mini player,
/// without starting it over.
pub fn expand() -> Result<(), String> {
    let video = current().ok_or("no video is playing")?;
    app().navigate("watch", json!({ "video": video }), true);
    Ok(())
}

/// The video after the one shown, if YouTube suggested any.
pub fn up_next() -> Option<Video> {
    UP_NEXT.with(|u| u.borrow().first().cloned())
}

/// Plays the next video.
pub fn next() -> Result<(), String> {
    let video = up_next().ok_or("there is no next video")?;
    play(video, None);
    Ok(())
}

/// Stops the countdown to the next video.
pub fn cancel_up_next() {
    if let Some(timer) = COUNTDOWN.with(|c| c.borrow_mut().take()) {
        timer.remove();
        app()
            .ext
            .events
            .emit("player.up_next_cancelled", &Value::Null);
    }
}

/// When a video ends: the sleep timer for the end of the video pauses
/// here; otherwise, with autoplay on, the next video plays after a
/// countdown (`player.up_next`).
fn ended(state: &PlayerState) {
    if SLEEP.with(|s| s.borrow().as_ref().is_some_and(|s| s.at.is_none())) {
        set_sleep_timer(None);
        app().toast("Sleep timer: paused at the end of the video");
        return;
    }
    if state.looping || !app().settings.get().autoplay {
        return;
    }
    let Some(video) = up_next() else { return };
    cancel_up_next();
    app().ext.events.emit(
        "player.up_next",
        &json!({ "video": video, "seconds": UP_NEXT_DELAY }),
    );
    let timer = glib::timeout_add_seconds_local_once(UP_NEXT_DELAY, || {
        COUNTDOWN.with(|c| c.borrow_mut().take());
        if let Err(e) = next() {
            app().toast(&e);
        }
    });
    COUNTDOWN.with(|c| *c.borrow_mut() = Some(timer));
}

/// Shows captions in `lang` (loading them first), or hides them; and
/// remembers the choice for the next videos.
pub fn set_captions(lang: Option<String>) -> Result<(), String> {
    app().settings.update(|s| s.captions = lang.clone());
    show_captions(lang.as_deref())
}

fn show_captions(lang: Option<&str>) -> Result<(), String> {
    let app = app();
    let Some(lang) = lang else {
        app.player.select_captions(None);
        return Ok(());
    };
    let streams = app.player.streams().ok_or("no video is playing")?;
    let caption = best_caption(&streams.captions, lang)
        .ok_or("this video has no captions in that language")?
        .clone();
    if app.player.has_captions(&caption.lang) {
        app.player.select_captions(Some(&caption.lang));
        return Ok(());
    }
    let video_id = app.player.state().video_id;
    runtime::spawn_local(async move {
        let http = crate::app::app().http.clone();
        let url = caption.url.clone();
        let result =
            runtime::background(async move { yt_core::captions::fetch(&http, &url).await }).await;
        let app = crate::app::app();
        if app.player.state().video_id != video_id {
            return;
        }
        match result {
            Ok(srt) => app
                .player
                .show_captions(&caption.lang, &caption.label(), srt),
            Err(e) => app.toast(&format!("Couldn't load the captions: {e}")),
        }
    });
    Ok(())
}

/// The track for a language: an exact match, then the language without a
/// region, then another region; uploaded captions before automatic ones.
pub fn best_caption<'a>(
    captions: &'a [yt_core::streams::Caption],
    lang: &str,
) -> Option<&'a yt_core::streams::Caption> {
    let base = |l: &str| l.split('-').next().unwrap_or(l).to_lowercase();
    captions
        .iter()
        .filter(|c| base(&c.lang) == base(lang))
        .min_by_key(|c| (c.lang != lang, c.lang.to_lowercase() != base(lang), c.auto))
}

/// A new video turns on the captions the user last chose, if it has them.
fn restore_captions(state: &PlayerState) {
    if state.captions.is_some() {
        return;
    }
    if let Some(lang) = app().settings.get().captions
        && app()
            .player
            .streams()
            .is_some_and(|s| best_caption(&s.captions, &lang).is_some())
    {
        let _ = show_captions(Some(&lang));
    }
}

/// Pauses after `minutes`, or at the end of the video with `end_of_video`;
/// `None` turns the timer off. Announced as `player.sleep_timer`.
pub fn set_sleep_timer(minutes: Option<f64>) {
    set_sleep(minutes, minutes.is_some());
}

/// Pauses at the end of the video.
pub fn sleep_at_end() {
    set_sleep(None, true);
}

fn set_sleep(minutes: Option<f64>, on: bool) {
    if let Some(old) = SLEEP.with(|s| s.borrow_mut().take())
        && let Some(timer) = old.timer
    {
        timer.remove();
    }
    let mut payload = json!({ "on": on, "end_of_video": on && minutes.is_none() });
    if on {
        let at = minutes.map(|m| {
            let seconds = (m * 60.0).max(1.0) as u64;
            (
                glib::timeout_add_seconds_local_once(seconds as u32, || {
                    SLEEP.with(|s| s.borrow_mut().take());
                    let app = app();
                    app.player.set_paused(true);
                    app.toast("Sleep timer: paused");
                    app.ext
                        .events
                        .emit("player.sleep_timer", &json!({ "on": false }));
                }),
                now() + seconds,
            )
        });
        payload["ends_at"] = json!(at.as_ref().map(|(_, at)| *at));
        SLEEP.with(|s| {
            *s.borrow_mut() = Some(Sleep {
                at: at.as_ref().map(|(_, at)| *at),
                timer: at.map(|(timer, _)| timer),
            })
        });
    }
    app().ext.events.emit("player.sleep_timer", &payload);
}

/// The sleep timer: off, at the end of the video, or the minutes left.
pub fn sleep_timer() -> SleepTimer {
    SLEEP.with(|s| match s.borrow().as_ref() {
        None => SleepTimer::Off,
        Some(Sleep { at: None, .. }) => SleepTimer::EndOfVideo,
        Some(Sleep { at: Some(at), .. }) => {
            SleepTimer::Minutes(at.saturating_sub(now()) as f64 / 60.0)
        }
    })
}

pub enum SleepTimer {
    Off,
    EndOfVideo,
    Minutes(f64),
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Tells the player view (and plugins) playback failed, with an action to
/// offer: `{ label, command, args }`.
fn failed(message: &str, action: Option<Value>) {
    let app = app();
    tracing::warn!("{message}");
    app.ext.events.emit(
        "player.error",
        &json!({ "message": message, "action": action }),
    );
}

/// Where to resume when YouTube didn't say: the watched fraction on the
/// feed tile, unless the video was (nearly) finished.
fn fallback_resume(video: &Video) -> Option<f64> {
    let progress = video.progress.filter(|p| *p < 0.95)?;
    let seconds = parse_duration(video.duration.as_deref()?)?;
    Some(progress as f64 * seconds)
}

/// Parses "1:02:03" or "12:34" into seconds.
pub fn parse_duration(text: &str) -> Option<f64> {
    text.split(':').try_fold(0.0, |total, part| {
        Some(total * 60.0 + part.trim().parse::<f64>().ok()?)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caption_choice() {
        let track = |lang: &str, auto| yt_core::streams::Caption {
            lang: lang.into(),
            name: lang.into(),
            url: String::new(),
            auto,
        };
        let tracks = [
            track("en", true),
            track("en-GB", false),
            track("en", false),
            track("de", false),
        ];
        let pick = |lang| best_caption(&tracks, lang).map(|c| (c.lang.as_str(), c.auto));
        assert_eq!(pick("en"), Some(("en", false)), "uploaded before automatic");
        assert_eq!(
            pick("en-GB"),
            Some(("en-GB", false)),
            "the exact region first"
        );
        assert_eq!(pick("en-US"), Some(("en", false)), "else the same language");
        assert_eq!(pick("fr"), None);
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("12:34"), Some(754.0));
        assert_eq!(parse_duration("1:02:03"), Some(3723.0));
        assert_eq!(parse_duration("LIVE"), None);
    }
}
