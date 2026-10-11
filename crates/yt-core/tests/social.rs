//! Watch pages, comments, channels and the account, parsed from TV client
//! responses (cut down; comments, subscriptions and the account replaced
//! with placeholders, YouTube's nesting kept).

use serde_json::Value;
use yt_core::comments::{self, Vote};
use yt_core::text::Link;
use yt_core::{account, channel, watch};

fn load(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn watch_page() {
    let page = watch::parse(&load("watch_tv.json"), "Vd7iuPV_AF0");
    assert_eq!(
        page.title,
        "1 Hour of Magnificent Moments Narrated by Sir David Attenborough | BBC Earth"
    );
    assert_eq!(page.views.as_deref(), Some("2,673,306 views"));
    assert_eq!(page.published.as_deref(), Some("Published on Nov 30, 2025"));
    assert_eq!(page.likes.as_deref(), Some("13K"));

    let owner = page.owner.unwrap();
    assert_eq!(
        (
            owner.name.as_str(),
            owner.id.as_deref(),
            owner.handle.as_deref()
        ),
        (
            "BBC Earth",
            Some("UCwmZiChSryoWQCZMIQezgTg"),
            Some("@bbcearth")
        )
    );
    assert_eq!(owner.subscribers.as_deref(), Some("14.6M subscribers"));
    assert_eq!(
        owner.avatar.as_deref(),
        Some("https://yt3.ggpht.com/owner=s88")
    );
    assert!(owner.verified);

    let sub = page.subscription.unwrap();
    assert!(!sub.subscribed);
    assert_eq!(sub.channel_id, "UCwmZiChSryoWQCZMIQezgTg");
    assert!(sub.subscribe_params.is_some() && sub.unsubscribe_params.is_some());

    // The description's plain URLs became links.
    assert!(
        page.description
            .runs
            .iter()
            .any(|r| matches!(&r.link, Some(Link::Url { url }) if url.starts_with("http")))
    );

    let comments = page.comments.unwrap();
    assert_eq!(comments.count.as_deref(), Some("239"));
    let titles: Vec<_> = comments
        .sorts
        .iter()
        .map(|s| (s.title.as_str(), s.selected))
        .collect();
    assert_eq!(titles, vec![("Top", true), ("Newest", false)]);
    assert!(comments.token().is_some());
}

#[test]
fn comments_page() {
    let page = comments::parse_page(&load("comments_tv.json"), "vid");
    assert_eq!(page.comments.len(), 3, "the handoff note is skipped");
    let first = &page.comments[0];
    assert_eq!(first.author.name, "@commenter0");
    assert_eq!(first.author.handle.as_deref(), Some("@commenter0"));
    assert_eq!(
        first.author.avatar.as_deref(),
        Some("https://yt3.ggpht.com/author0=s88")
    );
    assert_eq!(
        first.text.plain(),
        "Comment 0 at 1:23, see https://example.org/0"
    );
    assert!(first.text.runs.iter().any(|r| r.link
        == Some(Link::Video {
            video_id: "vid".into(),
            start: Some(83.0)
        })));
    assert!(!first.id.is_empty() && first.thread.is_some());
    assert_eq!(first.vote, Vote::None);
    assert!(first.likes.is_some());
    assert!(page.next.is_some());
}

#[test]
fn comment_thread() {
    let thread = comments::parse_thread(&load("thread_tv.json"), "vid");
    let parent = thread.comment.expect("the thread starts with its comment");
    assert_eq!(parent.author.name, "@commenter100");
    let a = &thread.actions;
    assert!(a.like.is_some() && a.unlike.is_some() && a.dislike.is_some() && a.undislike.is_some());
    assert_eq!(a.token(Vote::None, Vote::Like), a.like.as_deref());
    assert_eq!(thread.replies.len(), 2);
    assert!(
        thread.replies.iter().all(|r| r.id.contains('.')),
        "reply IDs are parent.reply"
    );
}

#[test]
fn channel_page() {
    let page = channel::parse_page(&load("channel_tv.json"), "UCuAXFkgsw1L7xaCfnd5JJOw");
    let c = &page.channel;
    assert_eq!(
        (
            c.name.as_str(),
            c.handle.as_deref(),
            c.subscribers.as_deref()
        ),
        (
            "Rick Astley",
            Some("@RickAstleyYT"),
            Some("4.55M subscribers")
        )
    );
    assert!(
        c.avatar.is_some()
            && page
                .banner
                .as_deref()
                .is_some_and(|b| b.starts_with("https://"))
    );
    assert!(
        page.description
            .as_deref()
            .is_some_and(|d| d.starts_with("Richard Paul Astley"))
    );
    assert!(page.about.iter().any(|r| r.ends_with("videos")));
    let tabs: Vec<_> = page.tabs.iter().map(|t| t.title.as_str()).collect();
    assert_eq!(tabs, vec!["Videos", "Shorts", "Live", "Playlists"]);
    assert!(page.subscription.is_some());
    // Shorts tiles on channel pages are videos too.
    assert!(
        page.home
            .shelves
            .iter()
            .flat_map(|s| &s.videos)
            .any(|v| v.short)
    );
}

#[test]
fn subscriptions_and_account() {
    let (channels, _) = channel::parse_subscriptions(&load("subscriptions_tv.json"));
    assert!(!channels.is_empty());
    assert_eq!(channels[0].id.as_deref(), Some("UCchannel01"));
    assert_eq!(channels[0].name, "Channel 1");
    assert_eq!(
        channels[0].avatar.as_deref(),
        Some("https://yt3.ggpht.com/ch1=s176")
    );

    let me = account::parse(&load("account_tv.json")).unwrap();
    assert_eq!(
        (me.name.as_str(), me.handle.as_deref(), me.photo.as_deref()),
        (
            "Test Person",
            Some("@testperson"),
            Some("https://yt3.ggpht.com/me=s216")
        )
    );
}
