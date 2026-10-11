//! What a video does wherever it is shown (feed cards, search results): a
//! click plays it, resting the pointer on it looks its streams up so a click
//! plays at once, and its ⋮ menu offers what youtube.com's does.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use serde_json::{Value, json};

use yt_core::actions::{ActionKind, VideoAction};

use crate::app::app;

const PREFETCH_DELAY: Duration = Duration::from_millis(250);
/// Videos whose menu from YouTube is kept, once fetched.
const MENUS_KEPT: usize = 256;

thread_local! {
    /// YouTube's menu actions for videos whose menu comes on request, by
    /// video ID; `None` while being fetched.
    static FETCHED: RefCell<HashMap<String, Option<Vec<VideoAction>>>> = RefCell::default();
}

/// Plays the video `video()` returns when `widget` is clicked, and prefetches
/// it while the pointer rests on `widget`.
pub fn connect(widget: &impl IsA<gtk::Widget>, video: impl Fn() -> Option<Value> + 'static) {
    let video = Rc::new(video);
    let click = gtk::GestureClick::new();
    click.set_button(gtk::gdk::BUTTON_PRIMARY);
    let current = video.clone();
    click.connect_released(move |gesture, n_press, _, _| {
        if n_press == 1
            && let Some(video) = current()
        {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            run("video.play", json!({ "video": video }));
        }
    });
    widget.add_controller(click);

    let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
    let motion = gtk::EventControllerMotion::new();
    let (current, timer) = (video, pending.clone());
    motion.connect_enter(move |_, _, _| {
        let (current, slot) = (current.clone(), timer.clone());
        let id = glib::timeout_add_local_once(PREFETCH_DELAY, move || {
            slot.borrow_mut().take();
            if let Some(video) = current() {
                run("video.prefetch", json!({ "video": video }));
            }
        });
        if let Some(old) = timer.borrow_mut().replace(id) {
            old.remove();
        }
    });
    motion.connect_leave(move |_| {
        if let Some(id) = pending.borrow_mut().take() {
            id.remove();
        }
    });
    widget.add_controller(motion);
}

/// Gives `menu` the video's menu (see [`menu_items`]), each command run
/// with `{ video }` added to its arguments. For a video whose menu YouTube
/// sends on request, its items join the open menu when they arrive.
pub fn attach_menu(menu: &gtk::MenuButton, video: impl Fn() -> Option<Value> + 'static) {
    let video = Rc::new(video);
    let items = {
        let video = video.clone();
        Rc::new(move || video().map_or_else(|| json!([]), |v| menu_items(&v)))
    };
    let shown = items.clone();
    super::menu::attach(menu, move || shown());
    menu.connect_active_notify(move |button| {
        if !button.is_active() {
            return;
        }
        let Some(v) = video() else { return };
        let (weak, items) = (button.downgrade(), items.clone());
        load_actions(&v, move || {
            if let Some(button) = weak.upgrade()
                && button.is_active()
                && let Some(popover) = button.popover()
            {
                let menu = app()
                    .ext
                    .components
                    .build("menu", &json!({ "items": items() }));
                popover.set_child(Some(&menu));
            }
        });
    });
}

/// Fetches YouTube's menu for a video whose menu comes on request (once),
/// then calls `done` and emits `video.menu_loaded` `{ video_id }`.
pub fn load_actions(video: &Value, done: impl FnOnce() + 'static) {
    let Some(id) = video
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    else {
        return;
    };
    // Videos that come without a menu ("Up next") get one asked for.
    let menu = video
        .get("menu")
        .and_then(Value::as_str)
        .map_or_else(|| yt_core::actions::menu_params_for(id), str::to_owned);
    if video
        .get("actions")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty())
        || FETCHED.with(|f| f.borrow().contains_key(id))
    {
        return;
    }
    FETCHED.with(|f| {
        let mut fetched = f.borrow_mut();
        if fetched.len() >= MENUS_KEPT {
            fetched.clear();
        }
        fetched.insert(id.to_owned(), None);
    });
    let id = id.to_owned();
    crate::runtime::spawn_local(async move {
        let youtube = app().youtube.clone();
        let result =
            crate::runtime::background(async move { youtube.video_actions(&menu).await }).await;
        match result {
            Ok(actions) => {
                FETCHED.with(|f| f.borrow_mut().insert(id.clone(), Some(actions)));
                done();
                app()
                    .ext
                    .events
                    .emit("video.menu_loaded", &json!({ "video_id": id }));
            }
            Err(e) => {
                tracing::warn!("menu of {id}: {e}");
                FETCHED.with(|f| f.borrow_mut().remove(&id));
            }
        }
    });
}

/// The menu items for a video: `{ label, icon?, command, args?, confirm? }`,
/// in youtube.com's order: the queue and saving, sharing, then what
/// YouTube itself offers for this video (from its tile: "Not interested",
/// "Remove from Watch later", ...); then the `video.menu` filter.
pub fn menu_items(video: &Value) -> Value {
    let signed_in = crate::social::signed_in();
    let mut defaults = vec![
        json!({ "label": "Add to queue", "icon": "yt-queue-symbolic", "command": "queue.add" }),
    ];
    if signed_in {
        defaults.push(json!({ "label": "Save to Watch later", "icon": "yt-watch-later-symbolic", "command": "video.save_to_watch_later" }));
        defaults.push(json!({ "label": "Save to playlist", "icon": "yt-playlist-add-symbolic", "command": "video.save" }));
    }
    let id = video.get("id").and_then(Value::as_str).unwrap_or_default();
    let live = video.get("live").and_then(Value::as_bool).unwrap_or(false);
    if crate::downloads::is_running(id) {
        defaults.push(json!({ "label": "Cancel download", "icon": "yt-close-symbolic", "command": "download.cancel", "args": { "video_id": id } }));
    } else if crate::downloads::get(id).is_some_and(|d| d.complete) {
        defaults.push(json!({ "label": "Delete download", "icon": "yt-delete-symbolic", "command": "download.delete", "args": { "video_id": id } }));
    } else if !live {
        defaults.push(json!({ "label": "Download", "icon": "yt-download-symbolic", "command": "download.start" }));
    }
    defaults.extend([
        json!({ "label": "Copy link", "icon": "yt-link-symbolic", "command": "video.copy_link" }),
        json!({ "label": "Open in browser", "icon": "yt-open-symbolic", "command": "video.open_in_browser" }),
        json!({ "label": "Go to channel", "icon": "yt-person-symbolic", "command": "channel.open", "args": {
            "name": video.get("channel"),
            "video_id": video.get("id"),
        } }),
    ]);
    let youtube = youtube_items(video);
    if !youtube.is_empty() {
        defaults.push(json!({ "separator": true }));
        defaults.extend(youtube);
    }
    let result = app()
        .ext
        .filters
        .apply("video.menu", json!({ "video": video, "items": defaults }));
    let mut items: Vec<Value> = result
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for item in items.iter_mut().filter(|i| !super::menu::is_separator(i)) {
        if item.get("args").is_none_or(Value::is_null) {
            item["args"] = json!({});
        }
        if item["args"].is_object() && item["args"].get("video").is_none() {
            item["args"]["video"] = video.clone();
        }
    }
    Value::Array(items)
}

/// YouTube's own actions for the video, less the ones the menu has anyway
/// (saving).
fn youtube_items(video: &Value) -> Vec<Value> {
    let mut actions: Vec<VideoAction> = video
        .get("actions")
        .and_then(|a| serde_json::from_value(a.clone()).ok())
        .unwrap_or_default();
    if actions.is_empty()
        && let Some(id) = video.get("id").and_then(Value::as_str)
    {
        actions = FETCHED.with(|f| f.borrow().get(id).cloned().flatten().unwrap_or_default());
    }
    actions
        .into_iter()
        .filter_map(|action| {
            let icon = match &action.kind {
                ActionKind::SaveToPlaylist => return None,
                ActionKind::EditPlaylist { actions, .. }
                    if actions.pointer("/0/action").and_then(Value::as_str)
                        == Some("ACTION_ADD_VIDEO") =>
                {
                    return None;
                }
                ActionKind::EditPlaylist { .. } | ActionKind::Unlike => "yt-delete-symbolic",
                // "Remove from watch history".
                ActionKind::Feedback { .. }
                    if action.label.to_lowercase().starts_with("remove") =>
                {
                    "yt-delete-symbolic"
                }
                ActionKind::Feedback { .. } if action.label.to_lowercase().contains("channel") => {
                    "yt-dont-recommend-symbolic"
                }
                ActionKind::Feedback { .. } => "yt-not-interested-symbolic",
            };
            let mut item = json!({
                "label": action.label,
                "icon": icon,
                "command": "video.action",
                "args": { "action": action },
            });
            if let ActionKind::Feedback {
                confirm: Some(confirm),
                ..
            } = &action.kind
            {
                item["confirm"] = json!({
                    "title": confirm.title,
                    "body": confirm.body,
                    "confirm": confirm.button,
                });
            }
            Some(item)
        })
        .collect()
}

fn run(command: &str, args: Value) {
    if let Err(e) = app().ext.commands.run(command, args) {
        app().toast(&e);
    }
}
