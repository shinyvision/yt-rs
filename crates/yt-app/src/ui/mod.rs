//! The built-in UI. Every piece is a component in the registry, and the
//! window is assembled from the registry by name, so plugins can replace or
//! wrap any part of it.

pub mod account;
pub mod aspect;
pub mod avatar;
pub mod bezel;
pub mod card;
pub mod channel;
pub mod comments;
pub mod controls;
pub mod cursor;
pub mod dialog;
pub mod feed;
pub mod icons;
pub mod logo;
pub mod menu;
pub mod mini_player;
pub mod player_dock;
pub mod player_settings;
pub mod player_view;
pub mod preview;
pub mod richtext;
pub mod search;
pub mod search_box;
pub mod search_filters;
pub mod search_results;
pub mod seekbar;
pub mod shell;
pub mod sidebar;
pub mod signin;
pub mod split;
pub mod subscribe;
pub mod topbar;
pub mod video;
pub mod video_input;
pub mod watch;

use serde_json::{Value, json};

use crate::app::{App, app, try_app};
use crate::ext::components::Component;
use crate::ext::pages::{NavItem, Page};

pub fn register(app: &App) {
    let c = &app.ext.components;
    c.define("shell", Component::new(shell::build));
    c.define("sidebar", Component::new(sidebar::build));
    c.define(
        "sidebar_subscriptions",
        Component::new(sidebar::build_subscriptions),
    );
    c.define("topbar", Component::new(topbar::build));
    c.define("account_button", Component::new(account::build));
    c.define(
        "feed",
        Component::new(feed::build).with_update(feed::update),
    );
    c.define("chip_bar", Component::new(feed::build_chip_bar));
    c.define(
        "video_card",
        Component::new(card::build).with_update(card::update),
    );
    c.define(
        "watch",
        Component::new(watch::build).with_update(watch::update),
    );
    c.define("player", Component::new(player_view::build));
    c.define("mini_player", Component::new(mini_player::build));
    c.define("player_controls", Component::new(controls::build));
    c.define("player_settings", Component::new(player_settings::build));
    c.define("signin", Component::new(signin::build));
    c.define(
        "channel",
        Component::new(channel::build).with_update(channel::update),
    );
    c.define(
        "comments",
        Component::new(comments::build).with_update(comments::update),
    );
    c.define("comment", Component::new(comments::build_comment));
    c.define("comment_composer", Component::new(comments::build_composer));
    c.define("subscribe_button", Component::new(subscribe::build));
    c.define("menu", Component::new(menu::build));
    c.define("confirm_dialog", Component::new(dialog::build));
    c.define("search_box", Component::new(search_box::build));
    c.define(
        "search",
        Component::new(search::build).with_update(search::update),
    );
    c.define("search_filters", Component::new(search_filters::build));
    c.define(
        "search_video",
        Component::new(search_results::build_video).with_update(search_results::update_video),
    );
    c.define(
        "search_channel",
        Component::new(search_results::build_channel).with_update(search_results::update_channel),
    );
    c.define(
        "search_playlist",
        Component::new(search_results::build_playlist).with_update(search_results::update_playlist),
    );
    c.define("search_shelf", Component::new(search_results::build_shelf));
    c.define(
        "search_correction",
        Component::new(search_results::build_correction),
    );
    c.define(
        "search_footer",
        Component::new(search_results::build_footer),
    );

    let pages = &app.ext.pages;
    let feed_page = |name: &str, title: &str, args: Value| Page {
        name: name.into(),
        title: title.into(),
        component: Some("feed".into()),
        args,
        keep: true,
    };
    pages.register(feed_page(
        "home",
        "Home",
        json!({ "browse_id": yt_core::innertube::HOME }),
    ));
    pages.register(feed_page(
        "subscriptions",
        "Subscriptions",
        json!({ "browse_id": "FEsubscriptions", "untitled": "Latest" }),
    ));
    pages.register(feed_page(
        "history",
        "History",
        json!({ "browse_id": "FEhistory", "untitled": "History", "refresh_on_show": true }),
    ));
    pages.register(Page {
        name: "watch".into(),
        title: "Watch".into(),
        component: None,
        args: Value::Null,
        keep: true,
    });
    pages.register(Page {
        name: "channel".into(),
        title: "Channel".into(),
        component: Some("channel".into()),
        args: Value::Null,
        keep: true,
    });
    pages.register(Page {
        name: "search".into(),
        title: "Search".into(),
        component: Some("search".into()),
        args: Value::Null,
        keep: true,
    });
    pages.register(Page {
        name: "signin".into(),
        title: "Sign in".into(),
        component: None,
        args: Value::Null,
        keep: false,
    });

    let nav = |id: &str, title: &str, icon: &str, order: i32, section: &str| NavItem {
        id: id.into(),
        title: title.into(),
        icon: icon.into(),
        page: id.into(),
        args: Value::Null,
        order,
        section: section.into(),
    };
    pages.add_nav(nav("home", "Home", "go-home-symbolic", 0, "main"));
    pages.add_nav(nav(
        "subscriptions",
        "Subscriptions",
        "view-list-symbolic",
        10,
        "main",
    ));
    pages.add_nav(nav(
        "history",
        "History",
        "document-open-recent-symbolic",
        0,
        "library",
    ));
}

/// Calls `handler` on every `event` for as long as `widget` exists.
///
/// Prefer this to `events.on` for widgets that come and go (cards,
/// comments), so their handlers go with them.
pub fn follow(
    widget: &impl gtk::prelude::IsA<gtk::Widget>,
    event: &str,
    handler: impl Fn(&Value) + 'static,
) {
    use gtk::prelude::*;
    let id = app().ext.events.on(event, handler);
    widget.as_ref().connect_destroy(move |_| {
        if let Some(app) = try_app() {
            app.ext.events.off(id);
        }
    });
}

/// Ties `value` to the lifetime of `object`: it is dropped when the object
/// is. Used to let a widget own its controller.
pub fn attach<T: 'static>(
    object: &impl gtk::prelude::IsA<gtk::glib::Object>,
    value: std::rc::Rc<T>,
) {
    use gtk::prelude::*;
    // SAFETY: the key is derived from `T`, so it only ever holds an `Rc<T>`.
    unsafe { object.set_data(std::any::type_name::<T>(), value) };
}

/// The value attached to `object` with [`attach`], if any.
pub fn attached<T: 'static>(
    object: &impl gtk::prelude::IsA<gtk::glib::Object>,
) -> Option<std::rc::Rc<T>> {
    use gtk::prelude::*;
    // SAFETY: see `attach`.
    unsafe {
        object
            .data::<std::rc::Rc<T>>(std::any::type_name::<T>())
            .map(|p| p.as_ref().clone())
    }
}

/// The first widget in `root`'s tree (including `root`) with this widget
/// name.
pub fn find_widget(root: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    use gtk::prelude::*;
    if root.widget_name() == name {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(w) = child {
        if let Some(found) = find_widget(&w, name) {
            return Some(found);
        }
        child = w.next_sibling();
    }
    None
}

/// Formats seconds as "1:02:03" or "2:03".
pub fn format_time(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    let (h, m, s) = (total / 3600, total / 60 % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}
