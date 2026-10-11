//! Prints the TV client's raw answer to any InnerTube request, for looking
//! into an endpoint: `cargo run -p yt-core --example call_raw -- <endpoint>
//! '<JSON body>'`. Careful: some endpoints change the account.

use std::sync::Arc;

use yt_core::auth::{OAuth, Session};
use yt_core::innertube::InnerTube;

#[tokio::main]
async fn main() -> yt_core::Result<()> {
    let mut args = std::env::args().skip(1);
    let endpoint = args.next().expect("an endpoint");
    let body =
        serde_json::from_str(&args.next().unwrap_or_else(|| "{}".into())).expect("a JSON body");
    let http = yt_core::http_client();
    let session = Arc::new(Session::load(
        OAuth::new(http.clone()),
        yt_core::paths::token_file(),
    ));
    let resp = InnerTube::new(http, session).call(&endpoint, body).await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&resp).unwrap_or_default()
    );
    Ok(())
}
