//! The `video_card` component: thumbnail with duration and watched progress,
//! then avatar, title, channel and stats, and a menu. The avatar and the
//! channel name open the channel. With `layout = "compact"` (the watch
//! page's "Up next") it is a row: a small thumbnail, then the text; with
//! `layout = "shelf"` (a row of a channel's home) a narrower card. With
//! `on_channel` (on the channel's own page) it leaves the channel out.
//!
//! Cards are reused as the grid scrolls, so [`update`] refills an existing
//! card. Hovering a card for a moment looks its streams up in the
//! background, so clicking it starts playback right away.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use serde_json::{Value, json};

use super::{attach, attached, avatar, follow};
use crate::app::app;

/// Thumbnail width to decode, in pixels at scale 1 (cards are 320–480 px).
const THUMBNAIL_WIDTH: i32 = 480;
/// A compact card's thumbnail width.
const COMPACT_WIDTH: i32 = 168;
/// A card's width in a shelf's row.
const SHELF_WIDTH: i32 = 246;

struct Card {
    picture: gtk::Picture,
    duration: gtk::Label,
    watched: gtk::ProgressBar,
    avatar: adw::Avatar,
    title: gtk::Label,
    channel: gtk::Label,
    meta: gtk::Label,
    video: RefCell<Value>,
}

pub fn build(props: &Value) -> gtk::Widget {
    let picture = gtk::Picture::new();
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.set_can_shrink(true);

    let duration = gtk::Label::new(None);
    duration.add_css_class("duration");
    duration.set_halign(gtk::Align::End);
    duration.set_valign(gtk::Align::End);
    let watched = gtk::ProgressBar::new();
    watched.add_css_class("watched");
    watched.set_valign(gtk::Align::End);

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&picture));
    overlay.add_overlay(&duration);
    overlay.add_overlay(&watched);
    let thumb = super::aspect::AspectBin::new(9.0 / 16.0, &overlay);
    thumb.add_css_class("thumb");
    thumb.set_overflow(gtk::Overflow::Hidden);

    let avatar = adw::Avatar::new(36, None, true);
    avatar.set_valign(gtk::Align::Start);
    let label = |class: &str, lines: i32| {
        let l = gtk::Label::new(None);
        l.set_xalign(0.0);
        l.set_wrap(true);
        l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        l.set_lines(lines);
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        l.add_css_class(class);
        l
    };
    let title = label("title", 2);
    let channel = label("meta", 1);
    channel.add_css_class("dim");
    let meta = label("meta", 1);
    meta.add_css_class("dim");
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    text.append(&title);
    text.append(&channel);
    text.append(&meta);

    let menu = gtk::MenuButton::new();
    menu.set_icon_name("view-more-symbolic");
    menu.add_css_class("menu-button");
    menu.set_valign(gtk::Align::Start);
    menu.set_tooltip_text(Some("More"));

    for target in [avatar.upcast_ref::<gtk::Widget>(), channel.upcast_ref()] {
        target.add_css_class("channel-link");
        target.set_tooltip_text(Some("Go to channel"));
    }

    let info = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    info.append(&avatar);
    info.append(&text);
    info.append(&menu);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    root.add_css_class("video-card");
    root.set_cursor_from_name(Some("pointer"));
    root.append(&thumb);
    root.append(&info);
    if props.get("layout").and_then(Value::as_str) == Some("compact") {
        root.set_orientation(gtk::Orientation::Horizontal);
        root.set_spacing(8);
        root.add_css_class("compact");
        thumb.set_size_request(COMPACT_WIDTH, -1);
        thumb.set_natural_width(COMPACT_WIDTH);
        thumb.set_valign(gtk::Align::Start);
        avatar.set_visible(false);
        title.set_lines(2);
    } else if props.get("layout").and_then(Value::as_str) == Some("shelf") {
        root.add_css_class("shelf");
        root.set_size_request(SHELF_WIDTH, -1);
        thumb.set_natural_width(SHELF_WIDTH);
    } else {
        root.set_size_request(320, -1);
    }

    let card = Rc::new(Card {
        picture,
        duration,
        watched,
        avatar,
        title,
        channel,
        meta,
        video: RefCell::new(Value::Null),
    });
    connect_input(&root, &card, &menu);
    // A row measures each card's width for the row's height, and asked
    // that, wrapping text wants its whole length on one line: the clamp
    // holds a shelf's card to its width.
    let widget: gtk::Widget = if root.has_css_class("shelf") {
        adw::Clamp::builder()
            .child(&root)
            .maximum_size(SHELF_WIDTH)
            .tightening_threshold(SHELF_WIDTH)
            .build()
            .upcast()
    } else {
        root.upcast()
    };
    attach(&widget, card.clone());
    fill(&widget, &card, props);
    widget
}

pub fn update(widget: &gtk::Widget, props: &Value) {
    if let Some(card) = attached::<Card>(widget) {
        fill(widget, &card, props);
    }
}

fn fill(root: &impl IsA<gtk::Widget>, card: &Card, video: &Value) {
    *card.video.borrow_mut() = video.clone();
    let text = |key: &str| video.get(key).and_then(Value::as_str).unwrap_or_default();
    card.title.set_label(text("title"));
    card.title
        .set_tooltip_text(Some(text("title")).filter(|t| !t.is_empty()));
    card.channel.set_label(text("channel"));
    // The channel's own page needn't say whose videos they are.
    let on_channel = video.get("on_channel").and_then(Value::as_bool) == Some(true);
    card.channel.set_visible(!on_channel);
    card.avatar
        .set_visible(!on_channel && !root.has_css_class("compact"));
    if !on_channel {
        show_avatar(card, text("channel"));
    }
    let meta: Vec<&str> = [text("views"), text("published")]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();
    card.meta.set_label(&meta.join(" • "));

    let duration = text("duration");
    card.duration.set_label(duration);
    card.duration.set_visible(!duration.is_empty());
    if video.get("live").and_then(Value::as_bool).unwrap_or(false) {
        card.duration.add_css_class("live");
    } else {
        card.duration.remove_css_class("live");
    }
    let progress = video.get("progress").and_then(Value::as_f64).unwrap_or(0.0);
    card.watched.set_fraction(progress);
    card.watched.set_visible(progress > 0.0);

    let thumbnail = video.get("thumbnail").and_then(Value::as_str);
    app().images.load_into(
        &card.picture,
        thumbnail,
        // Compact cards decode at twice their width, for sharpness.
        if root.has_css_class("compact") {
            COMPACT_WIDTH * 2
        } else {
            THUMBNAIL_WIDTH
        } * root.scale_factor(),
    );
}

/// The channel's picture, if the channel directory knows it; otherwise
/// the initials until the directory has looked the channel up.
fn show_avatar(card: &Card, channel: &str) {
    let channels = &app().channels;
    let known = channels.lookup(channel).and_then(|c| c.avatar);
    if known.is_none() {
        let video_id = card
            .video
            .borrow()
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        channels.want(channel, &video_id);
    }
    avatar::set(&card.avatar, channel, known.as_deref());
}

fn open_channel(card: &Card) {
    let video = card.video.borrow();
    let text = |key: &str| video.get(key).cloned().unwrap_or_default();
    run(
        "channel.open",
        json!({ "name": text("channel"), "video_id": text("id") }),
    );
}

fn connect_input(root: &gtk::Box, card: &Rc<Card>, menu: &gtk::MenuButton) {
    // The avatar and channel name open the channel instead of the video.
    for target in [
        card.avatar.upcast_ref::<gtk::Widget>(),
        card.channel.upcast_ref(),
    ] {
        let click = gtk::GestureClick::new();
        click.set_button(gtk::gdk::BUTTON_PRIMARY);
        click.connect_pressed(|gesture, _, _, _| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
        });
        let weak = Rc::downgrade(card);
        click.connect_released(move |_, _, _, _| {
            if let Some(card) = weak.upgrade() {
                open_channel(&card);
            }
        });
        target.add_controller(click);
    }
    // Avatars appear once the subscriptions list arrives, or the channel
    // has been looked up.
    let weak = Rc::downgrade(card);
    follow(root, "channels.updated", move |_| {
        if let Some(card) = weak.upgrade() {
            let channel = card.channel.label();
            show_avatar(&card, &channel);
        }
    });
    let weak = Rc::downgrade(card);
    follow(root, "channels.learned", move |payload| {
        if let Some(card) = weak.upgrade()
            && payload.get("name").and_then(Value::as_str) == Some(card.channel.label().as_str())
        {
            let channel = card.channel.label();
            show_avatar(&card, &channel);
        }
    });

    let video = |card: &Rc<Card>| {
        let weak = Rc::downgrade(card);
        move || weak.upgrade().map(|card| card.video.borrow().clone())
    };
    super::video_input::connect(root, video(card));
    super::video_input::attach_menu(menu, video(card));
}

fn run(command: &str, args: Value) {
    if let Err(e) = app().ext.commands.run(command, args) {
        app().toast(&e);
    }
}
