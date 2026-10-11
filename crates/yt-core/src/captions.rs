//! Caption tracks: YouTube's json3 format turned into SRT for the player.
//!
//! json3 is what YouTube's own player reads: a list of events, each a line
//! of text with a start, a duration and the moment each word appears.
//! Automatic captions roll: a line stays up while the next one is spoken
//! below it. Players stack overlapping cues the other way round (the newer
//! one on top), so the conversion makes one cue per moment instead, holding
//! the lines on screen in order, older first, each with the words spoken so
//! far. (YouTube's VTT for automatic captions repeats every line in the next
//! cue, which other players show twice.)

use std::fmt::Write;

use serde::Deserialize;

use crate::error::Result;

/// Lines shown at once, as YouTube shows rolling captions.
const MAX_LINES: usize = 2;

#[derive(Deserialize)]
struct Track {
    #[serde(default)]
    events: Vec<Event>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Event {
    #[serde(default)]
    t_start_ms: u64,
    #[serde(default)]
    d_duration_ms: u64,
    #[serde(default)]
    segs: Vec<Segment>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Segment {
    #[serde(default)]
    utf8: String,
    /// When the word appears, after the event's start.
    #[serde(default)]
    t_offset_ms: u64,
}

/// A line of captions, with when each of its words appears (ms).
struct Line {
    start: u64,
    end: u64,
    words: Vec<(u64, String)>,
}

impl Line {
    /// The line as it reads at `time`: the words spoken so far.
    fn at(&self, time: u64) -> String {
        let text: String = self
            .words
            .iter()
            .filter(|(at, _)| *at <= time)
            .map(|(_, word)| word.as_str())
            .collect();
        text.trim().to_owned()
    }
}

/// Downloads a caption track and returns it as SRT.
pub async fn fetch(http: &reqwest::Client, url: &str) -> Result<String> {
    let body = http
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(json3_to_srt(&body)?)
}

/// Converts a json3 caption track to SRT, one cue per moment the text on
/// screen changes. Events without text (YouTube's window and style setup,
/// line breaks of their own) are left out.
pub fn json3_to_srt(json: &str) -> serde_json::Result<String> {
    let track: Track = serde_json::from_str(json)?;
    let mut lines: Vec<Line> = track
        .events
        .into_iter()
        .filter(|e| e.d_duration_ms > 0 && e.segs.iter().any(|s| !s.utf8.trim().is_empty()))
        .map(|e| Line {
            start: e.t_start_ms,
            end: e.t_start_ms + e.d_duration_ms,
            words: e
                .segs
                .into_iter()
                .map(|s| (e.t_start_ms + s.t_offset_ms, s.utf8))
                .collect(),
        })
        .collect();
    lines.sort_by_key(|l| l.start);

    // Every moment the text can change: lines starting and ending, words
    // appearing.
    let mut moments: Vec<u64> = lines
        .iter()
        .flat_map(|l| {
            [l.start, l.end]
                .into_iter()
                .chain(l.words.iter().map(|w| w.0))
        })
        .collect();
    moments.sort_unstable();
    moments.dedup();

    // Sweep through them, keeping the lines on screen.
    let mut cues: Vec<(u64, u64, String)> = Vec::new();
    let mut next = 0;
    let mut showing: Vec<usize> = Vec::new();
    for pair in moments.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        while next < lines.len() && lines[next].start <= from {
            showing.push(next);
            next += 1;
        }
        showing.retain(|&i| lines[i].end > from);
        let texts: Vec<String> = showing
            .iter()
            .map(|&i| lines[i].at(from))
            .filter(|t| !t.is_empty())
            .collect();
        let text = texts[texts.len().saturating_sub(MAX_LINES)..].join("\n");
        if text.is_empty() {
            continue;
        }
        match cues.last_mut() {
            // The same text carries on.
            Some(last) if last.1 == from && last.2 == text => last.1 = to,
            _ => cues.push((from, to, text)),
        }
    }

    let mut srt = String::new();
    for (number, (start, end, text)) in cues.iter().enumerate() {
        let _ = write!(
            srt,
            "{}\n{} --> {}\n{text}\n\n",
            number + 1,
            timestamp(*start),
            timestamp(*end)
        );
    }
    Ok(srt)
}

/// `01:02:03,456`
fn timestamp(ms: u64) -> String {
    format!(
        "{:02}:{:02}:{:02},{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uploaded_captions_become_cues() {
        let json = r#"{"wireMagic":"pb3","events":[
            {"tStartMs":0,"dDurationMs":3720000,"id":1,"wpWinPosId":1,"wsWinStyleId":1},
            {"tStartMs":1360,"dDurationMs":2400,"segs":[{"utf8":"we're no strangers"}]},
            {"tStartMs":3723456,"dDurationMs":1000,"segs":[{"utf8":"to love"}]}
        ]}"#;
        assert_eq!(
            json3_to_srt(json).unwrap(),
            "1\n00:00:01,360 --> 00:00:03,760\nwe're no strangers\n\n\
             2\n01:02:03,456 --> 01:02:04,456\nto love\n\n"
        );
    }

    /// Automatic captions: words appear one by one, and a line stays up
    /// above the next one until it ends.
    #[test]
    fn rolling_captions_keep_the_older_line_on_top() {
        let json = r#"{"events":[
            {"tStartMs":0,"dDurationMs":5000,"segs":[
                {"utf8":"What happens"},
                {"utf8":" when there is no image at","tOffsetMs":1000}]},
            {"tStartMs":2900,"dDurationMs":10,"aAppend":1,"segs":[{"utf8":"\n"}]},
            {"tStartMs":3000,"dDurationMs":3000,"segs":[{"utf8":"all?"}]}
        ]}"#;
        assert_eq!(
            json3_to_srt(json).unwrap(),
            "1\n00:00:00,000 --> 00:00:01,000\nWhat happens\n\n\
             2\n00:00:01,000 --> 00:00:03,000\nWhat happens when there is no image at\n\n\
             3\n00:00:03,000 --> 00:00:05,000\nWhat happens when there is no image at\nall?\n\n\
             4\n00:00:05,000 --> 00:00:06,000\nall?\n\n"
        );
    }

    #[test]
    fn shows_at_most_two_lines() {
        let json = r#"{"events":[
            {"tStartMs":0,"dDurationMs":3000,"segs":[{"utf8":"one"}]},
            {"tStartMs":1000,"dDurationMs":3000,"segs":[{"utf8":"two"}]},
            {"tStartMs":2000,"dDurationMs":3000,"segs":[{"utf8":"three"}]}
        ]}"#;
        let srt = json3_to_srt(json).unwrap();
        assert!(
            srt.contains("00:00:02,000 --> 00:00:04,000\ntwo\nthree\n"),
            "{srt}"
        );
        assert!(!srt.contains("one\ntwo\nthree"), "{srt}");
    }
}
