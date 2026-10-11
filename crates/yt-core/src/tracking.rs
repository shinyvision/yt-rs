//! Reporting watch time to YouTube, so videos show up in the watch history
//! with a resume position and shape the recommendations.
//!
//! This follows the official TV app's protocol:
//!
//! 1. A signed-in `player` request returns `playbackTracking`, with base
//!    URLs for two kinds of report. The request needs the current
//!    `signatureTimestamp` from the TV player script; without it YouTube
//!    returns no tracking URLs.
//! 2. One **playback** report when playback starts creates the history entry.
//! 3. **Watchtime** reports carry the media ranges played since the previous
//!    report (`st`/`et`, comma-separated) and the position (`cmt`). They are
//!    sent 10, 20 and 30 seconds into the session and every 40 seconds after
//!    that (the schedule comes from the response), and also on pause, on
//!    resume and at the end, where the last one carries `final=1`.
//!
//! All reports of a session share one client playback nonce (`cpn`) and are
//! sent with the account's bearer token. [`WatchSession`] is a pure state
//! machine: it is told what the player does and when, and returns the report
//! URLs to send, which keeps the protocol testable.

use serde_json::Value;
use url::Url;

use crate::innertube::{CLIENT_NAME, TV_DOWNGRADED};

const CPN_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// A new client playback nonce: 16 random characters that tie together the
/// reports of one playback.
#[yt_export::export("tracking")]
pub fn new_cpn() -> String {
    (0..16)
        .map(|_| CPN_ALPHABET[(rand::random::<u8>() & 63) as usize] as char)
        .collect()
}

/// What a `player` response says about reporting.
#[derive(Clone, Debug, PartialEq)]
pub struct Tracking {
    pub playback_url: String,
    pub watchtime_url: String,
    /// Seconds into the session for the first reports, e.g. `[10, 20, 30]`.
    pub scheduled: Vec<f64>,
    /// Seconds between reports after the scheduled ones.
    pub interval: f64,
    /// The video's length in seconds.
    pub length: f64,
}

impl Tracking {
    /// Reads the reporting URLs out of a `player` response. `None` if it has
    /// none, e.g. because the video is unplayable or the response wasn't
    /// signed in.
    pub fn from_player_response(resp: &Value) -> Option<Self> {
        let tracking = resp.get("playbackTracking")?;
        let url = |key: &str| {
            tracking
                .pointer(&format!("/{key}/baseUrl"))?
                .as_str()
                .map(rewrite_host)
        };
        let playback_url = url("videostatsPlaybackUrl")?;
        let watchtime_url = url("videostatsWatchtimeUrl")?;
        let scheduled: Vec<f64> = tracking
            .get("videostatsScheduledFlushWalltimeSeconds")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_f64).collect())
            .filter(|v: &Vec<f64>| !v.is_empty())
            .unwrap_or_else(|| vec![10.0, 20.0, 30.0]);
        let interval = tracking
            .get("videostatsDefaultFlushIntervalSeconds")
            .and_then(Value::as_f64)
            .unwrap_or(40.0);
        let length = resp
            .pointer("/videoDetails/lengthSeconds")
            .and_then(|v| {
                v.as_str()
                    .and_then(|s| s.parse().ok())
                    .or_else(|| v.as_f64())
            })
            .or_else(|| query_param(&watchtime_url, "len").and_then(|l| l.parse().ok()))
            .unwrap_or(0.0);
        Some(Self {
            playback_url,
            watchtime_url,
            scheduled,
            interval,
            length,
        })
    }
}

/// The resume position the player response asks to start at, if any.
#[yt_export::export("tracking")]
pub fn start_seconds(resp: &Value) -> Option<f64> {
    let v = resp.pointer("/playerConfig/playbackStartConfig/startSeconds")?;
    v.as_f64()
        .or_else(|| v.as_str()?.parse().ok())
        .filter(|s| *s > 0.0)
}

/// Why a player response has no reporting URLs, from its playability status.
#[yt_export::export("tracking")]
pub fn unplayable_reason(resp: &Value) -> Option<String> {
    let status = resp.pointer("/playabilityStatus/status")?.as_str()?;
    if status == "OK" {
        return None;
    }
    let reason = resp
        .pointer("/playabilityStatus/reason")
        .and_then(Value::as_str)
        .unwrap_or("");
    Some(format!("{status}: {reason}"))
}

/// Reports are sent to the page's host, as the official player does.
fn rewrite_host(url: &str) -> String {
    url.replacen("://s.youtube.com/", "://www.youtube.com/", 1)
}

fn query_param(url: &str, key: &str) -> Option<String> {
    Url::parse(url)
        .ok()?
        .query_pairs()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

/// Formats seconds the way the official player does: up to three decimals,
/// without trailing zeros.
fn num(seconds: f64) -> String {
    let s = format!("{:.3}", seconds.max(0.0));
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() {
        "0".into()
    } else {
        s.to_owned()
    }
}

/// One playback's reporting. Every method takes `now`, wall-clock seconds
/// from any fixed origin, and returns the report URLs to send (GET, with the
/// bearer token).
#[derive(Debug)]
pub struct WatchSession {
    tracking: Tracking,
    cpn: String,
    /// `now` when playback first started; report times are relative to it.
    origin: Option<f64>,
    playing: bool,
    /// Where the media range being played started.
    range_start: Option<f64>,
    position: f64,
    /// Ranges played since the last watchtime report.
    ranges: Vec<(f64, f64)>,
    /// How many scheduled reports were sent, and when the next one is due.
    sent: usize,
    next_due: f64,
    finished: bool,
}

#[yt_export::export(
    skip = "a video's watch-time session, which the tracker drives (plugins use tracker.*)"
)]
impl WatchSession {
    pub fn new(tracking: Tracking, cpn: String) -> Self {
        Self {
            tracking,
            cpn,
            origin: None,
            playing: false,
            range_start: None,
            position: 0.0,
            ranges: Vec::new(),
            sent: 0,
            next_due: 0.0,
            finished: false,
        }
    }

    pub fn cpn(&self) -> &str {
        &self.cpn
    }

    /// Playback started or resumed at `position`.
    pub fn playing(&mut self, now: f64, position: f64) -> Vec<String> {
        if self.finished || self.playing {
            return Vec::new();
        }
        self.playing = true;
        self.position = position;
        self.range_start = Some(position);
        match self.origin {
            None => {
                // The first start: the playback report creates the history entry.
                self.origin = Some(now);
                self.next_due = self.delay_after(0);
                vec![self.playback_report(now)]
            }
            Some(_) => {
                self.next_due = self.rt(now).ceil() + self.delay_after(self.sent);
                vec![self.watchtime_report(now, "playing", false)]
            }
        }
    }

    /// The position moved during normal playback. Sends a scheduled report
    /// when one is due.
    pub fn position(&mut self, now: f64, position: f64) -> Vec<String> {
        if self.finished || !self.playing {
            return Vec::new();
        }
        self.position = position;
        if self.rt(now) < self.next_due {
            return Vec::new();
        }
        self.close_range();
        self.range_start = Some(position);
        self.sent += 1;
        self.next_due = self.rt(now).ceil() + self.delay_after(self.sent);
        vec![self.watchtime_report(now, "playing", false)]
    }

    /// Playback paused at `position`.
    pub fn paused(&mut self, now: f64, position: f64) -> Vec<String> {
        if self.finished || !self.playing {
            return Vec::new();
        }
        self.position = position;
        self.close_range();
        self.playing = false;
        vec![self.watchtime_report(now, "paused", false)]
    }

    /// The user is seeking away from `from`. The range played so far ends
    /// there; a new one starts when playback continues at the new position.
    pub fn seeking(&mut self, from: f64) {
        if self.finished {
            return;
        }
        if self.range_start.is_some() {
            self.position = from;
            self.close_range();
        }
    }

    /// A seek finished and playback continues at `position`.
    pub fn seeked(&mut self, position: f64) {
        self.position = position;
        if self.playing && !self.finished {
            self.range_start = Some(position);
        }
    }

    /// Playback ended or the video was closed: the last report.
    pub fn finish(&mut self, now: f64, position: f64) -> Vec<String> {
        if self.finished || self.origin.is_none() {
            self.finished = true;
            return Vec::new();
        }
        if self.playing {
            self.position = position;
            self.close_range();
        }
        self.playing = false;
        self.finished = true;
        vec![self.watchtime_report(now, "paused", true)]
    }

    fn close_range(&mut self) {
        if let Some(start) = self.range_start.take()
            && self.position > start + 0.01
        {
            self.ranges.push((start, self.position));
        }
    }

    /// Seconds from report `index` to the next one.
    fn delay_after(&self, index: usize) -> f64 {
        let scheduled = &self.tracking.scheduled;
        match index {
            0 => scheduled[0],
            i if i < scheduled.len() => scheduled[i] - scheduled[i - 1],
            _ => self.tracking.interval,
        }
    }

    fn rt(&self, now: f64) -> f64 {
        now - self.origin.unwrap_or(now)
    }

    fn playback_report(&self, now: f64) -> String {
        self.build(
            &self.tracking.playback_url,
            &[
                ("cmt", num(self.position)),
                ("rt", num(self.rt(now))),
                ("rtn", num(self.next_due)),
            ],
        )
    }

    fn watchtime_report(&mut self, now: f64, state: &str, last: bool) -> String {
        let mut params = vec![
            ("cmt", num(self.position)),
            ("rt", num(self.rt(now))),
            ("rtn", num(self.next_due)),
            ("state", state.to_owned()),
        ];
        if !self.ranges.is_empty() {
            let join = |f: fn(&(f64, f64)) -> f64| {
                self.ranges
                    .iter()
                    .map(|r| num(f(r)))
                    .collect::<Vec<_>>()
                    .join(",")
            };
            params.push(("st", join(|r| r.0)));
            params.push(("et", join(|r| r.1)));
        }
        if last {
            params.push(("final", "1".into()));
        }
        self.ranges.clear();
        self.build(&self.tracking.watchtime_url, &params)
    }

    /// The base URL with the given parameters set, keeping its own (`ei`,
    /// `vm`, `of`, `plid`, ... must stay) and replacing duplicates.
    fn build(&self, base: &str, params: &[(&str, String)]) -> String {
        let Ok(mut url) = Url::parse(base) else {
            return base.to_owned();
        };
        let mut set: Vec<(&str, String)> = vec![
            ("ns", "yt".into()),
            ("ver", "2".into()),
            ("cpn", self.cpn.clone()),
            ("len", num(self.tracking.length)),
            ("c", CLIENT_NAME.into()),
            ("cver", TV_DOWNGRADED.version.into()),
        ];
        set.extend(params.iter().cloned());
        let keep: Vec<(String, String)> = url
            .query_pairs()
            .filter(|(k, _)| !set.iter().any(|(s, _)| s == k))
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        url.query_pairs_mut()
            .clear()
            .extend_pairs(keep)
            .extend_pairs(set);
        url.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tracking() -> Tracking {
        Tracking::from_player_response(&json!({
            "videoDetails": { "lengthSeconds": "600" },
            "playbackTracking": {
                "videostatsPlaybackUrl": { "baseUrl": "https://s.youtube.com/api/stats/playback?cl=1&docid=vid&ei=EI&of=OF&vm=VM&len=600&ns=yt&el=leanback" },
                "videostatsWatchtimeUrl": { "baseUrl": "https://s.youtube.com/api/stats/watchtime?cl=1&docid=vid&ei=EI&of=OF&vm=VM&len=600&ns=yt&el=leanback" },
                "videostatsScheduledFlushWalltimeSeconds": [10, 20, 30],
                "videostatsDefaultFlushIntervalSeconds": 40
            }
        }))
        .unwrap()
    }

    fn params(url: &str) -> std::collections::HashMap<String, String> {
        Url::parse(url)
            .unwrap()
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect()
    }

    #[test]
    fn reads_tracking_from_the_player_response() {
        let t = tracking();
        assert!(
            t.playback_url
                .starts_with("https://www.youtube.com/api/stats/playback?")
        );
        assert_eq!(
            (t.scheduled.as_slice(), t.interval, t.length),
            ([10.0, 20.0, 30.0].as_slice(), 40.0, 600.0)
        );
        assert_eq!(
            Tracking::from_player_response(
                &json!({ "playabilityStatus": { "status": "UNPLAYABLE" } })
            ),
            None
        );
    }

    #[test]
    fn session_follows_the_official_schedule() {
        let mut s = WatchSession::new(tracking(), "CPN".into());
        // Starts at a resume position of 100 s.
        let [playback] = s.playing(1000.0, 100.0).try_into().unwrap();
        let p = params(&playback);
        assert!(playback.starts_with("https://www.youtube.com/api/stats/playback?"));
        assert_eq!(
            (
                p["cpn"].as_str(),
                p["cmt"].as_str(),
                p["ver"].as_str(),
                p["ei"].as_str(),
                p["of"].as_str()
            ),
            ("CPN", "100", "2", "EI", "OF")
        );
        assert_eq!(p["c"], "TVHTML5");

        // Nothing until 10 s in.
        assert!(s.position(1009.5, 109.5).is_empty());
        let [first] = s.position(1010.0, 110.0).try_into().unwrap();
        let p = params(&first);
        assert_eq!(
            (
                p["st"].as_str(),
                p["et"].as_str(),
                p["state"].as_str(),
                p["rt"].as_str()
            ),
            ("100", "110", "playing", "10")
        );
        assert!(!p.contains_key("final"));

        // Then at 20 and 30 s, then every 40 s.
        assert!(s.position(1019.0, 119.0).is_empty());
        assert_eq!(s.position(1020.0, 120.0).len(), 1);
        assert_eq!(s.position(1030.0, 130.0).len(), 1);
        assert!(s.position(1069.0, 169.0).is_empty());
        let [later] = s.position(1070.0, 170.0).try_into().unwrap();
        assert_eq!(
            (params(&later)["st"].as_str(), params(&later)["et"].as_str()),
            ("130", "170")
        );
    }

    #[test]
    fn pause_seek_resume_and_finish() {
        let mut s = WatchSession::new(tracking(), "CPN".into());
        s.playing(0.0, 0.0);
        s.position(4.0, 4.0);
        // Seek from 4 s to 60 s, play to 65 s, then pause.
        s.seeking(4.0);
        s.seeked(60.0);
        s.position(9.0, 65.0);
        let [paused] = s.paused(9.0, 65.0).try_into().unwrap();
        let p = params(&paused);
        assert_eq!(
            (
                p["st"].as_str(),
                p["et"].as_str(),
                p["state"].as_str(),
                p["cmt"].as_str()
            ),
            ("0,60", "4,65", "paused", "65")
        );

        // Resuming reports without ranges; finishing sends final=1.
        let [resumed] = s.playing(30.0, 65.0).try_into().unwrap();
        assert!(!params(&resumed).contains_key("st"));
        assert_eq!(params(&resumed)["state"], "playing");
        let [last] = s.finish(35.5, 70.5).try_into().unwrap();
        let p = params(&last);
        assert_eq!(
            (p["st"].as_str(), p["et"].as_str(), p["final"].as_str()),
            ("65", "70.5", "1")
        );
        assert!(s.finish(40.0, 70.5).is_empty(), "only one final report");
        assert!(s.playing(41.0, 70.5).is_empty());
    }

    #[test]
    fn a_session_that_never_played_reports_nothing() {
        let mut s = WatchSession::new(tracking(), new_cpn());
        assert!(s.finish(5.0, 0.0).is_empty());
        assert_eq!(s.cpn().len(), 16);
    }

    #[test]
    fn number_format() {
        assert_eq!(
            (num(10.0), num(1.5), num(2.123456), num(-1.0)),
            ("10".into(), "1.5".into(), "2.123".into(), "0".into())
        );
    }
}
