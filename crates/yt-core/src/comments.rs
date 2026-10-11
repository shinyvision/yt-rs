//! Comments: pages of comment threads, a thread's replies, and writing.
//!
//! The TV client gets comments in the `commentRenderer` format. A comment's
//! like and dislike tokens only come with its thread (the `next` call behind
//! its `thread` token), so voting loads the thread first. Posting, replying
//! and deleting need parameters the TV client never receives; they are
//! built here the way YouTube's own clients build them.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::channel::{ChannelRef, array, best_image, collect, find};
use crate::error::{Error, Result};
use crate::feed::text;
use crate::proto::Message;
use crate::text::RichText;

/// How the signed-in user voted on a comment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Vote {
    #[default]
    None,
    Like,
    Dislike,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    pub author: ChannelRef,
    pub text: RichText,
    /// E.g. "3 days ago".
    pub published: String,
    #[serde(default)]
    pub edited: bool,
    /// E.g. "1.2K"; none for comments without likes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub likes: Option<String>,
    #[serde(default)]
    pub vote: Vote,
    #[serde(default)]
    pub reply_count: u32,
    /// E.g. "Pinned by @Channel".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned: Option<String>,
    /// Loads the comment's thread: its replies and its vote tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
}

/// A page of top-level comments.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CommentsPage {
    pub comments: Vec<Comment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

/// The tokens that like, unlike, dislike and undislike a comment.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VoteActions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub like: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unlike: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dislike: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undislike: Option<String>,
}

impl VoteActions {
    /// The token that moves a comment from `from` to `to`.
    pub fn token(&self, from: Vote, to: Vote) -> Option<&str> {
        match (from, to) {
            (_, Vote::Like) => self.like.as_deref(),
            (_, Vote::Dislike) => self.dislike.as_deref(),
            (Vote::Like, Vote::None) => self.unlike.as_deref(),
            (Vote::Dislike, Vote::None) => self.undislike.as_deref(),
            (Vote::None, Vote::None) => None,
        }
    }
}

/// A comment's thread: the comment itself with its vote tokens (only on the
/// first page) and a page of replies.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CommentThread {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<Comment>,
    #[serde(default)]
    pub actions: VoteActions,
    pub replies: Vec<Comment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

/// Reads a page of comments (`next` with a comments token).
pub fn parse_page(resp: &Value, video_id: &str) -> CommentsPage {
    let section = section(resp);
    CommentsPage {
        comments: array(section.and_then(|s| s.get("contents")))
            .filter_map(|item| item.pointer("/commentThreadRenderer/comment/commentRenderer"))
            .map(|c| parse_comment(c, video_id))
            .collect(),
        next: section.and_then(next_token),
    }
}

/// Reads a thread (`next` with a comment's `thread` token, or the `next`
/// token of a previous replies page).
pub fn parse_thread(resp: &Value, video_id: &str) -> CommentThread {
    let section = section(resp);
    let mut thread = CommentThread::default();
    for item in array(section.and_then(|s| s.get("contents"))) {
        if let Some(c) = item.get("commentRenderer") {
            thread.comment = Some(parse_comment(c, video_id));
            thread.actions = vote_actions(c);
        } else if let Some(c) = item.pointer("/commentThreadRenderer/comment/commentRenderer") {
            thread.replies.push(parse_comment(c, video_id));
        }
    }
    thread.next = section.and_then(next_token);
    thread
}

/// Reads the comment a create or reply call returns, or why it failed.
///
/// The response also holds placeholder renderers (a reply's comes before
/// the reply itself), so this takes the one with the most of a comment.
pub fn parse_created(resp: &Value, video_id: &str) -> Result<Comment> {
    check_result(resp)?;
    let filled = |c: &Value| {
        [
            "commentId",
            "contentText",
            "authorText",
            "publishedTimeText",
        ]
        .iter()
        .filter(|key| c.get(**key).is_some())
        .count()
    };
    let mut best: Option<&Value> = None;
    collect(resp, "commentRenderer", &mut |c| {
        if best.is_none_or(|b| filled(c) > filled(b)) {
            best = Some(c);
        }
    });
    best.map(|c| parse_comment(c, video_id))
        .ok_or_else(|| Error::Api {
            endpoint: "comment".into(),
            status: 200,
            message: Some("YouTube didn't return the new comment".into()),
        })
}

/// Fails if an action's response reports a failure (they come with status 200).
pub fn check_result(resp: &Value) -> Result<()> {
    let mut failure = None;
    collect(resp, "actionResult", &mut |r| {
        if r.get("status").and_then(Value::as_str) == Some("STATUS_FAILED") {
            failure = Some(text(r.get("feedbackText")));
        }
    });
    match failure {
        Some(message) => Err(Error::Api {
            endpoint: "comment".into(),
            status: 200,
            message: Some(if message.is_empty() {
                "YouTube refused the action".into()
            } else {
                message
            }),
        }),
        None => Ok(()),
    }
}

/// `params` for posting a top-level comment on a video.
pub fn create_params(video_id: &str) -> String {
    Message::new()
        .string(2, video_id)
        .message(5, Message::new())
        .varint(10, 7)
        .to_token()
}

/// `params` for replying to a top-level comment.
pub fn reply_params(video_id: &str, parent_id: &str) -> String {
    Message::new()
        .string(2, video_id)
        .string(4, parent_id)
        .message(5, Message::new().varint(1, 0))
        .varint(10, 7)
        .to_token()
}

/// The action token that deletes one of the user's own comments or replies.
pub fn delete_action(video_id: &str, comment_id: &str) -> String {
    Message::new()
        .varint(1, 6)
        .varint(2, 2)
        .string(3, comment_id)
        .string(5, video_id)
        .to_token()
}

fn section(resp: &Value) -> Option<&Value> {
    resp.pointer("/continuationContents/itemSectionContinuation")
        .or_else(|| find(resp, "itemSectionContinuation"))
}

fn next_token(section: &Value) -> Option<String> {
    array(section.get("continuations"))
        .find_map(|c| c.pointer("/nextContinuationData/continuation")?.as_str())
        .map(str::to_owned)
}

fn parse_comment(c: &Value, video_id: &str) -> Comment {
    let published = text(c.get("publishedTimeText"));
    let edited = published.contains("(edited)");
    let name = text(c.get("authorText"));
    let mut thread = None;
    if let Some(endpoint) = c.get("detailViewEndpoint") {
        collect(endpoint, "reloadContinuationData", &mut |r| {
            thread = thread.take().or_else(|| {
                r.get("continuation")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            });
        });
    }
    Comment {
        id: c
            .get("commentId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        author: ChannelRef {
            id: c
                .pointer("/authorEndpoint/browseEndpoint/browseId")
                .and_then(Value::as_str)
                .map(str::to_owned),
            handle: name.starts_with('@').then(|| name.clone()),
            name,
            avatar: best_image(c.pointer("/authorThumbnail/thumbnails"), 88),
            ..ChannelRef::default()
        },
        text: RichText::from_json(c.get("contentText"), Some(video_id)),
        published: published.replace("(edited)", "").trim().to_owned(),
        edited,
        likes: Some(text(c.get("voteCount"))).filter(|v| !v.is_empty() && v != "0"),
        vote: match c.get("voteStatus").and_then(Value::as_str) {
            Some("LIKE") => Vote::Like,
            Some("DISLIKE") => Vote::Dislike,
            _ if c.get("isLiked").and_then(Value::as_bool) == Some(true) => Vote::Like,
            _ => Vote::None,
        },
        reply_count: c.get("replyCount").and_then(Value::as_u64).unwrap_or(0) as u32,
        pinned: Some(text(
            c.pointer("/pinnedCommentBadge/pinnedCommentBadgeRenderer/label"),
        ))
        .filter(|p| !p.is_empty()),
        thread,
    }
}

fn vote_actions(c: &Value) -> VoteActions {
    let buttons = c.pointer("/actionButtons/commentActionButtonsRenderer");
    let token = |button: &str, endpoint: &str| {
        buttons?
            .pointer(&format!(
                "/{button}/toggleButtonRenderer/{endpoint}/performCommentActionEndpoint/action"
            ))?
            .as_str()
            .map(str::to_owned)
    };
    VoteActions {
        like: token("likeButton", "defaultServiceEndpoint"),
        unlike: token("likeButton", "toggledServiceEndpoint"),
        dislike: token("dislikeButton", "defaultServiceEndpoint"),
        undislike: token("dislikeButton", "toggledServiceEndpoint"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params() {
        assert_eq!(create_params("Gp3BRbNSLEA"), "EgtHcDNCUmJOU0xFQSoAUAc%3D");
        // Fields: 2 = video, 4 = parent comment.
        let reply = reply_params("vid", "parent");
        assert!(reply.ends_with("%3D") || !reply.contains('='));
        assert_ne!(reply, create_params("vid"));
    }

    #[test]
    fn created_reply_skips_the_placeholder() {
        // The shape of a TV reply response: a placeholder renderer, then the
        // reply inside the list mutation.
        let resp = serde_json::json!({
            "contents": { "commentRenderer": { "trackingParams": "x" } },
            "actions": [{ "listMutationCommand": { "operations": { "operations": [{
                "insertItemSectionContent": { "contents": [{ "commentThreadRenderer": { "comment": {
                    "commentRenderer": {
                        "commentId": "parent.reply",
                        "authorText": { "runs": [{ "text": "@me" }] },
                        "contentText": { "runs": [{ "text": "hello there" }] },
                        "publishedTimeText": { "runs": [{ "text": "just now" }] },
                    }
                } } }] }
            }] } } }],
            "actionResult": { "status": "STATUS_SUCCEEDED" },
        });
        let reply = parse_created(&resp, "vid").unwrap();
        assert_eq!(reply.id, "parent.reply");
        assert_eq!(reply.author.name, "@me");
        assert_eq!(reply.text.plain(), "hello there");
        assert_eq!(reply.published, "just now");
    }

    #[test]
    fn vote_tokens() {
        let actions = VoteActions {
            like: Some("L".into()),
            unlike: Some("UL".into()),
            dislike: Some("D".into()),
            undislike: Some("UD".into()),
        };
        assert_eq!(actions.token(Vote::None, Vote::Like), Some("L"));
        assert_eq!(actions.token(Vote::Like, Vote::None), Some("UL"));
        assert_eq!(actions.token(Vote::Like, Vote::Dislike), Some("D"));
        assert_eq!(actions.token(Vote::Dislike, Vote::None), Some("UD"));
        assert_eq!(actions.token(Vote::None, Vote::None), None);
    }
}
