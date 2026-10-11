//! The rows of the search page, as on youtube.com:
//!
//! - `search_video`: a large thumbnail (with its length, a note for music,
//!   and how much has been watched), then the title, views and age, the
//!   channel with its avatar, the description's matching part with the
//!   query in bold, and badges ("4K", "CC", "New").
//! - `search_channel`: a large avatar, the name, handle and subscribers,
//!   and a subscribe button.
//! - `search_playlist`: a stacked thumbnail with its number of videos.
//! - `search_shelf`: a titled group, "People also watched" (a few rows, then
//!   "Show more") or a row of Shorts.
//! - `search_correction`: "Showing results for ..." or "Did you mean ...".
//! - `search_footer`: loading more, a retry button, or the end.
//!
//! Each takes the item's JSON as properties. Videos, channels and playlists
//! have an `update`, so the list refills them as it scrolls.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use serde_json::{Value, json};
use yt_core::channel::{ChannelRef, Subscription};
use yt_core::search::{Correction, SearchChannel, SearchPlaylist, SearchShelf, SearchVideo};
use yt_core::text::{RichText, Run};

use super::aspect::AspectBin;
use super::split::SplitBox;
use super::{attach, attached, avatar, follow, video_input};
use crate::app::app;

/// The widest a result's thumbnail gets (youtube.com's), and the width it
/// is decoded at.
const THUMBNAIL_WIDTH: i32 = 500;
const CHANNEL_AVATAR: i32 = 136;
/// A shelf shows this many rows until "Show more".
const SHELF_FOLDED: usize = 3;
const SHORT_WIDTH: i32 = 210;

// Videos

struct VideoRow {
    picture: gtk::Picture,
    duration: gtk::Box,
    duration_icon: gtk::Image,
    duration_label: gtk::Label,
    watched: gtk::ProgressBar,
    title: gtk::Label,
    meta: gtk::Label,
    owner: gtk::Box,
    avatar: adw::Avatar,
    channel: gtk::Label,
    verified: gtk::Image,
    snippet: gtk::Label,
    badges: gtk::Box,
    video: RefCell<Value>,
}

pub fn build_video(props: &Value) -> gtk::Widget {
    let (thumb, picture, duration, duration_icon, duration_label, watched) = thumbnail();

    let title = text_label("search-title", 2);
    let menu = menu_button();
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    head.append(&title);
    head.append(&menu);
    let meta = text_label("meta", 1);
    meta.add_css_class("dim");

    let avatar = adw::Avatar::new(24, None, true);
    let channel = text_label("meta", 1);
    channel.add_css_class("dim");
    channel.set_hexpand(false);
    let verified = gtk::Image::from_icon_name("yt-verified-symbolic");
    verified.add_css_class("verified");
    let owner = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    owner.add_css_class("search-owner");
    owner.add_css_class("channel-link");
    owner.set_halign(gtk::Align::Start);
    owner.set_tooltip_text(Some("Go to channel"));
    owner.append(&avatar);
    owner.append(&channel);
    owner.append(&verified);

    let snippet = text_label("search-snippet", 2);
    snippet.add_css_class("dim");
    let badges = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    badges.add_css_class("search-badges");

    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.add_css_class("search-text");
    for part in [
        head.upcast_ref::<gtk::Widget>(),
        meta.upcast_ref(),
        owner.upcast_ref(),
        snippet.upcast_ref(),
        badges.upcast_ref(),
    ] {
        text.append(part);
    }

    let root = SplitBox::new(&thumb, &text);
    root.add_css_class("search-video");
    root.set_cursor_from_name(Some("pointer"));
    let row = Rc::new(VideoRow {
        picture,
        duration,
        duration_icon,
        duration_label,
        watched,
        title,
        meta,
        owner,
        avatar,
        channel,
        verified,
        snippet,
        badges,
        video: RefCell::new(Value::Null),
    });
    attach(&root, row.clone());

    let weak = Rc::downgrade(&row);
    on_click(&row.owner, move || {
        if let Some(row) = weak.upgrade() {
            let video = row.video.borrow();
            run(
                "channel.open",
                json!({
                    "channel_id": video.pointer("/owner/id"),
                    "name": video.get("channel"),
                    "video_id": video.get("id"),
                }),
            );
        }
    });
    // The channel's avatar, once the channel directory has looked it up.
    let weak = Rc::downgrade(&row);
    follow(&root, "channels.learned", move |payload| {
        if let Some(row) = weak.upgrade()
            && payload.get("name").and_then(Value::as_str) == Some(row.channel.label().as_str())
            && let Some(url) = payload.pointer("/channel/avatar").and_then(Value::as_str)
        {
            avatar::set(&row.avatar, &row.channel.label(), Some(url));
        }
    });
    let video = |row: &Rc<VideoRow>| {
        let weak = Rc::downgrade(row);
        move || weak.upgrade().map(|row| row.video.borrow().clone())
    };
    video_input::connect(&root, video(&row));
    video_input::attach_menu(&menu, video(&row));
    fill_video(&root, &row, props);
    root.upcast()
}

pub fn update_video(widget: &gtk::Widget, props: &Value) {
    if let Some(row) = attached::<VideoRow>(widget) {
        fill_video(widget, &row, props);
    }
}

fn fill_video(root: &impl IsA<gtk::Widget>, row: &VideoRow, props: &Value) {
    *row.video.borrow_mut() = props.clone();
    let video: SearchVideo = serde_json::from_value(props.clone()).unwrap_or_default();
    let v = &video.video;
    row.title.set_label(&v.title);
    row.title
        .set_tooltip_text(Some(v.title.as_str()).filter(|t| !t.is_empty()));
    row.meta
        .set_label(&dotted(&[v.views.as_deref(), v.published.as_deref()]));
    row.meta.set_visible(!row.meta.label().is_empty());

    let owner = video.owner.clone().unwrap_or_else(|| ChannelRef {
        name: v.channel.clone().unwrap_or_default(),
        ..ChannelRef::default()
    });
    if owner.avatar.is_some() {
        app().channels.remember(&owner);
    }
    let picture = owner
        .avatar
        .clone()
        .or_else(|| app().channels.lookup(&owner.name).and_then(|c| c.avatar));
    if picture.is_none() {
        app().channels.want(&owner.name, &v.id);
    }
    avatar::set(&row.avatar, &owner.name, picture.as_deref());
    row.channel.set_label(&owner.name);
    row.verified.set_visible(owner.verified);
    row.owner.set_visible(!owner.name.is_empty());

    match &video.snippet {
        Some(snippet) if !snippet.is_empty() => {
            row.snippet.set_markup(&plain_markup(snippet));
            row.snippet.set_visible(true);
        }
        _ => row.snippet.set_visible(false),
    }
    show_badges(&row.badges, &video.badges);

    let live = v.live || v.duration.as_deref() == Some("LIVE");
    let length = v.duration.clone().filter(|d| !d.is_empty());
    row.duration_label.set_label(if live {
        "LIVE"
    } else {
        length.as_deref().unwrap_or_default()
    });
    row.duration.set_visible(live || length.is_some());
    row.duration_icon.set_visible(video.music && !live);
    if live {
        row.duration.add_css_class("live");
    } else {
        row.duration.remove_css_class("live");
    }
    let progress = v.progress.unwrap_or(0.0) as f64;
    row.watched.set_fraction(progress);
    row.watched.set_visible(progress > 0.0);
    app().images.load_into(
        &row.picture,
        v.thumbnail.as_deref(),
        THUMBNAIL_WIDTH * root.scale_factor(),
    );
}

/// A 16:9 thumbnail with the video's length and watched progress.
fn thumbnail() -> (
    AspectBin,
    gtk::Picture,
    gtk::Box,
    gtk::Image,
    gtk::Label,
    gtk::ProgressBar,
) {
    let picture = gtk::Picture::new();
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.set_can_shrink(true);
    let icon = gtk::Image::from_icon_name("yt-music-symbolic");
    let label = gtk::Label::new(None);
    let duration = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    duration.add_css_class("duration");
    duration.set_halign(gtk::Align::End);
    duration.set_valign(gtk::Align::End);
    duration.append(&icon);
    duration.append(&label);
    let watched = gtk::ProgressBar::new();
    watched.add_css_class("watched");
    watched.set_valign(gtk::Align::End);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&picture));
    overlay.add_overlay(&duration);
    overlay.add_overlay(&watched);
    let thumb = AspectBin::new(9.0 / 16.0, &overlay);
    thumb.set_natural_width(THUMBNAIL_WIDTH);
    thumb.add_css_class("thumb");
    thumb.add_css_class("search-thumb");
    thumb.set_overflow(gtk::Overflow::Hidden);
    thumb.set_valign(gtk::Align::Start);
    (thumb, picture, duration, icon, label, watched)
}

fn show_badges(container: &gtk::Box, badges: &[String]) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    for badge in badges {
        let label = gtk::Label::new(Some(badge));
        label.add_css_class("search-badge");
        container.append(&label);
    }
    container.set_visible(!badges.is_empty());
}

// Channels

struct ChannelRow {
    avatar: adw::Avatar,
    name: gtk::Label,
    verified: gtk::Image,
    meta: gtk::Label,
    description: gtk::Label,
    subscribe: gtk::Box,
    channel: RefCell<SearchChannel>,
}

pub fn build_channel(props: &Value) -> gtk::Widget {
    let avatar = adw::Avatar::new(CHANNEL_AVATAR, None, true);
    let start = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    start.set_halign(gtk::Align::Center);
    start.append(&avatar);

    let name = text_label("search-title", 1);
    name.set_hexpand(false);
    let verified = gtk::Image::from_icon_name("yt-verified-symbolic");
    verified.add_css_class("verified");
    let title = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    title.append(&name);
    title.append(&verified);
    let meta = text_label("meta", 2);
    meta.add_css_class("dim");
    let description = text_label("search-snippet", 2);
    description.add_css_class("dim");
    let text = gtk::Box::new(gtk::Orientation::Vertical, 4);
    text.add_css_class("search-text");
    text.set_valign(gtk::Align::Center);
    text.set_hexpand(true);
    text.append(&title);
    text.append(&meta);
    text.append(&description);
    let subscribe = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    subscribe.set_valign(gtk::Align::Center);
    let end = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    end.set_valign(gtk::Align::Center);
    end.append(&text);
    end.append(&subscribe);

    let root = SplitBox::new(&start, &end);
    root.add_css_class("search-channel");
    root.set_cursor_from_name(Some("pointer"));
    let row = Rc::new(ChannelRow {
        avatar,
        name,
        verified,
        meta,
        description,
        subscribe,
        channel: RefCell::default(),
    });
    attach(&root, row.clone());
    let weak = Rc::downgrade(&row);
    on_click(&root, move || {
        if let Some(row) = weak.upgrade() {
            let c = row.channel.borrow().channel.clone();
            run(
                "channel.open",
                json!({ "channel_id": c.id, "handle": c.handle, "name": c.name }),
            );
        }
    });
    fill_channel(&row, props);
    root.upcast()
}

pub fn update_channel(widget: &gtk::Widget, props: &Value) {
    if let Some(row) = attached::<ChannelRow>(widget) {
        fill_channel(&row, props);
    }
}

fn fill_channel(row: &ChannelRow, props: &Value) {
    let item: SearchChannel = serde_json::from_value(props.clone()).unwrap_or_default();
    let c = &item.channel;
    app().channels.remember(c);
    avatar::set(&row.avatar, &c.name, c.avatar.as_deref());
    row.name.set_label(&c.name);
    row.verified.set_visible(c.verified);
    row.meta.set_label(&dotted(&[
        c.handle.as_deref(),
        c.subscribers.as_deref(),
        item.video_count.as_deref(),
    ]));
    match &item.description {
        Some(d) if !d.is_empty() => {
            row.description.set_markup(&plain_markup(d));
            row.description.set_visible(true);
        }
        _ => row.description.set_visible(false),
    }
    while let Some(child) = row.subscribe.first_child() {
        row.subscribe.remove(&child);
    }
    if let Some(id) = &c.id {
        let subscribed = app()
            .channels
            .subscriptions()
            .iter()
            .any(|s| s.id.as_ref() == Some(id));
        let subscription = Subscription {
            channel_id: id.clone(),
            subscribed,
            ..Subscription::default()
        };
        let props = json!({ "subscription": subscription, "channel_name": c.name });
        row.subscribe
            .append(&app().ext.components.build("subscribe_button", &props));
    }
    *row.channel.borrow_mut() = item;
}

// Playlists

struct PlaylistRow {
    picture: gtk::Picture,
    count: gtk::Label,
    title: gtk::Label,
    meta: gtk::Label,
    playlist: RefCell<SearchPlaylist>,
}

pub fn build_playlist(props: &Value) -> gtk::Widget {
    let picture = gtk::Picture::new();
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.set_can_shrink(true);
    let count = gtk::Label::new(None);
    let badge = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    badge.add_css_class("duration");
    badge.set_halign(gtk::Align::End);
    badge.set_valign(gtk::Align::End);
    badge.append(&gtk::Image::from_icon_name("yt-playlist-symbolic"));
    badge.append(&count);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&picture));
    overlay.add_overlay(&badge);
    let thumb = AspectBin::new(9.0 / 16.0, &overlay);
    thumb.set_natural_width(THUMBNAIL_WIDTH);
    thumb.add_css_class("thumb");
    thumb.set_overflow(gtk::Overflow::Hidden);
    // The cards stacked behind it.
    let stack = gtk::Box::new(gtk::Orientation::Vertical, 0);
    stack.add_css_class("playlist-stack");
    stack.set_valign(gtk::Align::Start);
    for depth in ["back", "middle"] {
        let card = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        card.add_css_class(&format!("playlist-stack-{depth}"));
        stack.append(&card);
    }
    stack.append(&thumb);

    let title = text_label("search-title", 2);
    let menu = menu_button();
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    head.append(&title);
    head.append(&menu);
    let meta = text_label("meta", 1);
    meta.add_css_class("dim");
    let view = gtk::Label::new(Some("View full playlist"));
    view.add_css_class("search-view-playlist");
    view.set_xalign(0.0);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 4);
    text.add_css_class("search-text");
    text.append(&head);
    text.append(&meta);
    text.append(&view);

    let root = SplitBox::new(&stack, &text);
    root.add_css_class("search-playlist");
    root.set_cursor_from_name(Some("pointer"));
    let row = Rc::new(PlaylistRow {
        picture,
        count,
        title,
        meta,
        playlist: RefCell::default(),
    });
    attach(&root, row.clone());
    let weak = Rc::downgrade(&row);
    on_click(&root, move || {
        if let Some(row) = weak.upgrade() {
            open_playlist(&row.playlist.borrow());
        }
    });
    let weak = Rc::downgrade(&row);
    super::menu::attach(&menu, move || {
        weak.upgrade()
            .map_or_else(|| json!([]), |row| playlist_menu(&row.playlist.borrow()))
    });
    fill_playlist(&root, &row, props);
    root.upcast()
}

pub fn update_playlist(widget: &gtk::Widget, props: &Value) {
    if let Some(row) = attached::<PlaylistRow>(widget) {
        fill_playlist(widget, &row, props);
    }
}

fn fill_playlist(root: &impl IsA<gtk::Widget>, row: &PlaylistRow, props: &Value) {
    let playlist: SearchPlaylist = serde_json::from_value(props.clone()).unwrap_or_default();
    row.title.set_label(&playlist.title);
    row.count
        .set_label(playlist.video_count.as_deref().unwrap_or("Playlist"));
    row.meta
        .set_label(&dotted(&[playlist.channel.as_deref(), Some("Playlist")]));
    app().images.load_into(
        &row.picture,
        playlist.thumbnail.as_deref(),
        THUMBNAIL_WIDTH * root.scale_factor(),
    );
    *row.playlist.borrow_mut() = playlist;
}

fn playlist_url(playlist: &SearchPlaylist) -> String {
    format!("https://www.youtube.com/playlist?list={}", playlist.id)
}

/// Plays the playlist's first video; without one, opens it on youtube.com.
fn open_playlist(playlist: &SearchPlaylist) {
    match &playlist.first_video {
        Some(id) => run("video.play", json!({ "video": { "id": id } })),
        None => run("app.open_url", json!({ "url": playlist_url(playlist) })),
    }
}

/// Through the `playlist.menu` filter: `{ playlist, items }`.
fn playlist_menu(playlist: &SearchPlaylist) -> Value {
    let url = playlist_url(playlist);
    let mut items = vec![
        json!({ "label": "Open in browser", "icon": "yt-open-symbolic", "command": "app.open_url", "args": { "url": url } }),
        json!({ "label": "Copy link", "icon": "yt-link-symbolic", "command": "app.copy_text", "args": { "text": url, "toast": "Link copied" } }),
    ];
    if let Some(id) = &playlist.first_video {
        items.insert(
            0,
            json!({ "label": "Play", "icon": "yt-play-symbolic", "command": "video.play", "args": { "video": { "id": id } } }),
        );
    }
    let value = app().ext.filters.apply(
        "playlist.menu",
        json!({ "playlist": playlist, "items": items }),
    );
    value.get("items").cloned().unwrap_or_else(|| json!([]))
}

// Shelves

pub fn build_shelf(props: &Value) -> gtk::Widget {
    let shelf: SearchShelf = serde_json::from_value(props.clone()).unwrap_or_default();
    let title = gtk::Label::new(Some(&shelf.title));
    title.add_css_class("search-shelf-title");
    title.set_xalign(0.0);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    if shelf.shorts {
        let icon = gtk::Image::from_icon_name("yt-shorts-symbolic");
        icon.add_css_class("shorts-icon");
        header.append(&icon);
    }
    header.append(&title);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 16);
    root.add_css_class("search-shelf");
    root.append(&header);
    if shelf.shorts {
        root.add_css_class("shorts");
        root.append(&shorts_row(&shelf.videos));
    } else {
        root.append(&shelf_rows(&shelf.videos));
    }
    root.upcast()
}

/// The shelf's first rows, and "Show more" for the rest.
fn shelf_rows(videos: &[SearchVideo]) -> gtk::Widget {
    let props: Vec<Value> = videos
        .iter()
        .map(|v| serde_json::to_value(v).unwrap_or_default())
        .collect();
    let list = gtk::Box::new(gtk::Orientation::Vertical, 16);
    let add = |list: &gtk::Box, props: &[Value]| {
        for video in props {
            list.append(&app().ext.components.build("search_video", video));
        }
    };
    add(&list, &props[..props.len().min(SHELF_FOLDED)]);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.append(&list);
    if props.len() > SHELF_FOLDED {
        let more = adw::ButtonContent::builder()
            .icon_name("yt-expand-symbolic")
            .label(format!("Show {} more", props.len() - SHELF_FOLDED))
            .build();
        let button = gtk::Button::builder()
            .child(&more)
            .css_classes(["flat", "search-shelf-more"])
            .halign(gtk::Align::Fill)
            .build();
        let rest = props[SHELF_FOLDED..].to_vec();
        button.connect_clicked(move |button| {
            add(&list, &rest);
            button.set_visible(false);
        });
        root.append(&button);
    }
    root.upcast()
}

/// A row of Shorts that scrolls sideways, with arrows on the sides.
fn shorts_row(videos: &[SearchVideo]) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    for video in videos {
        row.append(&short_card(video));
    }
    let scroller = gtk::ScrolledWindow::builder()
        .child(&row)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .hscrollbar_policy(gtk::PolicyType::External)
        .build();
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&scroller));
    for (icon, align, direction) in [
        ("yt-chevron-left-symbolic", gtk::Align::Start, -1.0),
        ("yt-chevron-right-symbolic", gtk::Align::End, 1.0),
    ] {
        let arrow = gtk::Button::from_icon_name(icon);
        arrow.add_css_class("shelf-arrow");
        arrow.set_halign(align);
        arrow.set_valign(gtk::Align::Center);
        let (adj, target) = (scroller.hadjustment(), scroller.downgrade());
        arrow.connect_clicked(move |_| {
            if let Some(scroller) = target.upgrade() {
                scroll_by(
                    scroller.upcast_ref(),
                    &adj,
                    direction * adj.page_size() * 0.8,
                );
            }
        });
        let adj = scroller.hadjustment();
        let shown = arrow.downgrade();
        let update = move |adj: &gtk::Adjustment| {
            if let Some(arrow) = shown.upgrade() {
                let room = adj.upper() - adj.page_size();
                arrow.set_visible(if direction < 0.0 {
                    adj.value() > 1.0
                } else {
                    adj.value() < room - 1.0
                });
            }
        };
        adj.connect_value_changed(update.clone());
        adj.connect_changed(update);
        overlay.add_overlay(&arrow);
    }
    overlay.upcast()
}

/// Scrolls `adj` smoothly by `by`, one animation at a time.
fn scroll_by(widget: &gtk::Widget, adj: &gtk::Adjustment, by: f64) {
    let target = (adj.value() + by).clamp(adj.lower(), adj.upper() - adj.page_size());
    let moved = adj.clone();
    let animation = adw::TimedAnimation::new(
        widget,
        adj.value(),
        target,
        250,
        adw::CallbackAnimationTarget::new(move |value| moved.set_value(value)),
    );
    animation.set_easing(adw::Easing::EaseOutCubic);
    animation.play();
    // Kept with the widget until the next one replaces it.
    attach(widget, Rc::new(animation));
}

fn short_card(video: &SearchVideo) -> gtk::Widget {
    let picture = gtk::Picture::new();
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.set_can_shrink(true);
    let thumb = AspectBin::new(16.0 / 9.0, &picture);
    thumb.set_natural_width(SHORT_WIDTH);
    thumb.add_css_class("thumb");
    thumb.set_overflow(gtk::Overflow::Hidden);
    let title = text_label("short-title", 2);
    title.set_label(&video.video.title);
    let views = text_label("meta", 1);
    views.add_css_class("dim");
    views.set_label(video.video.views.as_deref().unwrap_or_default());
    views.set_visible(video.video.views.is_some());
    let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
    card.add_css_class("short-card");
    card.set_size_request(SHORT_WIDTH, -1);
    card.set_cursor_from_name(Some("pointer"));
    card.append(&thumb);
    card.append(&title);
    card.append(&views);
    app().images.load_into(
        &picture,
        video.video.thumbnail.as_deref(),
        SHORT_WIDTH * card.scale_factor(),
    );
    let props = serde_json::to_value(video).unwrap_or_default();
    video_input::connect(&card, move || Some(props.clone()));
    card.upcast()
}

// Corrections and the footer

pub fn build_correction(props: &Value) -> gtk::Widget {
    let correction: Correction = serde_json::from_value(props.clone()).unwrap_or_default();
    let link = |text: &str, bold: bool| {
        let escaped = gtk::glib::markup_escape_text(text);
        let shown = if bold {
            format!("<b><i>{escaped}</i></b>")
        } else {
            escaped.to_string()
        };
        format!("<a href=\"search\">{shown}</a>")
    };
    let root = gtk::Box::new(gtk::Orientation::Vertical, 4);
    root.add_css_class("search-correction");
    let line = |markup: String, query: String, exact: bool| {
        let label = gtk::Label::new(None);
        label.set_markup(&markup);
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.connect_activate_link(move |_, _| {
            let filters = if exact {
                json!({ "exact": true })
            } else {
                Value::Null
            };
            run("search.open", json!({ "query": query, "filters": filters }));
            gtk::glib::Propagation::Stop
        });
        label
    };
    let corrected = gtk::glib::markup_escape_text(&correction.query);
    if correction.applied {
        root.append(&line(
            format!("Showing results for <b><i>{corrected}</i></b>"),
            correction.query.clone(),
            false,
        ));
        let instead = line(
            format!("Search instead for {}", link(&correction.original, false)),
            correction.original.clone(),
            true,
        );
        instead.add_css_class("dim");
        root.append(&instead);
    } else {
        root.append(&line(
            format!("Did you mean: {}", link(&correction.query, true)),
            correction.query.clone(),
            false,
        ));
    }
    root.upcast()
}

/// `{ state = "loading" | "error" | "end", message? }`.
pub fn build_footer(props: &Value) -> gtk::Widget {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    root.add_css_class("search-footer");
    root.set_halign(gtk::Align::Center);
    match props.get("state").and_then(Value::as_str) {
        Some("loading") => {
            let spinner = adw::Spinner::new();
            spinner.set_size_request(32, 32);
            root.append(&spinner);
        }
        Some("error") => {
            let message = props
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Couldn't load more results");
            let label = gtk::Label::new(Some(message));
            label.add_css_class("dim");
            label.set_wrap(true);
            label.set_justify(gtk::Justification::Center);
            let retry = gtk::Button::with_label("Try again");
            retry.add_css_class("pill-button");
            retry.set_halign(gtk::Align::Center);
            retry.connect_clicked(|_| run("search.more", Value::Null));
            root.append(&label);
            root.append(&retry);
        }
        _ => {
            let label = gtk::Label::new(Some("No more results"));
            label.add_css_class("dim");
            root.append(&label);
        }
    }
    root.upcast()
}

// Helpers

fn text_label(class: &str, lines: i32) -> gtk::Label {
    let label = gtk::Label::new(None);
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_lines(lines);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_hexpand(true);
    label.add_css_class(class);
    label
}

fn menu_button() -> gtk::MenuButton {
    let menu = gtk::MenuButton::new();
    menu.set_icon_name("view-more-symbolic");
    menu.add_css_class("menu-button");
    menu.set_valign(gtk::Align::Start);
    menu.set_tooltip_text(Some("More"));
    menu
}

/// "a • b", skipping what is missing.
fn dotted(parts: &[Option<&str>]) -> String {
    parts
        .iter()
        .flatten()
        .filter(|p| !p.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" • ")
}

/// Markup for text inside a row that is clicked as a whole: bold kept,
/// links dropped.
fn plain_markup(text: &RichText) -> String {
    super::richtext::markup(&RichText {
        runs: text
            .runs
            .iter()
            .map(|r| Run {
                link: None,
                ..r.clone()
            })
            .collect(),
    })
}

/// Runs `action` on a click on `widget`, and keeps the click from also
/// reaching the row (which would play the video).
fn on_click(widget: &impl IsA<gtk::Widget>, action: impl Fn() + 'static) {
    let click = gtk::GestureClick::new();
    click.set_button(gtk::gdk::BUTTON_PRIMARY);
    click.connect_pressed(|gesture, _, _, _| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
    });
    click.connect_released(move |_, n_press, _, _| {
        if n_press == 1 {
            action();
        }
    });
    widget.add_controller(click);
}

fn run(command: &str, args: Value) {
    if let Err(e) = app().ext.commands.run(command, args) {
        app().toast(&e);
    }
}

#[cfg(test)]
mod tests {
    use super::dotted;

    #[test]
    fn dotted_skips_missing_parts() {
        assert_eq!(
            dotted(&[Some("3.2B views"), Some("10 years ago")]),
            "3.2B views • 10 years ago"
        );
        assert_eq!(dotted(&[None, Some("Playlist")]), "Playlist");
        assert_eq!(dotted(&[Some(""), None]), "");
    }
}
