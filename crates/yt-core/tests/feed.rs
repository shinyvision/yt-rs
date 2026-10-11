//! The fixtures are responses to the TVHTML5 client for a signed-in home
//! feed, cut down and with titles, channels and video IDs replaced, but with
//! YouTube's nesting left as it was.

use yt_core::feed::{self, FeedPage, Shelf, Video};

fn load(name: &str) -> FeedPage {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    feed::parse(&json)
}

fn video(
    id: &str,
    title: &str,
    channel: &str,
    views: &str,
    published: Option<&str>,
    duration: &str,
) -> Video {
    Video {
        id: id.into(),
        title: title.into(),
        channel: Some(channel.into()),
        views: Some(views.into()),
        published: published.map(Into::into),
        duration: Some(duration.into()),
        live: duration == "LIVE",
        ..Video::default()
    }
}

/// Compares the videos, checking thumbnails separately.
fn check(page: &FeedPage, want: &[Shelf]) {
    assert_eq!(
        page.shelves.len(),
        want.len(),
        "shelves: {:#?}",
        page.shelves
    );
    for (got, want) in page.shelves.iter().zip(want) {
        assert_eq!(got.title, want.title);
        assert_eq!(got.continuation, want.continuation, "shelf {:?}", got.title);
        let got_videos: Vec<Video> = got
            .videos
            .iter()
            .map(|v| {
                // The largest thumbnail up to 720 px wide.
                assert_eq!(
                    v.thumbnail.as_deref(),
                    Some(format!("https://i.ytimg.com/vi/{}/hq720_custom_1.jpg", v.id).as_str())
                );
                Video {
                    thumbnail: None,
                    ..v.clone()
                }
            })
            .collect();
        assert_eq!(got_videos, want.videos, "shelf {:?}", got.title);
    }
}

fn a() -> Video {
    video(
        "video-A-id",
        "Video A",
        "Channel A",
        "2.6M views",
        Some("10 months ago"),
        "1:04:53",
    )
}
fn c() -> Video {
    video(
        "video-C-id",
        "Video C",
        "Channel C",
        "36K views",
        Some("5 hours ago"),
        "47:40",
    )
}

#[test]
fn first_page() {
    let page = load("home_tv.json");
    check(
        &page,
        &[
            Shelf {
                playlists: vec![],
                title: "Recommended".into(),
                // The ad in front of A is skipped.
                videos: vec![
                    a(),
                    video(
                        "video-B-id",
                        "Video B",
                        "Channel B",
                        "2 watching",
                        None,
                        "LIVE",
                    ),
                    c(),
                ],
                continuation: Some("MORE_RECOMMENDED".into()),
            },
            Shelf {
                playlists: vec![],
                title: "Recently uploaded".into(),
                videos: vec![
                    Video {
                        progress: Some(0.4),
                        ..video(
                            "video-D-id",
                            "Video D",
                            "Channel D",
                            "2.6M views",
                            Some("10 months ago"),
                            "1:04:53",
                        )
                    },
                    a(),
                ],
                continuation: Some("MORE_RECENT".into()),
            },
        ],
    );
    assert_eq!(page.continuation.as_deref(), Some("MORE_SHELVES"));
}

#[test]
fn more_of_a_shelf() {
    let page = load("home_tv_more_row.json");
    check(
        &page,
        &[Shelf {
            playlists: vec![],
            title: String::new(),
            videos: vec![
                video(
                    "video-E-id",
                    "Video E",
                    "Channel E",
                    "2.6M views",
                    Some("10 months ago"),
                    "1:04:53",
                ),
                c(),
            ],
            continuation: None,
        }],
    );
    assert_eq!(page.continuation, None);
}

#[test]
fn more_shelves_skip_shows() {
    let page = load("home_tv_more_shelves.json");
    check(
        &page,
        &[
            Shelf {
                playlists: vec![],
                title: "Recommended shows".into(),
                videos: vec![video(
                    "video-F-id",
                    "Video F",
                    "Channel F",
                    "2.6M views",
                    Some("10 months ago"),
                    "1:04:53",
                )],
                continuation: Some("MORE_SHOWS".into()),
            },
            Shelf {
                playlists: vec![],
                title: "Nature and more".into(),
                videos: vec![video(
                    "video-G-id",
                    "Video G",
                    "Channel G",
                    "2.6M views",
                    Some("10 months ago"),
                    "1:04:53",
                )],
                continuation: None,
            },
        ],
    );
}

#[test]
fn lockups() {
    let page = load("lockups_tv.json");
    let mut short = video(
        "j19HFPplMQ0",
        "Knife Skills Progression",
        "Nick DiGiovanni",
        "132M views",
        Some("1 year ago"),
        "",
    );
    short.short = true;
    short.duration = None;
    let [shelf] = page.shelves.as_slice() else {
        panic!("{:#?}", page.shelves)
    };
    let got: Vec<Video> = shelf
        .videos
        .iter()
        .map(|v| Video {
            thumbnail: None,
            ..v.clone()
        })
        .collect();
    assert_eq!(
        got,
        vec![
            video(
                "QDleZC5l_rk",
                "I Asked Famous Youtubers To Cook in Their Homes",
                "Joshua Weissman",
                "3.2M views",
                Some("6 days ago"),
                "24:52"
            ),
            short,
        ]
    );
}

#[test]
fn nudge_instead_of_a_feed() {
    let json = serde_json::json!({"contents": {"tvBrowseRenderer": {"content": {"tvSurfaceContentRenderer": {"content": {
        "sectionListRenderer": {"contents": [{"feedNudgeRenderer": {"title": {"runs": [{"text": "Your YouTube history is off"}]}}}]}
    }}}}}});
    let page = feed::parse(&json);
    assert!(page.shelves.is_empty());
    assert_eq!(page.nudge.as_deref(), Some("Your YouTube history is off"));
}
