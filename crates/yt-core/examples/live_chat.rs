//! Reads a live stream's chat for a while and prints what it parsed:
//! `cargo run -p yt-core --example live_chat -- <channel ID> <video ID>
//! [top|live] [pages]`. Only reads.

use std::collections::BTreeMap;
use std::sync::Arc;

use yt_core::auth::{OAuth, Session};
use yt_core::innertube::InnerTube;
use yt_core::live_chat::{self, ChatAction, ChatView};

#[tokio::main]
async fn main() -> yt_core::Result<()> {
    let mut args = std::env::args().skip(1);
    let channel = args.next().expect("a channel ID");
    let video = args.next().expect("a video ID");
    let view = match args.next().as_deref() {
        Some("live") => ChatView::Live,
        _ => ChatView::Top,
    };
    let pages: usize = args.next().and_then(|n| n.parse().ok()).unwrap_or(3);
    let http = yt_core::http_client();
    let session = Arc::new(Session::load(
        OAuth::new(http.clone()),
        yt_core::paths::token_file(),
    ));
    let youtube = InnerTube::new(http, session);
    let mut token = live_chat::token(&channel, &video, view);
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    for n in 0..pages {
        let page = youtube.live_chat(&token).await?;
        println!(
            "page {n}: {} actions, wait {} ms, viewer {:?}",
            page.actions.len(),
            page.wait_ms,
            page.viewer
        );
        for action in &page.actions {
            let key = match action {
                ChatAction::Add { message } => {
                    let kind = serde_json::to_value(&message.kind).unwrap_or_default();
                    format!("add {}", kind["kind"].as_str().unwrap_or("?"))
                }
                other => serde_json::to_value(other).unwrap_or_default()["action"]
                    .as_str()
                    .unwrap_or("?")
                    .to_owned(),
            };
            *kinds.entry(key).or_default() += 1;
        }
        if let Some(ChatAction::Add { message }) = page.actions.last() {
            println!(
                "  last: {} | {:?}",
                message.author.name,
                message
                    .text
                    .runs
                    .iter()
                    .map(|r| r.text.as_str())
                    .collect::<String>()
            );
        }
        let Some(next) = page.continuation else {
            println!("the chat is over");
            break;
        };
        token = next;
        tokio::time::sleep(std::time::Duration::from_millis(page.wait_ms)).await;
    }
    println!("{kinds:#?}");
    Ok(())
}
