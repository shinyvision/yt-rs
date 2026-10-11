//! The `playlist` component (the "playlist" page, `{ playlist_id }`), as
//! youtube.com lays it out: on the left a panel with the playlist's
//! picture, title, owner, privacy, video count and views, "Play all" and
//! "Shuffle", and a menu (deleting the user's own playlists); on the
//! right its videos, numbered, more loading as the list scrolls. On a
//! narrow window the panel goes above the list.
//!
//! A video taken out of the playlist (`playlist.changed`) leaves the list
//! at once. Clicking a video plays the playlist from it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use serde_json::{Value, json};
use yt_core::feed::Video;
use yt_core::playlist::{LIKED, PlaylistPage, WATCH_LATER};

use super::{attach, attached, follow};
use crate::app::app;
use crate::runtime;

/// Narrower than this, the panel goes above the list.
const NARROW: f64 = 1000.0;
/// Loads more when the end is this close, in pixels.
const PRELOAD_DISTANCE: f64 = 800.0;

struct View {
    playlist_id: RefCell<String>,
    page: RefCell<Option<PlaylistPage>>,
    continuation: RefCell<Option<String>>,
    loading: Cell<bool>,
    generation: Cell<u64>,
    content: gtk::Stack,
    status: adw::StatusPage,
    layout: gtk::Box,
    picture: gtk::Picture,
    title: gtk::Label,
    owner: gtk::Label,
    meta: gtk::Label,
    updated: gtk::Label,
    description: gtk::Label,
    menu: gtk::MenuButton,
    model: gtk::gio::ListStore,
    scroller: gtk::ScrolledWindow,
    thumb: super::aspect::AspectBin,
}

/// The panel's picture beside the list, as youtube.com's.
const THUMB_WIDTH: i32 = 312;

pub fn build(props: &Value) -> gtk::Widget {
    let picture = gtk::Picture::new();
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.set_can_shrink(true);
    let thumb = super::aspect::AspectBin::new(9.0 / 16.0, &picture);
    thumb.add_css_class("playlist-thumb");
    thumb.set_overflow(gtk::Overflow::Hidden);
    thumb.set_natural_width(THUMB_WIDTH);
    thumb.set_max_width(THUMB_WIDTH);

    let label = |class: &str| {
        let l = gtk::Label::new(None);
        l.add_css_class(class);
        l.set_xalign(0.0);
        l.set_wrap(true);
        l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        l
    };
    let title = label("playlist-title");
    let owner = label("playlist-owner");
    let meta = label("playlist-meta");
    meta.add_css_class("dim");
    let updated = label("playlist-meta");
    updated.add_css_class("dim");
    let description = label("playlist-description");

    let play = pill("yt-play-symbolic", "Play all", "playlist-play");
    play.add_css_class("primary");
    let shuffle = pill("yt-shuffle-symbolic", "Shuffle", "playlist-shuffle");
    let menu = gtk::MenuButton::new();
    menu.set_icon_name("yt-more-symbolic");
    menu.add_css_class("icon-button");
    menu.set_tooltip_text(Some("More"));
    menu.set_valign(gtk::Align::Center);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.add_css_class("playlist-buttons");
    play.set_hexpand(true);
    shuffle.set_hexpand(true);
    buttons.append(&play);
    buttons.append(&shuffle);
    buttons.append(&menu);

    let panel = gtk::Box::new(gtk::Orientation::Vertical, 12);
    panel.add_css_class("playlist-panel");
    panel.set_valign(gtk::Align::Start);
    // Its own width beside the list: the buttons expanding inside mustn't
    // make it take the list's room.
    panel.set_hexpand(false);

    panel.append(&thumb);
    panel.append(&title);
    panel.append(&owner);
    panel.append(&meta);
    panel.append(&updated);
    panel.append(&buttons);
    panel.append(&description);
    panel.append(
        &app()
            .ext
            .slots
            .create("playlist.panel", gtk::Orientation::Vertical, 8),
    );

    let model = gtk::gio::ListStore::new::<glib::BoxedAnyObject>();
    let list = gtk::ListView::new(
        Some(gtk::NoSelection::new(Some(model.clone()))),
        Some(row_factory()),
    );
    list.add_css_class("playlist-videos");
    list.set_widget_name("playlist-videos");
    list.set_can_focus(false);
    let scroller = gtk::ScrolledWindow::builder()
        .child(&list)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .hexpand(true)
        .vexpand(true)
        .build();

    let layout = gtk::Box::new(gtk::Orientation::Horizontal, 24);
    layout.add_css_class("playlist-layout");
    layout.append(&panel);
    layout.append(&scroller);

    let spinner = adw::Spinner::new();
    spinner.set_size_request(48, 48);
    spinner.set_halign(gtk::Align::Center);
    spinner.set_valign(gtk::Align::Center);
    let status = adw::StatusPage::new();
    let content = gtk::Stack::new();
    content.add_named(&spinner, Some("loading"));
    content.add_named(&status, Some("status"));
    content.add_named(&layout, Some("playlist"));

    let root = adw::BreakpointBin::new();
    root.set_size_request(360, 300);
    root.set_widget_name("page-playlist");
    root.add_css_class("playlist-page");
    root.set_child(Some(&content));

    let view = Rc::new(View {
        playlist_id: RefCell::default(),
        page: RefCell::default(),
        continuation: RefCell::default(),
        loading: Cell::new(false),
        generation: Cell::new(0),
        content,
        status,
        layout,
        picture,
        title,
        owner,
        meta,
        updated,
        description,
        menu,
        model,
        scroller,
        thumb,
    });
    attach(&root, view.clone());
    connect(&view, &root, &play, &shuffle);
    show_playlist(&view, props);
    root.upcast()
}

pub fn update(widget: &gtk::Widget, props: &Value) {
    if let Some(view) = attached::<View>(widget) {
        show_playlist(&view, props);
    }
}

/// A rounded button with an icon and a label.
fn pill(icon: &str, label: &str, name: &str) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    content.set_halign(gtk::Align::Center);
    content.append(&gtk::Image::from_icon_name(icon));
    content.append(&gtk::Label::new(Some(label)));
    let button = gtk::Button::new();
    button.set_child(Some(&content));
    button.add_css_class("pill-button");
    button.add_css_class("playlist-button");
    button.set_widget_name(name);
    button
}

fn connect(view: &Rc<View>, root: &adw::BreakpointBin, play: &gtk::Button, shuffle: &gtk::Button) {
    let breakpoint = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        NARROW,
        adw::LengthUnit::Px,
    ));
    breakpoint.add_setter(
        &view.layout,
        "orientation",
        Some(&gtk::Orientation::Vertical.to_value()),
    );
    // Beside the list, the panel's picture is its width (the box measures
    // the panel wider while sharing out room); above it, the page's.
    let thumb = view.thumb.clone();
    breakpoint.connect_apply(move |_| thumb.set_max_width(0));
    let thumb = view.thumb.clone();
    breakpoint.connect_unapply(move |_| thumb.set_max_width(THUMB_WIDTH));
    root.add_breakpoint(breakpoint);

    for (button, shuffle) in [(play, false), (shuffle, true)] {
        let weak = Rc::downgrade(view);
        button.connect_clicked(move |_| {
            let Some(view) = weak.upgrade() else { return };
            let args = json!({ "playlist_id": *view.playlist_id.borrow(), "shuffle": shuffle });
            if let Err(e) = app().ext.commands.run("playlist.play", args) {
                app().toast(&e);
            }
        });
    }

    let weak = Rc::downgrade(view);
    view.scroller
        .vadjustment()
        .connect_value_changed(move |adj| {
            if adj.value() + adj.page_size() > adj.upper() - PRELOAD_DISTANCE
                && let Some(view) = weak.upgrade()
            {
                load_more(&view);
            }
        });

    // Taken out here or elsewhere: gone from the list.
    let weak = Rc::downgrade(view);
    follow(root, "playlist.changed", move |payload| {
        let Some(view) = weak.upgrade() else { return };
        let ours = payload.get("playlist_id").and_then(Value::as_str)
            == Some(view.playlist_id.borrow().as_str());
        let removed = payload.get("added").and_then(Value::as_bool) == Some(false);
        let Some(video_id) = payload.get("video_id").and_then(Value::as_str) else {
            return;
        };
        if ours && removed {
            remove(&view, video_id);
        } else if ours {
            // Added: the list is out of date; read it again.
            reload(&view);
        }
    });
    let weak = Rc::downgrade(view);
    follow(root, "playlist.deleted", move |payload| {
        let Some(view) = weak.upgrade() else { return };
        if payload.get("playlist_id").and_then(Value::as_str)
            == Some(view.playlist_id.borrow().as_str())
            && !app().back()
        {
            app().navigate("playlists", Value::Null, false);
        }
    });
}

fn show_playlist(view: &Rc<View>, props: &Value) {
    let id = props
        .get("playlist_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    if id == *view.playlist_id.borrow() && view.page.borrow().is_some() {
        return;
    }
    *view.playlist_id.borrow_mut() = id;
    reload(view);
}

fn reload(view: &Rc<View>) {
    let generation = view.generation.get() + 1;
    view.generation.set(generation);
    view.loading.set(true);
    if view.page.borrow().is_none() {
        view.content.set_visible_child_name("loading");
    }
    let id = view.playlist_id.borrow().clone();
    let weak = Rc::downgrade(view);
    runtime::spawn_local(async move {
        let youtube = app().youtube.clone();
        let playlist_id = id.clone();
        let result = runtime::background(async move { youtube.playlist(&playlist_id).await }).await;
        let Some(view) = weak.upgrade().filter(|v| v.generation.get() == generation) else {
            return;
        };
        view.loading.set(false);
        match result {
            Ok(page) => {
                app().ext.events.emit(
                    "playlist.loaded",
                    &json!({ "playlist_id": id, "page": page }),
                );
                fill(&view, page);
            }
            // Watch later and Liked videos signed out.
            Err(yt_core::Error::NotSignedIn) => {
                let (icon, title, description) = super::signin::wording("playlist");
                view.status.set_icon_name(Some(icon));
                view.status.set_title(title);
                view.status.set_description(Some(description));
                let button = super::signin::button();
                button.set_halign(gtk::Align::Center);
                view.status.set_child(Some(&button));
                view.content.set_visible_child_name("status");
            }
            Err(e) => {
                view.status.set_child(None::<&gtk::Widget>);
                view.status.set_title("Couldn't load the playlist");
                view.status.set_description(Some(&e.to_string()));
                view.content.set_visible_child_name("status");
            }
        }
    });
}

fn fill(view: &Rc<View>, page: PlaylistPage) {
    let playlist = &page.playlist;
    view.title.set_label(&playlist.title);
    view.owner
        .set_label(page.owner.as_deref().unwrap_or_default());
    view.owner.set_visible(page.owner.is_some());
    let privacy = page.privacy.as_deref().map(|p| match p {
        "private" => "Private",
        "unlisted" => "Unlisted",
        "public" => "Public",
        other => other,
    });
    let meta: Vec<&str> = [
        Some("Playlist"),
        privacy,
        playlist.count.as_deref(),
        page.views.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect();
    view.meta.set_label(&meta.join(" • "));
    view.updated
        .set_label(page.updated.as_deref().unwrap_or_default());
    view.updated.set_visible(page.updated.is_some());
    view.description
        .set_label(page.description.as_deref().unwrap_or_default());
    view.description.set_visible(page.description.is_some());
    app().images.load_into(
        &view.picture,
        playlist.thumbnail.as_deref(),
        640 * view.picture.scale_factor(),
    );
    let mut items = Vec::new();
    if page.editable && ![WATCH_LATER, LIKED].contains(&playlist.id.as_str()) {
        items.push(json!({
            "label": "Delete playlist",
            "icon": "yt-delete-symbolic",
            "command": "playlist.delete",
            "args": { "playlist_id": playlist.id },
            "confirm": {
                "title": "Delete playlist",
                "body": format!("Are you sure you want to delete {}?\n\nNote: Deleting playlists is a permanent action and cannot be undone.", playlist.title),
                "confirm": "Delete",
            },
        }));
    }
    items.push(json!({ "label": "Copy link", "icon": "yt-link-symbolic", "command": "app.copy_text", "args": { "text": playlist.url() } }));
    items.push(json!({ "label": "Open in browser", "icon": "yt-open-symbolic", "command": "app.open_url", "args": { "url": playlist.url() } }));
    let menu_items = Value::Array(items);
    super::menu::attach(&view.menu, move || menu_items.clone());

    let objects: Vec<glib::BoxedAnyObject> = page
        .videos
        .iter()
        .cloned()
        .map(|video| {
            glib::BoxedAnyObject::new(Entry {
                video,
                playlist_id: playlist.id.clone(),
            })
        })
        .collect();
    view.model.splice(0, view.model.n_items(), &objects);
    *view.continuation.borrow_mut() = page.continuation.clone();
    *view.page.borrow_mut() = Some(page);
    view.content.set_visible_child_name("playlist");
    if view.scroller.vadjustment().value() > 0.0 {
        view.scroller.vadjustment().set_value(0.0);
    }
}

fn load_more(view: &Rc<View>) {
    if view.loading.get() {
        return;
    }
    let Some(token) = view.continuation.borrow().clone() else {
        return;
    };
    view.loading.set(true);
    let (weak, generation) = (Rc::downgrade(view), view.generation.get());
    runtime::spawn_local(async move {
        let youtube = app().youtube.clone();
        let result = runtime::background(async move { youtube.continuation(&token).await }).await;
        let Some(view) = weak.upgrade().filter(|v| v.generation.get() == generation) else {
            return;
        };
        view.loading.set(false);
        let Ok(more) = result else { return };
        let playlist_id = view.playlist_id.borrow().clone();
        for video in more.shelves.into_iter().flat_map(|s| s.videos) {
            if let Some(page) = view.page.borrow_mut().as_mut() {
                page.videos.push(video.clone());
            }
            view.model.append(&glib::BoxedAnyObject::new(Entry {
                video,
                playlist_id: playlist_id.clone(),
            }));
        }
        *view.continuation.borrow_mut() = more.continuation;
    });
}

/// Takes the video out of the list (and of the page's videos).
fn remove(view: &View, video_id: &str) {
    if let Some(page) = view.page.borrow_mut().as_mut() {
        page.videos.retain(|v| v.id != video_id);
    }
    let position = (0..view.model.n_items()).find(|&i| {
        view.model
            .item(i)
            .and_downcast::<glib::BoxedAnyObject>()
            .is_some_and(|o| o.borrow::<Entry>().video.id == video_id)
    });
    if let Some(position) = position {
        view.model.remove(position);
    }
}

/// A row's video, and the playlist it is in.
struct Entry {
    video: Video,
    playlist_id: String,
}

/// Rows: the video's place in the list, then the video as a compact card,
/// which knows its playlist and place (so a click plays the playlist from
/// it).
fn row_factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
        let index = gtk::Label::new(None);
        index.add_css_class("playlist-index");
        index.add_css_class("dim");
        index.set_valign(gtk::Align::Center);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("playlist-row");
        row.append(&index);
        row.append(
            &app()
                .ext
                .components
                .build("video_card", &json!({ "layout": "compact" })),
        );
        item.set_child(Some(&row));
        item.set_activatable(false);
        item.set_focusable(false);
    });
    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
        let (Some(row), Some(object)) = (
            item.child(),
            item.item().and_downcast::<glib::BoxedAnyObject>(),
        ) else {
            return;
        };
        let Some(index) = row.first_child().and_downcast::<gtk::Label>() else {
            return;
        };
        index.set_label(&(item.position() + 1).to_string());
        let entry = object.borrow::<Entry>();
        let mut props = serde_json::to_value(&entry.video).unwrap_or_default();
        props["playlist_id"] = json!(entry.playlist_id);
        props["playlist_index"] = json!(item.position());
        let components = &app().ext.components;
        if let Some(card) = index.next_sibling()
            && !components.update("video_card", &card, &props)
        {
            row.downcast_ref::<gtk::Box>()
                .expect("a row box")
                .remove(&card);
            let mut build = props.clone();
            build["layout"] = json!("compact");
            row.downcast_ref::<gtk::Box>()
                .expect("a row box")
                .append(&components.build("video_card", &build));
        }
    });
    factory
}
