//! Search results from real TV and web responses for "hello", trimmed. The
//! TV one is signed in; its personal shelves were replaced with public ones
//! and its progress bar is made up.

use serde_json::Value;
use yt_core::search::{self, SearchItem};

fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn tv_results() {
    let tv = search::parse_tv(&fixture("search_tv.json"));
    assert_eq!(tv.items.len(), 5, "three videos, a channel and a playlist");
    let SearchItem::Video(first) = &tv.items[0] else {
        panic!("{:?}", tv.items[0])
    };
    assert_eq!(first.video.id, "YQHsXMglC9A");
    assert_eq!(first.video.title, "Adele - Hello (Official Music Video)");
    assert_eq!(first.video.channel.as_deref(), Some("Adele"));
    assert_eq!(first.video.views.as_deref(), Some("3.2B views"));
    assert_eq!(first.video.duration.as_deref(), Some("6:07"));
    assert!(
        first.badges.contains(&"CC".to_owned()),
        "{:?}",
        first.badges
    );
    assert!(first.music);
    let SearchItem::Video(second) = &tv.items[1] else {
        panic!()
    };
    assert_eq!(second.video.progress, Some(0.3));

    let SearchItem::Channel(channel) = &tv.items[3] else {
        panic!("{:?}", tv.items[3])
    };
    assert_eq!(
        channel.channel.id.as_deref(),
        Some("UC5QrlJmN0CA88HBJcOS99XA")
    );
    assert_eq!(channel.channel.handle.as_deref(), Some("@HELLOmag"));
    assert_eq!(
        channel.channel.subscribers.as_deref(),
        Some("237K subscribers")
    );
    assert!(
        channel
            .channel
            .avatar
            .as_deref()
            .is_some_and(|a| a.starts_with("https://"))
    );

    let SearchItem::Playlist(playlist) = &tv.items[4] else {
        panic!("{:?}", tv.items[4])
    };
    assert_eq!(playlist.title, "Hello Channel");
    assert_eq!(playlist.video_count.as_deref(), Some("25 videos"));
    assert_eq!(playlist.channel.as_deref(), Some("Stefan Forosan"));

    let titles: Vec<&str> = tv.shelves.iter().map(|s| s.title.as_str()).collect();
    assert!(
        titles.contains(&"Shorts") && titles.contains(&"People also watched"),
        "{titles:?}"
    );
    assert!(
        tv.shelves
            .iter()
            .find(|s| s.title == "Shorts")
            .unwrap()
            .shorts
    );
    assert!(tv.next.is_some());
    assert!(tv.estimated_results.is_some_and(|n| n > 1000));
}

#[test]
fn web_details() {
    let web = search::parse_web(&fixture("search_web.json"));
    let adele = &web.videos["YQHsXMglC9A"];
    assert!(adele.snippet.is_some(), "a description snippet");
    assert!(
        web.videos.values().any(|v| v
            .snippet
            .as_ref()
            .is_some_and(|s| s.runs.iter().any(|r| r.bold))),
        "the query in bold where the description has it"
    );
    let owner = adele.owner.as_ref().unwrap();
    assert!(owner.id.as_deref().is_some_and(|id| id.starts_with("UC")));
    assert!(owner.avatar.is_some() && owner.verified);

    let top = &web.top_channels[0];
    assert_eq!(top.channel.name, "Adele");
    assert_eq!(top.channel.id.as_deref(), Some("UCsRM0YB_dabtEPGPTKo-gcw"));
    assert_eq!(top.channel.handle.as_deref(), Some("@adele"));
    assert_eq!(
        top.channel.subscribers.as_deref(),
        Some("33.5M subscribers")
    );
    assert_eq!(top.video_count.as_deref(), Some("67 videos"));

    let chips: Vec<&str> = web.chips.iter().map(|c| c.title.as_str()).collect();
    assert_eq!(
        chips,
        [
            "All",
            "Shorts",
            "Unwatched",
            "Videos",
            "Recently uploaded",
            "Live"
        ]
    );
    assert!(web.chips[0].selected && web.chips[0].token.is_none());
    assert!(web.chips[1..].iter().all(|c| c.token.is_some()));

    let correction = web.correction.as_ref().unwrap();
    assert!(correction.applied && !correction.query.is_empty());
    assert!(web.next.is_some());
}

#[test]
fn merged_page() {
    let page = search::merge(
        Some(search::parse_tv(&fixture("search_tv.json"))),
        Some(search::parse_web(&fixture("search_web.json"))),
    );
    let kinds: Vec<&str> = page
        .items
        .iter()
        .map(|i| match i {
            SearchItem::Video(_) => "video",
            SearchItem::Channel(_) => "channel",
            SearchItem::Playlist(_) => "playlist",
            SearchItem::Shelf(_) => "shelf",
        })
        .collect();
    // The query's channel leads; a shelf follows the fourth result.
    assert_eq!(
        kinds,
        [
            "channel", "video", "video", "video", "channel", "shelf", "playlist", "shelf"
        ]
    );
    let SearchItem::Video(first) = &page.items[1] else {
        panic!()
    };
    assert!(first.snippet.is_some(), "the web client's snippet");
    assert!(
        first.owner.as_ref().is_some_and(|o| o.avatar.is_some()),
        "and its channel"
    );
    assert!(first.music, "the TV client's details stay");
    assert_eq!(page.chips.len(), 6);
    let next = page.next.unwrap();
    assert!(next.tv.is_some() && next.web.is_some());
}

#[test]
fn web_results_alone() {
    let page = search::merge(None, Some(search::parse_web(&fixture("search_web.json"))));
    assert_eq!(page.items.len(), 4, "the channel and three videos");
    assert!(page.next.is_some_and(|n| n.tv.is_none() && n.web.is_some()));
}
