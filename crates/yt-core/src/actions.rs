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
    fn no_menu_no_actions() {
        assert!(from_tile(&json!({})).is_empty());
        assert!(from_tile(&tile(json!([{ "menuServiceItemRenderer": {} }]))).is_empty());
    }
}
