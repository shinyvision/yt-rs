//! Prints the user's playlists, a playlist's page and the save options for
//! a video: `cargo run -p yt-core --example playlists -- [playlist ID]
//! [video ID]`. Only reads.

use std::sync::Arc;

use yt_core::auth::{OAuth, Session};
use yt_core::innertube::InnerTube;

#[tokio::main]
async fn main() -> yt_core::Result<()> {
    let mut args = std::env::args().skip(1);
    let playlist_id = args.next().unwrap_or_else(|| "WL".into());
    let video_id = args.next().unwrap_or_else(|| "dQw4w9WgXcQ".into());
    let http = yt_core::http_client();
    let session = Arc::new(Session::load(
        OAuth::new(http.clone()),
        yt_core::paths::token_file(),
    ));
    let api = InnerTube::new(http, session);
    for p in api.playlists().await? {
        println!(
            "{} | {} | {:?} | {:?} | picture {}",
            p.id,
            p.title,
            p.count,
            p.details,
            p.thumbnail.is_some()
        );
    }
    let page = api.playlist(&playlist_id).await?;
    println!(
        "\n{} ({}): owner {:?}, {:?}, {:?}, {:?}, editable {}, {} videos here, more {}",
        page.playlist.title,
        page.playlist.id,
        page.owner,
        page.privacy,
        page.playlist.count,
        page.views,
        page.editable,
        page.videos.len(),
        page.continuation.is_some()
    );
    for v in page.videos.iter().take(3) {
        let actions: Vec<&str> = v.actions.iter().map(|a| a.label.as_str()).collect();
        println!(
            "   {} | {} | {:?} | {:?}",
            v.id, v.title, v.duration, actions
        );
    }
    if let Some(token) = &page.continuation {
        let more = api.continuation(token).await?;
        let n: usize = more.shelves.iter().map(|s| s.videos.len()).sum();
        println!(
            "   + {n} more videos, more after: {}",
            more.continuation.is_some()
        );
    }
    println!("\nsave {video_id} to:");
    for o in api.save_options(&video_id).await? {
        println!(
            "   {} | {} | {:?} | holds it: {}",
            o.playlist_id, o.title, o.privacy, o.contains
        );
    }
    Ok(())
}
