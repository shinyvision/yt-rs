//! The sidebar: the registered navigation entries, then the channels the
//! user subscribes to (`sidebar_subscriptions`); with `{ mini = true }`,
//! the rail of icons instead. Where each shows is the shell's business.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use serde_json::{Value, json};
use yt_core::channel::ChannelRef;

use super::{attach, avatar, follow};
use crate::app::app;
use crate::ext::pages::NavItem;

const SUBSCRIPTIONS: &str = "sidebar-subscriptions";
/// Channels shown before "Show more".
const FOLDED: usize = 7;

pub fn build(props: &Value) -> gtk::Widget {
    let app = app();
    let mini = props.get("mini").and_then(Value::as_bool).unwrap_or(false);
    let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 2);
    sidebar.set_widget_name(if mini { "sidebar-mini" } else { "sidebar" });
    sidebar.add_css_class("sidebar");
    if mini {
        sidebar.add_css_class("mini");
    }

    let items = gtk::Box::new(gtk::Orientation::Vertical, 2);
    sidebar.append(
        &app.ext
            .slots
            .create("sidebar.top", gtk::Orientation::Vertical, 2),
    );
    let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
    column.append(&items);
    if !mini {
        let subscriptions = app
            .ext
            .components
            .build("sidebar_subscriptions", &Value::Null);
        subscriptions.set_widget_name(SUBSCRIPTIONS);
        column.append(&subscriptions);
    }
    // Scrolls, so a long subscriptions list can't make the window taller.
    let scroller = gtk::ScrolledWindow::builder()
        .child(&column)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    sidebar.append(&scroller);
    sidebar.append(
        &app.ext
            .slots
            .create("sidebar.bottom", gtk::Orientation::Vertical, 2),
    );

    fill(&items, mini);
    let weak = items.downgrade();
    app.ext.pages.on_nav_change(move || {
        if let Some(items) = weak.upgrade() {
            fill(&items, mini);
        }
    });
    for event in ["auth.signed_in", "auth.signed_out"] {
        let weak = items.downgrade();
        app.ext.events.on(event, move |_| {
            if let Some(items) = weak.upgrade() {
                fill(&items, mini);
            }
        });
    }
    let weak = items.downgrade();
    app.ext.events.on("page.changed", move |payload| {
        let page = payload
            .get("page")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if let Some(items) = weak.upgrade() {
            highlight(&items, page, payload.get("args").unwrap_or(&Value::Null));
        }
    });
    sidebar.upcast()
}

fn fill(items: &gtk::Box, mini: bool) {
    items.set_widget_name("sidebar-items");
    while let Some(child) = items.first_child() {
        items.remove(&child);
    }
    // Signed out, what needs an account goes, and youtube.com's sign-in
    // box comes after "You".
    let signed_in = crate::social::signed_in();
    let nav: Vec<NavItem> = app()
        .ext
        .pages
        .nav()
        .into_iter()
        .filter(|i| signed_in || !i.needs_account)
        .collect();
    // The rail is youtube.com's: the main pages, then "You".
    if mini {
        for item in nav.iter().filter(|i| i.section == "main") {
            items.append(&nav_button(item, mini));
        }
        items.append(&nav_button(&you(), mini));
    } else {
        let mut section = String::new();
        for item in &nav {
            if item.section != section {
                if !section.is_empty() {
                    items.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
                }
                if section == "library" && !signed_in {
                    items.append(&sign_in_box());
                    items.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
                }
                if item.section == "library" {
                    items.append(&you_heading());
                }
                section = item.section.clone();
            }
            items.append(&nav_button(item, mini));
        }
        if section == "library" && !signed_in {
            items.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
            items.append(&sign_in_box());
        }
    }
    if let Some(page) = app().current_page() {
        highlight(items, &page, &app().current_args().unwrap_or_default());
    }
}

/// "Sign in to like videos, comment, and subscribe.", as youtube.com's
/// sidebar has it signed out.
fn sign_in_box() -> gtk::Widget {
    let text = gtk::Label::new(Some("Sign in to like videos, comment, and subscribe."));
    text.set_wrap(true);
    text.set_xalign(0.0);
    text.set_max_width_chars(24);
    let button = super::signin::button();
    button.set_halign(gtk::Align::Start);
    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    column.add_css_class("sidebar-signin");
    column.set_widget_name("sidebar-signin");
    column.append(&text);
    column.append(&button);
    column.upcast()
}

/// The "You" page's entry.
fn you() -> NavItem {
    NavItem {
        id: "library".into(),
        title: "You".into(),
        icon: "yt-library-symbolic".into(),
        page: "library".into(),
        args: Value::Null,
        order: 0,
        section: "library".into(),
        needs_account: false,
    }
}

/// "You ›", which opens the "You" page, over the user's own pages.
fn you_heading() -> gtk::Button {
    section_link("You", "nav:library", "library")
}

/// A section's title that opens its page: "You ›", "Subscriptions ›".
fn section_link(title: &str, name: &str, page: &'static str) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let label = gtk::Label::new(Some(title));
    label.add_css_class("section-title");
    content.append(&label);
    content.append(&gtk::Image::from_icon_name("yt-chevron-right-symbolic"));
    let button = gtk::Button::builder()
        .child(&content)
        .css_classes(["nav-item", "section-link"])
        .build();
    button.set_widget_name(name);
    button.connect_clicked(move |_| app().navigate(page, Value::Null, true));
    button
}

fn nav_button(item: &NavItem, mini: bool) -> gtk::Button {
    let icon = gtk::Image::from_icon_name(if item.icon.is_empty() {
        "application-x-executable-symbolic"
    } else {
        &item.icon
    });
    icon.set_pixel_size(22);
    let label = gtk::Label::new(Some(&item.title));
    label.add_css_class("nav-title");
    label.set_xalign(0.0);
    let content = gtk::Box::new(
        if mini {
            gtk::Orientation::Vertical
        } else {
            gtk::Orientation::Horizontal
        },
        0,
    );
    content.append(&icon);
    content.append(&label);
    if mini {
        content.set_halign(gtk::Align::Center);
    }
    let button = gtk::Button::builder()
        .child(&content)
        .css_classes(["nav-item"])
        .build();
    button.set_widget_name(&format!("nav:{}", item.id));
    button.set_tooltip_text(mini.then_some(item.title.as_str()));
    let (page, args) = (item.page.clone(), item.args.clone());
    button.connect_clicked(move |_| {
        if let Err(e) = app()
            .ext
            .commands
            .run("app.navigate", json!({ "page": page, "args": args }))
        {
            app().toast(&e);
        }
    });
    button
}

/// Marks the entry for the page shown: the same page, and (for entries with
/// arguments, as Watch later and Liked videos are both playlists) the same
/// arguments.
fn highlight(items: &gtk::Box, page: &str, args: &Value) {
    let mut nav = app().ext.pages.nav();
    nav.push(you());
    let matches = |item: &NavItem| {
        item.page == page
            && item.args.as_object().is_none_or(|wanted| {
                wanted
                    .iter()
                    .all(|(key, value)| args.get(key) == Some(value))
            })
    };
    let mut child = items.first_child();
    while let Some(w) = child {
        if let Some(id) = w.widget_name().strip_prefix("nav:") {
            let current =
                !w.has_css_class("section-link") && nav.iter().any(|i| i.id == id && matches(i));
            if current {
                w.add_css_class("current");
            } else {
                w.remove_css_class("current");
            }
        }
        child = w.next_sibling();
    }
}

struct Subscriptions {
    list: gtk::Box,
    more: gtk::Button,
    expanded: Cell<bool>,
    channels: RefCell<Vec<ChannelRef>>,
}

/// The `sidebar_subscriptions` component: subscribed channels with their
/// avatars, from the channel directory, kept up to date by
/// `channels.updated`.
pub fn build_subscriptions(_props: &Value) -> gtk::Widget {
    // "Subscriptions ›" opens all of them, as on youtube.com.
    let title = section_link("Subscriptions", "nav:channels", "channels");
    let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let more = gtk::Button::new();
    more.add_css_class("nav-item");
    let root = gtk::Box::new(gtk::Orientation::Vertical, 2);
    root.add_css_class("subscriptions");
    root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    root.append(&title);
    root.append(&list);
    root.append(&more);

    let this = Rc::new(Subscriptions {
        list,
        more: more.clone(),
        expanded: Cell::new(false),
        channels: RefCell::new(app().channels.subscriptions()),
    });
    attach(&root, this.clone());
    show_subscriptions(&this);
    let weak = Rc::downgrade(&this);
    more.connect_clicked(move |_| {
        if let Some(this) = weak.upgrade() {
            this.expanded.set(!this.expanded.get());
            show_subscriptions(&this);
        }
    });
    let weak = Rc::downgrade(&this);
    follow(&root, "channels.updated", move |_| {
        if let Some(this) = weak.upgrade() {
            *this.channels.borrow_mut() = app().channels.subscriptions();
            show_subscriptions(&this);
        }
    });
    let weak = Rc::downgrade(&this);
    follow(&root, "page.changed", move |payload| {
        if let Some(this) = weak.upgrade() {
            highlight_channel(&this.list, payload);
        }
    });
    root.upcast()
}

fn show_subscriptions(this: &Subscriptions) {
    while let Some(child) = this.list.first_child() {
        this.list.remove(&child);
    }
    let channels = this.channels.borrow();
    let shown = if this.expanded.get() {
        channels.len()
    } else {
        channels.len().min(FOLDED)
    };
    for channel in &channels[..shown] {
        this.list.append(&channel_button(channel));
    }
    if let Some(root) = this.list.parent() {
        root.set_visible(!channels.is_empty());
    }
    let hidden = channels.len() - shown;
    this.more.set_visible(channels.len() > FOLDED);
    let (icon, label) = if this.expanded.get() {
        ("yt-collapse-symbolic", "Show fewer".to_owned())
    } else {
        ("yt-expand-symbolic", format!("Show {hidden} more"))
    };
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let image = gtk::Image::from_icon_name(icon);
    image.set_pixel_size(22);
    let text = gtk::Label::new(Some(&label));
    text.add_css_class("nav-title");
    content.append(&image);
    content.append(&text);
    this.more.set_child(Some(&content));
}

fn channel_button(channel: &ChannelRef) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    content.append(&avatar::new(24, &channel.name, channel.avatar.as_deref()));
    let name = gtk::Label::new(Some(&channel.name));
    name.add_css_class("nav-title");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    content.append(&name);
    let button = gtk::Button::builder()
        .child(&content)
        .css_classes(["nav-item", "channel-item"])
        .build();
    button.set_widget_name(&format!(
        "channel:{}",
        channel.id.as_deref().unwrap_or_default()
    ));
    button.set_tooltip_text(Some(&channel.name));
    let id = channel.id.clone();
    button.connect_clicked(move |_| {
        if let Err(e) = app()
            .ext
            .commands
            .run("channel.open", json!({ "channel_id": id }))
        {
            app().toast(&e);
        }
    });
    button
}

fn highlight_channel(list: &gtk::Box, payload: &Value) {
    let on_channel = payload.get("page").and_then(Value::as_str) == Some("channel");
    let id = payload
        .pointer("/args/channel_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut child = list.first_child();
    while let Some(w) = child {
        if on_channel && !id.is_empty() && w.widget_name().strip_prefix("channel:") == Some(id) {
            w.add_css_class("current");
        } else {
            w.remove_css_class("current");
        }
        child = w.next_sibling();
    }
}
