//! The window layout, as on youtube.com: the top bar across the top, and
//! below it the sidebar (YouTube's "guide") beside the pages.
//!
//! On a wide window the full sidebar is docked, and ☰ switches it to a
//! rail of icons. Narrower than 1313 px the rail stays and ☰ opens the full
//! sidebar as a drawer over the whole window (top bar included, with ☰ and
//! the logo at its top) and dims the rest; narrower than 792 px there is no
//! rail; and the watch page has only the drawer, at any width, so the
//! player gets the room. Going to another page, a click beside the drawer
//! and Escape close it.

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
    /// The drawer, over everything.
    /// The shell itself, weakly: it owns this.
    drawer: gtk::glib::WeakRef<adw::OverlaySplitView>,
    /// Dims the window while the drawer is open; a click closes it.
    scrim: gtk::Box,
    /// The full sidebar, docked beside the pages.
    guide: gtk::Widget,
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

impl Shell {
    /// The shell's own widget (gone only as the window closes).
    fn drawer(&self) -> Option<adw::OverlaySplitView> {
        self.drawer.upgrade()
    }
}

pub fn build(_props: &Value) -> gtk::Widget {
    let app = app();
    let topbar = app.ext.components.build("topbar", &Value::Null);
    let guide = app.ext.components.build("sidebar", &Value::Null);
    let rail = app
        .ext
        .components
        .build("sidebar", &json!({ "mini": true }));

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.set_widget_name("content");
    row.set_vexpand(true);
    row.append(&guide);
    row.append(&rail);
    row.append(&app.page_host());
    // The mini player floats over the pages, in the bottom-right corner.
    let below = gtk::Overlay::new();
    below.set_child(Some(&row));
    below.add_overlay(&app.ext.components.build("mini_player", &Value::Null));
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&topbar);
    content.append(&below);

    let scrim = gtk::Box::new(gtk::Orientation::Vertical, 0);
    scrim.add_css_class("guide-scrim");
    scrim.set_visible(false);
    let close = gtk::GestureClick::new();
    close.connect_pressed(|gesture, _, _, _| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        close_drawer();
    });
    scrim.add_controller(close);
    let dimmed = gtk::Overlay::new();
    dimmed.set_child(Some(&content));
    dimmed.add_overlay(&scrim);

    let header = super::topbar::menu_and_logo();
    header.add_css_class("drawer-header");
    let drawer_guide = app.ext.components.build("sidebar", &Value::Null);
    drawer_guide.set_widget_name("sidebar-drawer");
    drawer_guide.set_vexpand(true);
    let drawer_content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    drawer_content.add_css_class("guide-drawer");
    drawer_content.append(&header);
    drawer_content.append(&drawer_guide);

    let drawer = adw::OverlaySplitView::builder()
        .sidebar(&drawer_content)
        .content(&dimmed)
        .collapsed(true)
        .show_sidebar(false)
        .min_sidebar_width(SIDEBAR_WIDTH)
        .max_sidebar_width(SIDEBAR_WIDTH)
        .build();
    drawer.add_css_class("guide");
    drawer.set_widget_name("shell");

    let shell = Rc::new(Shell {
        drawer: drawer.downgrade(),
        scrim,
        guide,
        rail,
        topbar,
        wide: Cell::new(true),
        narrow: Cell::new(false),
        watch: Cell::new(false),
        full: Cell::new(true),
        fullscreen: Cell::new(false),
    });
    attach(&drawer, shell.clone());
    layout(&shell);
    let weak = Rc::downgrade(&shell);
    drawer.connect_show_sidebar_notify(move |_| {
        if let Some(shell) = weak.upgrade() {
            shell
                .scrim
                .set_visible(shell.drawer().is_some_and(|d| d.shows_sidebar()));
        }
    });

    // The window's width decides how the sidebar shows.
    let weak = Rc::downgrade(&shell);
    drawer.connect_realize(move |drawer| {
        let (Some(shell), Some(window)) = (
            weak.upgrade(),
            drawer.root().and_downcast::<adw::ApplicationWindow>(),
        ) else {
            return;
        };
        // Only one breakpoint applies at a time (the narrowest that
        // matches), and leaving one applies the next, if it matches: so
        // each sets the whole state, and leaving one sets a wide window's.
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
                    shell.wide.set(false);
                    shell.narrow.set(narrow);
                    layout(&shell);
                }
            });
            breakpoint.connect_unapply(move |_| {
                if let Some(shell) = off.upgrade() {
                    shell.wide.set(true);
                    shell.narrow.set(false);
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
        if let Some(drawer) = shell.drawer() {
            drawer.set_show_sidebar(false);
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
        if let Some(drawer) = shell.drawer() {
            drawer.set_show_sidebar(false);
        }
        layout(&shell);
    });
    drawer.upcast()
}

/// ☰: switches a docked sidebar between full and icons, or opens and
/// closes the drawer.
pub fn toggle_sidebar() {
    let Some(shell) = find() else { return };
    if docked(&shell) {
        shell.full.set(!shell.full.get());
        layout(&shell);
    } else {
        if let Some(drawer) = shell.drawer() {
            drawer.set_show_sidebar(!shell.drawer().is_some_and(|d| d.shows_sidebar()));
        }
    }
    announce(&shell);
}

/// Closes the drawer; false if it wasn't open.
pub fn close_drawer() -> bool {
    let Some(shell) = find() else { return false };
    let open = shell.drawer().is_some_and(|d| d.shows_sidebar());
    if open {
        if let Some(drawer) = shell.drawer() {
            drawer.set_show_sidebar(false);
        }
        announce(&shell);
    }
    open
}

fn announce(shell: &Shell) {
    app().ext.events.emit(
        "sidebar.toggled",
        &json!({
            "mini": docked(shell) && !shell.full.get(),
            "drawer": shell.drawer().is_some_and(|d| d.shows_sidebar()),
        }),
    );
}

fn find() -> Option<Rc<Shell>> {
    let window = app().window()?;
    let root = super::find_widget(window.upcast_ref(), "shell")?;
    attached::<Shell>(&root)
}

/// Whether the sidebar is docked beside the pages (in full or as icons).
fn docked(shell: &Shell) -> bool {
    shell.wide.get() && !shell.watch.get() && !shell.fullscreen.get()
}

fn layout(shell: &Shell) {
    let fullscreen = shell.fullscreen.get();
    let docked = docked(shell);
    if docked {
        if let Some(drawer) = shell.drawer() {
            drawer.set_show_sidebar(false);
        }
    }
    shell.topbar.set_visible(!fullscreen);
    let full_docked = docked && shell.full.get();
    shell.guide.set_visible(full_docked);
    shell
        .rail
        .set_visible(!full_docked && !shell.watch.get() && !shell.narrow.get() && !fullscreen);
    let size = match (shell.wide.get(), shell.narrow.get()) {
        (true, _) => "wide",
        (false, false) => "medium",
        (false, true) => "narrow",
    };
    app().ext.events.emit(
        "shell.layout",
        &json!({ "size": size, "docked": docked, "mini": docked && !shell.full.get() }),
    );
}
