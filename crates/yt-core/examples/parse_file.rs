//! Parses a saved InnerTube response: `cargo run -p yt-core --example parse_file -- response.json`.

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: parse_file <response.json>");
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let page = yt_core::feed::parse(&json);
    for shelf in &page.shelves {
        println!(
            "{:?}: {} videos, more: {}",
            shelf.title,
            shelf.videos.len(),
            shelf.continuation.is_some()
        );
        for v in shelf.videos.iter().take(2) {
            println!(
                "   {} | {:.50} | {:?} | {:?} | {:?} | progress {:?}",
                v.id, v.title, v.channel, v.published, v.duration, v.progress
            );
            let actions: Vec<&str> = v.actions.iter().map(|a| a.label.as_str()).collect();
            println!("      actions: {actions:?}");
        }
    }
    println!(
        "more shelves: {} | nudge: {:?}",
        page.continuation.is_some(),
        page.nudge
    );
}
