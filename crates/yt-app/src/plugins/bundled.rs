//! The plugins that come with the app, compiled in from `plugins/` at the
//! top of the repository. A plugin reads its own files (icons, fonts,
//! scripts) from its directory, so the first start of each version writes
//! them to the cache directory, and they load from there.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

include!(concat!(env!("OUT_DIR"), "/bundled.rs"));

/// The directory the bundled plugins are in, written now if this version's
/// aren't there yet.
pub fn dir() -> io::Result<PathBuf> {
    let parent = yt_core::paths::cache_dir().join("plugins");
    let dir = parent.join(BUNDLED_HASH);
    if dir.is_dir() {
        return Ok(dir);
    }
    // Written beside it and renamed into place, so a yt-rs starting at the
    // same time never finds half of it. When one does, the first to finish
    // wins.
    let partial = parent.join(format!("{BUNDLED_HASH}.{}", std::process::id()));
    if let Err(e) = write(&partial).and_then(|()| fs::rename(&partial, &dir)) {
        let _ = fs::remove_dir_all(&partial);
        if !dir.is_dir() {
            return Err(e);
        }
    }
    // Earlier versions' plugins, and any left half written.
    for entry in fs::read_dir(&parent).into_iter().flatten().flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with(BUNDLED_HASH) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
    Ok(dir)
}

fn write(dir: &Path) -> io::Result<()> {
    for (path, contents) in BUNDLED {
        let file = dir.join(path);
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(file, contents)?;
    }
    Ok(())
}
