//! Formatted text from YouTube (descriptions, comments) as structured runs:
//! plain text, bold and italic, and links to URLs, moments in the video,
//! channels, videos and hashtags.

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A piece of text with one style and at most one link.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub text: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bold: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub italic: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<Link>,
}

/// Where a link goes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Link {
    Url {
        url: String,
    },
    /// A moment in a video, e.g. "1:23" in a comment.
    Video {
        video_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start: Option<f64>,
    },
    Channel {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        handle: Option<String>,
    },
    Search {
        query: String,
    },
    Playlist {
        playlist_id: String,
    },
}

/// What a YouTube link points at: a video (and a moment in it), a
/// playlist, a channel or a search. `None` for other links.
pub fn parse_url(url: &str) -> Option<Link> {
    let parsed = url::Url::parse(url.trim()).ok()?;
    let host = parsed
        .host_str()?
        .trim_start_matches("www.")
        .trim_start_matches("m.");
    let query = |key: &str| {
        parsed
            .query_pairs()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.into_owned())
            .filter(|v| !v.is_empty())
    };
    let start = query("t")
        .or_else(|| query("start"))
        .and_then(|t| parse_offset(&t));
    let mut segments = parsed.path_segments()?.filter(|s| !s.is_empty());
    let first = segments.next();
    let video = |id: &str| {
        (id.len() == 11
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .then(|| Link::Video {
            video_id: id.to_owned(),
            start,
        })
    };
    match (host, first) {
        ("youtu.be", Some(id)) => video(id),
        ("youtube.com" | "youtube-nocookie.com", Some("watch")) => match query("v") {
            Some(id) => video(&id),
            None => query("list").map(|playlist_id| Link::Playlist { playlist_id }),
        },
        ("youtube.com" | "youtube-nocookie.com", Some("shorts" | "live" | "embed" | "v")) => {
            video(segments.next()?)
        }
        ("youtube.com", Some("playlist")) => {
            query("list").map(|playlist_id| Link::Playlist { playlist_id })
        }
        ("youtube.com", Some("channel")) => Some(Link::Channel {
            channel_id: Some(segments.next()?.to_owned()).filter(|id| id.starts_with("UC")),
            handle: None,
        })
        .filter(|l| {
            matches!(
                l,
                Link::Channel {
                    channel_id: Some(_),
                    ..
                }
            )
        }),
        ("youtube.com", Some(handle)) if handle.starts_with('@') && handle.len() > 1 => {
            Some(Link::Channel {
                channel_id: None,
                handle: Some(handle.to_owned()),
            })
        }
        ("youtube.com", Some("results")) => {
            query("search_query").map(|query| Link::Search { query })
        }
        _ => None,
    }
}

/// A link's moment: "90", "90s", "1m30s", "1h2m3s".
fn parse_offset(text: &str) -> Option<f64> {
    if let Ok(seconds) = text.parse::<f64>() {
        return Some(seconds);
    }
    let mut total = 0.0;
    let mut number = String::new();
    for c in text.chars() {
        match c {
            '0'..='9' => number.push(c),
            'h' | 'm' | 's' => {
                let n: f64 = number.parse().ok()?;
                number.clear();
                total += n * match c {
                    'h' => 3600.0,
                    'm' => 60.0,
                    _ => 1.0,
                };
            }
            _ => return None,
        }
    }
    number.is_empty().then_some(total)
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RichText {
    pub runs: Vec<Run>,
}

impl RichText {
    /// Text without formatting or links.
    pub fn from_plain(text: &str) -> Self {
        Self {
            runs: vec![Run {
                text: text.to_owned(),
                ..Run::default()
            }],
        }
    }

    pub fn plain(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.runs.iter().all(|r| r.text.is_empty())
    }

    /// The start of the text, at most `max_lines` lines and `max_chars`
    /// characters, for a folded description or a long comment. `None` if
    /// the whole text fits.
    pub fn truncated(&self, max_lines: usize, max_chars: usize) -> Option<RichText> {
        let (mut lines, mut chars) = (1, 0);
        let mut runs = Vec::new();
        for run in &self.runs {
            let mut cut = None;
            for (i, c) in run.text.char_indices() {
                chars += 1;
                if c == '\n' {
                    lines += 1;
                }
                if lines > max_lines || chars > max_chars {
                    cut = Some(i);
                    break;
                }
            }
            match cut {
                None => runs.push(run.clone()),
                Some(i) => {
                    runs.push(Run {
                        text: run.text[..i].trim_end().to_owned(),
                        ..run.clone()
                    });
                    runs.retain(|r| !r.text.is_empty());
                    return Some(RichText { runs });
                }
            }
        }
        None
    }

    /// Reads `{"runs": [...]}` or `{"simpleText": "..."}`. Plain stretches of
    /// text get links for URLs, timestamps (pointing into `video_id`, when
    /// given), @handles and #hashtags, as youtube.com shows them.
    pub fn from_json(node: Option<&Value>, video_id: Option<&str>) -> Self {
        let Some(node) = node else {
            return Self::default();
        };
        let mut runs = Vec::new();
        if let Some(s) = node.get("simpleText").and_then(Value::as_str) {
            linkify(s, false, false, video_id, &mut runs);
        }
        for run in node
            .get("runs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let text = run.get("text").and_then(Value::as_str).unwrap_or_default();
            let bold = run.get("bold").and_then(Value::as_bool).unwrap_or(false);
            let italic = run.get("italics").and_then(Value::as_bool).unwrap_or(false);
            match run.get("navigationEndpoint").and_then(link_from_endpoint) {
                Some(link) => runs.push(Run {
                    text: text.to_owned(),
                    bold,
                    italic,
                    link: Some(link),
                }),
                None => linkify(text, bold, italic, video_id, &mut runs),
            }
        }
        Self { runs }
    }
}

/// Reads the link out of a navigation endpoint.
pub(crate) fn link_from_endpoint(endpoint: &Value) -> Option<Link> {
    if let Some(url) = endpoint.pointer("/urlEndpoint/url").and_then(Value::as_str) {
        return Some(Link::Url {
            url: unwrap_redirect(url),
        });
    }
    if let Some(watch) = endpoint.get("watchEndpoint") {
        return Some(Link::Video {
            video_id: watch.get("videoId")?.as_str()?.to_owned(),
            start: watch.get("startTimeSeconds").and_then(Value::as_f64),
        });
    }
    if let Some(reel) = endpoint.get("reelWatchEndpoint") {
        return Some(Link::Video {
            video_id: reel.get("videoId")?.as_str()?.to_owned(),
            start: None,
        });
    }
    if let Some(browse) = endpoint.get("browseEndpoint") {
        let id = browse
            .get("browseId")
            .and_then(Value::as_str)
            .filter(|id| id.starts_with("UC"));
        let handle = browse
            .get("canonicalBaseUrl")
            .and_then(Value::as_str)
            .and_then(|u| u.strip_prefix('/'))
            .filter(|u| u.starts_with('@'));
        if id.is_some() || handle.is_some() {
            return Some(Link::Channel {
                channel_id: id.map(str::to_owned),
                handle: handle.map(str::to_owned),
            });
        }
    }
    if let Some(query) = endpoint
        .pointer("/searchEndpoint/query")
        .and_then(Value::as_str)
    {
        return Some(Link::Search {
            query: query.to_owned(),
        });
    }
    None
}

/// YouTube wraps outside links in `youtube.com/redirect?...&q=<url>`.
fn unwrap_redirect(url: &str) -> String {
    let Ok(parsed) = url::Url::parse(url) else {
        return url.to_owned();
    };
    if parsed.path() == "/redirect"
        && let Some((_, target)) = parsed.query_pairs().find(|(k, _)| k == "q")
    {
        return target.into_owned();
    }
    url.to_owned()
}

static LINKABLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?P<url>https?://[^\s<>()]+[^\s<>().,!?:;'\x22])|(?P<time>\b(?:\d{1,2}:)?\d{1,2}:\d{2}\b)|(?P<handle>(?:^|\B)@[\w.-]{3,})|(?P<tag>(?:^|\B)#\w+)")
        .expect("valid pattern")
});

/// Splits plain text into runs, linking URLs, timestamps, handles and hashtags.
fn linkify(text: &str, bold: bool, italic: bool, video_id: Option<&str>, out: &mut Vec<Run>) {
    let mut last = 0;
    let plain = |s: &str, out: &mut Vec<Run>| {
        if !s.is_empty() {
            out.push(Run {
                text: s.to_owned(),
                bold,
                italic,
                link: None,
            });
        }
    };
    for caps in LINKABLE.captures_iter(text) {
        let whole = caps.get(0).expect("group 0 always matches");
        let link = if let Some(url) = caps.name("url") {
            Some(Link::Url {
                url: url.as_str().to_owned(),
            })
        } else if let Some(time) = caps.name("time") {
            video_id.and_then(|id| {
                Some(Link::Video {
                    video_id: id.to_owned(),
                    start: Some(parse_timestamp(time.as_str())?),
                })
            })
        } else if let Some(handle) = caps.name("handle") {
            Some(Link::Channel {
                channel_id: None,
                handle: Some(handle.as_str().to_owned()),
            })
        } else {
            caps.name("tag").map(|t| Link::Search {
                query: t.as_str().to_owned(),
            })
        };
        let Some(link) = link else { continue };
        plain(&text[last..whole.start()], out);
        out.push(Run {
            text: whole.as_str().to_owned(),
            bold,
            italic,
            link: Some(link),
        });
        last = whole.end();
    }
    plain(&text[last..], out);
}

/// "1:02:03" or "2:03" in seconds.
pub fn parse_timestamp(text: &str) -> Option<f64> {
    text.split(':').try_fold(0.0, |total, part| {
        Some(total * 60.0 + part.parse::<f64>().ok()?)
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_youtube_links() {
        let video = |id: &str, start: Option<f64>| {
            Some(Link::Video {
                video_id: id.into(),
                start,
            })
        };
        assert_eq!(
            parse_url("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
            video("dQw4w9WgXcQ", None)
        );
        assert_eq!(
            parse_url("https://youtu.be/dQw4w9WgXcQ?t=90"),
            video("dQw4w9WgXcQ", Some(90.0))
        );
        assert_eq!(
            parse_url("https://m.youtube.com/watch?v=dQw4w9WgXcQ&t=1m30s"),
            video("dQw4w9WgXcQ", Some(90.0))
        );
        assert_eq!(
            parse_url("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=1h2m3s&list=PL1"),
            video("dQw4w9WgXcQ", Some(3723.0))
        );
        assert_eq!(
            parse_url("https://youtube.com/shorts/abcdefghijk"),
            video("abcdefghijk", None)
        );
        assert_eq!(
            parse_url("https://www.youtube.com/live/abcdefghijk?si=x"),
            video("abcdefghijk", None)
        );
        assert_eq!(
            parse_url("https://www.youtube-nocookie.com/embed/abcdefghijk"),
            video("abcdefghijk", None)
        );
        assert_eq!(
            parse_url("https://www.youtube.com/playlist?list=PL2qCIlusf8"),
            Some(Link::Playlist {
                playlist_id: "PL2qCIlusf8".into()
            })
        );
        assert_eq!(
            parse_url("https://www.youtube.com/@RickAstleyYT/videos"),
            Some(Link::Channel {
                channel_id: None,
                handle: Some("@RickAstleyYT".into())
            })
        );
        assert_eq!(
            parse_url("https://www.youtube.com/channel/UCuAXFkgsw1L7xaCfnd5JJOw"),
            Some(Link::Channel {
                channel_id: Some("UCuAXFkgsw1L7xaCfnd5JJOw".into()),
                handle: None
            })
        );
        assert_eq!(
            parse_url("https://www.youtube.com/results?search_query=hello+world"),
            Some(Link::Search {
                query: "hello world".into()
            })
        );
        // Not YouTube's, or not something the app shows.
        for url in [
            "https://example.com/watch?v=dQw4w9WgXcQ",
            "https://www.youtube.com/watch?v=short",
            "https://www.youtube.com/",
            "https://www.youtube.com/feed/trending",
            "https://www.youtube.com/channel/notachannel",
            "not a url",
            "https://youtu.be/dQw4w9WgXcQ?t=bad",
        ] {
            let parsed = parse_url(url);
            assert!(
                parsed.is_none() || url.ends_with("t=bad"),
                "{url}: {parsed:?}"
            );
        }
        // A bad moment is no moment.
        assert_eq!(
            parse_url("https://youtu.be/dQw4w9WgXcQ?t=bad"),
            video("dQw4w9WgXcQ", None)
        );
    }

    #[test]
    fn truncates_by_lines_and_length() {
        let text = RichText {
            runs: vec![
                Run {
                    text: "one\ntwo ".into(),
                    ..Run::default()
                },
                Run {
                    text: "link".into(),
                    link: Some(Link::Url {
                        url: "https://a.b".into(),
                    }),
                    ..Run::default()
                },
                Run {
                    text: "\nthree\nfour".into(),
                    ..Run::default()
                },
            ],
        };
        let short = text.truncated(3, 1000).unwrap();
        assert_eq!(short.plain(), "one\ntwo link\nthree");
        assert_eq!(short.runs[1].link, text.runs[1].link, "links survive");
        assert_eq!(text.truncated(2, 1000).unwrap().plain(), "one\ntwo link");
        assert_eq!(text.truncated(10, 5).unwrap().plain(), "one\nt");
        assert_eq!(text.truncated(4, 1000), None);
    }

    use super::*;
    use serde_json::json;

    #[test]
    fn links_in_plain_text() {
        let text = RichText::from_json(
            Some(
                &json!({ "simpleText": "Wow at 1:23, see https://example.org/a. by @some.one #Nature" }),
            ),
            Some("vid"),
        );
        let links: Vec<_> = text
            .runs
            .iter()
            .filter_map(|r| r.link.clone().map(|l| (r.text.clone(), l)))
            .collect();
        assert_eq!(
            links,
            vec![
                (
                    "1:23".into(),
                    Link::Video {
                        video_id: "vid".into(),
                        start: Some(83.0)
                    }
                ),
                (
                    "https://example.org/a".into(),
                    Link::Url {
                        url: "https://example.org/a".into()
                    }
                ),
                (
                    "@some.one".into(),
                    Link::Channel {
                        channel_id: None,
                        handle: Some("@some.one".into())
                    }
                ),
                (
                    "#Nature".into(),
                    Link::Search {
                        query: "#Nature".into()
                    }
                ),
            ]
        );
        assert_eq!(
            text.plain(),
            "Wow at 1:23, see https://example.org/a. by @some.one #Nature"
        );
    }

    #[test]
    fn runs_with_endpoints() {
        let text = RichText::from_json(
            Some(&json!({ "runs": [
                { "text": "Go to " },
                { "text": "this", "bold": true, "navigationEndpoint": { "urlEndpoint": { "url": "https://www.youtube.com/redirect?event=x&q=https%3A%2F%2Fexample.org%2F" } } },
                { "text": " or ", "italics": true },
                { "text": "@Chan", "navigationEndpoint": { "browseEndpoint": { "browseId": "UCabc", "canonicalBaseUrl": "/@Chan" } } }
            ] })),
            None,
        );
        assert_eq!(
            text.runs[1],
            Run {
                text: "this".into(),
                bold: true,
                italic: false,
                link: Some(Link::Url {
                    url: "https://example.org/".into()
                })
            }
        );
        assert!(text.runs[2].italic);
        assert_eq!(
            text.runs[3].link,
            Some(Link::Channel {
                channel_id: Some("UCabc".into()),
                handle: Some("@Chan".into())
            })
        );
    }

    #[test]
    fn timestamps_need_a_video() {
        let text = RichText::from_json(Some(&json!({ "simpleText": "at 1:23" })), None);
        assert!(text.runs.iter().all(|r| r.link.is_none()));
    }
}
