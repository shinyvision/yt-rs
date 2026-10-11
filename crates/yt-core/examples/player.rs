//! Shows what a signed-in player response says about watch-time reporting,
//! without reporting anything: `cargo run -p yt-core --example player -- VIDEO_ID`.

use std::sync::Arc;

use yt_core::auth::{OAuth, Session};
use yt_core::innertube::InnerTube;
use yt_core::tracking::{self, Tracking};

#[tokio::main]
async fn main() -> yt_core::Result<()> {
    let id = std::env::args().nth(1).expect("usage: player VIDEO_ID");
    let http = yt_core::http_client();
    let session = Arc::new(Session::load(
        OAuth::new(http.clone()),
        yt_core::paths::token_file(),
    ));
    let api = InnerTube::new(http, session);
    let resp = api.player(&id, &tracking::new_cpn()).await?;
    println!(
        "playability: {:?}",
        tracking::unplayable_reason(&resp).unwrap_or_else(|| "OK".into())
    );
    println!("title: {:?}", resp.pointer("/videoDetails/title"));
    println!("start seconds: {:?}", tracking::start_seconds(&resp));
    println!(
        "vss3Config present: {}",
        resp.pointer("/playbackTracking/vss3Config").is_some()
    );
    match Tracking::from_player_response(&resp) {
        Some(t) => {
            println!(
                "length {} s, schedule {:?} then every {} s",
                t.length, t.scheduled, t.interval
            );
            let keys = |u: &str| {
                url::Url::parse(u)
                    .unwrap()
                    .query_pairs()
                    .map(|(k, _)| k.into_owned())
                    .collect::<Vec<_>>()
                    .join(",")
            };
            println!(
                "playback url: {} [{}]",
                t.playback_url.split('?').next().unwrap(),
                keys(&t.playback_url)
            );
            println!(
                "watchtime url: {} [{}]",
                t.watchtime_url.split('?').next().unwrap(),
                keys(&t.watchtime_url)
            );
        }
        None => println!(
            "no tracking URLs; playbackTracking keys: {:?}",
            resp.get("playbackTracking")
                .map(|t| t.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()))
        ),
    }
    Ok(())
}
