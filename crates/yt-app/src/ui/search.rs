//! The `search` page: a query's results, as on youtube.com. Chips and the
//! Filters button on top, then a column of results that loads more as you
//! scroll.
//!
//! Every row is a component, named in the row: `search_video`,
//! `search_channel`, `search_playlist`, `search_shelf`, `search_correction`
//! and `search_footer` (see `search_results`). Each page of results becomes
//! rows `{ component, props }` that go through the `search.rows` filter, so
//! a plugin can restyle a kind of row by replacing its component, or add
//! rows of its own components anywhere. The list only builds the rows on
//! screen and reuses them while scrolling.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gio, glib};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use yt_core::search::{
    SearchChip, SearchContinuation, SearchFilters, SearchItem, SearchPage, SearchVideo,
};

use super::{attach, attached, follow};
use crate::app::app;
use crate::{runtime, search};

/// Load more when this close (in pixels) to the bottom.
const PRELOAD_DISTANCE: f64 = 1600.0;
/// The results column's widest, as on youtube.com.
const COLUMN_WIDTH: i32 = 1096;
/// How often, at most, scrolling checks for the end of the results; by
/// then the list has measured its new rows.
const END_CHECK: Duration = Duration::from_millis(100);

/// A row of the list: the component that shows it and its properties.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Row {
    pub component: String,
    #[serde(default)]
    pub props: Value,
}

impl Row {
    fn new(component: &str, props: impl Serialize) -> Self {
        Self {
            component: component.to_owned(),
            props: serde_json::to_value(props).unwrap_or_default(),
        }
    }
}

#[derive(Default)]
struct State {
    query: String,
    filters: SearchFilters,
    /// The selected chip's token; `None` for "All".
    chip: Option<String>,
    chips: Vec<SearchChip>,
    next: Option<SearchContinuation>,
    /// The web client's details for videos later pages may list.
    details: HashMap<String, SearchVideo>,
    /// Results shown, so a later page doesn't repeat them.
    seen: HashSet<String>,
    loading: bool,
    /// Loading more failed; only the footer's button tries again.
    failed: bool,
    /// The list ends with a `search_footer` row.
    footer: bool,
    /// Bumped on every new search, so older responses are dropped.
    generation: u64,
}

struct SearchView {
    state: RefCell<State>,
    chips: gtk::Box,
    filters: gtk::Button,
    content: gtk::Stack,
    status: adw::StatusPage,
    status_button: gtk::Button,
    model: RefCell<gio::ListStore>,
    list: gtk::ListView,
    scroller: gtk::ScrolledWindow,
    /// A check for the end of the results is scheduled.
    checking: Cell<bool>,
}

pub fn build(props: &Value) -> gtk::Widget {
    let model = gio::ListStore::new::<glib::BoxedAnyObject>();
    let list = gtk::ListView::new(
        Some(gtk::NoSelection::new(Some(model.clone()))),
        Some(row_factory()),
    );
    list.set_widget_name("search-results");
    list.add_css_class("search-results");
    list.set_can_focus(false);
    let scroller = gtk::ScrolledWindow::builder()
        .child(&list)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();

    let spinner = adw::Spinner::new();
    spinner.set_size_request(48, 48);
    spinner.set_halign(gtk::Align::Center);
    spinner.set_valign(gtk::Align::Center);
    let status = adw::StatusPage::new();
    let status_button = gtk::Button::new();
    status_button.add_css_class("pill-button");
    status_button.set_halign(gtk::Align::Center);
    status.set_child(Some(&status_button));
    let content = gtk::Stack::new();
    content.add_named(&spinner, Some("loading"));
    content.add_named(&status, Some("status"));
    content.add_named(&scroller, Some("results"));

    let chips = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    chips.set_hexpand(true);
    let filters = gtk::Button::new();
    filters.add_css_class("search-filters-button");
    filters.set_valign(gtk::Align::Center);
    filters.set_tooltip_text(Some("Search filters"));
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    bar.add_css_class("search-header");
    bar.append(&chips);
    bar.append(&filters);
    let header = adw::Clamp::builder()
        .maximum_size(COLUMN_WIDTH)
        .tightening_threshold(COLUMN_WIDTH)
        .child(&bar)
        .build();

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("search-page");
    root.append(&header);
    root.append(
        &app()
            .ext
            .slots
            .create("search.header", gtk::Orientation::Vertical, 0),
    );
    root.append(&content);

    let view = Rc::new(SearchView {
        state: RefCell::default(),
        chips,
        filters,
        content,
        status,
        status_button,
        model: RefCell::new(model),
        list,
        scroller,
        checking: Cell::new(false),
    });
    attach(&root, view.clone());
    connect(&view, &root);
    configure(&view, props);
    reload(&view);
    root.upcast()
}

/// Going back to the page shows the results as they were; another query,
/// filter or chip searches again.
pub fn update(widget: &gtk::Widget, props: &Value) {
    if let Some(view) = attached::<SearchView>(widget)
        && configure(&view, props)
    {
        reload(&view);
    }
}

fn connect(view: &Rc<SearchView>, root: &gtk::Box) {
    // Scrolling, and the list growing or shrinking, may bring the end of
    // the results near.
    let weak = Rc::downgrade(view);
    let near_end = move |_: &gtk::Adjustment| {
        if let Some(view) = weak.upgrade() {
            check_end(&view);
        }
    };
    let adjustment = view.scroller.vadjustment();
    adjustment.connect_value_changed(near_end.clone());
    adjustment.connect_changed(near_end);

    let weak = Rc::downgrade(view);
    view.filters.connect_clicked(move |button| {
        if let Some(view) = weak.upgrade() {
            let state = view.state.borrow();
            super::search_filters::present(button, &state.query, &state.filters);
        }
    });
    let weak = Rc::downgrade(view);
    view.status_button.connect_clicked(move |_| {
        let Some(view) = weak.upgrade() else { return };
        if view.status_button.has_css_class("clear-filters") {
            let query = view.state.borrow().query.clone();
            run("search.open", json!({ "query": query }));
        } else {
            reload(&view);
        }
    });
    let weak = Rc::downgrade(view);
    follow(root, "search.chip_selected", move |payload| {
        let Some(view) = weak.upgrade() else { return };
        let Some(index) = payload.get("index").and_then(Value::as_u64) else {
            return;
        };
        let state = view.state.borrow();
        let Some(chip) = state.chips.get(index as usize) else {
            return;
        };
        let mut chips = state.chips.clone();
        for (i, c) in chips.iter_mut().enumerate() {
            c.selected = i == index as usize;
        }
        // In place of the current entry, so Back leaves the search and
        // coming back shows this chip.
        let args = json!({
            "query": state.query,
            "filters": state.filters,
            "chip": chip.token,
            "chips": chips,
        });
        drop(state);
        app().navigate("search", args, false);
    });
    for event in ["search.refresh", "feed.refresh"] {
        let weak = Rc::downgrade(view);
        follow(root, event, move |_| {
            if let Some(view) = weak.upgrade()
                && view.content.is_mapped()
            {
                reload(&view);
            }
        });
    }
    let weak = Rc::downgrade(view);
    follow(root, "search.more", move |_| {
        if let Some(view) = weak.upgrade() {
            load_more(&view, true);
        }
    });
}

/// Applies the page's arguments; returns whether they ask for another
/// search than the one shown.
fn configure(view: &SearchView, props: &Value) -> bool {
    let query = props
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned();
    let filters: SearchFilters = props
        .get("filters")
        .and_then(|f| serde_json::from_value(f.clone()).ok())
        .unwrap_or_default();
    let chip = props.get("chip").and_then(Value::as_str).map(str::to_owned);
    let mut state = view.state.borrow_mut();
    let changed = state.query != query || state.filters != filters || state.chip != chip;
    if changed && (state.query != query || state.filters != filters) {
        state.chips = props
            .get("chips")
            .and_then(|c| serde_json::from_value(c.clone()).ok())
            .unwrap_or_default();
    }
    state.query = query;
    state.filters = filters;
    state.chip = chip;
    changed
}

fn reload(view: &Rc<SearchView>) {
    let (query, filters, chip, generation) = {
        let mut state = view.state.borrow_mut();
        state.generation += 1;
        state.loading = true;
        state.failed = false;
        state.next = None;
        state.details.clear();
        state.seen.clear();
        (
            state.query.clone(),
            state.filters.clone(),
            state.chip.clone(),
            state.generation,
        )
    };
    show_header(view);
    if query.is_empty() {
        view.state.borrow_mut().loading = false;
        show_status(view, Status::Empty);
        return;
    }
    view.content.set_visible_child_name("loading");
    let weak = Rc::downgrade(view);
    runtime::spawn_local(async move {
        let result = search::fetch(&query, &filters, chip.as_deref()).await;
        let Some(view) = current(&weak, generation) else {
            return;
        };
        view.state.borrow_mut().loading = false;
        match result {
            Ok(mut page) => {
                search::publish(&query, true, &mut page);
                show_first(&view, page);
                // Reading the responses left megabytes of freed memory.
                runtime::release_memory_soon();
            }
            Err(e) => show_status(&view, Status::Failed(e)),
        }
    });
}

fn show_first(view: &Rc<SearchView>, page: SearchPage) {
    {
        let mut state = view.state.borrow_mut();
        if !page.chips.is_empty() {
            state.chips = page.chips.clone();
        }
    }
    show_header(view);
    let rows = rows(view, &page, true);
    let model = gio::ListStore::new::<glib::BoxedAnyObject>();
    model.extend_from_slice(
        &rows
            .into_iter()
            .map(glib::BoxedAnyObject::new)
            .collect::<Vec<_>>(),
    );
    let empty = model.n_items() == 0;
    view.list
        .set_model(Some(&gtk::NoSelection::new(Some(model.clone()))));
    *view.model.borrow_mut() = model;
    view.state.borrow_mut().footer = false;
    if empty {
        show_status(view, Status::NoResults);
        return;
    }
    view.content.set_visible_child_name("results");
    // From the top: the list would otherwise keep the scroll position of
    // the results it showed before.
    view.list.scroll_to(0, gtk::ListScrollFlags::NONE, None);
    end_of_results(view);
    check_end(view);
}

/// Loads the next page, unless one is loading. After a failure, only
/// `retry` (the footer's button) tries again, rather than every scroll.
fn load_more(view: &Rc<SearchView>, retry: bool) {
    let (query, next, generation) = {
        let state = view.state.borrow();
        if state.loading || state.failed && !retry || view.model.borrow().n_items() == 0 {
            return;
        }
        let Some(next) = state.next.clone() else {
            return;
        };
        (state.query.clone(), next, state.generation)
    };
    {
        let mut state = view.state.borrow_mut();
        state.loading = true;
        state.failed = false;
    }
    set_footer(view, Some(json!({ "state": "loading" })));
    let weak = Rc::downgrade(view);
    runtime::spawn_local(async move {
        let result = search::fetch_more(&next).await;
        let Some(view) = current(&weak, generation) else {
            return;
        };
        view.state.borrow_mut().loading = false;
        match result {
            Ok(mut page) => {
                page.fill_details(&view.state.borrow().details);
                search::publish(&query, false, &mut page);
                set_footer(&view, None);
                let rows: Vec<glib::BoxedAnyObject> = rows(&view, &page, false)
                    .into_iter()
                    .map(glib::BoxedAnyObject::new)
                    .collect();
                view.model.borrow().extend_from_slice(&rows);
                end_of_results(&view);
                check_end(&view);
                runtime::release_memory_soon();
            }
            Err(e) => {
                view.state.borrow_mut().failed = true;
                set_footer(&view, Some(json!({ "state": "error", "message": e })));
            }
        }
    });
}

/// The rows for a page of results, run through `search.rows`. Remembers
/// what the page brings for later pages.
fn rows(view: &SearchView, page: &SearchPage, first: bool) -> Vec<Row> {
    let mut state = view.state.borrow_mut();
    state.next = page.next.clone();
    for video in &page.details {
        state.details.insert(video.video.id.clone(), video.clone());
    }
    let mut rows = Vec::new();
    if first && let Some(correction) = &page.correction {
        rows.push(Row::new("search_correction", correction));
    }
    for item in &page.items {
        let (component, key) = match item {
            SearchItem::Video(v) => ("search_video", v.video.id.clone()),
            SearchItem::Channel(c) => (
                "search_channel",
                format!(
                    "channel:{}",
                    c.channel.id.as_deref().unwrap_or(&c.channel.name)
                ),
            ),
            SearchItem::Playlist(p) => ("search_playlist", format!("playlist:{}", p.id)),
            SearchItem::Shelf(s) if s.videos.is_empty() => continue,
            SearchItem::Shelf(s) => ("search_shelf", format!("shelf:{}", s.title)),
        };
        // Shelves come back on later pages ("Previously watched"); a title
        // is shown once.
        if state.seen.insert(key) {
            rows.push(Row::new(component, item));
        }
    }
    let context = json!({ "query": state.query, "first": first });
    drop(state);
    if let Err(e) = app()
        .ext
        .filters
        .apply_to("search.rows", context, "rows", &mut rows)
    {
        app().toast(&e);
    }
    rows
}

/// "No more results" once there is nothing left to load.
fn end_of_results(view: &SearchView) {
    if view.state.borrow().next.is_none() {
        set_footer(view, Some(json!({ "state": "end" })));
    }
}

/// Replaces the footer row (loading, an error, the end), or removes it.
fn set_footer(view: &SearchView, props: Option<Value>) {
    let model = view.model.borrow();
    let mut state = view.state.borrow_mut();
    if state.footer && model.n_items() > 0 {
        model.remove(model.n_items() - 1);
    }
    state.footer = props.is_some();
    if let Some(props) = props {
        model.append(&glib::BoxedAnyObject::new(Row::new("search_footer", props)));
    }
}

/// Loads more if the end of the results is near (or they don't fill the
/// view). Checked a moment later, once the list has measured what changed:
/// right after a new model its height still reads 0.
fn check_end(view: &Rc<SearchView>) {
    if view.checking.replace(true) {
        return;
    }
    let weak = Rc::downgrade(view);
    glib::timeout_add_local_once(END_CHECK, move || {
        let Some(view) = weak.upgrade() else { return };
        view.checking.set(false);
        let adj = view.scroller.vadjustment();
        if view.content.is_mapped()
            && adj.page_size() > 0.0
            && adj.value() + adj.page_size() > adj.upper() - PRELOAD_DISTANCE
        {
            load_more(&view, false);
        }
    });
}

fn current(weak: &Weak<SearchView>, generation: u64) -> Option<Rc<SearchView>> {
    weak.upgrade()
        .filter(|v| v.state.borrow().generation == generation)
}

enum Status {
    /// Nothing searched for.
    Empty,
    NoResults,
    Failed(String),
}

fn show_status(view: &SearchView, status: Status) {
    let filtered = !view.state.borrow().filters.is_empty();
    let (icon, title, description, button) = match &status {
        Status::Empty => ("yt-search-symbolic", "Search YouTube", String::new(), None),
        Status::NoResults => (
            "yt-search-symbolic",
            "No results found",
            if filtered {
                "Try different keywords or remove search filters".to_owned()
            } else {
                "Try different keywords".to_owned()
            },
            filtered.then_some("Remove filters"),
        ),
        Status::Failed(e) => (
            "network-error-symbolic",
            "Couldn't search",
            e.clone(),
            Some("Try again"),
        ),
    };
    view.status.set_icon_name(Some(icon));
    view.status.set_title(title);
    view.status
        .set_description(Some(description.as_str()).filter(|d| !d.is_empty()));
    view.status_button.set_visible(button.is_some());
    view.status_button.set_label(button.unwrap_or_default());
    if matches!(status, Status::NoResults) {
        view.status_button.add_css_class("clear-filters");
    } else {
        view.status_button.remove_css_class("clear-filters");
    }
    view.content.set_visible_child_name("status");
}

/// The chips, and the Filters button with the number of filters on.
fn show_header(view: &SearchView) {
    let state = view.state.borrow();
    while let Some(child) = view.chips.first_child() {
        view.chips.remove(&child);
    }
    if !state.chips.is_empty() {
        // Each response has chips with new tokens, and marks the one shown.
        let selected = state
            .chips
            .iter()
            .position(|c| c.selected)
            .or_else(|| state.chips.iter().position(|c| c.token == state.chip))
            .unwrap_or(0);
        let titles: Vec<&str> = state.chips.iter().map(|c| c.title.as_str()).collect();
        let props =
            json!({ "chips": titles, "selected": selected, "event": "search.chip_selected" });
        let bar = app().ext.components.build("chip_bar", &props);
        bar.set_hexpand(true);
        view.chips.append(&bar);
    }
    let count = filter_count(&state.filters);
    let content = adw::ButtonContent::builder()
        .icon_name("yt-tune-symbolic")
        .label(if count == 0 {
            "Filters".to_owned()
        } else {
            format!("Filters ({count})")
        })
        .build();
    view.filters.set_child(Some(&content));
    if count == 0 {
        view.filters.remove_css_class("active");
    } else {
        view.filters.add_css_class("active");
    }
}

fn filter_count(filters: &SearchFilters) -> usize {
    [
        filters.upload_date.is_some(),
        filters.kind.is_some(),
        filters.duration.is_some(),
        filters.sort != Default::default(),
    ]
    .into_iter()
    .filter(|on| *on)
    .count()
        + filters.features.len()
}

/// Builds each row with its component, reusing the widget when the row
/// shown before in that place had the same component. Rows are centered,
/// at most [`COLUMN_WIDTH`] wide.
fn row_factory() -> gtk::SignalListItemFactory {
    struct BuiltBy(String);

    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let item = item
            .downcast_ref::<gtk::ListItem>()
            .expect("a ListView item");
        let clamp = adw::Clamp::builder()
            .maximum_size(COLUMN_WIDTH)
            .tightening_threshold(COLUMN_WIDTH)
            .build();
        clamp.add_css_class("search-row");
        item.set_child(Some(&clamp));
        item.set_activatable(false);
        item.set_focusable(false);
    });
    factory.connect_bind(|_, item| {
        let item = item
            .downcast_ref::<gtk::ListItem>()
            .expect("a ListView item");
        let (Some(object), Some(clamp)) = (
            item.item().and_downcast::<glib::BoxedAnyObject>(),
            item.child().and_downcast::<adw::Clamp>(),
        ) else {
            return;
        };
        let row = object.borrow::<Row>();
        let components = &app().ext.components;
        let reused = clamp.child().is_some_and(|child| {
            attached::<BuiltBy>(&child).is_some_and(|b| b.0 == row.component)
                && components.update(&row.component, &child, &row.props)
        });
        if !reused {
            let widget = components.build(&row.component, &row.props);
            attach(&widget, Rc::new(BuiltBy(row.component.clone())));
            clamp.set_child(Some(&widget));
        }
    });
    factory
}

fn run(command: &str, args: Value) {
    if let Err(e) = app().ext.commands.run(command, args) {
        app().toast(&e);
    }
}
