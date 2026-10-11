//! The `channels` component (the "All subscriptions" page, youtube.com's
//! feed/channels): every channel the user subscribes to, with its picture,
//! name, handle and subscribers and a subscribe button, as YouTube orders
//! them or A to Z. The sidebar's "Subscriptions ›" opens it. Signed out,
//! youtube.com's prompt to sign in.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use serde_json::{Value, json};
use yt_core::channel::ChannelRef;

use super::{attach, avatar, follow};
use crate::app::app;

/// How wide the list gets, as youtube.com's.
const WIDTH: i32 = 1096;

struct Page {
    list: gtk::Box,
    status: adw::StatusPage,
    /// A to Z, rather than as YouTube orders them.
    by_name: Cell<bool>,
}

pub fn build(_props: &Value) -> gtk::Widget {
    if !crate::social::signed_in() {
        let prompt = super::signin::prompt("subscriptions");
        prompt.set_widget_name("page-channels");
        return prompt;
    }
    let title = gtk::Label::new(Some("All subscriptions"));
    title.add_css_class("page-title");
    title.set_xalign(0.0);
    title.set_hexpand(true);
    let sort = gtk::DropDown::from_strings(&["Most relevant", "A\u{2013}Z"]);
    sort.add_css_class("channels-sort");
    sort.set_valign(gtk::Align::Center);
    sort.set_tooltip_text(Some("Sort"));
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    header.append(&title);
    header.append(&sort);

    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.add_css_class("channels-list");
    let status = adw::StatusPage::builder()
        .icon_name("view-list-symbolic")
        .title("No subscriptions yet")
        .description("Channels you subscribe to show here.")
        .visible(false)
        .build();
    let column = gtk::Box::new(gtk::Orientation::Vertical, 16);
    column.add_css_class("channels-page");
    column.append(&header);
    column.append(&list);
    column.append(&status);
    let clamp = adw::Clamp::builder()
        .child(&column)
        .maximum_size(WIDTH)
        .tightening_threshold(WIDTH)
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .child(&clamp)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    scroller.set_widget_name("page-channels");

    let page = Rc::new(Page {
        list,
        status,
        by_name: Cell::new(false),
    });
    attach(&scroller, page.clone());
    let weak = Rc::downgrade(&page);
    sort.connect_selected_notify(move |sort| {
        if let Some(page) = weak.upgrade() {
            page.by_name.set(sort.selected() == 1);
            fill(&page);
        }
    });
    // The list arriving. A channel unsubscribed from stays, with its
    // button saying "Subscribe", as on youtube.com.
    let weak = Rc::downgrade(&page);
    follow(&scroller, "channels.updated", move |_| {
        if let Some(page) = weak.upgrade()
            && page.list.first_child().is_none()
        {
            fill(&page);
        }
    });
    fill(&page);
    scroller.upcast()
}

fn fill(page: &Page) {
    while let Some(row) = page.list.first_child() {
        page.list.remove(&row);
    }
    let mut channels = app().channels.subscriptions();
    if page.by_name.get() {
        channels.sort_by_key(|c| c.name.to_lowercase());
    }
    page.status.set_visible(channels.is_empty());
    for channel in &channels {
        page.list.append(&row(channel));
    }
}

/// A channel: its picture and name open it; the button subscribes.
fn row(channel: &ChannelRef) -> gtk::Widget {
    let picture = avatar::new(136, &channel.name, channel.avatar.as_deref());
    let name = gtk::Label::new(Some(&channel.name));
    name.add_css_class("channels-name");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let title = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    title.append(&name);
    if channel.verified {
        let check = gtk::Image::from_icon_name("yt-verified-symbolic");
        check.add_css_class("dim");
        check.set_tooltip_text(Some("Verified"));
        title.append(&check);
    }
    let facts: Vec<&str> = [channel.handle.as_deref(), channel.subscribers.as_deref()]
        .into_iter()
        .flatten()
        .collect();
    let meta = gtk::Label::new(Some(&facts.join(" \u{2022} ")));
    meta.add_css_class("dim");
    meta.add_css_class("channels-meta");
    meta.set_xalign(0.0);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 4);
    text.set_valign(gtk::Align::Center);
    text.set_hexpand(true);
    text.append(&title);
    text.append(&meta);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 24);
    row.add_css_class("channels-row");
    row.set_widget_name(&format!(
        "channels:{}",
        channel.id.as_deref().unwrap_or_default()
    ));
    row.append(&picture);
    row.append(&text);
    if let Some(id) = &channel.id {
        let subscription = json!({ "channel_id": id, "subscribed": true });
        let button = app().ext.components.build(
            "subscribe_button",
            &json!({ "subscription": subscription, "channel_name": channel.name }),
        );
        button.set_valign(gtk::Align::Center);
        row.append(&button);
    }
    for target in [picture.upcast_ref::<gtk::Widget>(), text.upcast_ref()] {
        open_on_click(target, channel.id.clone());
    }
    row.upcast()
}

/// Clicking `widget` opens the channel.
fn open_on_click(widget: &gtk::Widget, channel_id: Option<String>) {
    widget.set_cursor_from_name(Some("pointer"));
    let click = gtk::GestureClick::new();
    click.connect_released(move |gesture, n, _, _| {
        if n == 1 {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            if let Err(e) = app()
                .ext
                .commands
                .run("channel.open", json!({ "channel_id": channel_id }))
            {
                app().toast(&e);
            }
        }
    });
    widget.add_controller(click);
}
