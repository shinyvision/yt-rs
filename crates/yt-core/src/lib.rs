//! YouTube logic without any UI: signing in, the InnerTube API, parsing
//! feeds and search results, reporting watch time and looking up playable
//! streams.
//!
//! Everything here is plain async Rust, so it can be tested without a
//! display and reused by any front end.

pub mod account;
pub mod auth;
pub mod captions;
pub mod channel;
pub mod comments;
pub mod error;
pub mod feed;
pub mod innertube;
pub mod paths;
mod proto;
pub mod search;
pub mod solver;
pub mod streams;
pub mod text;
pub mod tracking;
pub mod watch;

pub use error::{Error, Result};

/// The HTTP client every part of the app shares, so connections to YouTube
/// are reused.
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        // youtube.com/tv sends more than the 16 KiB of HTTP/2 headers that
        // hyper accepts by default.
        .http2_max_header_list_size(256 * 1024)
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(60))
        .pool_max_idle_per_host(8)
        .build()
        .expect("the HTTP client configuration is valid")
}
