//! Every channel the app has seen, with its avatar: from the subscriptions
//! list, watch pages, channel pages and comments.
//!
//! The TV feed doesn't give a video's channel ID or avatar, so cards look
//! channels up here by name. A channel the directory doesn't know yet is
//! looked up through one of its videos ([`ChannelDirectory::want`]), a few
//! at a time. The directory is saved to disk, so each channel is looked up
//! once and avatars appear immediately on the next start.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::json;
use yt_core::channel::ChannelRef;

use crate::app::app;
use crate::runtime;

/// Lookups of unknown channels running at once.
const MAX_LOOKUPS: usize = 2;

#[derive(Default)]
pub struct ChannelDirectory {
    by_key: RefCell<HashMap<String, ChannelRef>>,
    subscriptions: RefCell<Vec<ChannelRef>>,
    file: PathBuf,
    /// Names looked up (or being looked up) this session.
    wanted: RefCell<HashSet<String>>,
    /// (channel name, one of its videos) waiting to be looked up.
    queue: RefCell<VecDeque<(String, String)>>,
    running: Cell<usize>,
}

impl ChannelDirectory {
    pub fn load() -> Rc<Self> {
        let file = yt_core::paths::cache_dir().join("channels.json");
        let saved: Vec<ChannelRef> = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let directory = Rc::new(Self {
            file,
            ..Self::default()
        });
        for channel in saved {
            directory.insert(channel);
        }
        directory
    }

    /// The channel known by this ID, `@handle` or display name.
    pub fn lookup(&self, key: &str) -> Option<ChannelRef> {
        self.by_key.borrow().get(&normalize(key)).cloned()
    }

    pub fn subscriptions(&self) -> Vec<ChannelRef> {
        self.subscriptions.borrow().clone()
    }

    /// Adds or updates a channel, keeping details it already had.
    pub fn remember(&self, channel: &ChannelRef) {
        if channel.avatar.is_none() && channel.id.is_none() {
            return;
        }
        self.insert(channel.clone());
    }

    fn insert(&self, channel: ChannelRef) {
        let mut map = self.by_key.borrow_mut();
        let mut merged = keys(&channel)
            .iter()
            .find_map(|k| map.get(k).cloned())
            .unwrap_or_default();
        // Filed under its old keys as well as its new ones.
        let mut all = keys(&merged);
        merged.id = channel.id.or(merged.id);
        merged.handle = channel.handle.or(merged.handle);
        merged.avatar = channel.avatar.or(merged.avatar);
        merged.subscribers = channel.subscribers.or(merged.subscribers);
        merged.verified |= channel.verified;
        // A display name beats a bare @handle.
        if merged.name.is_empty() || !channel.name.is_empty() && !channel.name.starts_with('@') {
            merged.name = channel.name;
        }
        all.extend(keys(&merged));
        for key in all {
            map.insert(key, merged.clone());
        }
    }

    /// Loads the subscriptions list in the background, then tells the UI
    /// (`channels.updated`) so visible cards pick up the avatars.
    pub fn refresh(self: &Rc<Self>) {
        let this = self.clone();
        runtime::spawn_local(async move {
            let youtube = app().youtube.clone();
            match runtime::background(async move { youtube.subscriptions().await }).await {
                Ok(channels) => {
                    for channel in &channels {
                        this.remember(channel);
                    }
                    *this.subscriptions.borrow_mut() = channels.clone();
                    this.save();
                    app()
                        .ext
                        .events
                        .emit("channels.updated", &json!({ "subscriptions": channels }));
                }
                Err(e) => tracing::warn!("couldn't load subscriptions: {e}"),
            }
        });
    }

    /// Looks up a channel known only by name through one of its videos,
    /// unless its avatar is known. Emits `channels.learned` with the channel.
    pub fn want(self: &Rc<Self>, name: &str, video_id: &str) {
        if name.is_empty()
            || video_id.is_empty()
            || self.lookup(name).is_some_and(|c| c.avatar.is_some())
        {
            return;
        }
        if self.wanted.borrow_mut().insert(normalize(name)) {
            self.queue
                .borrow_mut()
                .push_back((name.to_owned(), video_id.to_owned()));
            self.pump();
        }
    }

    fn pump(self: &Rc<Self>) {
        while self.running.get() < MAX_LOOKUPS {
            let Some((name, video_id)) = self.queue.borrow_mut().pop_front() else {
                return;
            };
            self.running.set(self.running.get() + 1);
            let this = self.clone();
            runtime::spawn_local(async move {
                let youtube = app().youtube.clone();
                let page = runtime::background(async move { youtube.watch(&video_id).await }).await;
                match page.map(|p| p.owner) {
                    Ok(Some(owner)) => {
                        this.remember(&owner);
                        // The feed may spell the name differently.
                        this.insert(ChannelRef {
                            name: name.clone(),
                            ..owner.clone()
                        });
                        app().ext.events.emit(
                            "channels.learned",
                            &json!({ "name": name, "channel": owner }),
                        );
                    }
                    Ok(None) => {}
                    Err(e) => tracing::debug!("couldn't look up channel {name:?}: {e}"),
                }
                this.running.set(this.running.get() - 1);
                if this.running.get() == 0 && this.queue.borrow().is_empty() {
                    this.save();
                }
                this.pump();
            });
        }
    }

    pub fn save(&self) {
        let mut unique: Vec<ChannelRef> = Vec::new();
        for channel in self.by_key.borrow().values() {
            if !unique.iter().any(|c| c == channel) {
                unique.push(channel.clone());
            }
        }
        let (file, data) = (
            self.file.clone(),
            serde_json::to_vec(&unique).unwrap_or_default(),
        );
        runtime::handle().spawn_blocking(move || {
            if let Some(dir) = file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(file, data);
        });
    }
}

/// The keys a channel is found by: its ID, handle and name.
fn keys(channel: &ChannelRef) -> Vec<String> {
    [
        channel.id.as_deref(),
        channel.handle.as_deref(),
        Some(channel.name.as_str()),
    ]
    .into_iter()
    .flatten()
    .filter(|k| !k.is_empty())
    .map(normalize)
    .collect()
}

fn normalize(key: &str) -> String {
    key.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(
        id: Option<&str>,
        name: &str,
        handle: Option<&str>,
        avatar: Option<&str>,
    ) -> ChannelRef {
        ChannelRef {
            id: id.map(Into::into),
            name: name.into(),
            handle: handle.map(Into::into),
            avatar: avatar.map(Into::into),
            ..ChannelRef::default()
        }
    }

    #[test]
    fn merges_what_it_learns() {
        let dir = ChannelDirectory::default();
        dir.remember(&channel(Some("UC1"), "BBC Earth", None, Some("a.jpg")));
        // A comment author: only a handle and a newer avatar.
        dir.remember(&channel(
            Some("UC1"),
            "@bbcearth",
            Some("@bbcearth"),
            Some("b.jpg"),
        ));
        let found = dir.lookup("bbc earth").unwrap();
        assert_eq!(
            (
                found.id.as_deref(),
                found.name.as_str(),
                found.handle.as_deref(),
                found.avatar.as_deref()
            ),
            (Some("UC1"), "BBC Earth", Some("@bbcearth"), Some("b.jpg"))
        );
        assert_eq!(dir.lookup("@BBCEarth"), Some(found.clone()));
        assert_eq!(dir.lookup("UC1"), Some(found));
        dir.remember(&channel(None, "Nobody", None, None));
        assert_eq!(dir.lookup("Nobody"), None);
    }
}
