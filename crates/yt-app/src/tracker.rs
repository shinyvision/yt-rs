//! Reports watch time to YouTube while videos play, so they land in the
//! watch history with a resume position and tune the recommendations.
//!
//! The protocol lives in [`yt_core::tracking::WatchSession`]; this module
//! feeds it the player's events and sends the reports it produces.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use serde_json::json;
use yt_core::innertube::InnerTube;
use yt_core::tracking::{self, Tracking, WatchSession};

use crate::app::app;
use crate::player::{Player, PlayerEvent, PlayerState};
use crate::runtime;

struct Active {
    video_id: String,
    session: WatchSession,
}

pub struct WatchTracker {
    youtube: Arc<InnerTube>,
    active: RefCell<Option<Active>>,
    origin: Instant,
    enabled: std::cell::Cell<bool>,
}

impl WatchTracker {
    pub fn new(youtube: Arc<InnerTube>, _http: reqwest::Client) -> Rc<Self> {
        Rc::new(Self {
            youtube,
            active: RefCell::new(None),
            origin: Instant::now(),
            enabled: std::cell::Cell::new(true),
        })
    }

    /// Turns reporting on or off, like pausing YouTube's watch history.
    /// Takes effect from the next video.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.set(enabled);
    }

    /// Gets ready to report on a video that is about to play, and returns
    /// where YouTube says to resume it. Reporting is skipped (with a log
    /// message) if YouTube returns no reporting URLs.
    pub async fn prepare(&self, video_id: &str) -> Option<f64> {
        self.finish();
        if !self.enabled.get() {
            return None;
        }
        let cpn = tracking::new_cpn();
        let (youtube, id, c) = (self.youtube.clone(), video_id.to_owned(), cpn.clone());
        let resp = match runtime::background(async move { youtube.player(&id, &c).await }).await {
            Ok(resp) => resp,
            Err(e) => {
                tracing::warn!(video_id, error = %e, "can't report watch time");
                return None;
            }
        };
        let resume = tracking::start_seconds(&resp);
        match Tracking::from_player_response(&resp) {
            Some(t) => {
                *self.active.borrow_mut() = Some(Active {
                    video_id: video_id.to_owned(),
                    session: WatchSession::new(t, cpn),
                });
            }
            None => {
                let reason = tracking::unplayable_reason(&resp)
                    .unwrap_or_else(|| "no reporting URLs".into());
                tracing::warn!(
                    video_id,
                    reason,
                    "YouTube gave no watch-time reporting URLs"
                );
            }
        }
        resume
    }

    /// Follows the player's events.
    pub fn attach(self: &Rc<Self>, player: &Player) {
        let this = Rc::downgrade(self);
        player.connect(move |event, state| {
            if let Some(this) = this.upgrade() {
                this.on_player_event(event, state);
            }
        });
    }

    /// Sends the final report for the current video, if any.
    pub fn finish(&self) {
        let position = app().player.state().position;
        self.with_session(|s, now| s.finish(now, position));
        self.active.borrow_mut().take();
    }

    /// Like [`WatchTracker::finish`], but sends the report before returning,
    /// for when the app is quitting and the main loop no longer runs.
    pub fn finish_blocking(&self, timeout: std::time::Duration) {
        let position = app().player.state().position;
        let now = self.origin.elapsed().as_secs_f64();
        let Some(mut active) = self.active.borrow_mut().take() else {
            return;
        };
        let youtube = self.youtube.clone();
        let reports = active.session.finish(now, position);
        runtime::block_on(async move {
            for url in reports {
                let _ = tokio::time::timeout(timeout, youtube.report(&url)).await;
            }
        });
    }

    fn on_player_event(&self, event: &PlayerEvent, state: &PlayerState) {
        let is_current =
            |active: &Active| state.video_id.as_deref() == Some(active.video_id.as_str());
        if !self.active.borrow().as_ref().is_some_and(is_current) {
            return;
        }
        match event {
            PlayerEvent::Position(position) => {
                let playing = !state.paused && !state.buffering;
                self.with_session(|s, now| {
                    let mut reports = if playing {
                        s.playing(now, *position)
                    } else {
                        Vec::new()
                    };
                    reports.extend(s.position(now, *position));
                    reports
                });
            }
            PlayerEvent::Paused(true) => self.with_session(|s, now| s.paused(now, state.position)),
            PlayerEvent::Seeking { from, .. } => self.with_session(|s, _| {
                s.seeking(*from);
                Vec::new()
            }),
            PlayerEvent::Seeked { position } => self.with_session(|s, _| {
                s.seeked(*position);
                Vec::new()
            }),
            PlayerEvent::Ended => self.finish(),
            PlayerEvent::Stopped | PlayerEvent::Error(_) => self.finish(),
            _ => {}
        }
    }

    /// Runs `f` on the current session and sends the reports it returns.
    fn with_session(&self, f: impl FnOnce(&mut WatchSession, f64) -> Vec<String>) {
        let now = self.origin.elapsed().as_secs_f64();
        let (video_id, reports) = {
            let mut active = self.active.borrow_mut();
            let Some(active) = active.as_mut() else {
                return;
            };
            (active.video_id.clone(), f(&mut active.session, now))
        };
        for url in reports {
            self.send(&video_id, url);
        }
    }

    fn send(&self, video_id: &str, url: String) {
        let kind = if url.contains("/stats/playback") {
            "playback"
        } else {
            "watchtime"
        };
        let youtube = self.youtube.clone();
        let video_id = video_id.to_owned();
        runtime::spawn_local(async move {
            let result = runtime::background({
                let url = url.clone();
                async move { youtube.report(&url).await }
            })
            .await;
            match &result {
                Ok(()) => tracing::debug!(video_id, kind, "reported watch time"),
                Err(e) => tracing::warn!(video_id, kind, error = %e, "watch-time report failed"),
            }
            app().ext.events.emit(
                "watch.reported",
                &json!({
                    "video_id": video_id,
                    "kind": kind,
                    "ok": result.is_ok(),
                    "url": url,
                }),
            );
        });
    }
}
