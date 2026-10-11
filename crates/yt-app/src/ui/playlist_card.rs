//! The `playlist_card` component: a playlist as youtube.com shows one in a
//! grid or a row: its picture with two edges stacked behind it and the
//! number of videos on it, then the title, the lines under it ("Private",
//! "Updated today") and "View full playlist". A click opens the playlist.
//! Properties: a playlist (`{ id, title, thumbnail?, count?, details? }`),
//! plus `layout = "shelf"` for a row's narrower card.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use serde_json::{Value, json};

use super::aspect::AspectBin;
use super::{attach, attached};
use crate::app::app;

/// Picture width to decode, in pixels at scale 1.
const THUMBNAIL_WIDTH: i32 = 480;
const SHELF_WIDTH: i32 = 246;

struct Card {
    picture: gtk::Picture,
    count: gtk::Label,
    title: gtk::Label,
    details: gtk::Label,
    playlist: RefCell<Value>,
    /// The width its picture is decoded at: the card's widest.
    decode_width: i32,
}

pub fn build(props: &Value) -> gtk::Widget {
    let picture = gtk::Picture::new();
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.set_can_shrink(true);
    let count = gtk::Label::new(None);
    count.add_css_class("duration");
    count.add_css_class("playlist-count");
    count.set_halign(gtk::Align::End);
    count.set_valign(gtk::Align::End);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&picture));
    overlay.add_overlay(&count);
    let thumb = AspectBin::new(9.0 / 16.0, &overlay);
    thumb.add_css_class("thumb");
    thumb.set_overflow(gtk::Overflow::Hidden);

    // The two edges of the playlist's other videos, behind the picture.
    let stack = gtk::Box::new(gtk::Orientation::Vertical, 2);
    stack.add_css_class("playlist-stack");
    for class in ["back", "middle"] {
        let edge = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        edge.add_css_class("playlist-edge");
        edge.add_css_class(class);
        stack.append(&edge);
    }
    stack.append(&thumb);

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
    let details = label("meta", 2);
    // A grid fits as many cards in a row as their natural widths allow,
    // and wrapping text's is its whole length: the picture's decides.
    for label in [&title, &details] {
        label.set_max_width_chars(1);
    }
    details.add_css_class("dim");
    let view = gtk::Label::new(Some("View full playlist"));
    view.add_css_class("meta");
    view.add_css_class("dim");
    view.add_css_class("playlist-view");
    view.set_xalign(0.0);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.append(&title);
    text.append(&details);
    text.append(&view);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
    root.add_css_class("video-card");
    root.add_css_class("playlist-card");
    root.set_cursor_from_name(Some("pointer"));
    root.append(&stack);
    root.append(&text);

    let card = Rc::new(Card {
        picture,
        count,
        title,
        details,
        playlist: RefCell::new(Value::Null),
        decode_width: if props.get("layout").and_then(Value::as_str) == Some("shelf") {
            SHELF_WIDTH
        } else {
            THUMBNAIL_WIDTH
        },
    });
    let shelf = props.get("layout").and_then(Value::as_str) == Some("shelf");
    let widget: gtk::Widget = if shelf {
        root.add_css_class("shelf");
        root.set_size_request(SHELF_WIDTH, -1);
        thumb.set_natural_width(SHELF_WIDTH);
        // Held to its width in a row (see the video card).
        adw::Clamp::builder()
            .child(&root)
            .maximum_size(SHELF_WIDTH)
            .tightening_threshold(SHELF_WIDTH)
            .build()
            .upcast()
    } else {
        root.set_size_request(320, -1);
        root.clone().upcast()
    };
    let click = gtk::GestureClick::new();
    let weak = Rc::downgrade(&card);
    click.connect_released(move |gesture, n, _, _| {
        let Some(card) = weak.upgrade() else { return };
        let id = card.playlist.borrow().get("id").cloned();
        if n == 1
            && let Some(id) = id
        {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            if let Err(e) = app()
                .ext
                .commands
                .run("playlist.open", json!({ "playlist_id": id }))
            {
                app().toast(&e);
            }
        }
    });
    root.add_controller(click);
    attach(&widget, card.clone());
    fill(&widget, &card, props);
    widget
}

pub fn update(widget: &gtk::Widget, props: &Value) {
    if let Some(card) = attached::<Card>(widget) {
        fill(widget, &card, props);
    }
}

fn fill(widget: &gtk::Widget, card: &Card, playlist: &Value) {
    *card.playlist.borrow_mut() = playlist.clone();
    let text = |key: &str| {
        playlist
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
    };
    card.title.set_label(text("title"));
    card.title
        .set_tooltip_text(Some(text("title")).filter(|t| !t.is_empty()));
    let details: Vec<&str> = playlist
        .get("details")
        .and_then(Value::as_array)
        .map(|d| d.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    card.details.set_label(&details.join("\n"));
    card.details.set_visible(!details.is_empty());
    let count = text("count");
    card.count.set_label(count);
    card.count.set_visible(!count.is_empty());
    app().images.load_into(
        &card.picture,
        playlist.get("thumbnail").and_then(Value::as_str),
        card.decode_width * widget.scale_factor(),
    );
}

/// Makes a list's playlist cards (built with `build`), refilling cards as
/// they are reused.
pub fn factory(build: Value) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    let setup = build.clone();
    factory.connect_setup(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
        item.set_child(Some(&app().ext.components.build("playlist_card", &setup)));
        item.set_activatable(false);
        item.set_focusable(false);
    });
    factory.connect_bind(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
        let Some(object) = item.item().and_downcast::<gtk::glib::BoxedAnyObject>() else {
            return;
        };
        let mut props = serde_json::to_value(&*object.borrow::<yt_core::playlist::Playlist>())
            .unwrap_or_default();
        if let (Some(props), Some(build)) = (props.as_object_mut(), build.as_object()) {
            props.extend(build.clone());
        }
        let components = &app().ext.components;
        let updated = item
            .child()
            .is_some_and(|c| components.update("playlist_card", &c, &props));
        if !updated {
            item.set_child(Some(&components.build("playlist_card", &props)));
        }
    });
    factory
}
