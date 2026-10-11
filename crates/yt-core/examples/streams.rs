//! Looks up videos' streams with the saved sign-in and checks that they
//! download: `cargo run -p yt-core --example streams -- VIDEO_ID...`.

use std::sync::Arc;
use std::time::Instant;

use yt_core::auth::{OAuth, Session};
use yt_core::innertube::InnerTube;
use yt_core::streams::{Resolver, ResolverOptions, Stream};

async fn check(http: &reqwest::Client, label: &str, stream: &Stream) {
    let url = match stream.segment_duration {
        Some(_) => stream.url.clone(),
        None => format!("{}&range=0-262143", stream.url),
    };
    let t = Instant::now();
    match http.get(&url).send().await {
        Ok(resp) => {
            let status = resp.status();
            let bytes = resp.bytes().await.map(|b| b.len()).unwrap_or(0);
            println!("  {label}: {status}, {bytes} bytes in {:?}", t.elapsed());
        }
        Err(e) => println!("  {label}: {e}"),
    }
}

#[tokio::main]
async fn main() -> yt_core::Result<()> {
    let ids: Vec<String> = std::env::args().skip(1).collect();
    let http = yt_core::http_client();
    let session = Arc::new(Session::load(
        OAuth::new(http.clone()),
        yt_core::paths::token_file(),
    ));
    let youtube = Arc::new(InnerTube::new(http.clone(), session));
    let resolver = Resolver::new(
        youtube,
        ResolverOptions::default(),
        tokio::runtime::Handle::current(),
    );
    for id in ids {
        let t = Instant::now();
        match resolver.resolve(&id).await {
            Ok(set) => {
                println!(
                    "{id}: found in {:?} | live {} | {:?} {:?} + {:?} | {} qualities: {:?} | {} captions | storyboard {}",
                    t.elapsed(),
                    set.live,
                    set.video.quality_label(),
                    set.video.vcodec,
                    set.audio.as_ref().and_then(|a| a.acodec.clone()),
                    set.qualities.len(),
                    set.qualities
                        .iter()
                        .filter_map(Stream::quality_label)
                        .collect::<Vec<_>>(),
                    set.captions.len(),
                    set.storyboard.as_ref().map_or(0, |s| s.fragments.len()),
                );
                if std::env::var("SHOW_URL").is_ok() {
                    println!("URL {}", set.video.url);
                    if let Some(audio) = &set.audio {
                        println!("AUDIO_URL {}", audio.url);
                    }
                }
                check(&http, "video", &set.video).await;
                if let Some(audio) = &set.audio {
                    check(&http, "audio", audio).await;
                }
                if let Some(q) = set.qualities.last() {
                    check(&http, "lowest quality", q).await;
                }
                if let Some(c) = set.captions.first() {
                    let body = http.get(&c.url).send().await.ok().map(|r| r.status());
                    println!("  captions {} ({}): {body:?}", c.label(), c.lang);
                }
            }
            Err(e) => println!("{id}: {e} ({:?})", t.elapsed()),
        }
    }
    let t = Instant::now();
    let again = resolver.resolve("dQw4w9WgXcQ").await.map(|_| ());
    println!("cached lookup: {again:?} in {:?}", t.elapsed());
    Ok(())
}
