//! Prints the TV client's raw answer to a browse request, for looking into
//! a page's structure: `cargo run -p yt-core --example browse_raw -- <browse
//! ID> [params]`. Only reads.

use std::sync::Arc;

use serde_json::json;
use yt_core::auth::{OAuth, Session};
use yt_core::innertube::InnerTube;

#[tokio::main]
async fn main() -> yt_core::Result<()> {
    let mut args = std::env::args().skip(1);
    let browse_id = args.next().expect("a browse ID");
    let mut body = json!({ "browseId": browse_id });
    if let Some(params) = args.next() {
        body["params"] = params.into();
    }
    let http = yt_core::http_client();
    let session = Arc::new(Session::load(
        OAuth::new(http.clone()),
        yt_core::paths::token_file(),
    ));
    let resp = InnerTube::new(http, session).call("browse", body).await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&resp).unwrap_or_default()
    );
    Ok(())
}
