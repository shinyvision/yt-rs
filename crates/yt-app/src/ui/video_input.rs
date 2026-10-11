//! What a video does wherever it is shown (feed cards, search results): a
//! click plays it, resting the pointer on it looks its streams up so a click
//! plays at once, and its ⋮ menu has the `video.menu` filter's items.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use serde_json::{Value, json};

use crate::app::app;

const PREFETCH_DELAY: Duration = Duration::from_millis(250);

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

/// Gives `menu` the video's menu: the `video.menu` filter's items, each
/// command run with `{ video }` added to its arguments.
pub fn attach_menu(menu: &gtk::MenuButton, video: impl Fn() -> Option<Value> + 'static) {
    super::menu::attach(menu, move || {
        video().map_or_else(|| json!([]), |v| menu_items(&v))
    });
}

/// The menu items for a video: `{ label, icon?, command, args?, confirm? }`.
pub fn menu_items(video: &Value) -> Value {
    let defaults = json!([
        { "label": "Play", "icon": "yt-play-symbolic", "command": "video.play" },
        { "label": "Go to channel", "icon": "yt-person-symbolic", "command": "channel.open", "args": {
            "name": video.get("channel"),
            "video_id": video.get("id"),
        } },
        { "label": "Copy link", "icon": "yt-link-symbolic", "command": "video.copy_link" },
        { "label": "Open in browser", "icon": "yt-open-symbolic", "command": "video.open_in_browser" },
    ]);
    let result = app()
        .ext
        .filters
        .apply("video.menu", json!({ "video": video, "items": defaults }));
    let mut items: Vec<Value> = result
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for item in &mut items {
        if item.get("args").is_none_or(Value::is_null) {
            item["args"] = json!({});
        }
        if item["args"].is_object() && item["args"].get("video").is_none() {
            item["args"]["video"] = video.clone();
        }
    }
    Value::Array(items)
}

fn run(command: &str, args: Value) {
    if let Err(e) = app().ext.commands.run(command, args) {
        app().toast(&e);
    }
}
