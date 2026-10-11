//! Live chat: a live stream's chat messages as they come, read with the TV
//! client (signed in, the user's own view of it).
//!
//! The TV client's watch page has no chat, so the first token is built
//! ([`token`]) as youtube.com's are, from the channel and the video. Each
//! answer ([`ChatPage`]) brings what happened since the last (messages
//! added, replaced or deleted, a message pinned), the next token, and how
//! long to wait before asking with it.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::feed::{best_thumbnail, str_at, text};
use crate::proto::Message;
use crate::text::RichText;

/// How long to wait between pages when YouTube doesn't say, and the bounds
/// on what it says (it asks web pages, which are pushed new messages, to
/// wait long).
const DEFAULT_WAIT_MS: u64 = 5_000;
const WAIT_MS: std::ops::RangeInclusive<u64> = 1_000..=8_000;

/// Which messages: YouTube's pick (without potential spam), or all of them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatView {
    #[default]
    Top,
    Live,
}

impl ChatView {
    pub const ALL: [Self; 2] = [Self::Top, Self::Live];

    /// As youtube.com's menu names it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Top => "Top chat",
            Self::Live => "Live chat",
        }
    }

    /// As youtube.com's menu explains it.
    pub fn description(self) -> &'static str {
        match self {
            Self::Top => "Some messages, such as potential spam, may not be visible",
            Self::Live => "All messages are visible",
        }
    }

    fn code(self) -> u64 {
        match self {
            Self::Top => 4,
            Self::Live => 1,
        }
    }
}

/// The token for a live stream's chat, from now on: built as youtube.com's
/// own, which names the channel and the video.
#[yt_export::export("live_chat")]
pub fn token(channel_id: &str, video_id: &str, view: ChatView) -> String {
    let ids = Message::new().string(1, channel_id).string(2, video_id);
    let video = Message::new()
        .message(1, Message::new().message(5, ids))
        .string(2, video_id)
        .message(
            3,
            Message::new().message(48_687_757, Message::new().string(1, video_id)),
        )
        .varint(4, 1)
        .varint(6, 0);
    let options = Message::new()
        .varint(1, view.code())
        .varint(3, 2)
        .varint(4, 0)
        .varint(5, 0)
        .varint(6, 0)
        .varint(7, 0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_micros() as u64);
    let chat = Message::new()
        .string(3, &video.to_token())
        .varint(6, 1)
        .message(16, options)
        .varint(17, 1)
        .varint(20, now)
        .varint(21, 0);
    Message::new().message(119_693_434, chat).to_token()
}

/// Who wrote a message.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatAuthor {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    /// The stream's channel itself.
    #[serde(default)]
    pub owner: bool,
    #[serde(default)]
    pub moderator: bool,
    #[serde(default)]
    pub verified: bool,
    /// A member of the channel: "Member (2 years)", with the channel's badge
    /// for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_badge: Option<String>,
}

/// What kind of message, with what it has besides text. Colors are CSS
/// colors (`rgba(...)`), as YouTube picks them for the amount.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChatKind {
    #[default]
    Text,
    /// A Super Chat: paid for, shown in color.
    SuperChat {
        amount: String,
        header_color: String,
        body_color: String,
    },
    /// A Super Sticker.
    Sticker {
        amount: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sticker: Option<String>,
        color: String,
    },
    /// A new member ("Welcome to ...!"), or a member's milestone ("Member
    /// for 13 months", with what they wrote).
    Membership {
        header: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subtext: Option<String>,
    },
    /// Memberships gifted, one received, or a gift sent ("sent Chili").
    Gift {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        image: Option<String>,
    },
    /// From YouTube rather than a viewer ("Welcome to live chat!").
    Notice,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub author: ChatAuthor,
    pub text: RichText,
    /// When it was sent, in microseconds since 1970.
    #[serde(default)]
    pub time_usec: u64,
    #[serde(flatten)]
    pub kind: ChatKind,
}

/// Something that happened in the chat.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ChatAction {
    Add {
        message: ChatMessage,
    },
    /// A message changed (one held for review, now shown).
    Replace {
        id: String,
        message: ChatMessage,
    },
    /// A message deleted, or all of an author's (a moderator removed them).
    Remove {
        id: String,
    },
    RemoveByAuthor {
        channel_id: String,
    },
    /// A message pinned at the top (`id` is the pin's), or unpinned.
    Pin {
        id: String,
        message: ChatMessage,
    },
    Unpin {
        id: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatPage {
    pub actions: Vec<ChatAction>,
    /// Asks for what comes next; `None` when the chat is over (or off).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation: Option<String>,
    /// How long to wait before asking, in milliseconds.
    pub wait_ms: u64,
    /// The signed-in viewer's name in the chat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewer: Option<String>,
}

#[yt_export::export("live_chat")]
pub fn parse(resp: &Value) -> ChatPage {
    let Some(chat) = resp.pointer("/continuationContents/liveChatContinuation") else {
        return ChatPage::default();
    };
    let mut page = ChatPage {
        viewer: str_at(chat, "/viewerName").map(str::to_owned),
        wait_ms: DEFAULT_WAIT_MS,
        ..ChatPage::default()
    };
    let next = array(chat.get("continuations")).find_map(|c| {
        [
            "timedContinuationData",
            "invalidationContinuationData",
            "reloadContinuationData",
        ]
        .into_iter()
        .find_map(|key| c.get(key))
    });
    if let Some(next) = next {
        page.continuation = str_at(next, "/continuation").map(str::to_owned);
        page.wait_ms = next
            .get("timeoutMs")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_WAIT_MS)
            .clamp(*WAIT_MS.start(), *WAIT_MS.end());
    }
    page.actions = array(chat.get("actions")).filter_map(action).collect();
    page
}

fn action(action: &Value) -> Option<ChatAction> {
    if let Some(item) = action.pointer("/addChatItemAction/item") {
        return message(item).map(|message| ChatAction::Add { message });
    }
    if let Some(replace) = action.get("replaceChatItemAction") {
        return Some(ChatAction::Replace {
            id: str_at(replace, "/targetItemId")?.to_owned(),
            message: message(replace.get("replacementItem")?)?,
        });
    }
    for key in ["markChatItemAsDeletedAction", "removeChatItemAction"] {
        if let Some(remove) = action.get(key) {
            return Some(ChatAction::Remove {
                id: str_at(remove, "/targetItemId")?.to_owned(),
            });
        }
    }
    for key in [
        "markChatItemsByAuthorAsDeletedAction",
        "removeChatItemByAuthorAction",
    ] {
        if let Some(remove) = action.get(key) {
            return Some(ChatAction::RemoveByAuthor {
                channel_id: str_at(remove, "/externalChannelId")?.to_owned(),
            });
        }
    }
    if let Some(banner) =
        action.pointer("/addBannerToLiveChatCommand/bannerRenderer/liveChatBannerRenderer")
    {
        // Pinned messages; the AI summaries and polls aren't messages.
        return Some(ChatAction::Pin {
            id: str_at(banner, "/actionId")?.to_owned(),
            message: message(banner.get("contents")?)?,
        });
    }
    if let Some(unpin) = action.get("removeBannerForLiveChatCommand") {
        return Some(ChatAction::Unpin {
            id: str_at(unpin, "/targetActionId")?.to_owned(),
        });
    }
    None
}

/// A message, from any of the renderers YouTube sends them as.
fn message(item: &Value) -> Option<ChatMessage> {
    let (renderer, r) = item.as_object()?.iter().next()?;
    // Gift memberships keep their author in a header.
    let header = r.pointer("/header/liveChatSponsorshipsHeaderRenderer");
    let author_node = header.unwrap_or(r);
    let mut text = RichText::from_json(r.get("message"), None);
    let kind = match renderer.as_str() {
        "liveChatTextMessageRenderer" => ChatKind::Text,
        "liveChatPaidMessageRenderer" => ChatKind::SuperChat {
            amount: crate::feed::text(r.get("purchaseAmountText")),
            header_color: color(r, "headerBackgroundColor"),
            body_color: color(r, "bodyBackgroundColor"),
        },
        "liveChatPaidStickerRenderer" => ChatKind::Sticker {
            amount: crate::feed::text(r.get("purchaseAmountText")),
            sticker: best_thumbnail(array(r.pointer("/sticker/thumbnails"))),
            color: color(r, "backgroundColor"),
        },
        "liveChatMembershipItemRenderer" => {
            let primary = crate::feed::text(r.get("headerPrimaryText"));
            let sub = crate::feed::text(r.get("headerSubtext"));
            // A milestone has both ("Member for 13 months", the level); a
            // new member only the welcome.
            if primary.is_empty() {
                ChatKind::Membership {
                    header: sub,
                    subtext: None,
                }
            } else {
                ChatKind::Membership {
                    header: primary,
                    subtext: Some(sub).filter(|s| !s.is_empty()),
                }
            }
        }
        "liveChatSponsorshipsGiftPurchaseAnnouncementRenderer" => {
            text = RichText::from_json(header.and_then(|h| h.get("primaryText")), None);
            ChatKind::Gift {
                image: header.and_then(|h| best_thumbnail(array(h.pointer("/image/thumbnails")))),
            }
        }
        "liveChatSponsorshipsGiftRedemptionAnnouncementRenderer" => ChatKind::Gift { image: None },
        "giftMessageViewModel" => {
            text = RichText::from_plain(str_at(r, "/text/content").unwrap_or_default());
            ChatKind::Gift {
                image: best_thumbnail(array(r.pointer("/giftImage/sources"))),
            }
        }
        "liveChatViewerEngagementMessageRenderer" | "liveChatModeChangeMessageRenderer" => {
            ChatKind::Notice
        }
        _ => return None,
    };
    Some(ChatMessage {
        id: str_at(r, "/id")?.to_owned(),
        author: author(author_node, r),
        text,
        time_usec: str_at(r, "/timestampUsec")
            .and_then(|t| t.parse().ok())
            .unwrap_or(0),
        kind,
    })
}

fn author(node: &Value, message: &Value) -> ChatAuthor {
    let name = match node.get("authorName") {
        Some(name) if name.get("content").is_some() => {
            str_at(name, "/content").unwrap_or_default().to_owned()
        }
        name => text(name),
    };
    let avatar = best_thumbnail(array(node.pointer("/authorPhoto/thumbnails"))).or_else(|| {
        best_thumbnail(array(
            node.pointer("/authorAvatar/avatarViewModel/image/sources"),
        ))
    });
    let mut author = ChatAuthor {
        name: name.trim().to_owned(),
        channel_id: str_at(message, "/authorExternalChannelId")
            .or_else(|| str_at(node, "/authorExternalChannelId"))
            .map(str::to_owned),
        avatar,
        ..ChatAuthor::default()
    };
    for badge in array(node.get("authorBadges")) {
        let Some(badge) = badge.get("liveChatAuthorBadgeRenderer") else {
            continue;
        };
        match str_at(badge, "/icon/iconType") {
            Some("OWNER") => author.owner = true,
            Some("MODERATOR") => author.moderator = true,
            Some("VERIFIED") => author.verified = true,
            _ if badge.get("customThumbnail").is_some() => {
                author.member = str_at(badge, "/tooltip").map(str::to_owned);
                author.member_badge =
                    best_thumbnail(array(badge.pointer("/customThumbnail/thumbnails")));
            }
            _ => {}
        }
    }
    author
}

/// A color YouTube sends as a number (`0xAARRGGBB`), for CSS.
fn color(node: &Value, key: &str) -> String {
    let argb = node.get(key).and_then(Value::as_u64).unwrap_or(0xff1e88e5);
    let [a, r, g, b] = (argb as u32).to_be_bytes();
    format!("rgba({r}, {g}, {b}, {:.2})", f64::from(a) / 255.0)
}

fn array(node: Option<&Value>) -> impl Iterator<Item = &Value> {
    node.and_then(Value::as_array).into_iter().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn builds_tokens_as_youtube_does() {
        let decode = |token: &str| {
            base64::Engine::decode(
                &base64::engine::general_purpose::URL_SAFE,
                token.replace("%3D", "="),
            )
            .unwrap()
        };
        let bytes = decode(&token(
            "UCxxxxxxxxxxxxxxxxxxxxxx",
            "abcdefghijk",
            ChatView::Live,
        ));
        // youtube.com's outer field, and the view: 1 for all messages.
        assert_eq!(&bytes[..4], &[0xd2, 0x87, 0xcc, 0xc8]);
        assert!(
            bytes
                .windows(5)
                .any(|w| w == [0x82, 0x01, 0x0c, 0x08, 0x01])
        );
        // Inside (field 3), as base64 again: the channel and the video.
        let varint = |at: &mut usize| {
            let mut n = 0usize;
            for shift in (0..).step_by(7) {
                let byte = bytes[*at];
                *at += 1;
                n |= usize::from(byte & 0x7f) << shift;
                if byte & 0x80 == 0 {
                    break;
                }
            }
            n
        };
        let mut at = 5;
        varint(&mut at);
        assert_eq!(bytes[at], 0x1a);
        at += 1;
        let len = varint(&mut at);
        let field = String::from_utf8(bytes[at..at + len].to_vec()).unwrap();
        let inner = String::from_utf8_lossy(&decode(&field)).into_owned();
        assert!(inner.contains("UCxxxxxxxxxxxxxxxxxxxxxx") && inner.contains("abcdefghijk"));
        // Top chat: 4.
        let top = decode(&token(
            "UCxxxxxxxxxxxxxxxxxxxxxx",
            "abcdefghijk",
            ChatView::Top,
        ));
        assert!(top.windows(5).any(|w| w == [0x82, 0x01, 0x0c, 0x08, 0x04]));
    }

    fn page(actions: Value) -> ChatPage {
        parse(&json!({ "continuationContents": { "liveChatContinuation": {
            "continuations": [{ "timedContinuationData": { "timeoutMs": 4321, "continuation": "next" } }],
            "viewerName": "@me",
            "actions": actions,
        } } }))
    }

    #[test]
    fn reads_messages() {
        let page = page(json!([
            { "addChatItemAction": { "item": { "liveChatTextMessageRenderer": {
                "id": "m1",
                "timestampUsec": "1791691341112047",
                "authorName": { "simpleText": "@viewer" },
                "authorExternalChannelId": "UCviewer",
                "authorPhoto": { "thumbnails": [
                    { "url": "https://yt4.ggpht.com/a=s32", "width": 32 },
                    { "url": "https://yt4.ggpht.com/a=s64", "width": 64 },
                ] },
                "authorBadges": [
                    { "liveChatAuthorBadgeRenderer": { "icon": { "iconType": "MODERATOR" }, "tooltip": "Moderator" } },
                    { "liveChatAuthorBadgeRenderer": { "customThumbnail": { "thumbnails": [
                        { "url": "https://yt3.ggpht.com/badge=s16", "width": 16 },
                    ] }, "tooltip": "Member (2 years)" } },
                ],
                "message": { "runs": [
                    { "text": "hi " },
                    { "emoji": { "emojiId": "\u{1f60a}", "shortcuts": [":blush:"], "image": { "thumbnails": [{ "url": "https://fonts.gstatic.com/x.png" }] } } },
                    { "emoji": { "emojiId": "UCchannel/abc", "shortcuts": [":yt:"], "isCustomEmoji": true,
                        "image": { "thumbnails": [{ "url": "https://yt3.ggpht.com/yt=w24", "width": 24 }, { "url": "https://yt3.ggpht.com/yt=w48", "width": 48 }] } } },
                ] },
            } } } },
            { "addChatItemAction": { "item": { "liveChatPaidMessageRenderer": {
                "id": "m2",
                "authorName": { "simpleText": "@fan" },
                "purchaseAmountText": { "simpleText": "$19.99" },
                "headerBackgroundColor": 4293284096u64,
                "bodyBackgroundColor": 4294278144u64,
                "message": { "runs": [{ "text": "Happy birthday" }] },
            } } } },
            { "addChatItemAction": { "item": { "liveChatMembershipItemRenderer": {
                "id": "m3",
                "authorName": { "simpleText": "@member" },
                "headerPrimaryText": { "runs": [{ "text": "Member for " }, { "text": "13" }, { "text": " months" }] },
                "headerSubtext": { "simpleText": "Elite" },
            } } } },
            { "addChatItemAction": { "item": { "giftMessageViewModel": {
                "id": "m4",
                "text": { "content": "sent Chili" },
                "authorName": { "content": "@giver " },
                "authorAvatar": { "avatarViewModel": { "image": { "sources": [{ "url": "https://yt4.ggpht.com/g=s64", "width": 64 }] } } },
                "giftImage": { "sources": [{ "url": "//www.gstatic.com/chili.png", "width": 480 }] },
            } } } },
            { "addChatItemAction": { "item": { "liveChatViewerEngagementMessageRenderer": {
                "id": "m5",
                "message": { "runs": [{ "text": "Welcome to live chat!" }] },
            } } } },
            { "addChatItemAction": { "item": { "liveChatPlaceholderItemRenderer": { "id": "m6" } } } },
            { "replaceChatItemAction": { "targetItemId": "m1", "replacementItem": { "liveChatTextMessageRenderer": {
                "id": "m1", "authorName": { "simpleText": "@viewer" }, "message": { "runs": [{ "text": "edited" }] },
            } } } },
            { "markChatItemAsDeletedAction": { "targetItemId": "m2" } },
            { "removeChatItemByAuthorAction": { "externalChannelId": "UCspammer" } },
            { "addBannerToLiveChatCommand": { "bannerRenderer": { "liveChatBannerRenderer": {
                "actionId": "pin1",
                "contents": { "liveChatTextMessageRenderer": {
                    "id": "m7", "authorName": { "simpleText": "@host" },
                    "authorBadges": [{ "liveChatAuthorBadgeRenderer": { "icon": { "iconType": "OWNER" } } }],
                    "message": { "runs": [{ "text": "Rules: be nice" }] },
                } },
            } } } },
            { "addBannerToLiveChatCommand": { "bannerRenderer": { "liveChatBannerRenderer": {
                "actionId": "summary", "contents": { "liveChatBannerChatSummaryRenderer": {} },
            } } } },
            { "removeBannerForLiveChatCommand": { "targetActionId": "pin1" } },
            { "liveChatReportModerationStateCommand": {} },
        ]));
        assert_eq!(page.continuation.as_deref(), Some("next"));
        assert_eq!(page.wait_ms, 4321);
        assert_eq!(page.viewer.as_deref(), Some("@me"));
        let added: Vec<&ChatMessage> = page
            .actions
            .iter()
            .filter_map(|a| match a {
                ChatAction::Add { message } => Some(message),
                _ => None,
            })
            .collect();
        // The placeholder isn't a message.
        assert_eq!(added.len(), 5);

        let text = added[0];
        assert_eq!(text.id, "m1");
        assert_eq!(text.time_usec, 1_791_691_341_112_047);
        assert_eq!(text.author.name, "@viewer");
        assert_eq!(text.author.channel_id.as_deref(), Some("UCviewer"));
        assert_eq!(
            text.author.avatar.as_deref(),
            Some("https://yt4.ggpht.com/a=s64")
        );
        assert!(text.author.moderator && !text.author.owner);
        assert_eq!(text.author.member.as_deref(), Some("Member (2 years)"));
        assert_eq!(
            text.author.member_badge.as_deref(),
            Some("https://yt3.ggpht.com/badge=s16")
        );
        let runs: Vec<(&str, Option<&str>)> = text
            .text
            .runs
            .iter()
            .map(|r| (r.text.as_str(), r.emoji.as_deref()))
            .collect();
        assert_eq!(
            runs,
            vec![
                ("hi ", None),
                ("\u{1f60a}", None),
                (":yt:", Some("https://yt3.ggpht.com/yt=w48")),
            ]
        );

        assert_eq!(
            added[1].kind,
            ChatKind::SuperChat {
                amount: "$19.99".into(),
                header_color: "rgba(230, 81, 0, 1.00)".into(),
                body_color: "rgba(245, 124, 0, 1.00)".into(),
            }
        );
        assert_eq!(
            added[2].kind,
            ChatKind::Membership {
                header: "Member for 13 months".into(),
                subtext: Some("Elite".into()),
            }
        );
        assert_eq!(added[3].author.name, "@giver");
        assert_eq!(added[3].text.runs[0].text, "sent Chili");
        assert_eq!(
            added[3].kind,
            ChatKind::Gift {
                image: Some("https://www.gstatic.com/chili.png".into())
            }
        );
        assert_eq!(added[4].kind, ChatKind::Notice);

        let rest: Vec<&ChatAction> = page
            .actions
            .iter()
            .filter(|a| !matches!(a, ChatAction::Add { .. }))
            .collect();
        assert!(
            matches!(rest[0], ChatAction::Replace { id, message } if id == "m1" && message.text.runs[0].text == "edited")
        );
        assert_eq!(rest[1], &ChatAction::Remove { id: "m2".into() });
        assert_eq!(
            rest[2],
            &ChatAction::RemoveByAuthor {
                channel_id: "UCspammer".into()
            }
        );
        assert!(
            matches!(rest[3], ChatAction::Pin { id, message } if id == "pin1" && message.author.owner)
        );
        // The summary isn't a message to pin.
        assert_eq!(rest[4], &ChatAction::Unpin { id: "pin1".into() });
        assert_eq!(rest.len(), 5);
    }

    #[test]
    fn an_ended_chat_has_no_next_page() {
        let page = parse(&json!({ "responseContext": {} }));
        assert!(page.continuation.is_none() && page.actions.is_empty());
        // YouTube's long waits for web pages are cut short.
        let page = parse(&json!({ "continuationContents": { "liveChatContinuation": {
            "continuations": [{ "invalidationContinuationData": { "timeoutMs": 10000, "continuation": "c" } }],
        } } }));
        assert_eq!(page.wait_ms, 8_000);
    }
}
