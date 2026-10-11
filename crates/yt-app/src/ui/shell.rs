//! The window layout, as on youtube.com: the top bar across the top, and
//! below it the sidebar (YouTube's "guide") beside the pages.
//!
//! On a wide window the full sidebar is docked, and ☰ switches it to a
//! rail of icons. Narrower than 1313 px the rail stays and ☰ opens the full
//! sidebar over the page, as a drawer; narrower than 792 px there is no
//! rail; and the watch page has only the drawer, at any width, so the
//! player gets the room. Going to another page closes the drawer.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use serde_json::{Value, json};

use super::{attach, attached};
use crate::app::app;

/// Narrower than this, the sidebar is a rail of icons (and a drawer).
const WIDE: f64 = 1313.0;
/// Narrower than this, there is no rail either.
const NARROW: f64 = 792.0;
const SIDEBAR_WIDTH: f64 = 240.0;

struct Shell {
    split: adw::OverlaySplitView,
    /// Dims the page while the drawer is open; a click closes it.
    scrim: gtk::Box,
    rail: gtk::Widget,
    topbar: gtk::Widget,
    wide: Cell<bool>,
    narrow: Cell<bool>,
    /// On the watch page.
    watch: Cell<bool>,
    /// Docked in full (rather than as a rail) on a wide window: the user's
    /// choice with ☰.
    full: Cell<bool>,
    fullscreen: Cell<bool>,
}

pub fn build(_props: &Value) -> gtk::Widget {
    let app = app();
    let topbar = app.ext.components.build("topbar", &Value::Null);
    let sidebar = app.ext.components.build("sidebar", &Value::Null);
    let rail = app
        .ext
        .components
        .build("sidebar", &json!({ "mini": true }));

    let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    content.set_widget_name("content");
    content.append(&rail);
    content.append(&app.page_host());
    let scrim = gtk::Box::new(gtk::Orientation::Vertical, 0);
    scrim.add_css_class("guide-scrim");
    scrim.set_visible(false);
    let close = gtk::GestureClick::new();
    close.connect_pressed(|gesture, _, _, _| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        toggle_sidebar();
    });
    scrim.add_controller(close);
    let dimmed = gtk::Overlay::new();
    dimmed.set_child(Some(&content));
    dimmed.add_overlay(&scrim);

    let split = adw::OverlaySplitView::builder()
        .sidebar(&sidebar)
        .content(&dimmed)
        .min_sidebar_width(SIDEBAR_WIDTH)
        .max_sidebar_width(SIDEBAR_WIDTH)
        .vexpand(true)
        .build();
    split.add_css_class("guide");

    // The mini player floats over the pages, in the bottom-right corner.
    let below = gtk::Overlay::new();
    below.set_child(Some(&split));
    below.add_overlay(&app.ext.components.build("mini_player", &Value::Null));
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.set_widget_name("shell");
    root.append(&topbar);
    root.append(&below);

    let shell = Rc::new(Shell {
        split: split.clone(),
        scrim: scrim.clone(),
        rail,
        topbar,
        wide: Cell::new(true),
        narrow: Cell::new(false),
        watch: Cell::new(false),
        full: Cell::new(true),
        fullscreen: Cell::new(false),
    });
    attach(&root, shell.clone());
    layout(&shell);
    let weak = Rc::downgrade(&shell);
    split.connect_show_sidebar_notify(move |_| {
        if let Some(shell) = weak.upgrade() {
            show_scrim(&shell);
        }
    });

    // The window's width decides how the sidebar shows.
    let weak = Rc::downgrade(&shell);
    split.connect_realize(move |split| {
        let (Some(shell), Some(window)) = (
            weak.upgrade(),
            split.root().and_downcast::<adw::ApplicationWindow>(),
        ) else {
            return;
        };
        for (width, narrow) in [(WIDE, false), (NARROW, true)] {
            let condition = adw::BreakpointCondition::new_length(
                adw::BreakpointConditionLengthType::MaxWidth,
                width - 1.0,
                adw::LengthUnit::Px,
            );
            let breakpoint = adw::Breakpoint::new(condition);
            let (on, off) = (Rc::downgrade(&shell), Rc::downgrade(&shell));
            breakpoint.connect_apply(move |_| {
                if let Some(shell) = on.upgrade() {
                    if narrow {
                        shell.narrow.set(true);
                    } else {
                        shell.wide.set(false);
                    }
                    layout(&shell);
                }
            });
            breakpoint.connect_unapply(move |_| {
                if let Some(shell) = off.upgrade() {
                    if narrow {
                        shell.narrow.set(false);
                    } else {
                        shell.wide.set(true);
                    }
                    layout(&shell);
                }
            });
            window.add_breakpoint(breakpoint);
        }
    });
    let weak = Rc::downgrade(&shell);
    app.ext.events.on("page.changed", move |payload| {
        let Some(shell) = weak.upgrade() else { return };
        shell
            .watch
            .set(payload.get("page").and_then(Value::as_str) == Some("watch"));
        if shell.split.is_collapsed() {
            shell.split.set_show_sidebar(false);
        }
        layout(&shell);
    });
    // In fullscreen only the page (the video) is shown.
    let weak = Rc::downgrade(&shell);
    app.ext.events.on("window.fullscreen", move |payload| {
        let Some(shell) = weak.upgrade() else { return };
        shell.fullscreen.set(
            payload
                .get("fullscreen")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        );
        layout(&shell);
    });
    root.upcast()
}

/// ☰: switches a docked sidebar between full and icons, or opens and
/// closes the drawer.
pub fn toggle_sidebar() {
    let Some(shell) = find() else { return };
    if shell.split.is_collapsed() {
        shell.split.set_show_sidebar(!shell.split.shows_sidebar());
    } else {
        shell.full.set(!shell.full.get());
        layout(&shell);
    }
    let docked = !shell.split.is_collapsed();
    app().ext.events.emit(
        "sidebar.toggled",
        &json!({
            "mini": docked && !shell.full.get(),
            "drawer": shell.split.is_collapsed() && shell.split.shows_sidebar(),
        }),
    );
}

fn find() -> Option<Rc<Shell>> {
    let window = app().window()?;
    let root = super::find_widget(window.upcast_ref(), "shell")?;
    attached::<Shell>(&root)
}

fn show_scrim(shell: &Shell) {
    shell
        .scrim
        .set_visible(shell.split.is_collapsed() && shell.split.shows_sidebar());
}

fn layout(shell: &Shell) {
    let fullscreen = shell.fullscreen.get();
    let docked = shell.wide.get() && !shell.watch.get() && !fullscreen;
    shell.topbar.set_visible(!fullscreen);
    shell.split.set_collapsed(!docked);
    if docked {
        shell.split.set_show_sidebar(shell.full.get());
    } else if fullscreen {
        shell.split.set_show_sidebar(false);
    }
    let full_docked = docked && shell.full.get();
    shell
        .rail
        .set_visible(!full_docked && !shell.watch.get() && !shell.narrow.get() && !fullscreen);
    show_scrim(shell);
}
