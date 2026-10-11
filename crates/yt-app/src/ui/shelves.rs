//! The `shelves` component: a feed page's shelves as rows, one under the
//! other, as a channel's home shows them on youtube.com: each a title over
//! a row of cards that scrolls sideways (with buttons at its ends). More
//! shelves load as the page scrolls down. Properties: `{ page, browse_id,
//! on_channel? }`, `page` being the first page of shelves.
//!
//! Each row is a list view, so only the cards on screen exist.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use serde_json::{Value, json};
use yt_core::feed::{FeedPage, Shelf};

use super::{attach, attached};
use crate::app::app;
use crate::runtime;

/// Loads more shelves when the bottom is this close, in pixels.
const PRELOAD_DISTANCE: f64 = 600.0;

struct Shelves {
    browse_id: RefCell<String>,
    column: gtk::Box,
    /// The component itself, weakly: it owns this.
    scroller: gtk::glib::WeakRef<gtk::ScrolledWindow>,
    on_channel: bool,
    continuation: RefCell<Option<String>>,
    loading: std::cell::Cell<bool>,
    /// Bumped when filled anew, so a load for older shelves is dropped.
    generation: std::cell::Cell<u64>,
}

pub fn build(props: &Value) -> gtk::Widget {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    column.add_css_class("shelves");
    let scroller = gtk::ScrolledWindow::builder()
        .child(&column)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    scroller.set_widget_name("shelves");
    let shelves = Rc::new(Shelves {
        browse_id: RefCell::default(),
        column,
        scroller: scroller.downgrade(),
        on_channel: props.get("on_channel").and_then(Value::as_bool) == Some(true),
        continuation: RefCell::default(),
        loading: Default::default(),
        generation: Default::default(),
    });
    attach(&scroller, shelves.clone());
    let weak = Rc::downgrade(&shelves);
    scroller.vadjustment().connect_value_changed(move |adj| {
        if adj.value() + adj.page_size() > adj.upper() - PRELOAD_DISTANCE
            && let Some(shelves) = weak.upgrade()
        {
            load_more(&shelves);
        }
    });
    fill(&shelves, props);
    scroller.upcast()
}

pub fn update(widget: &gtk::Widget, props: &Value) {
    if let Some(shelves) = attached::<Shelves>(widget) {
        fill(&shelves, props);
    }
}

fn fill(shelves: &Rc<Shelves>, props: &Value) {
    shelves.generation.set(shelves.generation.get() + 1);
    shelves.loading.set(false);
    while let Some(row) = shelves.column.first_child() {
        shelves.column.remove(&row);
    }
    if let Some(scroller) = shelves.scroller.upgrade() {
        scroller.vadjustment().set_value(0.0);
    }
    *shelves.browse_id.borrow_mut() = props
        .get("browse_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let page: FeedPage = props
        .get("page")
        .and_then(|p| serde_json::from_value(p.clone()).ok())
        .unwrap_or_default();
    add(shelves, page);
}

fn add(shelves: &Shelves, page: FeedPage) {
    *shelves.continuation.borrow_mut() = page.continuation;
    for shelf in page.shelves {
        if let Some(row) = shelf_row(shelf, shelves.on_channel) {
            shelves.column.append(&row);
        }
    }
}

/// A shelf as a row: its videos, or its playlists; none if it has neither.
fn shelf_row(shelf: Shelf, on_channel: bool) -> Option<gtk::Widget> {
    let items = if !shelf.videos.is_empty() {
        Items::Videos(shelf.videos)
    } else if !shelf.playlists.is_empty() {
        Items::Playlists(shelf.playlists)
    } else {
        return None;
    };
    Some(row(&shelf.title, items, on_channel, None))
}

/// What a row shows.
pub enum Items {
    Videos(Vec<yt_core::feed::Video>),
    Playlists(Vec<yt_core::playlist::Playlist>),
}

/// A titled row of cards that scrolls sideways; `link` (a page and its
/// arguments) adds "View all" beside the title.
pub fn row(
    title: &str,
    items: Items,
    on_channel: bool,
    link: Option<(&str, Value)>,
) -> gtk::Widget {
    let heading = gtk::Label::new(Some(title));
    heading.add_css_class("shelf-title");
    heading.set_xalign(0.0);
    heading.set_hexpand(true);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    header.add_css_class("shelf-header");
    header.append(&heading);
    header.set_visible(!title.is_empty());
    if let Some((page, args)) = link {
        let all = gtk::Button::with_label("View all");
        all.add_css_class("pill-button");
        all.add_css_class("outline");
        all.set_valign(gtk::Align::Center);
        let page = page.to_owned();
        all.connect_clicked(move |_| {
            app().navigate(&page, args.clone(), true);
        });
        header.append(&all);
    }

    let model = gtk::gio::ListStore::new::<glib::BoxedAnyObject>();
    let factory = match items {
        Items::Videos(videos) => {
            for video in videos {
                model.append(&glib::BoxedAnyObject::new(video));
            }
            super::feed::card_factory(
                json!({ "layout": "shelf" }),
                json!({ "on_channel": on_channel }),
            )
        }
        Items::Playlists(playlists) => {
            for playlist in playlists {
                model.append(&glib::BoxedAnyObject::new(playlist));
            }
            super::playlist_card::factory(json!({ "layout": "shelf" }))
        }
    };
    let list = gtk::ListView::new(Some(gtk::NoSelection::new(Some(model))), Some(factory));
    list.set_orientation(gtk::Orientation::Horizontal);
    list.add_css_class("shelf-row");
    list.set_can_focus(false);
    let scroller = gtk::ScrolledWindow::builder()
        .child(&list)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .hscrollbar_policy(gtk::PolicyType::External)
        .build();
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&scroller));
    super::scroll_arrows::add(&overlay, &scroller);

    let shelf_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    shelf_box.add_css_class("shelf");
    shelf_box.append(&header);
    shelf_box.append(&overlay);
    shelf_box.upcast()
}

fn load_more(shelves: &Rc<Shelves>) {
    if shelves.loading.get() {
        return;
    }
    let Some(token) = shelves.continuation.borrow().clone() else {
        return;
    };
    shelves.loading.set(true);
    let (weak, generation) = (Rc::downgrade(shelves), shelves.generation.get());
    runtime::spawn_local(async move {
        let youtube = app().youtube.clone();
        let result = runtime::background(async move { youtube.continuation(&token).await }).await;
        let Some(shelves) = weak.upgrade().filter(|s| s.generation.get() == generation) else {
            return;
        };
        shelves.loading.set(false);
        match result {
            Ok(page) => {
                let browse_id = shelves.browse_id.borrow().clone();
                add(&shelves, super::feed::filtered(page, &browse_id));
            }
            Err(e) => tracing::warn!("more shelves: {e}"),
        }
    });
}
