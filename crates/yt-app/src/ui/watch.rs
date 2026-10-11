//! The `watch` page, laid out as on youtube.com, all of it scrolling
//! together so the player scrolls away on the way to the comments:
//!
//! - default: the player, title, description and comments on the left, "Up
//!   next" (the suggested videos) on the right;
//! - theater (`watch.toggle_theater`, T): the player across the whole width,
//!   the rest below it;
//! - narrow windows: one column, "Up next" before the comments;
//! - fullscreen: only the player, filling the screen.
//!
//! The player is 16:9 but leaves room below it for the title. The layouts
//! only move cells of one grid, so the player is never taken out of the page
//! (which would restart its video output).
//!
//! The page shows what the feed card knew right away and fills in the rest
//! when `watch.loaded` arrives ([`crate::playback`] loads it).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use serde_json::{Value, json};
use yt_core::comments::Vote;
use yt_core::feed::Video;
use yt_core::text::RichText;
use yt_core::watch::WatchPage;

use super::aspect::AspectBin;
use super::{attach, attached, avatar, follow, richtext};
use crate::app::app;

/// Room left below the player for the title and channel, in pixels.
const BELOW_PLAYER: i32 = 128;
/// In theater mode, room left below the player (youtube.com's 169 px less
/// the top bar).
const BELOW_THEATER: i32 = 113;
/// The page's padding, and the gap between the columns.
const PADDING: i32 = 24;
/// The player is never shorter than this.
const MIN_PLAYER_HEIGHT: i32 = 240;
/// "Up next" is this wide beside the player, and shows this many videos.
const RELATED_WIDTH: i32 = 402;
const RELATED_COUNT: usize = 20;
/// Narrower than this, everything is in one column.
const NARROW: f64 = 1000.0;
/// The folded description: this many lines or characters.
const FOLDED_LINES: usize = 3;
const FOLDED_CHARS: usize = 280;

struct Watch {
    video_id: RefCell<String>,
    scroller: gtk::ScrolledWindow,
    grid: gtk::Grid,
    /// Where the player shows beside "Up next".
    player: AspectBin,
    /// Where it shows full width, above the columns: in theater mode and
    /// in fullscreen.
    theater: AspectBin,
    info: gtk::Box,
    comments_area: gtk::Box,
    related: gtk::Box,
    /// "Up next", at most its width.
    related_column: adw::Clamp,
    fullscreen: Cell<bool>,
    narrow: Cell<bool>,
    title: gtk::Label,
    owner: gtk::Box,
    like: gtk::Button,
    dislike: gtk::Button,
    vote: Cell<Vote>,
    likes: RefCell<Option<String>>,
    stats: gtk::Label,
    description: gtk::Label,
    full_description: RefCell<RichText>,
    expanded: Cell<bool>,
    more: gtk::Button,
    comments: gtk::Box,
    /// The video shown, as its card or the watch page knows it.
    video: RefCell<Video>,
    save: gtk::Button,
    download: gtk::Button,
    download_icon: gtk::Image,
    download_label: gtk::Label,
}

pub fn build(props: &Value) -> gtk::Widget {
    let app = app();

    let title = gtk::Label::new(None);
    title.add_css_class("video-title");
    title.set_xalign(0.0);
    title.set_wrap(true);

    let owner = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    owner.set_hexpand(true);
    let like = gtk::Button::new();
    like.set_tooltip_text(Some("I like this"));
    let dislike = gtk::Button::new();
    dislike.set_tooltip_text(Some("I dislike this"));
    let rating = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    rating.add_css_class("linked");
    rating.add_css_class("rating");
    rating.set_valign(gtk::Align::Center);
    rating.append(&like);
    rating.append(&dislike);
    let byline = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    byline.append(&owner);
    byline.append(
        &app.ext
            .slots
            .create("watch.actions", gtk::Orientation::Horizontal, 8),
    );
    byline.append(&rating);
    // Save and Download, as youtube.com has them beside the likes.
    let save = action_pill("yt-playlist-add-symbolic", "Save");
    save.set_tooltip_text(Some("Save to a playlist"));
    let (download, download_icon, download_label) = {
        let icon = gtk::Image::from_icon_name("yt-download-symbolic");
        let label = gtk::Label::new(Some("Download"));
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        content.append(&icon);
        content.append(&label);
        let button = gtk::Button::new();
        button.set_child(Some(&content));
        button.add_css_class("watch-action");
        button.set_valign(gtk::Align::Center);
        (button, icon, label)
    };
    byline.append(&save);
    byline.append(&download);

    let stats = gtk::Label::new(None);
    stats.add_css_class("description-stats");
    stats.set_xalign(0.0);
    let description = richtext::label(&RichText::default());
    let more = gtk::Button::with_label("…more");
    more.add_css_class("flat");
    more.add_css_class("description-more");
    more.set_halign(gtk::Align::Start);
    let description_card = gtk::Box::new(gtk::Orientation::Vertical, 6);
    description_card.add_css_class("description-card");
    description_card.append(&stats);
    description_card.append(&description);
    description_card.append(&more);

    let info = gtk::Box::new(gtk::Orientation::Vertical, 16);
    info.add_css_class("watch-details");
    info.set_hexpand(true);
    info.set_valign(gtk::Align::Start);
    info.append(&title);
    info.append(&byline);
    info.append(&description_card);
    info.append(
        &app.ext
            .slots
            .create("watch.below_player", gtk::Orientation::Vertical, 8),
    );
    let comments = gtk::Box::new(gtk::Orientation::Vertical, 0);
    // Takes the height "Up next" leaves over, so no gaps open elsewhere.
    let comments_area = gtk::Box::new(gtk::Orientation::Vertical, 0);
    comments_area.add_css_class("watch-comments");
    comments_area.set_vexpand(true);
    comments_area.set_valign(gtk::Align::Start);
    comments_area.append(&comments);
    let related = gtk::Box::new(gtk::Orientation::Vertical, 8);
    related.add_css_class("watch-related");
    related.set_widget_name("watch-related");
    related.set_valign(gtk::Align::Start);

    // The list playing (a playlist, the queue) goes above "Up next".
    let side = gtk::Box::new(gtk::Orientation::Vertical, 16);
    side.append(&app.ext.components.build("list_panel", &Value::Null));
    side.append(&related);
    // Long titles would make it as wide as they are.
    let related_column = adw::Clamp::builder()
        .child(&side)
        .maximum_size(RELATED_WIDTH)
        .tightening_threshold(RELATED_WIDTH)
        .valign(gtk::Align::Start)
        .build();
    // Never wider than "Up next" is: what expands inside (the list panel's
    // rows) mustn't take width from the player.
    related_column.set_hexpand(false);

    // The player itself moves between these and the mini player.
    let frame = AspectBin::empty(9.0 / 16.0);
    frame.set_widget_name("watch-player");
    frame.set_hexpand(true);
    let theater = AspectBin::empty(9.0 / 16.0);
    theater.set_widget_name("watch-theater");
    theater.add_css_class("theater-player");
    theater.set_visible(false);
    super::player_dock::register("watch", &frame);
    super::player_dock::register("theater", &theater);
    let grid = gtk::Grid::new();
    grid.add_css_class("watch-column");
    grid.set_column_spacing(PADDING as u32);
    grid.attach(&frame, 0, 0, 1, 1);
    grid.attach(&info, 0, 1, 1, 1);
    grid.attach(&comments_area, 0, 2, 1, 1);
    grid.attach(&related_column, 1, 0, 1, 3);
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    column.append(&theater);
    column.append(&grid);
    let scroller = gtk::ScrolledWindow::builder()
        .child(&column)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    scroller.set_widget_name("watch-info");
    let root = adw::BreakpointBin::new();
    root.set_size_request(360, 300);
    root.set_widget_name("page-watch");
    root.add_css_class("watch-page");
    root.set_child(Some(&scroller));

    let watch = Rc::new(Watch {
        video_id: RefCell::default(),
        scroller,
        grid,
        player: frame,
        theater,
        info,
        comments_area,
        related,
        related_column,
        fullscreen: Cell::new(false),
        narrow: Cell::new(false),
        title,
        owner,
        like,
        dislike,
        vote: Cell::new(Vote::None),
        likes: RefCell::default(),
        stats,
        description,
        full_description: RefCell::default(),
        expanded: Cell::new(false),
        more,
        comments,
        video: RefCell::default(),
        save,
        download,
        download_icon,
        download_label,
    });
    attach(&root, watch.clone());
    connect(&watch, &root);
    fill_basic(&watch, props);
    root.upcast()
}

pub fn update(widget: &gtk::Widget, props: &Value) {
    if let Some(watch) = attached::<Watch>(widget) {
        fill_basic(&watch, props);
    }
}

fn connect(watch: &Rc<Watch>, root: &adw::BreakpointBin) {
    // One column when narrow.
    let breakpoint = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        NARROW,
        adw::LengthUnit::Px,
    ));
    let weak = Rc::downgrade(watch);
    breakpoint.connect_apply(move |_| {
        if let Some(watch) = weak.upgrade() {
            watch.narrow.set(true);
            arrange(&watch);
        }
    });
    let weak = Rc::downgrade(watch);
    breakpoint.connect_unapply(move |_| {
        if let Some(watch) = weak.upgrade() {
            watch.narrow.set(false);
            arrange(&watch);
        }
    });
    root.add_breakpoint(breakpoint);
    let weak = Rc::downgrade(watch);
    follow(root, "watch.theater", move |_| {
        if let Some(watch) = weak.upgrade() {
            arrange(&watch);
        }
    });
    arrange(watch);

    // The player's size follows the view's.
    let weak = Rc::downgrade(watch);
    watch.scroller.connect_map(move |_| {
        if let Some(watch) = weak.upgrade() {
            fit_player(&watch);
        }
    });
    for adjustment in [watch.scroller.vadjustment(), watch.scroller.hadjustment()] {
        let weak = Rc::downgrade(watch);
        adjustment.connect_changed(move |_| {
            if let Some(watch) = weak.upgrade() {
                fit_player(&watch);
            }
        });
    }

    let weak = Rc::downgrade(watch);
    follow(root, "watch.loaded", move |payload| {
        let Some(watch) = weak.upgrade() else { return };
        if payload.get("video_id").and_then(Value::as_str) != Some(watch.video_id.borrow().as_str())
        {
            return;
        }
        if let Ok(page) =
            serde_json::from_value::<WatchPage>(payload.get("page").cloned().unwrap_or_default())
        {
            fill_details(&watch, &page);
        }
    });
    let weak = Rc::downgrade(watch);
    follow(root, "video.rated", move |payload| {
        let Some(watch) = weak.upgrade() else { return };
        if payload.get("video_id").and_then(Value::as_str) == Some(watch.video_id.borrow().as_str())
        {
            watch
                .vote
                .set(match payload.get("rating").and_then(Value::as_str) {
                    Some("like") => Vote::Like,
                    Some("dislike") => Vote::Dislike,
                    _ => Vote::None,
                });
            show_rating(&watch);
        }
    });
    let rate = |target: Vote| {
        let weak = Rc::downgrade(watch);
        move |_: &gtk::Button| {
            let Some(watch) = weak.upgrade() else { return };
            let rating = match (watch.vote.get() == target, target) {
                (true, _) => "none",
                (false, Vote::Like) => "like",
                (false, _) => "dislike",
            };
            let args = json!({ "video_id": *watch.video_id.borrow(), "rating": rating });
            if let Err(e) = app().ext.commands.run("video.rate", args) {
                app().toast(&e);
            }
        }
    };
    watch.like.connect_clicked(rate(Vote::Like));
    watch.dislike.connect_clicked(rate(Vote::Dislike));

    let weak = Rc::downgrade(watch);
    watch.save.connect_clicked(move |_| {
        if let Some(watch) = weak.upgrade() {
            let id = watch.video_id.borrow().clone();
            if let Err(e) = app()
                .ext
                .commands
                .run("video.save", json!({ "video_id": id }))
            {
                app().toast(&e);
            }
        }
    });
    let weak = Rc::downgrade(watch);
    watch.download.connect_clicked(move |button| {
        let Some(watch) = weak.upgrade() else { return };
        let id = watch.video_id.borrow().clone();
        let run = |command: &str, args: Value| {
            if let Err(e) = app().ext.commands.run(command, args) {
                app().toast(&e);
            }
        };
        if crate::downloads::is_running(&id) {
            run("download.cancel", json!({ "video_id": id }));
        } else if crate::downloads::get(&id).is_some_and(|d| d.complete) {
            let props = json!({
                "title": "Delete download?",
                "body": "It won't play offline any more.",
                "confirm": "Delete",
            });
            super::dialog::confirm(button, &props, move || {
                if let Err(e) = app()
                    .ext
                    .commands
                    .run("download.delete", json!({ "video_id": id }))
                {
                    app().toast(&e);
                }
            });
        } else {
            let video = watch.video.borrow().clone();
            run("download.start", json!({ "video": video }));
        }
    });
    let weak = Rc::downgrade(watch);
    follow(root, "download.changed", move |payload| {
        if let Some(watch) = weak.upgrade()
            && payload.get("video_id").and_then(Value::as_str)
                == Some(watch.video_id.borrow().as_str())
        {
            show_download(&watch);
        }
    });
    let weak = Rc::downgrade(watch);
    watch.more.connect_clicked(move |_| {
        if let Some(watch) = weak.upgrade() {
            show_description(&watch, !watch.expanded.get());
        }
    });

    let (weak, page) = (Rc::downgrade(watch), root.downgrade());
    follow(root, "window.fullscreen", move |payload| {
        let fullscreen = payload
            .get("fullscreen")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let (Some(watch), Some(page)) = (weak.upgrade(), page.upgrade()) else {
            return;
        };
        watch.fullscreen.set(fullscreen);
        watch.grid.set_visible(!fullscreen);
        if fullscreen {
            page.add_css_class("fullscreen");
            watch.scroller.vadjustment().set_value(0.0);
        } else {
            page.remove_css_class("fullscreen");
        }
        arrange(&watch);
    });
}

/// A rounded button with an icon and a label, beside the likes.
fn action_pill(icon: &str, label: &str) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.append(&gtk::Image::from_icon_name(icon));
    content.append(&gtk::Label::new(Some(label)));
    let button = gtk::Button::new();
    button.set_child(Some(&content));
    button.add_css_class("watch-action");
    button.set_valign(gtk::Align::Center);
    button
}

/// The Download button as the video's download stands: "Download", how far
/// along it is, or "Downloaded".
fn show_download(watch: &Watch) {
    let id = watch.video_id.borrow().clone();
    let download = crate::downloads::get(&id);
    let running = crate::downloads::is_running(&id);
    let live = watch.video.borrow().live;
    let (icon, label, tooltip) = match (&download, running) {
        (Some(d), _) if d.complete => (
            "yt-downloaded-symbolic",
            "Downloaded".to_owned(),
            "Downloaded: plays offline. Click to delete",
        ),
        (Some(d), true) => (
            "yt-download-symbolic",
            format!("{:.0} %", d.progress() * 100.0),
            "Downloading. Click to stop",
        ),
        (_, true) => (
            "yt-download-symbolic",
            "Starting".to_owned(),
            "Downloading. Click to stop",
        ),
        _ => (
            "yt-download-symbolic",
            "Download".to_owned(),
            "Download to watch offline",
        ),
    };
    watch.download_icon.set_icon_name(Some(icon));
    watch.download_label.set_label(&label);
    watch.download.set_tooltip_text(Some(tooltip));
    watch.download.set_visible(!live);
}

/// Places the grid's cells for the layout: default, theater or narrow.
fn arrange(watch: &Watch) {
    let theater = crate::app::app().settings.get().theater;
    let place = |widget: &gtk::Widget, column, row, width, height| {
        if let Some(cell) = watch
            .grid
            .layout_manager()
            .and_downcast::<gtk::GridLayout>()
            .map(|layout| layout.layout_child(widget))
            .and_downcast::<gtk::GridLayoutChild>()
        {
            cell.set_column(column);
            cell.set_row(row);
            cell.set_column_span(width);
            cell.set_row_span(height);
        }
    };
    let (player, info, comments, related) = (
        watch.player.upcast_ref::<gtk::Widget>(),
        watch.info.upcast_ref::<gtk::Widget>(),
        watch.comments_area.upcast_ref::<gtk::Widget>(),
        watch.related_column.upcast_ref::<gtk::Widget>(),
    );
    if watch.narrow.get() {
        place(player, 0, 0, 1, 1);
        place(info, 0, 1, 1, 1);
        place(related, 0, 2, 1, 1);
        place(comments, 0, 3, 1, 1);
    } else if theater {
        // The player is in the row above.
        place(info, 0, 0, 1, 1);
        place(comments, 0, 1, 1, 1);
        place(related, 1, 0, 1, 2);
    } else {
        place(player, 0, 0, 1, 1);
        place(info, 0, 1, 1, 1);
        place(comments, 0, 2, 1, 1);
        place(related, 1, 0, 1, 3);
    }
    // Full width above the columns in theater mode and in fullscreen.
    let above = (theater && !watch.narrow.get()) || watch.fullscreen.get();
    watch.player.set_visible(!above);
    watch.theater.set_visible(above);
    super::player_dock::set_watch_slot(if above { "theater" } else { "watch" });
    watch.related_column.set_size_request(
        if watch.narrow.get() {
            -1
        } else {
            RELATED_WIDTH
        },
        -1,
    );
    watch
        .related_column
        .set_maximum_size(if watch.narrow.get() {
            i32::MAX
        } else {
            RELATED_WIDTH
        });
    if theater && !watch.narrow.get() {
        watch.grid.add_css_class("theater");
    } else {
        watch.grid.remove_css_class("theater");
    }
    if watch.narrow.get() {
        watch.related.add_css_class("narrow");
    } else {
        watch.related.remove_css_class("narrow");
    }
}

/// "Up next": the videos YouTube suggests, as compact cards.
fn show_related(watch: &Watch, page: &WatchPage) {
    while let Some(child) = watch.related.first_child() {
        watch.related.remove(&child);
    }
    let components = &crate::app::app().ext.components;
    let videos = page
        .related
        .iter()
        .flat_map(|shelf| &shelf.videos)
        .filter(|v| !v.short && v.id != page.video_id)
        .take(RELATED_COUNT);
    for video in videos {
        let mut props = serde_json::to_value(video).unwrap_or_default();
        props["layout"] = json!("compact");
        watch
            .related
            .append(&components.build("video_card", &props));
    }
}

/// Sizes the player to the view, as youtube.com does: 16:9, no taller than
/// the view less room for the title (so the columns narrow, centered, when
/// the window is short and wide); in theater mode, full width and no taller
/// than the view less a little; the whole view in fullscreen.
fn fit_player(watch: &Watch) {
    // The scroller's own size: its adjustments pass through odd values
    // while the page refills, which would flash a small player.
    if !watch.scroller.is_mapped() {
        return;
    }
    let (width, view) = (watch.scroller.width(), watch.scroller.height());
    if view <= 0 || width <= 0 {
        return;
    }
    let fullscreen = watch.fullscreen.get();
    watch.theater.set_fill(fullscreen);
    watch.theater.set_max_height(if fullscreen {
        view
    } else {
        (view - BELOW_THEATER).max(MIN_PLAYER_HEIGHT)
    });
    let max_height = (view - BELOW_PLAYER).max(MIN_PLAYER_HEIGHT);
    watch.player.set_max_height(max_height);
    // Centered columns when the player can't use the width.
    let beside = if watch.narrow.get() {
        0
    } else {
        RELATED_WIDTH + PADDING
    };
    let free = width - 2 * PADDING - beside;
    let player_width = free.min(max_height * 16 / 9);
    let margin = PADDING + ((free - player_width) / 2).max(0);
    if watch.grid.margin_start() != margin {
        watch.grid.set_margin_start(margin);
        watch.grid.set_margin_end(margin);
    }
}

/// What the feed card knew: shown at once, before the details load.
fn fill_basic(watch: &Watch, props: &Value) {
    let video: Video = props
        .get("video")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    if video.id.is_empty() || *watch.video_id.borrow() == video.id {
        return;
    }
    *watch.video_id.borrow_mut() = video.id.clone();
    *watch.video.borrow_mut() = video.clone();
    show_download(watch);
    watch.scroller.vadjustment().set_value(0.0);
    watch.title.set_label(&video.title);
    let name = video.channel.clone().unwrap_or_default();
    let known = app().channels.lookup(&name).unwrap_or_default();
    show_owner(
        watch,
        &name,
        known.avatar.as_deref(),
        known.subscribers.as_deref(),
        known.id.as_deref(),
        known.verified,
        None,
    );
    let stats: Vec<&str> = [video.views.as_deref(), video.published.as_deref()]
        .into_iter()
        .flatten()
        .collect();
    watch.stats.set_label(&stats.join("  •  "));
    *watch.full_description.borrow_mut() = RichText::default();
    show_description(watch, false);
    *watch.likes.borrow_mut() = None;
    watch.vote.set(Vote::None);
    show_rating(watch);
    while let Some(child) = watch.comments.first_child() {
        watch.comments.remove(&child);
    }
    while let Some(child) = watch.related.first_child() {
        watch.related.remove(&child);
    }
}

fn fill_details(watch: &Watch, page: &WatchPage) {
    if !page.title.is_empty() {
        watch.title.set_label(&page.title);
    }
    // What a download keeps, for a video opened by its ID alone.
    {
        let mut video = watch.video.borrow_mut();
        if video.title.is_empty() {
            video.title = page.title.clone();
        }
        if video.channel.is_none() {
            video.channel = page.owner.as_ref().map(|o| o.name.clone());
        }
    }
    show_related(watch, page);
    if let Some(owner) = &page.owner {
        // No subscribe button on your own videos.
        let subscription = page
            .subscription
            .as_ref()
            .filter(|_| !crate::social::is_mine(owner.handle.as_deref()))
            .map(|s| json!({ "subscription": s, "channel_name": owner.name }));
        show_owner(
            watch,
            &owner.name,
            owner.avatar.as_deref(),
            owner.subscribers.as_deref(),
            owner.id.as_deref(),
            owner.verified,
            subscription,
        );
    }
    let stats: Vec<&str> = [page.views.as_deref(), page.published.as_deref()]
        .into_iter()
        .flatten()
        .collect();
    watch.stats.set_label(&stats.join("  •  "));
    *watch.full_description.borrow_mut() = page.description.clone();
    show_description(watch, false);
    *watch.likes.borrow_mut() = page.likes.clone();
    watch.vote.set(page.vote);
    show_rating(watch);

    while let Some(child) = watch.comments.first_child() {
        watch.comments.remove(&child);
    }
    let section = match &page.comments {
        Some(entry) => app().ext.components.build(
            "comments",
            &json!({ "video_id": page.video_id, "count": entry.count, "sorts": entry.sorts }),
        ),
        None => {
            let off = gtk::Label::new(Some("Comments are turned off."));
            off.add_css_class("dim");
            off.set_xalign(0.0);
            off.upcast()
        }
    };
    watch.comments.append(&section);
}

fn show_owner(
    watch: &Watch,
    name: &str,
    avatar_url: Option<&str>,
    subscribers: Option<&str>,
    channel_id: Option<&str>,
    verified: bool,
    subscription: Option<Value>,
) {
    while let Some(child) = watch.owner.first_child() {
        watch.owner.remove(&child);
    }
    if name.is_empty() {
        return;
    }
    let title = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let label = gtk::Label::new(Some(name));
    label.add_css_class("channel");
    title.append(&label);
    if verified {
        let check = gtk::Image::from_icon_name("yt-verified-symbolic");
        check.add_css_class("verified");
        check.set_tooltip_text(Some("Verified"));
        title.append(&check);
    }
    let names = gtk::Box::new(gtk::Orientation::Vertical, 2);
    names.set_valign(gtk::Align::Center);
    names.append(&title);
    if let Some(subscribers) = subscribers {
        let label = gtk::Label::new(Some(subscribers));
        label.add_css_class("dim");
        label.add_css_class("caption");
        label.set_xalign(0.0);
        names.append(&label);
    }
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    content.append(&avatar::new(40, name, avatar_url));
    content.append(&names);
    let link = gtk::Button::builder()
        .child(&content)
        .css_classes(["flat", "owner-link"])
        .build();
    let (id, name) = (channel_id.map(str::to_owned), name.to_owned());
    link.set_sensitive(id.is_some());
    link.connect_clicked(move |_| {
        if let Err(e) = app()
            .ext
            .commands
            .run("channel.open", json!({ "channel_id": id, "name": name }))
        {
            app().toast(&e);
        }
    });
    watch.owner.append(&link);
    if let Some(props) = subscription {
        let button = app().ext.components.build("subscribe_button", &props);
        button.set_valign(gtk::Align::Center);
        watch.owner.append(&button);
    }
}

fn show_rating(watch: &Watch) {
    let vote = watch.vote.get();
    let content = adw::ButtonContent::new();
    content.set_icon_name(if vote == Vote::Like {
        "yt-liked-symbolic"
    } else {
        "yt-like-symbolic"
    });
    content.set_label(watch.likes.borrow().as_deref().unwrap_or(""));
    watch.like.set_child(Some(&content));
    watch.dislike.set_icon_name(if vote == Vote::Dislike {
        "yt-disliked-symbolic"
    } else {
        "yt-dislike-symbolic"
    });
}

/// The whole description, or its first lines with "…more".
fn show_description(watch: &Watch, expanded: bool) {
    let full = watch.full_description.borrow();
    let folded = full.truncated(FOLDED_LINES, FOLDED_CHARS);
    watch.expanded.set(expanded && folded.is_some());
    match (&folded, expanded) {
        (Some(folded), false) => richtext::set(&watch.description, folded),
        _ => richtext::set(&watch.description, &full),
    }
    watch.description.set_visible(!full.is_empty());
    watch.more.set_visible(folded.is_some());
    watch
        .more
        .set_label(if expanded { "Show less" } else { "…more" });
}
