//! Playlists: as cards ([`Playlist`], in shelves and grids), as a page
//! ([`PlaylistPage`]: its details and videos), and the list of the user's
//! playlists to save a video to ([`SaveOption`]). Watch later (`WL`) and
//! Liked videos (`LL`) are playlists too.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::channel::{array, collect, find};
use crate::feed::{self, FeedPage, Video, str_at, text};

/// The user's Watch later list.
pub const WATCH_LATER: &str = "WL";
/// The videos the user liked.
pub const LIKED: &str = "LL";

/// A playlist as a card shows it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Playlist {
    /// "PL…", or `WL`, `LL`.
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
    /// "59 videos".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<String>,
    /// The lines under the title: "Private", "Rick Astley • @RickAstleyYT",
    /// "Updated today".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<String>,
}

impl Playlist {
    pub fn url(&self) -> String {
        format!("https://www.youtube.com/playlist?list={}", self.id)
    }
}

/// A playlist's page: what it is, and its first videos.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PlaylistPage {
    pub playlist: Playlist,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// `private`, `unlisted` or `public`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privacy: Option<String>,
    /// "No views", "1.2K views".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub views: Option<String>,
    /// "Updated today".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The user may change it (their own).
    #[serde(default)]
    pub editable: bool,
    pub videos: Vec<Video>,
    /// Loads more videos ([`crate::innertube::InnerTube::continuation`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation: Option<String>,
}

/// One of the user's playlists, offered to save a video to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SaveOption {
    pub playlist_id: String,
    pub title: String,
    /// `private`, `unlisted` or `public`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privacy: Option<String>,
    /// The video is in it already.
    pub contains: bool,
}

/// Who may see a playlist.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Privacy {
    Private,
    Unlisted,
    Public,
}

impl Privacy {
    /// As InnerTube spells it.
    pub fn as_api(self) -> &'static str {
        match self {
            Privacy::Private => "PRIVATE",
            Privacy::Unlisted => "UNLISTED",
            Privacy::Public => "PUBLIC",
        }
    }
}

/// Reads the TV app's playlist tile.
pub(crate) fn from_tile(tile: &Value) -> Option<Playlist> {
    if str_at(tile, "/contentType") != Some("TILE_CONTENT_TYPE_PLAYLIST") {
        return None;
    }
    let header = tile.pointer("/header/tileHeaderRenderer");
    let metadata = tile.pointer("/metadata/tileMetadataRenderer");
    let count = array(header.and_then(|h| h.get("thumbnailOverlays")))
        .map(|o| text(o.pointer("/thumbnailOverlayTimeStatusRenderer/text")))
        .find(|t| !t.is_empty());
    let details = array(metadata.and_then(|m| m.get("lines")))
        .map(|line| {
            array(line.pointer("/lineRenderer/items"))
                .map(|item| text(item.pointer("/lineItemRenderer/text")))
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|line| !line.is_empty())
        .collect();
    Some(Playlist {
        id: str_at(tile, "/contentId")?.to_owned(),
        title: text(metadata.and_then(|m| m.get("title"))),
        thumbnail: thumbnail(header.and_then(|h| h.pointer("/thumbnail/thumbnails"))),
        count,
        details,
    })
}

/// A playlist's picture, unless YouTube has none ("no_thumbnail.jpg").
fn thumbnail(thumbnails: Option<&Value>) -> Option<String> {
    feed::best_thumbnail(array(thumbnails)).filter(|url| !url.contains("no_thumbnail"))
}

/// Reads a playlist's page (a `VL<id>` browse).
#[yt_export::export("playlist")]
pub fn parse_page(resp: &Value, playlist_id: &str) -> PlaylistPage {
    let mut page = PlaylistPage {
        playlist: Playlist {
            id: playlist_id.to_owned(),
            ..Playlist::default()
        },
        ..PlaylistPage::default()
    };
    if let Some(header) = find(resp, "entityMetadataRenderer") {
        page.playlist.title = text(header.get("title"));
        page.description = Some(text(header.get("description"))).filter(|d| !d.is_empty());
        // [badge] Owner • 59 videos • No views • Updated today
        for line in array(header.get("bylines")) {
            for item in array(line.pointer("/lineRenderer/items")) {
                let item = item.get("lineItemRenderer");
                if let Some(icon) =
                    item.and_then(|i| str_at(i, "/badge/metadataBadgeRenderer/icon/iconType"))
                {
                    page.privacy = icon.strip_prefix("PRIVACY_").map(str::to_lowercase);
                    continue;
                }
                let t = text(item.and_then(|i| i.get("text")));
                let lower = t.to_lowercase();
                if t.is_empty() || t == "•" {
                } else if lower.ends_with(" video") || lower.ends_with(" videos") {
                    page.playlist.count = Some(t);
                } else if lower.ends_with(" view") || lower.ends_with(" views") {
                    page.views = Some(t);
                } else if lower.starts_with("updated") || lower.starts_with("last updated") {
                    page.updated = Some(t);
                } else if page.owner.is_none() {
                    page.owner = Some(t);
                }
            }
        }
    }
    let list = find(resp, "playlistVideoListRenderer");
    page.editable = list
        .and_then(|l| l.get("isEditable"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let videos = list.map(feed::parse).unwrap_or_default();
    page.continuation = videos.continuation.clone();
    page.videos = videos.shelves.into_iter().flat_map(|s| s.videos).collect();
    page.playlist.thumbnail = page.videos.first().and_then(|v| v.thumbnail.clone());
    page
}

/// More of a playlist's videos, from a page's `continuation`.
#[yt_export::export("playlist")]
pub fn parse_more(resp: &Value) -> FeedPage {
    feed::parse(resp)
}

/// The user's playlists to save a video to
/// (`playlist/get_add_to_playlist`).
#[yt_export::export("playlist")]
pub fn parse_save_options(resp: &Value) -> Vec<SaveOption> {
    let mut options = Vec::new();
    collect(resp, "playlistAddToOptionRenderer", &mut |o| {
        if let Some(id) = str_at(o, "/playlistId") {
            options.push(SaveOption {
                playlist_id: id.to_owned(),
                title: text(o.get("title")),
                privacy: str_at(o, "/privacy").map(str::to_lowercase),
                contains: str_at(o, "/containsSelectedVideos") == Some("ALL"),
            });
        }
    });
    options
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn video_tile(id: &str, title: &str, set_video_id: &str) -> Value {
        json!({ "tileRenderer": {
            "contentId": id, "contentType": "TILE_CONTENT_TYPE_VIDEO",
            "onSelectCommand": { "watchEndpoint": { "videoId": id, "playlistId": "WL" } },
            "header": { "tileHeaderRenderer": {
                "thumbnail": { "thumbnails": [{ "url": format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"), "width": 480 }] },
                "thumbnailOverlays": [{ "thumbnailOverlayTimeStatusRenderer": { "text": { "simpleText": "3:33" } } }] } },
            "metadata": { "tileMetadataRenderer": { "title": { "simpleText": title }, "lines": [
                { "lineRenderer": { "items": [{ "lineItemRenderer": { "text": { "runs": [{ "text": "A channel" }] } } }] } },
                { "lineRenderer": { "items": [
                    { "lineItemRenderer": { "text": { "simpleText": "1.2M views" } } },
                    { "lineItemRenderer": { "text": { "simpleText": "•" } } },
                    { "lineItemRenderer": { "text": { "simpleText": "3 weeks ago" } } } ] } } ] } },
            "onLongPressCommand": { "showMenuCommand": { "menu": { "menuRenderer": { "items": [
                { "menuServiceItemRenderer": { "text": { "runs": [{ "text": "Remove from Watch later" }] },
                    "serviceEndpoint": { "playlistEditEndpoint": { "playlistId": "WL",
                        "actions": [{ "setVideoId": set_video_id, "action": "ACTION_REMOVE_VIDEO" }] } } } } ] } } } }
        } })
    }

    #[test]
    fn reads_a_playlist_page() {
        let resp = json!({ "contents": { "tvBrowseRenderer": { "content": { "tvSurfaceContentRenderer": { "content": {
            "twoColumnRenderer": {
                "leftColumn": { "entityMetadataRenderer": {
                    "title": { "simpleText": "Watch later" },
                    "bylines": [{ "lineRenderer": { "items": [
                        { "lineItemRenderer": { "badge": { "metadataBadgeRenderer": { "icon": { "iconType": "PRIVACY_PRIVATE" } } } } },
                        { "lineItemRenderer": { "text": { "simpleText": "Someone" } } },
                        { "lineItemRenderer": { "text": { "simpleText": "•" } } },
                        { "lineItemRenderer": { "text": { "simpleText": "59 videos" } } },
                        { "lineItemRenderer": { "text": { "simpleText": "•" } } },
                        { "lineItemRenderer": { "text": { "simpleText": "No views" } } } ] } }] } },
                "rightColumn": { "playlistVideoListRenderer": {
                    "contents": [video_tile("v1", "First", "S1"), video_tile("v2", "Second", "S2")],
                    "playlistId": "WL", "isEditable": true,
                    "continuations": [{ "nextContinuationData": { "continuation": "MORE" } }] } } } } } } } } });
        let page = parse_page(&resp, "WL");
        assert_eq!(page.playlist.title, "Watch later");
        assert_eq!(page.playlist.count.as_deref(), Some("59 videos"));
        assert_eq!(page.owner.as_deref(), Some("Someone"));
        assert_eq!(page.views.as_deref(), Some("No views"));
        assert_eq!(page.privacy.as_deref(), Some("private"));
        assert!(page.editable);
        assert_eq!(page.continuation.as_deref(), Some("MORE"));
        let ids: Vec<&str> = page.videos.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, ["v1", "v2"]);
        assert_eq!(page.videos[0].channel.as_deref(), Some("A channel"));
        assert_eq!(page.videos[0].duration.as_deref(), Some("3:33"));
        // Each video can be taken out of the list, by its place in it.
        let crate::actions::ActionKind::EditPlaylist { actions, .. } =
            &page.videos[1].actions[0].kind
        else {
            panic!("{:?}", page.videos[1].actions);
        };
        assert_eq!(actions[0]["setVideoId"], "S2");
        assert_eq!(page.playlist.thumbnail, page.videos[0].thumbnail);
    }

    #[test]
    fn reads_playlist_tiles() {
        let tile = json!({
            "contentId": "PL1", "contentType": "TILE_CONTENT_TYPE_PLAYLIST",
            "header": { "tileHeaderRenderer": {
                "thumbnail": { "thumbnails": [{ "url": "https://i.ytimg.com/img/no_thumbnail.jpg", "width": 480 }] },
                "thumbnailOverlays": [
                    { "thumbnailOverlayTimeStatusRenderer": { "text": { "runs": [{ "text": "3", "bold": true }, { "text": " videos" }] } } },
                    { "thumbnailOverlayStackingEffectRenderer": {} } ] } },
            "metadata": { "tileMetadataRenderer": { "title": { "simpleText": "Mine" }, "lines": [
                { "lineRenderer": { "items": [{ "lineItemRenderer": { "text": { "simpleText": "Public" } } }] } } ] } }
        });
        let playlist = from_tile(&tile).unwrap();
        assert_eq!(playlist.id, "PL1");
        assert_eq!(playlist.title, "Mine");
        assert_eq!(playlist.count.as_deref(), Some("3 videos"));
        assert_eq!(playlist.details, ["Public"]);
        assert_eq!(
            playlist.thumbnail, None,
            "YouTube's placeholder is no picture"
        );
        assert!(
            from_tile(&json!({ "contentType": "TILE_CONTENT_TYPE_VIDEO", "contentId": "v" }))
                .is_none()
        );
        // In a shelf, playlists go with the shelf's playlists.
        let page = feed::parse(
            &json!({ "shelfRenderer": { "content": { "horizontalListRenderer": { "items": [
            { "tileRenderer": tile } ] } } } }),
        );
        assert_eq!(page.shelves[0].playlists.len(), 1);
        assert!(page.shelves[0].videos.is_empty());
    }

    #[test]
    fn reads_save_options() {
        let resp = json!({ "contents": [{ "addToPlaylistRenderer": { "playlists": [
            { "playlistAddToOptionRenderer": { "playlistId": "WL", "title": { "simpleText": "Watch later" },
                "privacy": "PRIVATE", "containsSelectedVideos": "ALL" } },
            { "playlistAddToOptionRenderer": { "playlistId": "PL1", "title": { "simpleText": "Mine" },
                "privacy": "PUBLIC", "containsSelectedVideos": "NONE" } } ] } }] });
        let options = parse_save_options(&resp);
        assert_eq!(options.len(), 2);
        assert!(options[0].contains);
        assert_eq!(options[0].privacy.as_deref(), Some("private"));
        assert!(!options[1].contains);
        assert_eq!(options[1].title, "Mine");
    }
}
