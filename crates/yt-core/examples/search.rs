//! Searches with the saved sign-in and prints what came back:
//! `cargo run -p yt-core --example search -- "query"`. Signed-in searches go
//! into the account's search history, as on youtube.com.

use std::sync::Arc;

use yt_core::auth::{OAuth, Session};
use yt_core::innertube::InnerTube;
use yt_core::search::{Feature, Kind, SearchFilters, SearchItem, SearchPage, Sort, UploadDate};

fn show(label: &str, page: &SearchPage) {
    println!(
        "== {label}: {} items, next: {}",
        page.items.len(),
        page.next.is_some()
    );
    for item in page.items.iter().take(6) {
        match item {
            SearchItem::Video(v) => println!(
                "  video   {} | {} | {:?} {:?} | snippet {} | avatar {} | badges {:?}",
                v.video.id,
                v.video.title,
                v.video.views,
                v.video.duration,
                v.snippet.is_some(),
                v.owner.as_ref().is_some_and(|o| o.avatar.is_some()),
                v.badges
            ),
            SearchItem::Channel(c) => println!(
                "  channel {} {:?} {:?}",
                c.channel.name, c.channel.handle, c.channel.subscribers
            ),
            SearchItem::Playlist(p) => println!("  playlist {} {:?}", p.title, p.video_count),
            SearchItem::Shelf(s) => println!(
                "  shelf   {} ({} videos, shorts: {})",
                s.title,
                s.videos.len(),
                s.shorts
            ),
        }
    }
}

#[tokio::main]
async fn main() -> yt_core::Result<()> {
    let query = std::env::args().nth(1).unwrap_or_else(|| "hello".into());
    let http = yt_core::http_client();
    let session = Arc::new(Session::load(
        OAuth::new(http.clone()),
        yt_core::paths::token_file(),
    ));
    let api = InnerTube::new(http, session);

    let page = api.search(&query, &SearchFilters::default(), None).await?;
    show("all", &page);
    let chips: Vec<&str> = page.chips.iter().map(|c| c.title.as_str()).collect();
    println!("  chips {chips:?} | correction {:?}", page.correction);

    if let Some(chip) = page.chips.iter().find(|c| c.title == "Unwatched") {
        let unwatched = api
            .search(&query, &SearchFilters::default(), chip.token.as_deref())
            .await?;
        show("unwatched", &unwatched);
    }
    if let Some(next) = &page.next {
        show("page 2", &api.search_more(next).await?);
    }
    let filters = SearchFilters {
        upload_date: Some(UploadDate::Year),
        kind: Some(Kind::Video),
        features: vec![Feature::Hd, Feature::Subtitles],
        sort: Sort::Popularity,
        ..SearchFilters::default()
    };
    show(
        "this year, videos, HD, CC, by popularity",
        &api.search(&query, &filters, None).await?,
    );

    let suggestions = api.suggestions(&query[..query.len().min(3)]).await?;
    let texts: Vec<&str> = suggestions.iter().map(|s| s.text.as_str()).collect();
    println!("== suggestions: {texts:?}");
    Ok(())
}
