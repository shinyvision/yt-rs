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
    scroller: gtk::ScrolledWindow,
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
        scroller: scroller.clone(),
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
    shelves.scroller.vadjustment().set_value(0.0);
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
    for shelf in page.shelves.into_iter().filter(|s| !s.videos.is_empty()) {
        shelves.column.append(&row(shelves, shelf));
    }
}

/// A shelf: its title over its row of cards.
fn row(shelves: &Shelves, shelf: Shelf) -> gtk::Widget {
    let title = gtk::Label::new(Some(&shelf.title));
    title.add_css_class("shelf-title");
    title.set_xalign(0.0);
    title.set_visible(!shelf.title.is_empty());

    let model = gtk::gio::ListStore::new::<glib::BoxedAnyObject>();
    for video in shelf.videos {
        model.append(&glib::BoxedAnyObject::new(video));
    }
    let list = gtk::ListView::new(
        Some(gtk::NoSelection::new(Some(model))),
        Some(super::feed::card_factory(
            json!({ "layout": "shelf" }),
            json!({ "on_channel": shelves.on_channel }),
        )),
    );
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
    shelf_box.append(&title);
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
