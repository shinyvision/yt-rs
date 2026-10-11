//! Channels: as shown next to their videos and comments, their own pages,
//! and the subscribe button.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::feed::{self, FeedPage, str_at, text};

/// A channel as shown next to a video or comment.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ChannelRef {
    /// `UC...`; unknown for comment authors on the TV client.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    /// `@handle`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    /// E.g. "14.6M subscribers".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscribers: Option<String>,
    #[serde(default)]
    pub verified: bool,
}

/// The state of a subscribe button and what toggling it sends.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Subscription {
    pub channel_id: String,
    pub subscribed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscribe_params: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsubscribe_params: Option<String>,
}

/// A tab of a channel page, loaded with the channel's browse ID and `params`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChannelTab {
    pub title: String,
    pub params: String,
}

/// A channel's page: its header and the featured shelves of its home tab.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ChannelPage {
    pub channel: ChannelRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The "about" facts: country, join date, video and view counts.
    #[serde(default)]
    pub about: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription: Option<Subscription>,
    #[serde(default)]
    pub tabs: Vec<ChannelTab>,
    /// The home tab's featured shelves.
    #[serde(default)]
    pub home: FeedPage,
}

/// The TV client's channel tabs. Its tab tiles show pictures, not titles,
/// so the params tell which is which. Releases (albums) are YouTube
/// Music's, and left out.
const TABS: &[(&str, &str)] = &[
    (VIDEOS_TAB, "Videos"),
    ("8gYHEgVyAIABAg%3D%3D", "Shorts"),
    ("8gYHEgVKAIABAg%3D%3D", "Live"),
    (PLAYLISTS_TAB, "Playlists"),
];

/// The params of a channel's Videos tab.
pub const VIDEOS_TAB: &str = "8gYHEgVqAIABAg%3D%3D";

/// The params of a channel's Playlists tab.
pub const PLAYLISTS_TAB: &str = "8gYLEgliBAoCEGiAAQI%3D";

/// The order of a channel's videos, as the chips over them on youtube.com.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoOrder {
    #[default]
    Latest,
    Popular,
    Oldest,
}

impl VideoOrder {
    pub const ALL: [Self; 3] = [Self::Latest, Self::Popular, Self::Oldest];

    /// Its chip's label, as youtube.com writes it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Latest => "Latest",
            Self::Popular => "Popular",
            Self::Oldest => "Oldest",
        }
    }
}

/// The web client's Videos tab, which has the order chips (the TV
/// client's has none, and ignores any order in its params).
pub(crate) const WEB_VIDEOS_TAB: &str = "EgZ2aWRlb3PyBgQKAjoA";

/// The token of the chip for `order` on the web client's Videos tab.
pub(crate) fn order_token(tab: &Value, order: VideoOrder) -> Option<String> {
    let mut token = None;
    collect(tab, "chipViewModel", &mut |chip| {
        if token.is_none() && str_at(chip, "/text") == Some(order.label()) {
            token = str_at(
                chip,
                "/tapCommand/innertubeCommand/continuationCommand/token",
            )
            .map(str::to_owned);
        }
    });
    token
}

pub fn parse_page(resp: &Value, browse_id: &str) -> ChannelPage {
    let header = find(resp, "channelHeaderRenderer");
    let mut page = ChannelPage {
        home: feed::parse(resp),
        ..ChannelPage::default()
    };
    if let Some(h) = header {
        let subtitle: Vec<String> = array(h.pointer("/subtitle/lineRenderer/items"))
            .map(|i| text(i.pointer("/lineItemRenderer/text")))
            .filter(|t| !t.is_empty() && t != "•")
            .collect();
        page.channel = ChannelRef {
            id: Some(browse_id.to_owned()),
            name: text(h.get("title")),
            handle: subtitle.iter().find(|t| t.starts_with('@')).cloned(),
            avatar: best_image(h.pointer("/avatar/thumbnails"), 176),
            subscribers: subtitle.iter().find(|t| t.contains("subscriber")).cloned(),
            verified: false,
        };
        page.banner = best_image(h.pointer("/backgroundImage/thumbnails"), 2120);
        let description = h.pointer("/selectableDescription/selectableTextRenderer");
        match description.and_then(|d| find(d, "aboutChannelViewModel")) {
            // The full description, if the channel has one, and the facts.
            Some(about) => {
                page.description = about
                    .get("description")
                    .and_then(Value::as_str)
                    .filter(|d| !d.trim().is_empty())
                    .map(str::to_owned);
                page.about = array(about.get("infoRows"))
                    .filter_map(|r| r.get("label")?.as_str().map(str::to_owned))
                    .collect();
            }
            // Only the start of it, which ends in a " ...more" run.
            None => {
                let runs = array(description.and_then(|d| d.pointer("/compactDescription/runs")));
                let start: String = runs
                    .filter_map(|r| r.get("text")?.as_str())
                    .filter(|t| t.trim() != "...more")
                    .collect();
                page.description = Some(start.trim().to_owned()).filter(|d| !d.is_empty());
            }
        }
        page.subscription = find(h, "subscribeButtonRenderer").and_then(parse_subscription);
    }
    let mut offered = std::collections::HashSet::new();
    collect(resp, "browseEndpoint", &mut |e| {
        if let Some(params) = e.get("params").and_then(Value::as_str) {
            offered.insert(params.to_owned());
        }
    });
    page.tabs = TABS
        .iter()
        .filter(|(params, _)| offered.contains(*params))
        .map(|(params, title)| ChannelTab {
            title: (*title).to_owned(),
            params: (*params).to_owned(),
        })
        .collect();
    page
}

/// Reads a subscribe button (`subscribeButtonRenderer`).
pub fn parse_subscription(button: &Value) -> Option<Subscription> {
    let params = |endpoint: &str| {
        find(button, endpoint).and_then(|e| e.get("params")?.as_str().map(str::to_owned))
    };
    Some(Subscription {
        channel_id: str_at(button, "/channelId")?.to_owned(),
        subscribed: button
            .get("subscribed")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        subscribe_params: params("subscribeEndpoint"),
        unsubscribe_params: params("unsubscribeEndpoint"),
    })
}

/// Reads the list of subscribed channels (`browse FEchannels`).
pub fn parse_subscriptions(resp: &Value) -> (Vec<ChannelRef>, Option<String>) {
    let mut channels = Vec::new();
    collect(resp, "tileRenderer", &mut |tile| {
        if str_at(tile, "/contentType") != Some("TILE_CONTENT_TYPE_CHANNEL") {
            return;
        }
        let Some(id) = str_at(tile, "/onSelectCommand/browseEndpoint/browseId")
            .filter(|id| id.starts_with("UC"))
        else {
            return;
        };
        channels.push(ChannelRef {
            id: Some(id.to_owned()),
            name: text(tile.pointer("/metadata/tileMetadataRenderer/title")),
            avatar: best_image(
                tile.pointer("/header/tileHeaderRenderer/thumbnail/thumbnails"),
                176,
            ),
            ..ChannelRef::default()
        });
    });
    let mut next = None;
    collect(resp, "nextContinuationData", &mut |c| {
        next = c
            .get("continuation")
            .and_then(Value::as_str)
            .map(str::to_owned)
    });
    (channels, next)
}

/// The largest image up to `max` pixels wide, as an absolute URL.
pub(crate) fn best_image(thumbnails: Option<&Value>, max: u64) -> Option<String> {
    let mut sized: Vec<(u64, &str)> = array(thumbnails)
        .filter_map(|t| {
            Some((
                t.get("width").and_then(Value::as_u64).unwrap_or(0),
                t.get("url")?.as_str()?,
            ))
        })
        .collect();
    sized.sort_by_key(|(w, _)| *w);
    let (_, url) = sized
        .iter()
        .rev()
        .find(|(w, _)| *w <= max)
        .or(sized.first())?;
    Some(if let Some(rest) = url.strip_prefix("//") {
        format!("https://{rest}")
    } else {
        (*url).to_owned()
    })
}

pub(crate) fn array(node: Option<&Value>) -> impl Iterator<Item = &Value> {
    node.and_then(Value::as_array).into_iter().flatten()
}

/// The first value under `key` anywhere inside `node`, in document order.
pub(crate) fn find<'a>(node: &'a Value, key: &str) -> Option<&'a Value> {
    match node {
        Value::Object(map) => map
            .get(key)
            .or_else(|| map.values().find_map(|v| find(v, key))),
        Value::Array(items) => items.iter().find_map(|v| find(v, key)),
        _ => None,
    }
}

/// Calls `f` on every value under `key` anywhere inside `node`.
pub(crate) fn collect<'a>(node: &'a Value, key: &str, f: &mut impl FnMut(&'a Value)) {
    match node {
        Value::Object(map) => {
            for (k, v) in map {
                if k == key {
                    f(v);
                } else {
                    collect(v, key, f);
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|v| collect(v, key, f)),
        _ => {}
    }
}
