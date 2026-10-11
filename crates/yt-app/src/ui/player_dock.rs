//! Where the video shows. mpv draws into one GL area, so there is one
//! player widget (the `player` component), and it moves between places
//! that register here: the watch page's frame (`watch`), the full-width
//! row above it in theater mode and fullscreen (`theater`), and the mini
//! player (`mini`) while a video plays on another page. Moving it is cheap:
//! its GL area builds a new renderer where it lands.
//!
//! [`update`] puts it in the right place after anything that changes that:
//! the page, the watch page's layout, the player starting or stopping.
//! Each move is announced as `player.docked` with `{ slot }` (`null` when
//! it shows nowhere).

use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;

use gtk::glib;
use gtk::prelude::*;
use serde_json::{Value, json};

use super::aspect::AspectBin;
use crate::app::app;

#[derive(Default)]
struct Dock {
    player: OnceCell<gtk::Widget>,
    slots: RefCell<HashMap<String, glib::WeakRef<AspectBin>>>,
    /// Where the watch page wants the player: `watch` or `theater`.
    watch_slot: RefCell<String>,
    current: RefCell<Option<String>>,
}

thread_local! {
    static DOCK: Dock = Dock {
        watch_slot: RefCell::new("watch".into()),
        ..Dock::default()
    };
}

/// Moves the player after page changes and when a video starts or stops.
pub fn init() {
    let app = app();
    app.ext.events.on("page.changed", |_| update());
    app.player.connect(|event, _| {
        if matches!(
            event,
            crate::player::PlayerEvent::Loading { .. } | crate::player::PlayerEvent::Stopped
        ) {
            // After the player has updated its state.
            glib::idle_add_local_once(update);
        }
    });
}

/// Frees the video's renderer, which must go before mpv does.
pub fn shut_down() {
    let player = DOCK.with(|d| d.player.get().cloned());
    if let Some(video) = player.as_ref().and_then(|p| super::find_widget(p, "video")) {
        if let Some(video) = video.downcast_ref::<super::video::VideoView>() {
            video.stop();
        }
    }
}

/// The player widget, built the first time it's needed.
pub fn player() -> gtk::Widget {
    DOCK.with(|d| {
        d.player
            .get_or_init(|| app().ext.components.build("player", &Value::Null))
            .clone()
    })
}

/// Makes `bin` the place called `name`.
pub fn register(name: &str, bin: &AspectBin) {
    DOCK.with(|d| {
        d.slots
            .borrow_mut()
            .insert(name.to_owned(), bin.downgrade())
    });
}

/// Where the watch page shows the player, in its current layout.
pub fn set_watch_slot(name: &str) {
    DOCK.with(|d| *d.watch_slot.borrow_mut() = name.to_owned());
    update();
}

/// Where the player is: `watch`, `theater`, `mini` or `None`.
pub fn current() -> Option<String> {
    DOCK.with(|d| d.current.borrow().clone())
}

/// Puts the player where it belongs now: on the watch page when that is
/// shown, in the mini player while a video plays elsewhere, nowhere else.
pub fn update() {
    let app = app();
    let target = if app.current_page().as_deref() == Some("watch") {
        Some(DOCK.with(|d| d.watch_slot.borrow().clone()))
    } else if app.player.state().video_id.is_some() {
        Some("mini".to_owned())
    } else {
        None
    };
    place(target.as_deref());
}

fn place(target: Option<&str>) {
    let (moved, bin) = DOCK.with(|d| {
        let bin = target.and_then(|t| d.slots.borrow().get(t).and_then(glib::WeakRef::upgrade));
        // A place that isn't on screen (yet) can't take it.
        let target = bin.as_ref().and(target);
        let moved = d.current.borrow().as_deref() != target;
        *d.current.borrow_mut() = target.map(str::to_owned);
        (moved, bin)
    });
    if !moved {
        return;
    }
    let player = player();
    if let Some(old) = player.parent().and_downcast::<AspectBin>() {
        old.set_child(None::<&gtk::Widget>);
    } else if player.parent().is_some() {
        player.unparent();
    }
    if let Some(bin) = &bin {
        bin.set_child(Some(&player));
    }
    app()
        .ext
        .events
        .emit("player.docked", &json!({ "slot": current() }));
}
