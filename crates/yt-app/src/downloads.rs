//! Downloading videos to watch offline (see `yt_core::downloads`): the
//! commands, the downloads under way, and playing a downloaded video from
//! its files, offline or not.
//!
//! Each download runs on a thread of its own, the video stream and then the
//! audio stream, and reports how far it got: `download.changed`
//! `{ video_id, download }` (also when one starts, ends or is deleted, with
//! `download` null then), and `download.done` / `download.failed`. Downloads
//! that didn't finish carry on when the app starts again.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gtk::glib;
use serde_json::{Value, json};
use yt_core::downloads::{Download, Index};
use yt_core::feed::Video;
use yt_core::streams::chunked::ChunkedReader;
use yt_core::streams::{Stream, StreamSet};

use crate::app::{App, app};
use crate::commands::str_arg;
use crate::runtime;

/// How often a download says how far it got.
const REPORT_EVERY: Duration = Duration::from_millis(250);

thread_local! {
    static INDEX: RefCell<Option<Index>> = const { RefCell::new(None) };
    /// Downloads under way, and how to stop each.
    static RUNNING: RefCell<HashMap<String, Arc<AtomicBool>>> = RefCell::default();
}

fn with_index<T>(f: impl FnOnce(&mut Index) -> T) -> T {
    INDEX.with(|i| f(i.borrow_mut().get_or_insert_with(Index::load)))
}

pub fn register(this: &std::rc::Rc<App>) {
    let c = &this.ext.commands;
    c.register("download.start", |args| {
        let video: Video = args
            .get("video")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .filter(|v: &Video| !v.id.is_empty())
            .ok_or("no video")?;
        let quality = args
            .get("quality")
            .and_then(Value::as_str)
            .map(str::to_owned);
        start(video, quality);
        Ok(Value::Null)
    });
    c.register("download.cancel", |args| {
        cancel(str_arg(&args, "video_id")?);
        Ok(Value::Null)
    });
    c.register("download.delete", |args| {
        delete(str_arg(&args, "video_id")?)?;
        Ok(Value::Null)
    });
    c.register("download.list", |_| Ok(json!(list())));
    c.register("download.get", |args| {
        Ok(json!(get(str_arg(&args, "video_id")?)))
    });
    // Carry on with what didn't finish, once the app is up.
    glib::timeout_add_seconds_local_once(5, || {
        let unfinished: Vec<Download> = with_index(|i| {
            i.downloads
                .iter()
                .filter(|d| !d.complete)
                .cloned()
                .collect()
        });
        for download in unfinished {
            start(download.video, download.quality);
        }
    });
}

pub fn list() -> Vec<Download> {
    with_index(|i| i.downloads.clone())
}

pub fn get(video_id: &str) -> Option<Download> {
    with_index(|i| i.get(video_id).cloned())
}

pub fn is_running(video_id: &str) -> bool {
    RUNNING.with(|r| r.borrow().contains_key(video_id))
}

/// A downloaded video's streams: its files.
pub fn stream_set(video_id: &str) -> Option<StreamSet> {
    let download = get(video_id).filter(|d| d.complete)?;
    let path = |p: std::path::PathBuf| p.to_string_lossy().into_owned();
    if !download.video_path().is_file() {
        return None;
    }
    Some(StreamSet {
        title: Some(download.video.title.clone()).filter(|t| !t.is_empty()),
        duration: download.duration,
        video: Stream {
            url: path(download.video_path()),
            label: download.quality.clone(),
            ..Stream::default()
        },
        audio: download
            .audio_path()
            .filter(|p| p.is_file())
            .map(|p| Stream {
                url: path(p),
                ..Stream::default()
            }),
        ..StreamSet::default()
    })
}

fn changed(video_id: &str) {
    let download = get(video_id);
    app().ext.events.emit(
        "download.changed",
        &json!({ "video_id": video_id, "download": download, "running": is_running(video_id) }),
    );
}

fn save() {
    if let Err(e) = with_index(|i| i.save()) {
        tracing::warn!("downloads: {e}");
    }
}

/// The file name's ending for a stream, from its codec.
fn extension(stream: &Stream, audio: bool) -> &'static str {
    let codec = if audio {
        &stream.acodec
    } else {
        &stream.vcodec
    };
    match codec.as_deref().map(|c| c.split('.').next().unwrap_or(c)) {
        Some("vp9" | "vp09" | "opus" | "vp8" | "vorbis") => "webm",
        _ if audio => "m4a",
        _ => "mp4",
    }
}

fn start(video: Video, quality: Option<String>) {
    let id = video.id.clone();
    if get(&id).is_some_and(|d| d.complete) {
        return app().toast("Already downloaded");
    }
    if is_running(&id) {
        return;
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    RUNNING.with(|r| r.borrow_mut().insert(id.clone(), cancelled.clone()));
    runtime::spawn_local(async move {
        let app = app();
        let resolver = app.streams.clone();
        let lookup_id = id.clone();
        let streams = runtime::background(async move { resolver.resolve(&lookup_id).await }).await;
        let streams = match streams {
            Ok(streams) if streams.live => {
                return finish(&id, Err("Live streams can't be downloaded".into()));
            }
            Ok(streams) => streams,
            Err(e) => return finish(&id, Err(format!("Couldn't download: {e}"))),
        };
        let chosen = quality
            .as_deref()
            .and_then(|q| {
                streams
                    .qualities
                    .iter()
                    .find(|s| s.quality_label().as_deref() == Some(q))
            })
            .unwrap_or(&streams.video)
            .clone();
        let audio = streams.audio.clone();
        let mut download = get(&id)
            .filter(|d| d.quality == chosen.quality_label())
            .unwrap_or_else(|| {
                Download::new(
                    video.clone(),
                    streams.duration,
                    chosen.quality_label(),
                    extension(&chosen, false),
                    audio.as_ref().map(|a| extension(a, true)),
                )
            });
        download.video = Video {
            actions: Vec::new(),
            menu: None,
            ..video.clone()
        };
        with_index(|i| i.put(download.clone()));
        save();
        changed(&id);
        app.toast_with(&json!({
            "message": format!("Downloading \u{201c}{}\u{201d}", video.title),
            "action": { "label": "View", "command": "app.navigate", "args": { "page": "downloads" } },
        }));
        // Its picture, for the Downloads page offline.
        let (http, picture) = (app.http.clone(), download.thumbnail_path());
        let url = format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg");
        runtime::background(async move {
            if let Ok(resp) = http.get(&url).send().await
                && let Ok(bytes) = resp.bytes().await
            {
                if let Some(dir) = picture.parent() {
                    let _ = tokio::fs::create_dir_all(dir).await;
                }
                let _ = tokio::fs::write(picture, bytes).await;
            }
        })
        .await;
        let mut parts = vec![(chosen, download.video_path())];
        if let (Some(audio), Some(path)) = (audio, download.audio_path()) {
            parts.push((audio, path));
        }
        let (http, handle, resolver) = (app.http.clone(), runtime::handle(), app.streams.clone());
        let video_id = id.clone();
        std::thread::Builder::new()
            .name(format!("download {video_id}"))
            .spawn(move || {
                let result = fetch_all(&video_id, parts, http, handle, resolver, &cancelled);
                let id = video_id.clone();
                glib::MainContext::default().invoke(move || finish(&id, result));
            })
            .expect("a thread for the download");
    });
}

/// Saves the parts one after the other, saying how far it got.
fn fetch_all(
    video_id: &str,
    parts: Vec<(Stream, std::path::PathBuf)>,
    http: reqwest::Client,
    handle: tokio::runtime::Handle,
    resolver: Arc<yt_core::streams::Resolver>,
    cancelled: &AtomicBool,
) -> Result<u64, String> {
    // The sizes of the parts, so progress counts all of them.
    let sizes: Vec<u64> = parts
        .iter()
        .map(|(stream, _)| yt_core::streams::chunked::content_length(&stream.url).unwrap_or(0))
        .collect();
    let total: u64 = sizes.iter().sum();
    let mut before = 0;
    let mut last = Instant::now() - REPORT_EVERY;
    for ((stream, path), size) in parts.into_iter().zip(sizes) {
        let refresh = stream
            .itag()
            .map(|itag| yt_core::streams::refresher(resolver.clone(), video_id.to_owned(), itag));
        let reader = ChunkedReader::new(
            http.clone(),
            stream.url.clone(),
            stream.headers.get("User-Agent").cloned(),
            refresh,
            handle.clone(),
        );
        let id = video_id.to_owned();
        yt_core::downloads::fetch(reader, &path, |saved, _| {
            if last.elapsed() >= REPORT_EVERY {
                last = Instant::now();
                let (id, bytes) = (id.clone(), before + saved);
                glib::MainContext::default().invoke(move || progress(&id, bytes, total));
            }
            !cancelled.load(Ordering::Relaxed)
        })
        .map_err(|e| e.to_string())?;
        before += size;
    }
    Ok(total)
}

fn progress(video_id: &str, bytes: u64, total: u64) {
    with_index(|i| {
        if let Some(d) = i.get_mut(video_id) {
            d.bytes = bytes;
            d.total = total;
        }
    });
    changed(video_id);
}

fn finish(video_id: &str, result: Result<u64, String>) {
    let cancelled = RUNNING.with(|r| r.borrow_mut().remove(video_id));
    let was_cancelled = cancelled.is_some_and(|c| c.load(Ordering::Relaxed));
    let title = get(video_id).map(|d| d.video.title).unwrap_or_default();
    match result {
        Ok(total) => {
            with_index(|i| {
                if let Some(d) = i.get_mut(video_id) {
                    d.complete = true;
                    d.bytes = total;
                    d.total = total;
                }
            });
            save();
            app().toast(&format!("Downloaded \u{201c}{title}\u{201d}"));
            app()
                .ext
                .events
                .emit("download.done", &json!({ "video_id": video_id }));
        }
        Err(_) if was_cancelled => {}
        Err(e) => {
            save();
            tracing::warn!(video_id, "download failed: {e}");
            app().toast(&format!("Couldn't download \u{201c}{title}\u{201d}: {e}"));
            app().ext.events.emit(
                "download.failed",
                &json!({ "video_id": video_id, "error": e }),
            );
        }
    }
    changed(video_id);
}

fn cancel(video_id: &str) {
    if let Some(flag) = RUNNING.with(|r| r.borrow().get(video_id).cloned()) {
        flag.store(true, Ordering::Relaxed);
    }
}

fn delete(video_id: &str) -> Result<(), String> {
    cancel(video_id);
    // A download being cancelled may still be writing: delete after it stops.
    let id = video_id.to_owned();
    let remove = move || {
        let result = with_index(|i| i.remove(&id));
        save();
        changed(&id);
        result
    };
    if is_running(video_id) {
        let id = video_id.to_owned();
        glib::timeout_add_local(Duration::from_millis(200), move || {
            if is_running(&id) {
                return glib::ControlFlow::Continue;
            }
            if let Err(e) = remove() {
                app().toast(&format!("Couldn't delete the download: {e}"));
            }
            glib::ControlFlow::Break
        });
        return Ok(());
    }
    remove().map_err(|e| format!("Couldn't delete the download: {e}"))?;
    app().toast("Download deleted");
    Ok(())
}
