//! What YouTube offers to do with a video: the menu the TV app shows for a
//! tile ("Save to Watch later", "Not interested", "Remove from Liked
//! videos", ...), read into [`VideoAction`]s with what each one sends.
//! The app shows them in a card's menu, as YouTube words them.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::channel::{array, find};
use crate::feed::{str_at, text};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VideoAction {
    /// As YouTube says it: "Save to Watch later".
    pub label: String,
    #[serde(flatten)]
    pub kind: ActionKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionKind {
    /// Adds the video to a playlist or takes it out (`edit_playlist`).
    EditPlaylist {
        playlist_id: String,
        /// The edits, as YouTube wrote them.
        actions: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        params: Option<String>,
    },
    /// Pick the playlists to save it to.
    SaveToPlaylist,
    /// Tells YouTube ("Not interested", "Don't recommend channel"),
    /// after asking `confirm` if there is one.
    Feedback {
        token: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confirm: Option<Confirm>,
    },
    /// Takes the like away ("Remove from Liked videos").
    Unlike,
}

/// What to ask before an action.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Confirm {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    pub button: String,
}

/// For a tile whose menu comes on request (a context menu panel): the
/// params to ask for it with.
pub fn menu_params(tile: &Value) -> Option<String> {
    let panel = tile.pointer("/onLongPressCommand/showEngagementPanelEndpoint")?;
    (str_at(panel, "/identifier/tag") == Some("PAcontext_menu"))
        .then(|| str_at(panel, "/globalConfiguration/params").map(str::to_owned))
        .flatten()
}

/// The `params` asking for any video's menu (`get_panel`), for the videos
/// that come without one (the TV client's "Up next"): built as the Home
/// tiles' own are, naming the video and the feed it is in.
pub fn menu_params_for(video_id: &str) -> String {
    use crate::proto::Message;
    let feed = Message::new().varint(1, 2).string(2, "FEwhat_to_watch");
    let menu = Message::new()
        .string(1, video_id)
        .varint(4, 158)
        .message(8, feed);
    Message::new().message(175, menu).to_token()
}

/// The actions in a menu sent on request (`get_panel`): a list of items,
/// each with what a tap does.
pub fn from_panel(resp: &Value) -> Vec<VideoAction> {
    let mut actions = Vec::new();
    crate::channel::collect(resp, "listItemViewModel", &mut |item| {
        let label = str_at(item, "/title/content")
            .unwrap_or_default()
            .to_owned();
        let Some(tap) = item.pointer("/rendererContext/commandContext/onTap") else {
            return;
        };
        if let Some(kind) = deep_kind(tap)
            && !label.is_empty()
        {
            actions.push(VideoAction { label, kind });
        }
    });
    actions
}

/// What a tap does, from wherever in its commands the endpoint is. Saving
/// comes first: it also shows a popup, a notice ("Saved to Watch later")
/// rather than a question.
fn deep_kind(tap: &Value) -> Option<ActionKind> {
    if let Some(edit) = find(tap, "playlistEditEndpoint") {
        return Some(ActionKind::EditPlaylist {
            playlist_id: str_at(edit, "/playlistId")?.to_owned(),
            actions: edit.get("actions")?.clone(),
            params: str_at(edit, "/params").map(str::to_owned),
        });
    }
    if find(tap, "overlayPanelRenderer").is_some() {
        return kind(tap);
    }
    if let Some(feedback) = find(tap, "feedbackEndpoint") {
        return Some(ActionKind::Feedback {
            token: str_at(feedback, "/feedbackToken")?.to_owned(),
            confirm: None,
        });
    }
    find(tap, "likeEndpoint")
        .filter(|like| str_at(like, "/status") == Some("INDIFFERENT"))
        .map(|_| ActionKind::Unlike)
}

/// The actions in a tile's menu (`onLongPressCommand`), less the ones the
/// app has its own way to do (play, go to the channel).
pub fn from_tile(tile: &Value) -> Vec<VideoAction> {
    let items = tile.pointer("/onLongPressCommand/showMenuCommand/menu/menuRenderer/items");
    array(items)
        .filter_map(|item| {
            let entry = item.as_object()?.values().next()?;
            let label = text(entry.get("text"));
            let endpoint = ["serviceEndpoint", "navigationEndpoint", "command"]
                .iter()
                .find_map(|key| entry.get(*key))?;
            let kind = kind(endpoint)?;
            (!label.is_empty()).then_some(VideoAction { label, kind })
        })
        .collect()
}

fn kind(endpoint: &Value) -> Option<ActionKind> {
    if let Some(edit) = endpoint.get("playlistEditEndpoint") {
        return Some(ActionKind::EditPlaylist {
            playlist_id: str_at(edit, "/playlistId")?.to_owned(),
            actions: edit.get("actions")?.clone(),
            params: str_at(edit, "/params").map(str::to_owned),
        });
    }
    if endpoint.get("addToPlaylistEndpoint").is_some() {
        return Some(ActionKind::SaveToPlaylist);
    }
    if let Some(feedback) = endpoint.get("feedbackEndpoint") {
        return Some(ActionKind::Feedback {
            token: str_at(feedback, "/feedbackToken")?.to_owned(),
            confirm: None,
        });
    }
    if str_at(endpoint, "/likeEndpoint/status") == Some("INDIFFERENT") {
        return Some(ActionKind::Unlike);
    }
    // A popup that asks first, with the feedback behind its button.
    let panel = find(endpoint, "overlayPanelRenderer")?;
    let header = panel.pointer("/header/overlayPanelHeaderRenderer");
    let button = find(panel, "compactLinkRenderer")?;
    let token = str_at(find(button, "feedbackEndpoint")?, "/feedbackToken")?;
    Some(ActionKind::Feedback {
        token: token.to_owned(),
        confirm: Some(Confirm {
            title: text(header.and_then(|h| h.get("title"))),
            body: Some(text(header.and_then(|h| h.get("subtitle")))).filter(|s| !s.is_empty()),
            button: text(button.get("title")),
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_menu_params_as_youtube_does() {
        // A Home tile's, from YouTube.
        assert_eq!(
            menu_params_for("EfDomtxxFtY"),
            "-golCgtFZkRvbXR4eEZ0WSCeAUITCAISD0ZFd2hhdF90b193YXRjaA%3D%3D"
        );
    }
    use serde_json::json;

    fn tile(items: Value) -> Value {
        json!({ "onLongPressCommand": { "showMenuCommand": { "menu": { "menuRenderer": { "items": items } } } } })
    }

    #[test]
    fn reads_the_menu() {
        let t = tile(json!([
            { "menuNavigationItemRenderer": { "text": { "runs": [{ "text": "Play" }] },
                "navigationEndpoint": { "watchEndpoint": { "videoId": "v1" } } } },
            { "menuServiceItemRenderer": { "text": { "runs": [{ "text": "Save to Watch later" }] },
                "serviceEndpoint": { "playlistEditEndpoint": { "playlistId": "WL",
                    "actions": [{ "addedVideoId": "v1", "action": "ACTION_ADD_VIDEO" }] } } } },
            { "menuNavigationItemRenderer": { "text": { "runs": [{ "text": "Save to playlist" }] },
                "navigationEndpoint": { "addToPlaylistEndpoint": { "videoId": "v1" } } } },
            { "menuNavigationItemRenderer": { "text": { "runs": [{ "text": "Go to channel" }] },
                "navigationEndpoint": { "browseEndpoint": { "browseId": "UC1" } } } },
            { "menuServiceItemRenderer": { "text": { "runs": [{ "text": "Not interested" }] },
                "serviceEndpoint": { "feedbackEndpoint": { "feedbackToken": "T1",
                    "uiActions": { "hideEnclosingContainer": true } } } } },
            { "menuServiceItemRenderer": { "text": { "runs": [{ "text": "Don't recommend channel" }] },
                "command": { "openPopupAction": { "popup": { "overlaySectionRenderer": { "overlay": {
                    "overlayTwoPanelRenderer": { "actionPanel": { "overlayPanelRenderer": {
                        "header": { "overlayPanelHeaderRenderer": {
                            "title": { "runs": [{ "text": "Don't recommend channel" }] },
                            "subtitle": { "runs": [{ "text": "We won't recommend videos from this channel to you again" }] } } },
                        "content": { "overlayPanelItemListRenderer": { "items": [
                            { "compactLinkRenderer": { "title": { "runs": [{ "text": "Confirm" }] },
                                "serviceEndpoint": { "commandExecutorCommand": { "commands": [
                                    { "feedbackEndpoint": { "feedbackToken": "T2" } } ] } } } } ] } } } } } } } } } } } },
            { "menuServiceItemRenderer": { "text": { "runs": [{ "text": "Remove from Liked videos" }] },
                "serviceEndpoint": { "likeEndpoint": { "status": "INDIFFERENT", "target": { "videoId": "v1" } } } } },
            { "menuServiceItemRenderer": { "text": { "runs": [{ "text": "Remove from Watch later" }] },
                "serviceEndpoint": { "playlistEditEndpoint": { "playlistId": "WL", "params": "CAFAAQ",
                    "actions": [{ "setVideoId": "S1", "action": "ACTION_REMOVE_VIDEO" }] } } } }
        ]));
        let actions = from_tile(&t);
        let labels: Vec<&str> = actions.iter().map(|a| a.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Save to Watch later",
                "Save to playlist",
                "Not interested",
                "Don't recommend channel",
                "Remove from Liked videos",
                "Remove from Watch later"
            ]
        );
        assert_eq!(
            actions[0].kind,
            ActionKind::EditPlaylist {
                playlist_id: "WL".into(),
                actions: json!([{ "addedVideoId": "v1", "action": "ACTION_ADD_VIDEO" }]),
                params: None,
            }
        );
        assert_eq!(actions[1].kind, ActionKind::SaveToPlaylist);
        assert_eq!(
            actions[2].kind,
            ActionKind::Feedback {
                token: "T1".into(),
                confirm: None
            }
        );
        let ActionKind::Feedback {
            token,
            confirm: Some(confirm),
        } = &actions[3].kind
        else {
            panic!("no confirmation: {:?}", actions[3]);
        };
        assert_eq!(token, "T2");
        assert_eq!(confirm.button, "Confirm");
        assert!(confirm.body.as_deref().unwrap().starts_with("We won't"));
        assert_eq!(actions[4].kind, ActionKind::Unlike);
        let ActionKind::EditPlaylist { params, .. } = &actions[5].kind else {
            panic!();
        };
        assert_eq!(params.as_deref(), Some("CAFAAQ"));
        // They go to plugins and back as JSON.
        let json = serde_json::to_value(&actions).unwrap();
        assert_eq!(json[2]["kind"], "feedback");
        let back: Vec<VideoAction> = serde_json::from_value(json).unwrap();
        assert_eq!(back, actions);
    }

    #[test]
    fn reads_a_menu_sent_on_request() {
        let item = |label: &str, tap: Value| {
            json!({ "listItemViewModel": { "title": { "content": label },
                "rendererContext": { "commandContext": { "onTap": tap } } } })
        };
        let panel = json!({ "engagementPanelSectionListRenderer": { "content": { "listViewModel": { "listItems": [
            item("Play", json!({ "innertubeCommand": { "watchEndpoint": { "videoId": "v1" } } })),
            item("Save to Watch later", json!({ "innertubeCommand": { "commandExecutorCommand": { "commands": [
                { "signalServiceEndpoint": { "signal": "CLIENT_SIGNAL" } },
                { "playlistEditEndpoint": { "playlistId": "WL", "actions": [{ "addedVideoId": "v1", "action": "ACTION_ADD_VIDEO" }] } },
                { "openPopupAction": { "popup": { "overlayToastRenderer": { "title": { "simpleText": "Saved" } } }, "popupType": "TOAST" } } ] } } })),
            item("Not interested", json!({ "innertubeCommand": { "feedbackEndpoint": { "feedbackToken": "T1",
                "actions": [{ "openClientOverlayAction": { "type": "CLIENT_OVERLAY_TYPE_DISM" } }] } } })),
            item("Don't recommend channel", json!({ "innertubeCommand": { "openPopupAction": { "popup": { "overlaySectionRenderer": {
                "overlay": { "overlayTwoPanelRenderer": { "actionPanel": { "overlayPanelRenderer": {
                    "header": { "overlayPanelHeaderRenderer": { "title": { "runs": [{ "text": "Don't recommend channel" }] } } },
                    "content": { "overlayPanelItemListRenderer": { "items": [
                        { "compactLinkRenderer": { "title": { "runs": [{ "text": "Confirm" }] },
                            "serviceEndpoint": { "commandExecutorCommand": { "commands": [
                                { "feedbackEndpoint": { "feedbackToken": "T2" } } ] } } } } ] } } } } } } } } } } }))
        ] } } } });
        let actions = from_panel(&panel);
        let labels: Vec<&str> = actions.iter().map(|a| a.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Save to Watch later",
                "Not interested",
                "Don't recommend channel"
            ]
        );
        assert!(
            matches!(&actions[0].kind, ActionKind::EditPlaylist { playlist_id, .. } if playlist_id == "WL")
        );
        assert_eq!(
            actions[1].kind,
            ActionKind::Feedback {
                token: "T1".into(),
                confirm: None
            }
        );
        assert!(
            matches!(&actions[2].kind, ActionKind::Feedback { token, confirm: Some(_) } if token == "T2")
        );
    }

    #[test]
    fn knows_when_the_menu_comes_on_request() {
        let tile = json!({ "onLongPressCommand": { "showEngagementPanelEndpoint": {
            "identifier": { "tag": "PAcontext_menu" }, "globalConfiguration": { "params": "P1" } } } });
        assert_eq!(menu_params(&tile).as_deref(), Some("P1"));
        assert_eq!(menu_params(&json!({})), None);
    }

    #[test]
    fn no_menu_no_actions() {
        assert!(from_tile(&json!({})).is_empty());
        assert!(from_tile(&tile(json!([{ "menuServiceItemRenderer": {} }]))).is_empty());
    }
}
