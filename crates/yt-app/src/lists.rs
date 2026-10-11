//! Playing a list of videos, one after the other: a playlist ("Play all",
//! "Shuffle", or a click on one of its videos) or the queue ("Add to
//! queue"), as on youtube.com. One list plays at a time. Its videos play on
//! when one ends (autoplay or not), it can loop and shuffle, and more of a
//! long playlist loads as playback nears the end of what is known.
//!
//! Starting a video that isn't the list's next (a suggestion, say) leaves
//! the list. Every change is announced as `list.changed` with the list
//! (see [`List`]) or `null`.

use std::cell::RefCell;

use serde::Serialize;
use serde_json::{Value, json};
use yt_core::feed::Video;

use crate::app::app;
use crate::{playback, runtime};

/// Loads more of the playlist when this few videos are left.
const LOAD_MORE_WITHIN: usize = 3;

#[derive(Clone, Debug, Serialize)]
pub struct List {
    /// The playlist's ID; `None` for the queue.
    pub playlist_id: Option<String>,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    pub videos: Vec<Video>,
    /// The order they play in, as indices into `videos`.
    pub order: Vec<usize>,
    /// Where in `order` playback is.
    pub position: usize,
    pub looping: bool,
    pub shuffled: bool,
    /// More of the playlist, not loaded yet.
    #[serde(skip)]
    continuation: Option<String>,
    #[serde(skip)]
    loading: bool,
}

impl List {
    pub fn current(&self) -> Option<&Video> {
        self.order
            .get(self.position)
            .and_then(|&i| self.videos.get(i))
    }

    fn after(&self, position: usize) -> Option<usize> {
        match position + 1 {
            next if next < self.order.len() => Some(next),
            _ if self.looping && !self.order.is_empty() => Some(0),
            _ => None,
        }
    }
}

thread_local! {
    static LIST: RefCell<Option<List>> = const { RefCell::new(None) };
    /// The video the list is starting, so `started` knows it is the list's.
    static STARTING: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub fn current() -> Option<List> {
    LIST.with(|l| l.borrow().clone())
}

/// Whether a list is playing.
pub fn playing() -> bool {
    LIST.with(|l| l.borrow().is_some())
}

/// The list's next video, if there is one.
pub fn next_video() -> Option<Video> {
    LIST.with(|l| {
        let list = l.borrow();
        let list = list.as_ref()?;
        let next = list.after(list.position)?;
        list.videos.get(list.order[next]).cloned()
    })
}

/// Where a video of the list plays.
#[derive(Clone, Copy)]
enum Show {
    /// On the watch page: the user chose it.
    Watch,
    /// Where the player is (the watch page, or the mini player): the list
    /// playing on.
    Here,
}

/// Plays the list's next video, where the player is; an error at its end.
pub fn next() -> Result<(), String> {
    let next = LIST.with(|l| {
        let list = l.borrow();
        let list = list.as_ref()?;
        list.after(list.position)
    });
    start(
        next.ok_or("that was the last video in the list")?,
        Show::Here,
    )
}

/// Plays the list's previous video (the first again, at the start), where
/// the player is.
pub fn previous() -> Result<(), String> {
    let position = LIST
        .with(|l| l.borrow().as_ref().map(|list| list.position))
        .ok_or("no list is playing")?;
    start(position.saturating_sub(1), Show::Here)
}

/// Plays the video at `position` in the list's order, on the watch page.
pub fn play_at(position: usize) -> Result<(), String> {
    start(position, Show::Watch)
}

fn start(position: usize, show: Show) -> Result<(), String> {
    let video = LIST.with(|l| {
        let mut list = l.borrow_mut();
        let list = list.as_mut()?;
        let index = *list.order.get(position)?;
        list.position = position;
        list.videos.get(index).cloned()
    });
    let video = video.ok_or("no such video in the list")?;
    STARTING.with(|s| *s.borrow_mut() = Some(video.id.clone()));
    changed();
    match show {
        Show::Watch => playback::play(video, None),
        Show::Here => playback::play_here(video),
    }
    load_more_if_near_end();
    Ok(())
}

/// Called as any video starts: leaves the list unless it is the list's.
pub fn started(video_id: &str) {
    let ours = STARTING.with(|s| s.borrow_mut().take()).as_deref() == Some(video_id);
    if ours || !playing() {
        return;
    }
    // A click on the list's current video (the watch page shown again).
    let current = LIST.with(|l| {
        l.borrow()
            .as_ref()
            .and_then(|list| list.current().map(|v| v.id == video_id))
    });
    if current != Some(true) {
        clear();
    }
}

pub fn clear() {
    if LIST.with(|l| l.borrow_mut().take()).is_some() {
        changed();
    }
}

pub fn set_looping(looping: bool) {
    LIST.with(|l| {
        if let Some(list) = l.borrow_mut().as_mut() {
            list.looping = looping;
        }
    });
    changed();
}

/// Shuffles what is left of the list (the current video stays where it
/// is), or puts it back in order.
pub fn set_shuffled(shuffled: bool) {
    LIST.with(|l| {
        let mut list = l.borrow_mut();
        let Some(list) = list.as_mut() else { return };
        let current = list.order.get(list.position).copied();
        list.shuffled = shuffled;
        list.order = (0..list.videos.len()).collect();
        if shuffled {
            shuffle(&mut list.order);
            // The current video first, the rest after it.
            if let Some(current) = current {
                list.order.retain(|&i| i != current);
                list.order.insert(0, current);
            }
            list.position = 0;
        } else if let Some(current) = current {
            list.position = current;
        }
    });
    changed();
}

/// Plays a playlist: from the video at `index` (in the playlist's order),
/// or from the start, or shuffled.
pub fn play_playlist(playlist_id: &str, index: Option<usize>, shuffled: bool) {
    let id = playlist_id.to_owned();
    runtime::spawn_local(async move {
        let youtube = app().youtube.clone();
        let playlist_id = id.clone();
        let result = runtime::background(async move { youtube.playlist(&playlist_id).await }).await;
        let page = match result {
            Ok(page) if !page.videos.is_empty() => page,
            Ok(_) => return app().toast("This playlist has no videos"),
            Err(e) => return app().toast(&format!("Couldn't load the playlist: {e}")),
        };
        let videos = page.videos;
        let index = index.unwrap_or(0).min(videos.len() - 1);
        let list = List {
            playlist_id: Some(id),
            title: page.playlist.title,
            owner: page.owner,
            order: (0..videos.len()).collect(),
            videos,
            position: index,
            looping: false,
            shuffled: false,
            continuation: page.continuation,
            loading: false,
        };
        LIST.with(|l| *l.borrow_mut() = Some(list));
        if shuffled {
            set_shuffled(true);
        }
        let position = LIST.with(|l| l.borrow().as_ref().map_or(0, |list| list.position));
        if let Err(e) = play_at(position) {
            app().toast(&e);
        }
    });
}

/// Adds a video to the queue: the queue starts with the video playing (if
/// one is, outside a queue), and if nothing plays, the video plays.
pub fn add_to_queue(video: Video) {
    let in_queue = LIST.with(|l| {
        let mut list = l.borrow_mut();
        match list.as_mut() {
            Some(list) if list.playlist_id.is_none() => {
                list.order.push(list.videos.len());
                list.videos.push(video.clone());
                true
            }
            _ => false,
        }
    });
    if in_queue {
        changed();
        app().toast("Added to queue");
        return;
    }
    let playing = playback::current();
    let mut videos: Vec<Video> = playing.into_iter().collect();
    let start = videos.is_empty();
    videos.push(video);
    LIST.with(|l| {
        *l.borrow_mut() = Some(List {
            playlist_id: None,
            title: "Queue".into(),
            owner: None,
            order: (0..videos.len()).collect(),
            videos,
            position: 0,
            looping: false,
            shuffled: false,
            continuation: None,
            loading: false,
        });
    });
    changed();
    app().toast("Added to queue");
    // Nothing was playing: the queue starts in the mini player, as on
    // youtube.com (or on the watch page, if that is shown).
    if start && let Err(e) = self::start(0, Show::Here) {
        app().toast(&e);
    }
}

/// Takes the video at `position` (in the order) out of the queue.
pub fn remove(position: usize) {
    LIST.with(|l| {
        let mut list = l.borrow_mut();
        let Some(list) = list.as_mut() else { return };
        if position >= list.order.len() || position == list.position {
            return;
        }
        let index = list.order.remove(position);
        list.videos.remove(index);
        for i in &mut list.order {
            if *i > index {
                *i -= 1;
            }
        }
        if position < list.position {
            list.position -= 1;
        }
    });
    changed();
}

fn load_more_if_near_end() {
    let token = LIST.with(|l| {
        let mut list = l.borrow_mut();
        let list = list.as_mut()?;
        if list.loading || list.order.len() - list.position > LOAD_MORE_WITHIN {
            return None;
        }
        let token = list.continuation.clone()?;
        list.loading = true;
        Some(token)
    });
    let Some(token) = token else { return };
    runtime::spawn_local(async move {
        let youtube = app().youtube.clone();
        let result = runtime::background(async move { youtube.continuation(&token).await }).await;
        LIST.with(|l| {
            let mut list = l.borrow_mut();
            let Some(list) = list.as_mut() else { return };
            list.loading = false;
            let Ok(more) = result else { return };
            list.continuation = more.continuation;
            let mut added: Vec<usize> = Vec::new();
            for video in more.shelves.into_iter().flat_map(|s| s.videos) {
                added.push(list.videos.len());
                list.videos.push(video);
            }
            if list.shuffled {
                shuffle(&mut added);
            }
            list.order.extend(added);
        });
        changed();
    });
}

fn changed() {
    let payload = current().map_or(Value::Null, |list| json!(list));
    app()
        .ext
        .events
        .emit("list.changed", &json!({ "list": payload }));
}

/// A uniform shuffle (Fisher–Yates).
fn shuffle(items: &mut [usize]) {
    for i in (1..items.len()).rev() {
        items.swap(i, rand::random_range(0..=i));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(n: usize) -> List {
        List {
            playlist_id: Some("PL1".into()),
            title: "A list".into(),
            owner: None,
            videos: (0..n)
                .map(|i| Video {
                    id: format!("v{i}"),
                    ..Video::default()
                })
                .collect(),
            order: (0..n).collect(),
            position: 0,
            looping: false,
            shuffled: false,
            continuation: None,
            loading: false,
        }
    }

    #[test]
    fn ends_or_loops() {
        let mut l = list(3);
        assert_eq!(l.after(0), Some(1));
        assert_eq!(l.after(2), None);
        l.looping = true;
        assert_eq!(l.after(2), Some(0));
        assert_eq!(list(0).after(0), None);
    }

    #[test]
    fn shuffles_every_video_once() {
        let mut order: Vec<usize> = (0..50).collect();
        shuffle(&mut order);
        let mut sorted = order.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..50).collect::<Vec<_>>());
        assert_ne!(
            order, sorted,
            "50 in their own order: as likely as winning a lottery"
        );
    }

    #[test]
    fn the_current_video_is_in_order() {
        let mut l = list(5);
        l.order = vec![3, 1, 4, 0, 2];
        l.position = 2;
        assert_eq!(l.current().map(|v| v.id.as_str()), Some("v4"));
    }
}
