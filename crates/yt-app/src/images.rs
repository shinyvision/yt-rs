//! Loads images (mostly thumbnails) for the UI.
//!
//! Images are fetched on the tokio runtime, cached on disk, decoded off the
//! main thread at the size they are shown (JPEG decoders can downscale while
//! decoding, which is faster and uses a fraction of the memory), and kept in
//! a small in-memory cache so scrolling back is instant.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;
use gtk::{gdk, gdk_pixbuf, glib};
use sha2::{Digest, Sha256};

use crate::runtime;

/// Decoded images kept in memory, in bytes of pixels: about 60 thumbnails,
/// and avatars cost what they weigh rather than a thumbnail's slot.
const MEMORY_CACHE_BYTES: usize = 32 * 1024 * 1024;
const DISK_CACHE_BYTES: u64 = 300 * 1024 * 1024;
const CONCURRENT_DOWNLOADS: usize = 8;
const URL_KEY: &str = "yt-rs-image-url";

/// Raw pixels decoded on a worker thread, turned into a texture on the main
/// thread.
struct Decoded {
    bytes: glib::Bytes,
    width: i32,
    height: i32,
    stride: usize,
    alpha: bool,
}

type Waiter = Box<dyn FnOnce(Option<gdk::Texture>)>;

pub struct Images {
    http: reqwest::Client,
    dir: PathBuf,
    downloads: Arc<tokio::sync::Semaphore>,
    memory: RefCell<HashMap<String, (gdk::Texture, u64)>>,
    memory_bytes: Cell<usize>,
    clock: Cell<u64>,
    loading: RefCell<HashMap<String, Vec<Waiter>>>,
}

impl Images {
    pub fn new(http: reqwest::Client, dir: PathBuf) -> Rc<Self> {
        let _ = std::fs::create_dir_all(&dir);
        let prune_dir = dir.clone();
        runtime::handle().spawn_blocking(move || prune(&prune_dir, DISK_CACHE_BYTES));
        Rc::new(Self {
            http,
            dir,
            downloads: Arc::new(tokio::sync::Semaphore::new(CONCURRENT_DOWNLOADS)),
            memory: RefCell::new(HashMap::new()),
            memory_bytes: Cell::new(0),
            clock: Cell::new(0),
            loading: RefCell::new(HashMap::new()),
        })
    }

    /// Shows the image at `url` in `picture`, decoded `width` pixels wide
    /// (0 for full size). If the picture is given another URL before this
    /// one finishes loading, the late result is dropped, so reused list
    /// items never flash the wrong image.
    pub fn load_into(self: &Rc<Self>, picture: &gtk::Picture, url: Option<&str>, width: i32) {
        let key = url.map(|u| cache_key(u, width));
        // SAFETY: this key only ever holds an Option<String>.
        let current = unsafe {
            picture
                .data::<Option<String>>(URL_KEY)
                .map(|p| p.as_ref().clone())
        }
        .flatten();
        if current.is_some() && current == key {
            return;
        }
        unsafe { picture.set_data(URL_KEY, key.clone()) };
        picture.set_paintable(None::<&gdk::Paintable>);
        let (Some(url), Some(key)) = (url, key) else {
            return;
        };

        if let Some(texture) = self.cached(&key) {
            picture.set_paintable(Some(&texture));
            return;
        }
        let weak = picture.downgrade();
        self.texture_then(
            url,
            width,
            Box::new(move |texture| {
                let (Some(picture), Some(texture)) = (weak.upgrade(), texture) else {
                    return;
                };
                let still_wanted = unsafe {
                    picture
                        .data::<Option<String>>(URL_KEY)
                        .map(|p| p.as_ref().clone())
                }
                .flatten();
                if still_wanted.as_deref() == Some(key.as_str()) {
                    picture.set_paintable(Some(&texture));
                }
            }),
        );
    }

    /// Loads an image and passes the texture (or `None` on failure) to
    /// `done` on the main thread.
    pub fn texture_then(self: &Rc<Self>, url: &str, width: i32, done: Waiter) {
        let key = cache_key(url, width);
        if let Some(texture) = self.cached(&key) {
            done(Some(texture));
            return;
        }
        {
            let mut loading = self.loading.borrow_mut();
            if let Some(waiters) = loading.get_mut(&key) {
                waiters.push(done);
                return;
            }
            loading.insert(key.clone(), vec![done]);
        }
        let (http, dir, downloads, url) = (
            self.http.clone(),
            self.dir.clone(),
            self.downloads.clone(),
            url.to_owned(),
        );
        let this = self.clone();
        runtime::spawn_local(async move {
            let decoded = runtime::background(async move {
                fetch_and_decode(&http, &dir, &downloads, &url, width).await
            })
            .await;
            let texture = decoded.map(|d| {
                let format = if d.alpha {
                    gdk::MemoryFormat::R8g8b8a8
                } else {
                    gdk::MemoryFormat::R8g8b8
                };
                gdk::MemoryTexture::new(d.width, d.height, format, &d.bytes, d.stride)
                    .upcast::<gdk::Texture>()
            });
            if let Some(texture) = &texture {
                this.remember(&key, texture);
            }
            let waiters = this.loading.borrow_mut().remove(&key).unwrap_or_default();
            for waiter in waiters {
                waiter(texture.clone());
            }
        });
    }

    /// The decoded images kept in memory: how many, and their bytes.
    pub fn in_memory(&self) -> (usize, usize) {
        (self.memory.borrow().len(), self.memory_bytes.get())
    }

    fn cached(&self, key: &str) -> Option<gdk::Texture> {
        let tick = self.tick();
        let mut memory = self.memory.borrow_mut();
        let entry = memory.get_mut(key)?;
        entry.1 = tick;
        Some(entry.0.clone())
    }

    fn remember(&self, key: &str, texture: &gdk::Texture) {
        let tick = self.tick();
        let mut memory = self.memory.borrow_mut();
        let mut bytes = self.memory_bytes.get() + weight(texture);
        if let Some((old, _)) = memory.insert(key.to_owned(), (texture.clone(), tick)) {
            bytes -= weight(&old);
        }
        // Drop the least recently used until the cache fits again.
        while bytes > MEMORY_CACHE_BYTES && memory.len() > 1 {
            let Some(oldest) = memory
                .iter()
                .min_by_key(|(_, (_, t))| *t)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some((texture, _)) = memory.remove(&oldest) {
                bytes -= weight(&texture);
            }
        }
        self.memory_bytes.set(bytes);
    }

    fn tick(&self) -> u64 {
        self.clock.set(self.clock.get() + 1);
        self.clock.get()
    }
}

/// The memory a decoded texture's pixels take.
fn weight(texture: &gdk::Texture) -> usize {
    texture.width().max(0) as usize * texture.height().max(0) as usize * 4
}

fn cache_key(url: &str, width: i32) -> String {
    format!("{width}:{url}")
}

async fn fetch_and_decode(
    http: &reqwest::Client,
    dir: &std::path::Path,
    downloads: &tokio::sync::Semaphore,
    url: &str,
    width: i32,
) -> Option<Decoded> {
    let name: String = Sha256::digest(url.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let file = dir.join(name);
    let bytes = match tokio::fs::read(&file).await {
        Ok(bytes) => bytes,
        Err(_) => {
            let _permit = downloads.acquire().await.ok()?;
            let resp = http.get(url).send().await.ok()?.error_for_status().ok()?;
            let bytes = resp.bytes().await.ok()?.to_vec();
            let tmp = file.with_extension("tmp");
            if tokio::fs::write(&tmp, &bytes).await.is_ok() {
                let _ = tokio::fs::rename(&tmp, &file).await;
            }
            bytes
        }
    };
    tokio::task::spawn_blocking(move || decode(&bytes, width))
        .await
        .ok()
        .flatten()
}

/// Decodes an image, downscaling it to `width` while decoding if it is wider.
fn decode(bytes: &[u8], width: i32) -> Option<Decoded> {
    let loader = gdk_pixbuf::PixbufLoader::new();
    if width > 0 {
        loader.connect_size_prepared(move |loader, w, h| {
            if w > width {
                loader.set_size(width, (h as f64 * width as f64 / w as f64).round() as i32);
            }
        });
    }
    loader.write(bytes).ok()?;
    loader.close().ok()?;
    let pixbuf = loader.pixbuf()?;
    Some(Decoded {
        bytes: pixbuf.read_pixel_bytes(),
        width: pixbuf.width(),
        height: pixbuf.height(),
        stride: pixbuf.rowstride() as usize,
        alpha: pixbuf.has_alpha(),
    })
}

/// Deletes the least recently used files beyond `limit` bytes.
fn prune(dir: &std::path::Path, limit: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
        .filter_map(|e| {
            let e = e.ok()?;
            let meta = e.metadata().ok()?;
            Some((
                meta.accessed().or_else(|_| meta.modified()).ok()?,
                meta.len(),
                e.path(),
            ))
        })
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    let mut total = 0;
    for (_, len, path) in files {
        total += len;
        if total > limit {
            let _ = std::fs::remove_file(path);
        }
    }
}
