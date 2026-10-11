//! Prints the home feed of the saved sign-in: `cargo run -p yt-core --example home`.

use std::sync::Arc;

use yt_core::auth::{OAuth, Session};
use yt_core::innertube::{HOME, InnerTube};

#[tokio::main]
async fn main() -> yt_core::Result<()> {
    let http = yt_core::http_client();
    let session = Arc::new(Session::load(
        OAuth::new(http.clone()),
        yt_core::paths::token_file(),
    ));
    let api = InnerTube::new(http, session);
    let page = api.browse(HOME).await?;
    for shelf in &page.shelves {
        println!(
            "{} ({} videos, more: {})",
            shelf.title,
            shelf.videos.len(),
            shelf.continuation.is_some()
        );
        for v in shelf.videos.iter().take(3) {
            println!(
                "   {} | {} | {:?} | {:?} | progress {:?}",
                v.id, v.title, v.channel, v.duration, v.progress
            );
        }
    }
    println!("more shelves: {}", page.continuation.is_some());
    Ok(())
}
