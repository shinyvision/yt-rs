//! A feed page (home, subscriptions, history): its shelves as chips above a
//! grid of videos that loads more as you scroll.
//!
//! The TV feeds are columns of shelves. Each titled shelf becomes a chip;
//! untitled shelves (rows of a plain list) are joined into one section named
//! by the page's `untitled` property. The grid is a `GtkGridView`, which only
//! creates cards for what is on screen and reuses them while scrolling.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::{Rc, Weak};

use gtk::prelude::*;
use gtk::{gio, glib};
use serde_json::{Value, json};
use yt_core::channel::VideoOrder;
use yt_core::feed::{FeedPage, Video};

use super::{attach, attached, follow};
use crate::app::app;
use crate::runtime;

/// Load more when this close (in pixels) to the bottom.
const PRELOAD_DISTANCE: f64 = 1200.0;

#[derive(Default)]
struct Section {
    title: String,
    videos: Vec<Video>,
    ids: HashSet<String>,
    continuation: Option<String>,
}

impl Section {
    /// Adds videos that aren't in the section yet, returning the new ones.
    fn extend(&mut self, videos: Vec<Video>) -> Vec<Video> {
        let new: Vec<Video> = videos
            .into_iter()
            .filter(|v| self.ids.insert(v.id.clone()))
            .collect();
        self.videos.extend(new.iter().cloned());
        new
    }
}

#[derive(Default)]
struct State {
    browse_id: String,
    /// Browse params, e.g. a channel tab.
    params: Option<String>,
    /// A channel's videos, in this order (`sortable`); chips choose it.
    order: Option<VideoOrder>,
    /// A first page the caller already has, shown instead of loading it.
    preloaded: Option<FeedPage>,
    untitled: String,
    sections: Vec<Section>,
    selected: usize,
    continuation: Option<String>,
    loading: bool,
    /// Bumped on reload, so responses for an older load are ignored.
    generation: u64,
}

struct Feed {
    state: RefCell<State>,
    chips: gtk::Box,
    content: gtk::Stack,
    status: adw::StatusPage,
    /// The videos shown. Replaced, not emptied, when the grid is filled
    /// anew; see [`show_selected`].
    model: RefCell<gio::ListStore>,
    grid: gtk::GridView,
    scroller: gtk::ScrolledWindow,
}

pub fn build(props: &Value) -> gtk::Widget {
    let model = gio::ListStore::new::<glib::BoxedAnyObject>();
    // On a channel's page, its cards leave the channel out.
    let extra =
        json!({ "on_channel": props.get("on_channel").and_then(Value::as_bool) == Some(true) });
    // `layout = "list"`: one video a row, as youtube.com's History.
    let list = props.get("layout").and_then(Value::as_str) == Some("list");
    let build = if list {
        json!({ "layout": "list" })
    } else {
        Value::Null
    };
    let grid = gtk::GridView::new(
        Some(gtk::NoSelection::new(Some(model.clone()))),
        Some(card_factory(build, extra)),
    );
    grid.set_widget_name("video-grid");
    grid.add_css_class("video-grid");
    grid.set_min_columns(1);
    grid.set_max_columns(if list { 1 } else { 6 });
    if list {
        grid.add_css_class("video-list");
    }
    grid.set_can_focus(false);
    let scroller = gtk::ScrolledWindow::builder()
        .child(&grid)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();

    let spinner = adw::Spinner::new();
    spinner.set_size_request(48, 48);
    spinner.set_halign(gtk::Align::Center);
    spinner.set_valign(gtk::Align::Center);
    let status = adw::StatusPage::new();
    let retry = gtk::Button::with_label("Try again");
    retry.add_css_class("pill-button");
    retry.set_halign(gtk::Align::Center);
    status.set_child(Some(&retry));

    let content = gtk::Stack::new();
    content.add_named(&spinner, Some("loading"));
    content.add_named(&status, Some("status"));
    content.add_named(&scroller, Some("grid"));
    content.set_widget_name("feed-content");

    let chips = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("feed-page");
    // `title`: a heading over the videos ("Watch history"), lined up with
    // a list's rows.
    if let Some(title) = props.get("title").and_then(Value::as_str) {
        let heading = gtk::Label::new(Some(title));
        heading.add_css_class("page-title");
        heading.add_css_class("feed-title");
        heading.set_xalign(0.0);
        // The margins outside the clamp, as the list's padding is.
        let clamp = adw::Clamp::builder()
            .child(&heading)
            .maximum_size(1096)
            .tightening_threshold(1096)
            .margin_start(24)
            .margin_end(24)
            .build();
        root.append(&clamp);
    }
    root.append(&chips);
    root.append(
        &app()
            .ext
            .slots
            .create("feed.header", gtk::Orientation::Vertical, 0),
    );
    root.append(&content);

    let feed = Rc::new(Feed {
        state: RefCell::default(),
        chips,
        content,
        status,
        model: RefCell::new(model),
        grid: grid.clone(),
        scroller,
    });
    attach(&root, feed.clone());
    // A video the user told YouTube they don't want goes, as on youtube.com.
    let weak = Rc::downgrade(&feed);
    follow(&root, "video.hidden", move |payload| {
        if let (Some(feed), Some(id)) = (
            weak.upgrade(),
            payload.get("video_id").and_then(Value::as_str),
        ) {
            hide(&feed, id);
        }
    });

    // Load more when scrolled near the end, and also when the content
    // doesn't fill the view yet (then there is nothing to scroll).
    let weak = Rc::downgrade(&feed);
    // Only on screen: a page kept out of sight loads nothing.
    let near_end = move |adj: &gtk::Adjustment| {
        if adj.value() + adj.page_size() > adj.upper() - PRELOAD_DISTANCE
            && adj.page_size() > 0.0
            && let Some(feed) = weak.upgrade()
            && feed.grid.is_mapped()
            && feed.model.borrow().n_items() > 0
        {
            load_more(&feed);
        }
    };
    let adjustment = feed.scroller.vadjustment();
    adjustment.connect_value_changed(near_end.clone());
    adjustment.connect_changed(near_end);
    let weak = Rc::downgrade(&feed);
    retry.connect_clicked(move |_| {
        if let Some(feed) = weak.upgrade() {
            reload(&feed);
        }
    });
    let weak = Rc::downgrade(&feed);
    follow(&root, "feed.refresh", move |_| {
        if let Some(feed) = weak.upgrade()
            && feed.chips.is_mapped()
        {
            reload(&feed);
        }
    });
    let weak = Rc::downgrade(&feed);
    follow(&root, "feed.chip_selected", move |payload| {
        select_from_event(&weak, payload)
    });
    let weak = Rc::downgrade(&feed);
    follow(&root, "feed.order_selected", move |payload| {
        order_from_event(&weak, payload)
    });

    configure(&feed, props);
    reload(&feed);
    root.upcast()
}

/// Navigating to a kept feed page again: reload if it shows another feed or
/// asks to be refreshed whenever it is shown (history).
pub fn update(widget: &gtk::Widget, props: &Value) {
    let Some(feed) = attached::<Feed>(widget) else {
        return;
    };
    let changed = configure(&feed, props);
    if changed
        || props
            .get("refresh_on_show")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        reload(&feed);
    }
}

/// Applies the page properties; returns whether the feed changed.
fn configure(feed: &Feed, props: &Value) -> bool {
    let browse_id = props
        .get("browse_id")
        .and_then(Value::as_str)
        .unwrap_or(yt_core::innertube::HOME)
        .to_owned();
    let untitled = props
        .get("untitled")
        .and_then(Value::as_str)
        .unwrap_or("All")
        .to_owned();
    let params = props
        .get("params")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
        .map(str::to_owned);
    // `sortable`: a channel's Videos, with chips for Latest, Popular and
    // Oldest; `order` picks one.
    let order = (props.get("sortable").and_then(Value::as_bool) == Some(true)).then(|| {
        props
            .get("order")
            .and_then(|o| serde_json::from_value(o.clone()).ok())
            .unwrap_or_default()
    });
    let mut state = feed.state.borrow_mut();
    let changed = state.browse_id != browse_id || state.params != params || state.order != order;
    state.browse_id = browse_id;
    state.params = params;
    state.order = order;
    state.untitled = untitled;
    state.preloaded = props
        .get("page")
        .filter(|_| changed)
        .and_then(|p| serde_json::from_value(p.clone()).ok());
    changed
}

fn reload(feed: &Rc<Feed>) {
    let (browse_id, params, order, preloaded, generation) = {
        let mut state = feed.state.borrow_mut();
        state.generation += 1;
        state.loading = true;
        state.sections.clear();
        state.continuation = None;
        (
            state.browse_id.clone(),
            state.params.clone(),
            // Latest is the tab as it comes.
            state.order.filter(|o| *o != VideoOrder::Latest),
            state.preloaded.take(),
            state.generation,
        )
    };
    if feed.model.borrow().n_items() == 0 {
        feed.content.set_visible_child_name("loading");
    }
    // The order chips show from the start; other chips come with the page.
    let sortable = feed.state.borrow().order.is_some();
    if sortable != feed.chips.has_css_class("orders") {
        rebuild_chips(feed);
    }
    let weak = Rc::downgrade(feed);
    runtime::spawn_local(async move {
        let youtube = app().youtube.clone();
        let id = browse_id.clone();
        let result = match preloaded {
            Some(page) => Ok(page),
            None => {
                runtime::background(async move {
                    match (order, params) {
                        (Some(order), _) => youtube.channel_videos(&id, order).await,
                        (None, Some(params)) => youtube.browse_with(&id, &params).await,
                        (None, None) => youtube.browse(&id).await,
                    }
                })
                .await
            }
        };
        let Some(feed) = current(&weak, generation) else {
            return;
        };
        feed.state.borrow_mut().loading = false;
        match result {
            Ok(page) => {
                let page = filtered(page, &browse_id);
                let nudge = page.nudge.clone();
                merge(&feed, page);
                feed.state.borrow_mut().selected = 0;
                show_selected(&feed);
                if feed.state.borrow().sections.is_empty() {
                    show_status(
                        &feed,
                        nudge.as_deref().unwrap_or("Nothing here yet"),
                        "",
                        false,
                    );
                }
                // A second page of shelves gives the chip bar more choices.
                if feed.state.borrow().sections.len() < 6 {
                    load_more_shelves(&feed);
                }
            }
            Err(e) => show_status(&feed, "Couldn't load the feed", &e.to_string(), true),
        }
    });
}

/// Loads more of the selected section, or more shelves if it has no more.
fn load_more(feed: &Rc<Feed>) {
    let (token, generation, selected) = {
        let state = feed.state.borrow();
        if state.loading {
            return;
        }
        let Some(section) = state.sections.get(state.selected) else {
            return;
        };
        (
            section.continuation.clone(),
            state.generation,
            state.selected,
        )
    };
    let Some(token) = token else {
        load_more_shelves(feed);
        return;
    };
    feed.state.borrow_mut().loading = true;
    let weak = Rc::downgrade(feed);
    runtime::spawn_local(async move {
        let youtube = app().youtube.clone();
        let result = runtime::background(async move { youtube.continuation(&token).await }).await;
        let Some(feed) = current(&weak, generation) else {
            return;
        };
        feed.state.borrow_mut().loading = false;
        let page = match result {
            Ok(page) => page,
            Err(e) => return tracing::warn!("couldn't load more of the feed: {e}"),
        };
        let browse_id = feed.state.borrow().browse_id.clone();
        let page = filtered(page, &browse_id);
        let added = {
            let mut state = feed.state.borrow_mut();
            let Some(section) = state.sections.get_mut(selected) else {
                return;
            };
            let shelf = page.shelves.into_iter().find(|s| !s.videos.is_empty());
            section.continuation = shelf.as_ref().and_then(|s| s.continuation.clone());
            let new = shelf.map(|s| section.extend(s.videos)).unwrap_or_default();
            (state.selected == selected).then_some(new)
        };
        if let Some(new) = added {
            append(&feed, new);
        }
        fill_view(&feed);
    });
}

fn load_more_shelves(feed: &Rc<Feed>) {
    let (token, generation) = {
        let state = feed.state.borrow();
        if state.loading {
            return;
        }
        let Some(token) = state.continuation.clone() else {
            return;
        };
        (token, state.generation)
    };
    feed.state.borrow_mut().loading = true;
    let weak = Rc::downgrade(feed);
    runtime::spawn_local(async move {
        let youtube = app().youtube.clone();
        let result = runtime::background(async move { youtube.continuation(&token).await }).await;
        let Some(feed) = current(&weak, generation) else {
            return;
        };
        feed.state.borrow_mut().loading = false;
        let page = match result {
            Ok(page) => page,
            Err(e) => return tracing::warn!("couldn't load more shelves: {e}"),
        };
        let browse_id = feed.state.borrow().browse_id.clone();
        let selected_before = feed.state.borrow().selected;
        let len_before = feed
            .state
            .borrow()
            .sections
            .get(selected_before)
            .map_or(0, |s| s.videos.len());
        merge(&feed, filtered(page, &browse_id));
        rebuild_chips(&feed);
        // New rows may have been added to the section on screen.
        let new: Vec<Video> = feed
            .state
            .borrow()
            .sections
            .get(selected_before)
            .map(|s| s.videos[len_before..].to_vec())
            .unwrap_or_default();
        append(&feed, new);
        fill_view(&feed);
    });
}

/// After a load finishes: if the grid still doesn't reach past the bottom of
/// the view, load more. Checked once layout has caught up.
fn fill_view(feed: &Rc<Feed>) {
    let weak = Rc::downgrade(feed);
    glib::idle_add_local_once(move || {
        let Some(feed) = weak.upgrade() else { return };
        let adj = feed.scroller.vadjustment();
        if adj.page_size() > 0.0
            && adj.value() + adj.page_size() > adj.upper() - PRELOAD_DISTANCE
            && feed.grid.is_mapped()
            && feed.model.borrow().n_items() > 0
        {
            load_more(&feed);
        }
    });
}

/// The feed if it is still alive and still showing the load `generation`.
fn current(weak: &Weak<Feed>, generation: u64) -> Option<Rc<Feed>> {
    weak.upgrade()
        .filter(|f| f.state.borrow().generation == generation)
}

/// Runs a page through the `feed.page` filter, so plugins can change it,
/// and announces it as `feed.loaded`.
pub(super) fn filtered(mut page: FeedPage, browse_id: &str) -> FeedPage {
    let app = app();
    let context = json!({ "browse_id": browse_id });
    if let Err(e) = app
        .ext
        .filters
        .apply_to("feed.page", context, "page", &mut page)
    {
        app.toast(&e);
    }
    if app.ext.events.has_handlers("feed.loaded") {
        app.ext.events.emit(
            "feed.loaded",
            &json!({ "browse_id": browse_id, "page": page }),
        );
    }
    page
}

/// Adds a page's shelves to the sections.
fn merge(feed: &Feed, page: FeedPage) {
    let mut state = feed.state.borrow_mut();
    if page.continuation.is_some() || state.sections.is_empty() {
        state.continuation = page.continuation;
    }
    for shelf in page.shelves {
        if shelf.videos.is_empty() {
            continue;
        }
        let title = if shelf.title.is_empty() {
            state.untitled.clone()
        } else {
            shelf.title
        };
        let index = match state.sections.iter().position(|s| s.title == title) {
            Some(i) => i,
            None => {
                state.sections.push(Section {
                    title,
                    ..Section::default()
                });
                state.sections.len() - 1
            }
        };
        let section = &mut state.sections[index];
        section.extend(shelf.videos);
        if shelf.continuation.is_some() {
            section.continuation = shelf.continuation;
        }
    }
}

/// Takes a video out of every section and off the grid.
fn hide(feed: &Feed, video_id: &str) {
    for section in &mut feed.state.borrow_mut().sections {
        section.videos.retain(|v| v.id != video_id);
    }
    let model = feed.model.borrow();
    if let Some(position) = (0..model.n_items()).find(|&i| {
        model
            .item(i)
            .and_downcast::<glib::BoxedAnyObject>()
            .is_some_and(|o| o.borrow::<Video>().id == video_id)
    }) {
        model.remove(position);
    }
}

fn show_selected(feed: &Rc<Feed>) {
    rebuild_chips(feed);
    let videos: Vec<glib::BoxedAnyObject> = {
        let state = feed.state.borrow();
        state
            .sections
            .get(state.selected)
            .map(|s| {
                s.videos
                    .iter()
                    .cloned()
                    .map(glib::BoxedAnyObject::new)
                    .collect()
            })
            .unwrap_or_default()
    };
    // Emptied first, then filled: the grid keeps its scroll anchor across a
    // splice that replaces items, and would show the new ones scrolled down.
    // A new model rather than new items in the old one: the grid keeps its
    // scroll anchor (an item in the middle of the view) across a splice, and
    // only starts again from the top with a new model.
    let model = gio::ListStore::new::<glib::BoxedAnyObject>();
    model.extend_from_slice(&videos);
    feed.grid
        .set_model(Some(&gtk::NoSelection::new(Some(model.clone()))));
    *feed.model.borrow_mut() = model;
    if !videos.is_empty() {
        feed.content.set_visible_child_name("grid");
    }
    fill_view(feed);
}

fn append(feed: &Feed, videos: Vec<Video>) {
    let items: Vec<glib::BoxedAnyObject> =
        videos.into_iter().map(glib::BoxedAnyObject::new).collect();
    feed.model.borrow().extend_from_slice(&items);
}

fn show_status(feed: &Feed, title: &str, description: &str, retry: bool) {
    feed.status.set_title(title);
    feed.status
        .set_description(Some(description).filter(|d| !d.is_empty()));
    feed.status.set_icon_name(Some(if retry {
        "network-error-symbolic"
    } else {
        "video-display-symbolic"
    }));
    if let Some(button) = feed.status.child() {
        button.set_visible(retry);
    }
    feed.content.set_visible_child_name("status");
}

fn rebuild_chips(feed: &Feed) {
    let props = {
        let state = feed.state.borrow();
        match state.order {
            // A channel's videos: the orders.
            Some(order) => json!({
                "chips": VideoOrder::ALL.map(VideoOrder::label),
                "selected": VideoOrder::ALL.iter().position(|o| *o == order),
                "browse_id": state.browse_id,
                "event": "feed.order_selected",
            }),
            None if state.sections.len() > 1 => json!({
                "chips": state.sections.iter().map(|s| s.title.clone()).collect::<Vec<_>>(),
                "selected": state.selected,
                "browse_id": state.browse_id,
            }),
            None => Value::Null,
        }
    };
    while let Some(child) = feed.chips.first_child() {
        feed.chips.remove(&child);
    }
    if props.get("event").is_some() {
        feed.chips.add_css_class("orders");
    } else {
        feed.chips.remove_css_class("orders");
    }
    if !props.is_null() {
        feed.chips
            .append(&app().ext.components.build("chip_bar", &props));
    }
}

/// A chip for another order: the videos load again in it.
fn order_from_event(weak: &Weak<Feed>, payload: &Value) {
    let Some(feed) = weak.upgrade() else { return };
    let Some(order) = payload
        .get("index")
        .and_then(Value::as_u64)
        .and_then(|i| VideoOrder::ALL.get(i as usize).copied())
    else {
        return;
    };
    {
        let mut state = feed.state.borrow_mut();
        if state.order.is_none_or(|o| o == order)
            || state.browse_id
                != payload
                    .get("browse_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
        {
            return;
        }
        state.order = Some(order);
    }
    // The videos in the old order go at once.
    feed.content.set_visible_child_name("loading");
    feed.scroller.vadjustment().set_value(0.0);
    reload(&feed);
}

fn select_from_event(weak: &Weak<Feed>, payload: &Value) {
    let Some(feed) = weak.upgrade() else { return };
    let browse_id = payload
        .get("browse_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let Some(index) = payload.get("index").and_then(Value::as_u64) else {
        return;
    };
    {
        let mut state = feed.state.borrow_mut();
        if state.browse_id != browse_id
            || index as usize >= state.sections.len()
            || state.selected == index as usize
        {
            return;
        }
        state.selected = index as usize;
    }
    show_selected(&feed);
}

/// The `chip_bar` component: a row of chips. Clicking one emits `event`
/// (`feed.chip_selected` by default) with `{ browse_id, index }`. A row
/// wider than the bar fades out at its ends, with a button there that
/// scrolls it, as on youtube.com.
pub fn build_chip_bar(props: &Value) -> gtk::Widget {
    let event = props
        .get("event")
        .and_then(Value::as_str)
        .unwrap_or("feed.chip_selected")
        .to_owned();
    let titles: Vec<String> = props
        .get("chips")
        .and_then(|c| serde_json::from_value(c.clone()).ok())
        .unwrap_or_default();
    let selected = props.get("selected").and_then(Value::as_u64).unwrap_or(0) as usize;
    let browse_id = props
        .get("browse_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("chip-bar");
    let mut group: Option<gtk::ToggleButton> = None;
    for (index, title) in titles.iter().enumerate() {
        let chip = gtk::ToggleButton::with_label(title);
        chip.add_css_class("chip");
        chip.set_active(index == selected);
        if let Some(first) = &group {
            chip.set_group(Some(first));
        } else {
            group = Some(chip.clone());
        }
        let (browse_id, event) = (browse_id.clone(), event.clone());
        chip.connect_toggled(move |chip| {
            if chip.is_active() {
                app()
                    .ext
                    .events
                    .emit(&event, &json!({ "browse_id": browse_id, "index": index }));
            }
        });
        row.append(&chip);
    }
    let scroller = gtk::ScrolledWindow::builder()
        .child(&row)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .hscrollbar_policy(gtk::PolicyType::External)
        .build();
    let bar = gtk::Overlay::new();
    bar.set_child(Some(&scroller));
    bar.set_widget_name("chip-bar");
    super::scroll_arrows::add(&bar, &scroller);
    bar.upcast()
}

/// Makes a list's cards through the `video_card` component (built with
/// `build`), refilling cards as they are reused with each video and
/// `extra`'s fields.
pub fn card_factory(build: Value, extra: Value) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    let setup = build.clone();
    factory.connect_setup(move |_, item| {
        let item = item
            .downcast_ref::<gtk::ListItem>()
            .expect("a GridView item");
        item.set_child(Some(&app().ext.components.build("video_card", &setup)));
        item.set_activatable(false);
        item.set_focusable(false);
    });
    factory.connect_bind(move |_, item| {
        let item = item
            .downcast_ref::<gtk::ListItem>()
            .expect("a GridView item");
        let Some(object) = item.item().and_downcast::<glib::BoxedAnyObject>() else {
            return;
        };
        let video = object.borrow::<Video>();
        let app = app();
        let mut props = serde_json::to_value(&*video).unwrap_or_default();
        if let (Some(props), Some(extra)) = (props.as_object_mut(), extra.as_object()) {
            props.extend(extra.clone());
        }
        if !app.ext.filters.is_empty("video.card") {
            props = app.ext.filters.apply("video.card", props);
        }
        let child = item.child();
        let updated = child
            .as_ref()
            .is_some_and(|c| app.ext.components.update("video_card", c, &props));
        if !updated {
            let mut props = props;
            if let (Some(props), Some(build)) = (props.as_object_mut(), build.as_object()) {
                props.extend(build.clone());
            }
            item.set_child(Some(&app.ext.components.build("video_card", &props)));
        }
    });
    factory
}
