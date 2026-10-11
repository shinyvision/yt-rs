//! Where the app keeps its files. `YT_RS_CONFIG_DIR` and `YT_RS_CACHE_DIR`
//! override the XDG defaults, which tests and scripted runs use to stay
//! apart from the real profile.

use std::path::PathBuf;

const APP: &str = "yt-rs";

fn dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("", "", APP)
}

/// Settings, the saved sign-in and plugins (`~/.config/yt-rs`).
pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("YT_RS_CONFIG_DIR") {
        return dir.into();
    }
    dirs()
        .map(|d| d.config_dir().to_owned())
        .unwrap_or_else(|| PathBuf::from(".yt-rs"))
}

/// Disposable data such as thumbnails (`~/.cache/yt-rs`).
pub fn cache_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("YT_RS_CACHE_DIR") {
        return dir.into();
    }
    dirs()
        .map(|d| d.cache_dir().to_owned())
        .unwrap_or_else(|| std::env::temp_dir().join(APP))
}

pub fn token_file() -> PathBuf {
    config_dir().join("token.json")
}

pub fn plugins_dir() -> PathBuf {
    config_dir().join("plugins")
}

pub fn init_lua() -> PathBuf {
    config_dir().join("init.lua")
}
