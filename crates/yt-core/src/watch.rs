//! A video's watch page (`next`): title, stats, description, chapters, its
//! channel with the subscribe button, and the way into its comments.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::channel::{
    ChannelRef, Subscription, array, best_image, collect, find, parse_subscription,
};
use crate::comments::Vote;
use crate::feed::{Shelf, text};
use crate::text::RichText;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WatchPage {
    pub video_id: String,
    pub title: String,
    /// E.g. "2,673,306 views".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub views: Option<String>,
    /// E.g. "Published on Nov 30, 2025".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published: Option<String>,
    /// E.g. "13K".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub likes: Option<String>,
    /// How the user rated the video.
    #[serde(default)]
    pub vote: Vote,
    #[serde(default)]
    pub description: RichText,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<ChannelRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription: Option<Subscription>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comments: Option<CommentsEntry>,
    /// Suggested videos, in shelves.
    #[serde(default)]
    pub related: Vec<Shelf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chapters: Vec<Chapter>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Chapter {
    /// Where it starts, in seconds.
    pub start: f64,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
}

/// The comments section's header: the count and the sort orders, each with
/// the token that loads the comments in that order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CommentsEntry {
    /// E.g. "239".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<String>,
    pub sorts: Vec<CommentSort>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommentSort {
    /// "Top" or "Newest".
    pub title: String,
    pub token: String,
    pub selected: bool,
}

impl CommentsEntry {
    /// The token for the selected (or else the first) sort order.
    pub fn token(&self) -> Option<&str> {
        self.sorts
            .iter()
            .find(|s| s.selected)
            .or(self.sorts.first())
            .map(|s| s.token.as_str())
    }
}

pub fn parse(resp: &Value, video_id: &str) -> WatchPage {
    let mut page = WatchPage {
        video_id: video_id.to_owned(),
        ..WatchPage::default()
    };
    if let Some(meta) = find(resp, "videoMetadataRenderer") {
        page.title = text(meta.get("title"));
        page.views = Some(text(
            meta.pointer("/viewCount/videoViewCountRenderer/viewCount"),
        ))
        .filter(|s| !s.is_empty());
        page.published = Some(text(meta.get("publishedTimeText")))
            .filter(|s| !s.is_empty())
            .or_else(|| Some(text(meta.get("dateText"))).filter(|s| !s.is_empty()));
        if let Some(owner) = meta.pointer("/owner/videoOwnerRenderer") {
            page.owner = Some(parse_owner(owner));
            page.subscription = find(owner, "subscribeButtonRenderer").and_then(parse_subscription);
        }
    }
    if let Some(header) = find(resp, "videoDescriptionHeaderRenderer") {
        page.likes = array(header.get("factoid"))
            .filter_map(|f| f.get("factoidRenderer"))
            .find(|f| text(f.get("label")) == "Likes")
            .map(|f| text(f.get("value")));
        if page.title.is_empty() {
            page.title = text(header.get("title"));
        }
    }
    // The current rating; other likeStatus values in the response are what
    // the buttons would set.
    page.vote = match find(resp, "likeButtonRenderer").and_then(|b| b.get("likeStatus")?.as_str()) {
        Some("LIKE") => Vote::Like,
        Some("DISLIKE") => Vote::Dislike,
        _ => Vote::None,
    };
    page.description = RichText::from_json(
        find(resp, "expandableVideoDescriptionBodyRenderer")
            .and_then(|d| d.get("descriptionBodyText")),
        Some(video_id),
    );
    page.comments = comments_entry(resp);
    page.chapters = chapters(resp);
    page.related = crate::feed::parse(resp.get("contents").unwrap_or(&Value::Null)).shelves;
    page
}

/// The chapters, from the markers list the player bar shows.
fn chapters(resp: &Value) -> Vec<Chapter> {
    let mut chapters = Vec::new();
    collect(resp, "macroMarkersListEntity", &mut |entity| {
        let Some(list) = entity.get("markersList") else {
            return;
        };
        if list.get("markerType").and_then(Value::as_str) != Some("MARKER_TYPE_CHAPTERS")
            || !chapters.is_empty()
        {
            return;
        }
        chapters = array(list.get("markers"))
            .filter_map(|m| {
                let start = m
                    .get("startMillis")
                    .and_then(Value::as_str)?
                    .parse::<f64>()
                    .ok()?
                    / 1000.0;
                Some(Chapter {
                    start,
                    title: text(m.get("title")),
                    thumbnail: best_image(m.pointer("/thumbnailDetails/thumbnails"), 320),
                })
            })
            .collect();
    });
    chapters
}

fn parse_owner(owner: &Value) -> ChannelRef {
    let handle = owner
        .pointer("/navigationEndpoint/browseEndpoint/canonicalBaseUrl")
        .and_then(Value::as_str)
        .and_then(|u| u.strip_prefix('/'))
        .filter(|u| u.starts_with('@'))
        .map(str::to_owned);
    ChannelRef {
        id: owner
            .pointer("/navigationEndpoint/browseEndpoint/browseId")
            .and_then(Value::as_str)
            .map(str::to_owned),
        name: text(owner.get("title")),
        handle,
        avatar: best_image(owner.pointer("/thumbnail/thumbnails"), 176),
        subscribers: Some(text(owner.get("subscriberCountText"))).filter(|s| !s.is_empty()),
        verified: array(owner.get("badges")).any(|b| {
            b.pointer("/metadataBadgeRenderer/style")
                .and_then(Value::as_str)
                == Some("BADGE_STYLE_TYPE_VERIFIED")
        }),
    }
}

fn comments_entry(resp: &Value) -> Option<CommentsEntry> {
    let panel = array(resp.get("engagementPanels"))
        .filter_map(|p| p.get("engagementPanelSectionListRenderer"))
        .find(|p| {
            p.get("panelIdentifier").and_then(Value::as_str) == Some("comment-item-section")
        })?;
    let header = panel.pointer("/header/engagementPanelTitleHeaderRenderer");
    let mut sorts: Vec<CommentSort> =
        array(header.and_then(|h| h.pointer("/menu/sortFilterSubMenuRenderer/subMenuItems")))
            .filter_map(|item| {
                Some(CommentSort {
                    title: item.get("title")?.as_str()?.to_owned(),
                    token: item
                        .pointer("/continuation/reloadContinuationData/continuation")?
                        .as_str()?
                        .to_owned(),
                    selected: item
                        .get("selected")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                })
            })
            .collect();
    if sorts.is_empty() {
        // No sort menu: just the default order.
        let mut token = None;
        collect(panel, "reloadContinuationData", &mut |c| {
            token = token.take().or_else(|| {
                c.get("continuation")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            });
        });
        sorts.push(CommentSort {
            title: "Top".into(),
            token: token?,
            selected: true,
        });
    }
    Some(CommentsEntry {
        count: header
            .map(|h| text(h.get("contextualInfo")))
            .filter(|c| !c.is_empty()),
        sorts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chapters_from_the_markers_list() {
        // The shape of the TV `next` response's chapters, trimmed.
        let resp = serde_json::json!({ "frameworkUpdates": { "entityBatchUpdate": { "mutations": [
            { "payload": { "macroMarkersListEntity": { "markersList": {
                "markerType": "MARKER_TYPE_CHAPTERS",
                "markers": [
                    { "title": { "simpleText": "Introduction" }, "startMillis": "0",
                      "thumbnailDetails": { "thumbnails": [{ "url": "https://i.ytimg.com/vi/x/hq1.jpg", "width": 640 }] } },
                    { "title": { "simpleText": "REM Sleep" }, "startMillis": "55000" },
                ],
            } } } },
            { "payload": { "likeStatusEntity": {} } },
        ] } } });
        let chapters = chapters(&resp);
        assert_eq!(chapters.len(), 2);
        assert_eq!(
            (chapters[1].start, chapters[1].title.as_str()),
            (55.0, "REM Sleep")
        );
        assert!(chapters[0].thumbnail.is_some());
    }
}
