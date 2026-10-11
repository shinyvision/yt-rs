//! Feed data and the parser that reads it out of InnerTube responses.
//!
//! The TV home screen is a column of shelves ("Recommended", "Recently
//! uploaded", topic rows); each shelf is a row of video tiles and can load
//! more of itself, and the page can load more shelves. The parser walks the
//! whole response and picks items out wherever they are nested, so it keeps
//! working when YouTube reshuffles the wrapper renderers around them.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A video as shown in a feed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Video {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub views: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published: Option<String>,
    /// As displayed, e.g. "12:34"; "LIVE" for live streams.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
    /// How much of the video the user has watched, from 0 to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<f32>,
    #[serde(default)]
    pub live: bool,
    #[serde(default)]
    pub short: bool,
    /// What YouTube offers to do with it ("Save to Watch later", "Not
    /// interested", ...), from the tile's menu.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<crate::actions::VideoAction>,
    /// For a tile whose menu YouTube sends on request: what to ask for
    /// ([`crate::innertube::InnerTube::video_actions`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu: Option<String>,
}

impl Video {
    pub fn url(&self) -> String {
        if self.short {
            format!("https://www.youtube.com/shorts/{}", self.id)
        } else {
            format!("https://www.youtube.com/watch?v={}", self.id)
        }
    }
}

/// A row of videos (or playlists) with a title, which may be able to load
/// more of itself.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Shelf {
    pub title: String,
    pub videos: Vec<Video>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub playlists: Vec<crate::playlist::Playlist>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation: Option<String>,
}

/// One response's worth of feed: a first page, more shelves, or more of one
/// shelf (which comes as a single shelf without a title).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FeedPage {
    pub shelves: Vec<Shelf>,
    /// Loads more shelves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation: Option<String>,
    /// A message YouTube shows instead of a feed, e.g. "Your YouTube history is off".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nudge: Option<String>,
}

/// Marks a continuation token as the web client's, so
/// [`InnerTube::continuation`](crate::innertube::InnerTube::continuation)
/// sends it with that client. Tokens stay opaque to callers.
pub const WEB_TOKEN: &str = "web:";

/// A page of a web client grid (a channel's videos in another order, or
/// more of them): the videos as one untitled shelf, with a token (marked
/// as the web client's) for more.
pub fn parse_web_grid(resp: &Value) -> FeedPage {
    let mut shelf = Shelf::default();
    for action in resp
        .get("onResponseReceivedActions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let items = action
            .pointer("/reloadContinuationItemsCommand/continuationItems")
            .or_else(|| action.pointer("/appendContinuationItemsAction/continuationItems"))
            .and_then(Value::as_array);
        for item in items.into_iter().flatten() {
            if let Some(video) = item
                .pointer("/richItemRenderer/content/lockupViewModel")
                .and_then(from_lockup)
            {
                shelf.videos.push(video);
            } else if let Some(token) = str_at(
                item,
                "/continuationItemRenderer/continuationEndpoint/continuationCommand/token",
            ) {
                shelf.continuation = Some(format!("{WEB_TOKEN}{token}"));
            }
        }
    }
    FeedPage {
        shelves: if shelf.videos.is_empty() {
            Vec::new()
        } else {
            vec![shelf]
        },
        ..FeedPage::default()
    }
}

pub fn parse(resp: &Value) -> FeedPage {
    let mut parser = Parser::default();
    parser.walk(resp, "");
    parser.page
}

#[derive(Default)]
struct Parser {
    page: FeedPage,
    /// The shelf being read, as an index into `page.shelves`.
    current: Option<usize>,
}

impl Parser {
    /// The shelf being read. Videos outside any shelf, as in a response with
    /// more of one shelf, go into a shelf without a title.
    fn shelf(&mut self) -> &mut Shelf {
        let index = *self.current.get_or_insert_with(|| {
            self.page.shelves.push(Shelf::default());
            self.page.shelves.len() - 1
        });
        &mut self.page.shelves[index]
    }

    fn walk(&mut self, node: &Value, parent: &str) {
        match node {
            Value::Array(items) => items.iter().for_each(|item| self.walk(item, parent)),
            Value::Object(map) => {
                for (key, child) in map {
                    match key.as_str() {
                        // Ads and client state; no videos here.
                        "frameworkUpdates"
                        | "responseContext"
                        | "onResponseReceivedActions"
                        | "adSlotRenderer" => {}
                        "shelfRenderer" => {
                            let title = text(child.pointer("/headerRenderer/shelfHeaderRenderer/avatarLockup/avatarLockupRenderer/title"));
                            let previous = self.current;
                            self.page.shelves.push(Shelf {
                                title,
                                ..Shelf::default()
                            });
                            self.current = Some(self.page.shelves.len() - 1);
                            self.walk(child, key);
                            self.current = previous;
                        }
                        "tileRenderer" => {
                            if let Some(video) = from_tile(child) {
                                self.shelf().videos.push(video);
                            } else if let Some(playlist) = crate::playlist::from_tile(child) {
                                self.shelf().playlists.push(playlist);
                            }
                        }
                        "lockupViewModel" => {
                            if let Some(video) = from_lockup(child) {
                                self.shelf().videos.push(video);
                            }
                        }
                        "continuations" => {
                            if let Some(token) = next_continuation(child) {
                                // A shelf's own continuation loads more of that row.
                                if parent == "horizontalListRenderer"
                                    || parent == "horizontalListContinuation"
                                {
                                    self.shelf().continuation = Some(token);
                                } else {
                                    self.page.continuation = Some(token);
                                }
                            }
                        }
                        "feedNudgeRenderer" => {
                            self.page.nudge =
                                Some(text(child.get("title"))).filter(|t| !t.is_empty())
                        }
                        _ => self.walk(child, key),
                    }
                }
            }
            _ => {}
        }
    }
}

fn next_continuation(node: &Value) -> Option<String> {
    node.as_array()?
        .iter()
        .find_map(|c| c.pointer("/nextContinuationData/continuation")?.as_str())
        .map(str::to_owned)
}

/// Reads the TV app's video tile.
fn from_tile(tile: &Value) -> Option<Video> {
    // Shorts come as TILE_CONTENT_TYPE_SHORTS on channel pages.
    if !matches!(
        str_at(tile, "/contentType"),
        Some("TILE_CONTENT_TYPE_VIDEO" | "TILE_CONTENT_TYPE_SHORTS")
    ) {
        return None;
    }
    let select = tile.get("onSelectCommand");
    let short = select.and_then(|s| s.get("reelWatchEndpoint")).is_some();
    // Shows also have the video content type, but open a show page instead
    // of playing.
    if !short && select.and_then(|s| s.get("watchEndpoint")).is_none() {
        return None;
    }
    let mut video = Video {
        id: str_at(tile, "/contentId")?.to_owned(),
        short,
        ..Video::default()
    };
    // Shorts keep their metadata in an overlay on the thumbnail.
    let metadata = tile
        .pointer("/metadata/tileMetadataRenderer")
        .filter(|m| m.get("title").is_some())
        .or_else(|| find(tile.get("header"), "tileMetadataRenderer"));
    video.title = text(metadata.and_then(|m| m.get("title")));

    // The first line is the channel, the next one views and age, e.g.
    // [["Channel"], ["1.2M views", "•", "3 days ago"]].
    for line in array(metadata.and_then(|m| m.get("lines"))) {
        for item in array(line.pointer("/lineRenderer/items")) {
            let t = text(item.pointer("/lineItemRenderer/text"));
            if t != "•" {
                video.add_metadata(&t, "");
            }
        }
    }

    let header = tile.pointer("/header/tileHeaderRenderer");
    for overlay in array(header.and_then(|h| h.get("thumbnailOverlays"))) {
        if let Some(d) = Some(text(
            overlay.pointer("/thumbnailOverlayTimeStatusRenderer/text"),
        ))
        .filter(|t| !t.is_empty())
        {
            video.duration = Some(d);
        }
        if let Some(p) = overlay
            .pointer("/thumbnailOverlayResumePlaybackRenderer/percentDurationWatched")
            .and_then(Value::as_f64)
        {
            video.progress = progress(p);
        }
    }
    video.thumbnail = best_thumbnail(array(
        header.and_then(|h| h.pointer("/thumbnail/thumbnails")),
    ));
    video.live = video.duration.as_deref() == Some("LIVE");
    video.actions = crate::actions::from_tile(tile);
    video.menu = crate::actions::menu_params(tile);
    (!video.title.is_empty()).then_some(video)
}

/// Reads the view model YouTube is moving its clients to; the TV app already
/// uses it for search results.
pub(crate) fn from_lockup(lockup: &Value) -> Option<Video> {
    // Videos come as VIDEO, MUSIC or MOVIE; channels and playlists are read
    // elsewhere (search).
    let short = match str_at(lockup, "/contentType")? {
        "LOCKUP_CONTENT_TYPE_SHORT" => true,
        "LOCKUP_CONTENT_TYPE_CHANNEL"
        | "LOCKUP_CONTENT_TYPE_PLAYLIST"
        | "LOCKUP_CONTENT_TYPE_PODCAST" => {
            return None;
        }
        _ => false,
    };
    let mut video = Video {
        id: str_at(lockup, "/contentId")?.to_owned(),
        short,
        ..Video::default()
    };
    let metadata = lockup.pointer("/metadata/lockupMetadataViewModel");
    video.title = metadata
        .and_then(|m| str_at(m, "/title/content"))
        .unwrap_or_default()
        .to_owned();
    for row in
        array(metadata.and_then(|m| m.pointer("/metadata/contentMetadataViewModel/metadataRows")))
    {
        for part in array(row.get("metadataParts")) {
            let t = str_at(part, "/text/content").unwrap_or_default();
            video.add_metadata(t, str_at(part, "/accessibilityLabel").unwrap_or_default());
        }
    }
    let image = lockup.get("contentImage");
    video.duration = find(image, "thumbnailBadgeViewModel")
        .and_then(|b| str_at(b, "/text"))
        .map(str::to_owned);
    video.progress = find(image, "thumbnailOverlayProgressBarViewModel")
        .and_then(|p| p.get("startPercent")?.as_f64())
        .and_then(progress);
    video.thumbnail = best_thumbnail(array(
        image.and_then(|i| i.pointer("/thumbnailViewModel/image/sources")),
    ));
    video.live = video.duration.as_deref() == Some("LIVE");
    (!video.title.is_empty()).then_some(video)
}

impl Video {
    /// Sorts a metadata text ("Channel", "1.2M views", "3 days ago") into
    /// channel, views and age.
    fn add_metadata(&mut self, t: &str, label: &str) {
        if t.is_empty() {
        } else if is_views(t) || is_views(label) {
            self.views = Some(with_unit(t, label));
        } else if is_published(t) {
            self.published = Some(t.to_owned());
        } else if self.channel.is_none() {
            self.channel = Some(t.to_owned());
        }
    }
}

fn is_views(t: &str) -> bool {
    let lower = t.to_lowercase();
    ["views", "view", "watching"]
        .iter()
        .any(|w| lower.ends_with(&format!(" {w}")) || lower == *w)
}

fn is_published(t: &str) -> bool {
    let lower = t.to_lowercase();
    lower.ends_with(" ago")
        || ["streamed", "premiere", "scheduled"]
            .iter()
            .any(|w| lower.starts_with(w))
}

/// Turns a bare count like "977K" into "977K views", using the accessibility
/// label ("977 thousand views") to tell which unit it is.
fn with_unit(t: &str, label: &str) -> String {
    if is_views(t) {
        return t.to_owned();
    }
    let unit = label.rsplit(' ').next().unwrap_or_default().to_lowercase();
    if is_views(label) {
        format!("{t} {unit}")
    } else {
        t.to_owned()
    }
}

/// Converts a watched percentage; live streams report 0, which means nothing
/// to show.
fn progress(percent: f64) -> Option<f32> {
    (percent > 0.0).then(|| (percent / 100.0).min(1.0) as f32)
}

/// Picks the thumbnail closest to card size: the largest one up to 720 px
/// wide, or else the smallest one.
pub(crate) fn best_thumbnail<'a>(thumbnails: impl Iterator<Item = &'a Value>) -> Option<String> {
    let mut sized: Vec<(u64, &str)> = thumbnails
        .filter_map(|t| {
            Some((
                t.get("width").and_then(Value::as_u64).unwrap_or(0),
                t.get("url")?.as_str()?,
            ))
        })
        .collect();
    sized.sort_by_key(|(w, _)| *w);
    let pick = sized
        .iter()
        .rev()
        .find(|(w, _)| *w <= 720)
        .or(sized.first())?;
    Some(normalize_url(pick.1))
}

fn normalize_url(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("//") {
        format!("https://{rest}")
    } else {
        url.to_owned()
    }
}

// Small helpers for reading InnerTube's JSON.

pub(crate) fn str_at<'a>(node: &'a Value, pointer: &str) -> Option<&'a str> {
    node.pointer(pointer)?.as_str()
}

fn array(node: Option<&Value>) -> impl Iterator<Item = &Value> {
    node.and_then(Value::as_array).into_iter().flatten()
}

/// Reads a formatted string: `{"simpleText": "..."}` or `{"runs": [{"text": "..."}, ...]}`.
pub(crate) fn text(node: Option<&Value>) -> String {
    let Some(node) = node else {
        return String::new();
    };
    if let Some(s) = node.get("simpleText").and_then(Value::as_str) {
        return s.to_owned();
    }
    array(node.get("runs"))
        .filter_map(|r| r.get("text")?.as_str())
        .collect()
}

/// The first value stored under `key` anywhere inside `node`.
fn find<'a>(node: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    match node? {
        Value::Object(map) => map
            .get(key)
            .or_else(|| map.values().find_map(|v| find(Some(v), key))),
        Value::Array(items) => items.iter().find_map(|v| find(Some(v), key)),
        _ => None,
    }
}
