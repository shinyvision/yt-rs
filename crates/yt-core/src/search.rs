//! Search.
//!
//! Results come from the TV client, signed in, so they are the user's own:
//! with their watch progress, shelves like "Previously watched", and the
//! "Unwatched" chip filtering on their history. The web client, asked the
//! same thing anonymously at the same time, adds what youtube.com shows and
//! the TV client leaves out: description snippets with the query in bold,
//! channel avatars and badges, the channel matching the query, spelling
//! corrections, and the chips. Both take the same `params` (built from
//! [`SearchFilters`]) and the same chip tokens. If either fails, the other's
//! results still show.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::channel::{ChannelRef, array, best_image, collect, find};
use crate::feed::{self, Video, str_at, text};
use crate::proto::Message;
use crate::text::RichText;

// Filters

/// The filters of youtube.com's filter dialog; any combination.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchFilters {
    pub upload_date: Option<UploadDate>,
    #[serde(rename = "type")]
    pub kind: Option<Kind>,
    pub duration: Option<Length>,
    pub features: Vec<Feature>,
    pub sort: Sort,
    /// Search for exactly the query, without spelling correction ("Search
    /// instead for ...").
    pub exact: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadDate {
    Today,
    Week,
    Month,
    Year,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Video,
    Short,
    Channel,
    Playlist,
    Movie,
}

/// Under 3 minutes, 3 to 20 minutes, over 20 minutes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Length {
    Short,
    Medium,
    Long,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    Live,
    #[serde(rename = "4k")]
    FourK,
    Hd,
    Subtitles,
    CreativeCommons,
    #[serde(rename = "360")]
    Spherical,
    Vr180,
    #[serde(rename = "3d")]
    ThreeD,
    Hdr,
    Location,
    Purchased,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sort {
    #[default]
    Relevance,
    Popularity,
}

impl SearchFilters {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The `params` of a search request, in YouTube's format: a protobuf
    /// with the sort order (field 1), the filters (field 2) and whether to
    /// correct the spelling (field 8).
    pub fn params(&self) -> Option<String> {
        let mut filters = Message::new();
        if let Some(date) = self.upload_date {
            let value = match date {
                UploadDate::Today => 2,
                UploadDate::Week => 3,
                UploadDate::Month => 4,
                UploadDate::Year => 5,
            };
            filters = filters.varint(1, value);
        }
        if let Some(kind) = self.kind {
            let value = match kind {
                Kind::Video => 1,
                Kind::Channel => 2,
                Kind::Playlist => 3,
                Kind::Movie => 4,
                Kind::Short => 9,
            };
            filters = filters.varint(2, value);
        }
        if let Some(length) = self.duration {
            let value = match length {
                Length::Long => 2,
                Length::Short => 4,
                Length::Medium => 5,
            };
            filters = filters.varint(3, value);
        }
        let mut features = self.features.clone();
        features.sort_by_key(|f| feature_field(*f));
        features.dedup();
        for feature in features {
            filters = filters.varint(feature_field(feature), 1);
        }
        let mut params = Message::new();
        if self.sort == Sort::Popularity {
            params = params.varint(1, 3);
        }
        if !filters.is_empty() {
            params = params.message(2, filters);
        }
        if self.exact {
            params = params.message(8, Message::new().varint(1, 1));
        }
        (!params.is_empty()).then(|| params.to_token())
    }
}

/// Where each feature sits in the filters message.
fn feature_field(feature: Feature) -> u32 {
    match feature {
        Feature::Hd => 4,
        Feature::Subtitles => 5,
        Feature::CreativeCommons => 6,
        Feature::ThreeD => 7,
        Feature::Live => 8,
        Feature::Purchased => 9,
        Feature::FourK => 14,
        Feature::Spherical => 15,
        Feature::Location => 23,
        Feature::Hdr => 25,
        Feature::Vr180 => 26,
    }
}

// Results

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SearchPage {
    pub items: Vec<SearchItem>,
    /// "All", "Shorts", "Unwatched", ... for the query (on its first page).
    #[serde(default)]
    pub chips: Vec<SearchChip>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correction: Option<Correction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_results: Option<u64>,
    /// Loads more results.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<SearchContinuation>,
    /// The web client's details for videos not on this page, which the TV
    /// client may list on a later one; see [`SearchPage::fill_details`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<SearchVideo>,
}

impl SearchPage {
    /// Gives videos without the web client's details the ones `known` has
    /// (collected from earlier pages' `details`).
    pub fn fill_details(&mut self, known: &HashMap<String, SearchVideo>) {
        let fill = |v: &mut SearchVideo| {
            if v.snippet.is_none()
                && v.owner.is_none()
                && let Some(k) = known.get(&v.video.id)
            {
                v.snippet = k.snippet.clone();
                v.owner = k.owner.clone();
                for badge in &k.badges {
                    if !v.badges.contains(badge) {
                        v.badges.push(badge.clone());
                    }
                }
            }
        };
        for item in &mut self.items {
            match item {
                SearchItem::Video(v) => fill(v),
                SearchItem::Shelf(shelf) => shelf.videos.iter_mut().for_each(fill),
                _ => {}
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SearchItem {
    Video(SearchVideo),
    Channel(SearchChannel),
    Playlist(SearchPlaylist),
    Shelf(SearchShelf),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SearchVideo {
    #[serde(flatten)]
    pub video: Video,
    /// The part of the description that matches, the query in bold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<RichText>,
    /// "4K", "CC", "New", ...
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub badges: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<ChannelRef>,
    /// A music video (YouTube marks its length with a note).
    #[serde(default)]
    pub music: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SearchChannel {
    pub channel: ChannelRef,
    /// E.g. "67 videos".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_count: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<RichText>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SearchPlaylist {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
    /// E.g. "25 videos".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_count: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    /// The video it starts with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_video: Option<String>,
}

/// A titled group inside the results: "People also watched", "Shorts", ...
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SearchShelf {
    pub title: String,
    pub videos: Vec<SearchVideo>,
    /// All its videos are Shorts.
    #[serde(default)]
    pub shorts: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchChip {
    pub title: String,
    /// Loads the results with this chip; `None` for "All".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default)]
    pub selected: bool,
}

/// "Showing results for `query`; search instead for `original`", or (when
/// not `applied`) "Did you mean `query`?".
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Correction {
    pub query: String,
    pub original: String,
    pub applied: bool,
}

/// The next page from each client.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchContinuation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tv: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web: Option<String>,
}

/// One of the suggestions while typing a query.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Suggestion {
    pub text: String,
    /// For a person or topic: a picture and what it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

// The TV client's results

/// What the TV client returned: the main results, and the shelves after
/// them.
#[derive(Debug, Default)]
pub struct TvResults {
    pub items: Vec<SearchItem>,
    pub shelves: Vec<SearchShelf>,
    pub next: Option<String>,
    pub estimated_results: Option<u64>,
}

/// Reads a TV search response or continuation: shelves of lockups, the
/// first ("Search results for ...") holding the main results.
pub fn parse_tv(resp: &Value) -> TvResults {
    let mut results = TvResults {
        estimated_results: resp
            .get("estimatedResults")
            .and_then(Value::as_str)
            .and_then(|n| n.parse().ok()),
        ..TvResults::default()
    };
    let mut shelves = Vec::new();
    collect(resp, "shelfRenderer", &mut |shelf| shelves.push(shelf));
    for (index, shelf) in shelves.into_iter().enumerate() {
        let title = text(shelf.pointer(
            "/headerRenderer/shelfHeaderRenderer/avatarLockup/avatarLockupRenderer/title",
        ));
        let mut lockups = Vec::new();
        collect(shelf, "lockupViewModel", &mut |l| lockups.push(l));
        let items: Vec<SearchItem> = lockups.into_iter().filter_map(tv_item).collect();
        if index == 0 || title.starts_with("Search results") {
            results.items.extend(items);
        } else {
            let videos: Vec<SearchVideo> = items
                .into_iter()
                .filter_map(|i| match i {
                    SearchItem::Video(v) => Some(v),
                    _ => None,
                })
                .collect();
            if !videos.is_empty() {
                let shorts = videos.iter().all(|v| v.video.short);
                results.shelves.push(SearchShelf {
                    title,
                    videos,
                    shorts,
                });
            }
        }
    }
    collect(resp, "nextContinuationData", &mut |c| {
        if results.next.is_none() {
            results.next = str_at(c, "/continuation").map(str::to_owned);
        }
    });
    results
}

fn tv_item(lockup: &Value) -> Option<SearchItem> {
    let metadata = lockup.pointer("/metadata/lockupMetadataViewModel");
    let rows: Vec<&Value> =
        array(metadata.and_then(|m| m.pointer("/metadata/contentMetadataViewModel/metadataRows")))
            .collect();
    let parts = |row: usize| -> Vec<String> {
        rows.get(row)
            .map(|r| {
                array(r.get("metadataParts"))
                    .filter_map(|p| str_at(p, "/text/content"))
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    let title = metadata
        .and_then(|m| str_at(m, "/title/content"))
        .unwrap_or_default()
        .to_owned();
    match str_at(lockup, "/contentType")? {
        "LOCKUP_CONTENT_TYPE_CHANNEL" => {
            let image = lockup.pointer("/contentImage/thumbnailViewModel/image/sources");
            let mut channel = ChannelRef {
                id: str_at(lockup, "/contentId").map(str::to_owned),
                name: title,
                avatar: best_image(image, 176).map(|u| absolute(&u)),
                ..ChannelRef::default()
            };
            for part in parts(0).into_iter().chain(parts(1)) {
                if part.starts_with('@') {
                    channel.handle = Some(part);
                } else if part.contains("subscriber") {
                    channel.subscribers = Some(part);
                }
            }
            Some(SearchItem::Channel(SearchChannel {
                channel,
                ..SearchChannel::default()
            }))
        }
        "LOCKUP_CONTENT_TYPE_PLAYLIST" => {
            let image = lockup.get("contentImage");
            Some(SearchItem::Playlist(SearchPlaylist {
                id: str_at(lockup, "/contentId")?.to_owned(),
                title,
                thumbnail: find(image?, "thumbnailViewModel")
                    .and_then(|t| best_image(t.pointer("/image/sources"), 720))
                    .map(|u| absolute(&u)),
                video_count: find(image?, "thumbnailBadgeViewModel")
                    .and_then(|b| str_at(b, "/text"))
                    .map(str::to_owned),
                channel: parts(0).into_iter().next(),
                first_video: find(lockup, "watchEndpoint")
                    .and_then(|w| str_at(w, "/videoId"))
                    .map(str::to_owned),
            }))
        }
        _ => {
            let video = feed::from_lockup(lockup)?;
            let mut badges = Vec::new();
            for row in &rows {
                for badge in array(row.get("badges")) {
                    if let Some(t) = str_at(badge, "/badgeViewModel/badgeText") {
                        badges.push(t.to_owned());
                    }
                }
            }
            let music = lockup
                .get("contentImage")
                .and_then(|i| find(i, "thumbnailBadgeViewModel"))
                .and_then(|b| find(b, "imageName"))
                .and_then(Value::as_str)
                == Some("AUDIO");
            Some(SearchItem::Video(SearchVideo {
                video,
                badges,
                music,
                ..SearchVideo::default()
            }))
        }
    }
}

/// Avatars come without a scheme (`//yt3.ggpht.com/...`).
fn absolute(url: &str) -> String {
    match url.strip_prefix("//") {
        Some(rest) => format!("https://{rest}"),
        None => url.to_owned(),
    }
}

// The web client's results

/// What the anonymous web client adds, by video ID.
#[derive(Debug, Default)]
pub struct WebResults {
    pub videos: HashMap<String, SearchVideo>,
    /// Every result in order: videos and channels (used when the TV client
    /// fails).
    pub items: Vec<SearchItem>,
    /// The channels listed before the first video: the query's own channel.
    pub top_channels: Vec<SearchChannel>,
    pub chips: Vec<SearchChip>,
    pub correction: Option<Correction>,
    pub next: Option<String>,
}

pub fn parse_web(resp: &Value) -> WebResults {
    let mut results = WebResults::default();
    let mut seen_video = false;
    walk_web(resp, &mut results, &mut seen_video);
    collect(resp, "chipCloudChipRenderer", &mut |chip| {
        results.chips.push(SearchChip {
            title: text(chip.get("text")),
            token: find(chip, "continuationCommand")
                .and_then(|c| str_at(c, "/token"))
                .map(str::to_owned),
            selected: chip
                .get("isSelected")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
    });
    results
}

fn walk_web(node: &Value, results: &mut WebResults, seen_video: &mut bool) {
    match node {
        Value::Array(items) => items.iter().for_each(|i| walk_web(i, results, seen_video)),
        Value::Object(map) => {
            for (key, child) in map {
                match key.as_str() {
                    "videoRenderer" => {
                        if let Some(video) = web_video(child) {
                            *seen_video = true;
                            results.items.push(SearchItem::Video(video.clone()));
                            results.videos.insert(video.video.id.clone(), video);
                        }
                    }
                    // The artist or channel the query names, on top.
                    "officialCardViewModel" => {
                        if let Some(channel) = official_card(child) {
                            if !*seen_video {
                                results.top_channels.push(channel.clone());
                            }
                            results.items.push(SearchItem::Channel(channel));
                        }
                    }
                    "channelRenderer" => {
                        let channel = web_channel(child);
                        if !*seen_video {
                            results.top_channels.push(channel.clone());
                        }
                        results.items.push(SearchItem::Channel(channel));
                    }
                    "showingResultsForRenderer" => {
                        results.correction = Some(Correction {
                            query: text(child.get("correctedQuery")),
                            original: text(child.get("originalQuery")),
                            applied: true,
                        });
                    }
                    "didYouMeanRenderer" => {
                        results.correction = Some(Correction {
                            query: text(child.get("correctedQuery")),
                            original: String::new(),
                            applied: false,
                        });
                    }
                    "continuationItemRenderer" => {
                        if let Some(token) =
                            find(child, "continuationCommand").and_then(|c| str_at(c, "/token"))
                        {
                            results.next = Some(token.to_owned());
                        }
                    }
                    // Chips are read separately; the rest is client state.
                    "header" | "responseContext" | "frameworkUpdates" | "chipCloudRenderer" => {}
                    _ => walk_web(child, results, seen_video),
                }
            }
        }
        _ => {}
    }
}

fn web_video(r: &Value) -> Option<SearchVideo> {
    let id = str_at(r, "/videoId")?.to_owned();
    let snippet = r
        .pointer("/detailedMetadataSnippets/0/snippetText")
        .or_else(|| r.get("descriptionSnippet"))
        .map(|s| RichText::from_json(Some(s), None))
        .filter(|s| !s.is_empty());
    let badges = array(r.get("badges"))
        .filter_map(|b| str_at(b, "/metadataBadgeRenderer/label"))
        .map(str::to_owned)
        .collect();
    let owner_run = r.pointer("/ownerText/runs/0");
    let owner = owner_run.map(|run| ChannelRef {
        id: run
            .pointer("/navigationEndpoint/browseEndpoint/browseId")
            .and_then(Value::as_str)
            .map(str::to_owned),
        name: str_at(run, "/text").unwrap_or_default().to_owned(),
        handle: run
            .pointer("/navigationEndpoint/browseEndpoint/canonicalBaseUrl")
            .and_then(Value::as_str)
            .and_then(|u| u.strip_prefix('/'))
            .filter(|u| u.starts_with('@'))
            .map(str::to_owned),
        avatar: best_image(
            r.pointer(
                "/channelThumbnailSupportedRenderers/channelThumbnailWithLinkRenderer/thumbnail/thumbnails",
            ),
            176,
        ),
        subscribers: None,
        verified: verified(r.get("ownerBadges")),
    });
    let duration = Some(text(r.get("lengthText"))).filter(|t| !t.is_empty());
    let video = Video {
        id,
        title: text(r.get("title")),
        channel: owner.as_ref().map(|o| o.name.clone()),
        // "13M views", as youtube.com shows it, rather than every digit.
        views: [r.get("shortViewCountText"), r.get("viewCountText")]
            .into_iter()
            .map(text)
            .find(|t| !t.is_empty()),
        published: Some(text(r.get("publishedTimeText"))).filter(|t| !t.is_empty()),
        live: duration.is_none() && find(r, "BADGE_STYLE_TYPE_LIVE_NOW").is_some(),
        duration,
        thumbnail: best_image(r.pointer("/thumbnail/thumbnails"), 720),
        ..Video::default()
    };
    Some(SearchVideo {
        video,
        snippet,
        badges,
        owner,
        music: false,
    })
}

fn web_channel(r: &Value) -> SearchChannel {
    // The web client shows the handle where the subscriber count used to be,
    // and the count in "videoCountText".
    let handle = Some(text(r.get("subscriberCountText"))).filter(|t| t.starts_with('@'));
    let subscribers = Some(text(r.get("videoCountText")))
        .filter(|t| t.contains("subscriber"))
        .or_else(|| Some(text(r.get("subscriberCountText"))).filter(|t| t.contains("subscriber")));
    SearchChannel {
        video_count: None,
        channel: ChannelRef {
            id: str_at(r, "/channelId").map(str::to_owned),
            name: text(r.get("title")),
            handle,
            avatar: best_image(r.pointer("/thumbnail/thumbnails"), 176).map(|u| absolute(&u)),
            subscribers,
            verified: verified(r.get("ownerBadges")),
        },
        description: r
            .get("descriptionSnippet")
            .map(|s| RichText::from_json(Some(s), None))
            .filter(|s| !s.is_empty()),
    }
}

fn official_card(card: &Value) -> Option<SearchChannel> {
    let header = card.pointer("/header/pageHeaderViewModel")?;
    let id = find(header, "subscribeEndpoint")
        .and_then(|e| e.pointer("/channelIds/0"))
        .or_else(|| find(header, "browseId"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut channel = ChannelRef {
        id,
        name: str_at(header, "/title/dynamicTextViewModel/text/content")?.to_owned(),
        avatar: best_image(
            header.pointer("/image/contentPreviewImageViewModel/image/sources"),
            176,
        )
        .map(|u| absolute(&u)),
        // An artist's note or a verified check after the name.
        verified: find(header.get("title")?, "imageName").is_some(),
        ..ChannelRef::default()
    };
    let mut video_count = None;
    let mut parts = Vec::new();
    collect(header.get("metadata")?, "metadataParts", &mut |row| {
        parts.push(row)
    });
    for part in parts.into_iter().flat_map(|row| array(Some(row))) {
        let Some(t) = str_at(part, "/text/content") else {
            continue;
        };
        if t.starts_with('@') {
            channel.handle = Some(t.to_owned());
        } else if t.contains("subscriber") {
            channel.subscribers = Some(t.to_owned());
        } else if t.contains("video") {
            video_count = Some(t.to_owned());
        }
    }
    Some(SearchChannel {
        channel,
        video_count,
        description: None,
    })
}

fn verified(badges: Option<&Value>) -> bool {
    array(badges).any(|b| {
        matches!(
            str_at(b, "/metadataBadgeRenderer/style"),
            Some("BADGE_STYLE_TYPE_VERIFIED" | "BADGE_STYLE_TYPE_VERIFIED_ARTIST")
        )
    })
}

// Together

/// After how many results the first shelf goes, and between shelves.
const FIRST_SHELF_AFTER: usize = 4;
const SHELF_EVERY: usize = 6;

/// One page of results from what each client returned. The TV client's
/// results come first, with the web client's details for each video; the
/// web client's top channel leads, as on youtube.com. Without TV results,
/// the web client's are shown as they are.
pub fn merge(tv: Option<TvResults>, web: Option<WebResults>) -> SearchPage {
    let web = web.unwrap_or_default();
    let Some(tv) = tv else {
        return SearchPage {
            next: web.next.clone().map(|w| SearchContinuation {
                tv: None,
                web: Some(w),
            }),
            items: web.items,
            chips: web.chips,
            correction: web.correction,
            ..SearchPage::default()
        };
    };
    let enrich = |mut v: SearchVideo| {
        if let Some(w) = web.videos.get(&v.video.id) {
            v.snippet = w.snippet.clone();
            v.owner = w.owner.clone();
            for badge in &w.badges {
                if !v.badges.contains(badge) {
                    v.badges.push(badge.clone());
                }
            }
        }
        v
    };
    let mut items: Vec<SearchItem> = Vec::new();
    let tv_has_channel = |id: &Option<String>| {
        tv.items
            .iter()
            .any(|i| matches!(i, SearchItem::Channel(c) if c.channel.id == *id))
    };
    for channel in &web.top_channels {
        if !tv_has_channel(&channel.channel.id) {
            items.push(SearchItem::Channel(channel.clone()));
        }
    }
    let mut shelves = tv.shelves.into_iter();
    let mut main = 0;
    let mut next_shelf = FIRST_SHELF_AFTER;
    for item in tv.items {
        items.push(match item {
            SearchItem::Video(v) => SearchItem::Video(enrich(v)),
            SearchItem::Channel(mut c) => {
                // The web client knows its description and badge.
                if let Some(SearchItem::Channel(w)) = web
                    .items
                    .iter()
                    .find(|i| matches!(i, SearchItem::Channel(w) if w.channel.id == c.channel.id))
                {
                    c.description = w.description.clone();
                    c.channel.verified |= w.channel.verified;
                }
                SearchItem::Channel(c)
            }
            other => other,
        });
        main += 1;
        if main == next_shelf
            && let Some(mut shelf) = shelves.next()
        {
            shelf.videos = shelf.videos.into_iter().map(enrich).collect();
            items.push(SearchItem::Shelf(shelf));
            next_shelf += SHELF_EVERY;
        }
    }
    for mut shelf in shelves {
        shelf.videos = shelf.videos.into_iter().map(enrich).collect();
        items.push(SearchItem::Shelf(shelf));
    }
    let next = SearchContinuation {
        tv: tv.next,
        web: web.next,
    };
    let used: std::collections::HashSet<&str> = items
        .iter()
        .flat_map(|i| match i {
            SearchItem::Video(v) => vec![v.video.id.as_str()],
            SearchItem::Shelf(s) => s.videos.iter().map(|v| v.video.id.as_str()).collect(),
            _ => Vec::new(),
        })
        .collect();
    let details = web
        .videos
        .values()
        .filter(|v| !used.contains(v.video.id.as_str()))
        .cloned()
        .collect();
    SearchPage {
        items,
        chips: web.chips,
        correction: web.correction,
        estimated_results: tv.estimated_results,
        next: (next.tv.is_some() || next.web.is_some()).then_some(next),
        details,
    }
}

// Suggestions

/// Reads the suggestion endpoint's reply,
/// `window.google.ac.h(["query", [["suggestion", 0, [...], {...}], ...]])`.
pub fn parse_suggestions(body: &str) -> Vec<Suggestion> {
    let json = body
        .find('(')
        .and_then(|start| body.rfind(')').map(|end| &body[start + 1..end]))
        .unwrap_or(body);
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    array(value.get(1))
        .filter_map(|entry| {
            let text = entry.get(0)?.as_str()?.to_owned();
            let info = entry.get(3);
            Some(Suggestion {
                text,
                image: info
                    .and_then(|i| i.get("zai"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                description: info
                    .and_then(|i| i.get("zaf"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filters() -> SearchFilters {
        SearchFilters::default()
    }

    #[test]
    fn single_filters_match_youtube() {
        let with = |f: SearchFilters| f.params().unwrap_or_default();
        assert_eq!(filters().params(), None);
        assert_eq!(
            with(SearchFilters {
                kind: Some(Kind::Video),
                ..filters()
            }),
            "EgIQAQ%3D%3D"
        );
        assert_eq!(
            with(SearchFilters {
                kind: Some(Kind::Short),
                ..filters()
            }),
            "EgIQCQ%3D%3D"
        );
        assert_eq!(
            with(SearchFilters {
                duration: Some(Length::Short),
                ..filters()
            }),
            "EgIYBA%3D%3D"
        );
        assert_eq!(
            with(SearchFilters {
                duration: Some(Length::Long),
                ..filters()
            }),
            "EgIYAg%3D%3D"
        );
        assert_eq!(
            with(SearchFilters {
                upload_date: Some(UploadDate::Today),
                ..filters()
            }),
            "EgIIAg%3D%3D"
        );
        assert_eq!(
            with(SearchFilters {
                features: vec![Feature::FourK],
                ..filters()
            }),
            "EgJwAQ%3D%3D"
        );
        assert_eq!(
            with(SearchFilters {
                features: vec![Feature::Vr180],
                ..filters()
            }),
            "EgPQAQE%3D"
        );
        assert_eq!(
            with(SearchFilters {
                sort: Sort::Popularity,
                ..filters()
            }),
            "CAM%3D"
        );
        // youtube.com's "Search instead for" link.
        assert_eq!(
            with(SearchFilters {
                exact: true,
                ..filters()
            }),
            "QgIIAQ%3D%3D"
        );
    }

    #[test]
    fn filters_combine() {
        let f = SearchFilters {
            upload_date: Some(UploadDate::Week),
            kind: Some(Kind::Video),
            features: vec![Feature::Subtitles, Feature::Hd],
            sort: Sort::Popularity,
            ..filters()
        };
        // sort 3; filters { upload 3, type 1, hd 1, subtitles 1 }
        assert_eq!(f.params().unwrap(), "CAMSCAgDEAEgASgB");
    }

    #[test]
    fn suggestions_with_entities() {
        let body = r#"window.google.ac.h(["hel",[["hello",0,[512,433]],["helen meles",0,[512],{"zaf":"Eritrean singer","zai":"https://img"}]],{"k":1}])"#;
        assert_eq!(
            parse_suggestions(body),
            vec![
                Suggestion {
                    text: "hello".into(),
                    ..Suggestion::default()
                },
                Suggestion {
                    text: "helen meles".into(),
                    image: Some("https://img".into()),
                    description: Some("Eritrean singer".into())
                },
            ]
        );
        assert!(parse_suggestions("not json").is_empty());
    }
}
