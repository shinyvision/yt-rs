//! Downloads: videos kept for watching offline, as YouTube's own Downloads
//! are, inside the app. Each is a directory (`downloads/<video ID>/`) with
//! the video stream and the audio stream as YouTube sends them, which mpv
//! plays together, and a picture; [`Index`] lists them with what the app
//! knows about each.
//!
//! [`fetch`] saves a stream with a [`ChunkedReader`] (full speed, resumes
//! after lost connections, gets new URLs when they expire), carrying on
//! from what an earlier, interrupted download left.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::feed::Video;
use crate::streams::chunked::ChunkedReader;

/// What is downloaded, or downloading.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Download {
    pub video: Video,
    /// The video's length, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    /// "1080p".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    /// The files, relative to the download's directory.
    pub video_file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_file: Option<String>,
    /// Bytes saved, and in all (when known).
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub total: u64,
    #[serde(default)]
    pub complete: bool,
    /// When it was started, in seconds since 1970.
    #[serde(default)]
    pub added: u64,
}

impl Download {
    pub fn new(
        video: Video,
        duration: Option<f64>,
        quality: Option<String>,
        video_ext: &str,
        audio_ext: Option<&str>,
    ) -> Self {
        Self {
            video,
            duration,
            quality,
            video_file: format!("video.{video_ext}"),
            audio_file: audio_ext.map(|e| format!("audio.{e}")),
            bytes: 0,
            total: 0,
            complete: false,
            added: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
        }
    }

    pub fn dir(&self) -> PathBuf {
        dir_for(&self.video.id)
    }

    pub fn video_path(&self) -> PathBuf {
        self.dir().join(&self.video_file)
    }

    pub fn audio_path(&self) -> Option<PathBuf> {
        self.audio_file.as_ref().map(|f| self.dir().join(f))
    }

    pub fn thumbnail_path(&self) -> PathBuf {
        self.dir().join("thumbnail.jpg")
    }

    /// The fraction saved, from 0 to 1.
    pub fn progress(&self) -> f64 {
        match (self.complete, self.total) {
            (true, _) => 1.0,
            (_, 0) => 0.0,
            (_, total) => (self.bytes as f64 / total as f64).min(1.0),
        }
    }
}

/// Where a video's download is kept. Only plain IDs make a directory name.
pub fn dir_for(video_id: &str) -> PathBuf {
    let safe: String = video_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    crate::paths::downloads_dir().join(safe)
}

/// The list of downloads, newest first, kept as `downloads.json`.
#[derive(Debug, Default)]
pub struct Index {
    file: PathBuf,
    pub downloads: Vec<Download>,
}

impl Index {
    pub fn load() -> Self {
        let file = crate::paths::downloads_dir().join("downloads.json");
        let downloads = std::fs::read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self { file, downloads }
    }

    pub fn get(&self, video_id: &str) -> Option<&Download> {
        self.downloads.iter().find(|d| d.video.id == video_id)
    }

    pub fn get_mut(&mut self, video_id: &str) -> Option<&mut Download> {
        self.downloads.iter_mut().find(|d| d.video.id == video_id)
    }

    /// Adds or replaces a video's download, as the newest.
    pub fn put(&mut self, download: Download) {
        self.downloads.retain(|d| d.video.id != download.video.id);
        self.downloads.insert(0, download);
    }

    /// Forgets a download and deletes its files.
    pub fn remove(&mut self, video_id: &str) -> io::Result<()> {
        self.downloads.retain(|d| d.video.id != video_id);
        match std::fs::remove_dir_all(dir_for(video_id)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// Writes the list, whole, then puts it in place.
    pub fn save(&self) -> io::Result<()> {
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let partial = self.file.with_extension("json.partial");
        std::fs::write(&partial, serde_json::to_vec_pretty(&self.downloads)?)?;
        std::fs::rename(&partial, &self.file)
    }
}

/// Saves a stream to `path`, after what is there already (an interrupted
/// download), calling `progress(saved, total)` as it goes; it stops early
/// if that returns false (cancelled). Returns the bytes in the file.
/// Blocking: run it on a thread of its own.
pub fn fetch(
    mut reader: ChunkedReader,
    path: &Path,
    mut progress: impl FnMut(u64, u64) -> bool,
) -> io::Result<u64> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let mut saved = file.metadata()?.len();
    let total = reader.size()?;
    if saved > total {
        // Not this stream's: start again.
        file.set_len(0)?;
        saved = 0;
    }
    reader.seek(saved)?;
    let mut buf = vec![0; 1 << 20];
    while saved < total {
        if !progress(saved, total) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        saved += n as u64;
    }
    file.flush()?;
    progress(saved, total);
    if saved < total {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the stream ended early",
        ));
    }
    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_and_paths() {
        let mut d = Download::new(
            Video {
                id: "dQw4w9WgXcQ".into(),
                ..Video::default()
            },
            Some(213.0),
            Some("1080p".into()),
            "webm",
            Some("webm"),
        );
        assert_eq!(d.progress(), 0.0);
        d.total = 200;
        d.bytes = 50;
        assert_eq!(d.progress(), 0.25);
        d.complete = true;
        assert_eq!(d.progress(), 1.0);
        assert!(d.video_path().ends_with("dQw4w9WgXcQ/video.webm"));
        assert!(d.audio_path().unwrap().ends_with("dQw4w9WgXcQ/audio.webm"));
        // An ID can't climb out of the downloads directory.
        assert!(dir_for("../../etc").ends_with("downloads/etc"));
    }
}
