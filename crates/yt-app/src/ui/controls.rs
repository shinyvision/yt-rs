//! The `player_controls` component, laid out like youtube.com's: the seek bar
//! (with chapters), then play/pause, next, volume, time and chapter on the
//! left, and autoplay, captions, settings, theater and fullscreen on the
//! right, with slots for more buttons.
//!
//! Hovering the seek bar shows the frame at that moment (from the video's
//! storyboard), the chapter and the time.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use serde_json::{Value, json};
use yt_core::streams::Storyboard;
use yt_core::watch::Chapter;

use super::preview::FramePreview;
use super::seekbar::SeekBar;
use super::{follow, format_time};
use crate::app::app;
use crate::playback;
use crate::player::{LiveWindow, PlayerEvent, PlayerState};

/// The preview frame's size.
const PREVIEW: (i32, i32) = (160, 90);

struct Controls {
    seekbar: SeekBar,
    play: gtk::Button,
    next: gtk::Button,
    /// The previous video of the list playing (shown only then).
    previous: gtk::Button,
    mute: gtk::Button,
    volume: gtk::Scale,
    time: gtk::Label,
    /// "● LIVE": red at a live stream's edge; behind it, grey, and a click
    /// goes back to the edge.
    live: gtk::Button,
    chapter: gtk::Label,
    autoplay: gtk::Switch,
    captions: gtk::Button,
    theater: gtk::Button,
    fullscreen: gtk::Button,
    bubble: gtk::Box,
    preview: FramePreview,
    bubble_chapter: gtk::Label,
    bubble_time: gtk::Label,
    shown_second: Cell<i64>,
    updating: Cell<bool>,
    chapters: RefCell<Vec<Chapter>>,
    /// The chapters of the video the watch page shows, which may arrive
    /// before or after its streams.
    page_chapters: RefCell<(String, Vec<Chapter>)>,
    storyboard: RefCell<Option<Storyboard>>,
    /// Storyboard sheets loaded so far, by URL.
    sheets: RefCell<HashMap<String, gdk::Texture>>,
    hovered: Cell<Option<f64>>,
}

pub fn build(_props: &Value) -> gtk::Widget {
    let app = app();
    let button = |icon: &str, tooltip: &str, command: &'static str| {
        let b = gtk::Button::from_icon_name(icon);
        b.set_tooltip_text(Some(tooltip));
        b.connect_clicked(move |_| run(command, Value::Null));
        b
    };
    let play = button("yt-play-symbolic", "Play (k)", "player.toggle_pause");
    let next = button("yt-next-symbolic", "Next (Shift+N)", "player.next");
    next.set_visible(false);
    let previous = button(
        "yt-previous-symbolic",
        "Previous (Shift+P)",
        "list.previous",
    );
    previous.set_visible(false);
    let mute = button("yt-volume-up-symbolic", "Mute (m)", "player.toggle_mute");
    let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
    volume.set_width_request(64);
    volume.set_draw_value(false);
    volume.set_value(100.0);
    volume.add_css_class("volume");
    // The slider slides out while the pointer is over the volume controls.
    let volume_slider = gtk::Revealer::builder()
        .child(&volume)
        .transition_type(gtk::RevealerTransitionType::SlideRight)
        .transition_duration(150)
        .build();
    let volume_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    volume_box.append(&mute);
    volume_box.append(&volume_slider);
    let hover = gtk::EventControllerMotion::new();
    let slider = volume_slider.downgrade();
    hover.connect_enter(move |_, _, _| {
        if let Some(s) = slider.upgrade() {
            s.set_reveal_child(true);
        }
    });
    let slider = volume_slider.downgrade();
    hover.connect_leave(move |_| {
        if let Some(s) = slider.upgrade() {
            s.set_reveal_child(false);
        }
    });
    volume_box.add_controller(hover);

    let time = gtk::Label::new(Some("0:00 / 0:00"));
    time.add_css_class("time");
    let live_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("live-dot");
    dot.set_valign(gtk::Align::Center);
    live_content.append(&dot);
    live_content.append(&gtk::Label::new(Some("LIVE")));
    let live = gtk::Button::builder()
        .child(&live_content)
        .css_classes(["live-button"])
        .tooltip_text("Skip ahead to live broadcast")
        .visible(false)
        .build();
    live.connect_clicked(|_| run("player.go_live", Value::Null));
    let chapter = gtk::Label::new(None);
    chapter.add_css_class("chapter");
    chapter.set_ellipsize(gtk::pango::EllipsizeMode::End);
    chapter.set_max_width_chars(32);
    chapter.set_visible(false);

    let autoplay = gtk::Switch::new();
    autoplay.add_css_class("autoplay");
    autoplay.set_valign(gtk::Align::Center);
    autoplay.set_active(app.settings.get().autoplay);
    autoplay.set_can_focus(false);
    let captions = button(
        "yt-cc-symbolic",
        "Subtitles/closed captions (c)",
        "player.toggle_captions",
    );
    captions.add_css_class("captions");
    captions.set_visible(false);
    let settings = gtk::MenuButton::builder()
        .icon_name("yt-settings-symbolic")
        .tooltip_text("Settings")
        .css_classes(["player-settings-button"])
        .build();
    settings.set_direction(gtk::ArrowType::Up);
    settings.set_create_popup_func(|button| {
        let popover = gtk::Popover::builder()
            .has_arrow(false)
            .child(
                &crate::app::app()
                    .ext
                    .components
                    .build("player_settings", &super::player_settings::entries()),
            )
            .build();
        popover.add_css_class("player-settings-popover");
        popover.set_halign(gtk::Align::End);
        button.set_popover(Some(&popover));
    });
    let theater = button(
        "yt-theater-symbolic",
        "Theater mode (t)",
        "watch.toggle_theater",
    );
    let fullscreen = button(
        "yt-fullscreen-symbolic",
        "Fullscreen (f)",
        "window.toggle_fullscreen",
    );

    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    row.add_css_class("controls-row");
    for widget in [
        previous.upcast_ref::<gtk::Widget>(),
        play.upcast_ref(),
        next.upcast_ref(),
        volume_box.upcast_ref(),
        time.upcast_ref(),
        live.upcast_ref(),
        chapter.upcast_ref(),
    ] {
        row.append(widget);
    }
    row.append(
        &app.ext
            .slots
            .create("player.controls.start", gtk::Orientation::Horizontal, 2),
    );
    row.append(&spacer);
    row.append(
        &app.ext
            .slots
            .create("player.controls.end", gtk::Orientation::Horizontal, 2),
    );
    for widget in [
        autoplay.upcast_ref::<gtk::Widget>(),
        captions.upcast_ref(),
        settings.upcast_ref(),
        theater.upcast_ref(),
        fullscreen.upcast_ref(),
    ] {
        row.append(widget);
    }

    let seekbar = SeekBar::default();
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    column.add_css_class("player-controls");
    column.set_widget_name("player-controls");
    column.append(&seekbar);
    column.append(&row);

    // The bubble above the seek bar: preview frame, chapter, time.
    let preview = FramePreview::default();
    preview.set_size_request(PREVIEW.0, PREVIEW.1);
    preview.add_css_class("seek-preview");
    let bubble_chapter = gtk::Label::new(None);
    bubble_chapter.add_css_class("seek-chapter");
    bubble_chapter.set_ellipsize(gtk::pango::EllipsizeMode::End);
    bubble_chapter.set_max_width_chars(24);
    let bubble_time = gtk::Label::new(None);
    bubble_time.add_css_class("seek-time");
    let bubble = gtk::Box::new(gtk::Orientation::Vertical, 4);
    bubble.add_css_class("seek-bubble");
    bubble.append(&preview);
    bubble.append(&bubble_chapter);
    bubble.append(&bubble_time);
    bubble.set_halign(gtk::Align::Start);
    bubble.set_valign(gtk::Align::End);
    bubble.set_visible(false);
    bubble.set_can_target(false);

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&column));
    overlay.add_overlay(&bubble);
    // The bubble rises above the bar, over the video.
    overlay.set_overflow(gtk::Overflow::Visible);

    let controls = Rc::new(Controls {
        seekbar,
        play,
        next,
        previous,
        mute,
        volume,
        time,
        live,
        chapter,
        autoplay,
        captions,
        theater,
        fullscreen,
        bubble,
        preview,
        bubble_chapter,
        bubble_time,
        shown_second: Cell::new(-1),
        updating: Cell::new(false),
        chapters: RefCell::default(),
        page_chapters: RefCell::default(),
        storyboard: RefCell::default(),
        sheets: RefCell::default(),
        hovered: Cell::new(None),
    });
    super::attach(&overlay, controls.clone());
    connect(&controls, &overlay);
    overlay.upcast()
}

fn connect(controls: &Rc<Controls>, root: &gtk::Overlay) {
    controls.seekbar.connect_seek(|position, exact| {
        // A live stream opens again to seek, so it seeks once dragging ends.
        let position = match app().player.state().live {
            Some(_) if !exact => return,
            Some(window) => window.start + position,
            None => position,
        };
        run(
            "player.seek",
            json!({ "position": position, "exact": exact }),
        )
    });
    let weak = Rc::downgrade(controls);
    controls.seekbar.connect_hover(move |hover| {
        if let Some(controls) = weak.upgrade() {
            show_hover(&controls, hover);
        }
    });
    let weak = Rc::downgrade(controls);
    controls.volume.connect_value_changed(move |scale| {
        if let Some(controls) = weak.upgrade()
            && !controls.updating.get()
        {
            run("player.set_volume", json!({ "volume": scale.value() }));
        }
    });
    let weak = Rc::downgrade(controls);
    controls.autoplay.connect_active_notify(move |switch| {
        if let Some(controls) = weak.upgrade()
            && !controls.updating.get()
        {
            run(
                "player.set_autoplay",
                json!({ "enabled": switch.is_active() }),
            );
        }
    });
    controls.autoplay.set_tooltip_text(Some("Autoplay"));

    let weak = Rc::downgrade(controls);
    app().player.connect(move |event, state| {
        if let Some(controls) = weak.upgrade() {
            on_player_event(&controls, event, state);
        }
    });
    // The list playing: previous, and next within it.
    let weak = Rc::downgrade(controls);
    follow(root, "list.changed", move |_| {
        if let Some(controls) = weak.upgrade() {
            let list = crate::lists::current();
            controls.previous.set_visible(list.is_some());
            controls
                .previous
                .set_sensitive(list.as_ref().is_some_and(|l| l.position > 0));
            controls.next.set_visible(playback::up_next().is_some());
        }
    });
    let weak = Rc::downgrade(controls);
    follow(root, "watch.loaded", move |payload| {
        let Some(controls) = weak.upgrade() else {
            return;
        };
        controls.next.set_visible(playback::up_next().is_some());
        let video_id = payload
            .get("video_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let chapters: Vec<Chapter> = payload
            .pointer("/page/chapters")
            .and_then(|c| serde_json::from_value(c.clone()).ok())
            .unwrap_or_default();
        *controls.page_chapters.borrow_mut() = (video_id.to_owned(), chapters.clone());
        if app().player.state().video_id.as_deref() == Some(video_id) {
            show_chapters(&controls, chapters);
        }
    });
    let weak = Rc::downgrade(controls);
    follow(root, "player.autoplay", move |payload| {
        if let Some(controls) = weak.upgrade() {
            controls.updating.set(true);
            controls.autoplay.set_active(
                payload
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
            );
            controls.updating.set(false);
        }
    });
    let weak = Rc::downgrade(controls);
    follow(root, "window.fullscreen", move |payload| {
        let fullscreen = payload
            .get("fullscreen")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if let Some(controls) = weak.upgrade() {
            controls.fullscreen.set_icon_name(if fullscreen {
                "yt-fullscreen-exit-symbolic"
            } else {
                "yt-fullscreen-symbolic"
            });
            controls.fullscreen.set_tooltip_text(Some(if fullscreen {
                "Exit full screen (f)"
            } else {
                "Full screen (f)"
            }));
            controls.theater.set_visible(!fullscreen);
        }
    });
    let weak = Rc::downgrade(controls);
    let show_theater = move |theater: bool| {
        if let Some(controls) = weak.upgrade() {
            controls.theater.set_icon_name(if theater {
                "yt-theater-exit-symbolic"
            } else {
                "yt-theater-symbolic"
            });
            controls.theater.set_tooltip_text(Some(if theater {
                "Default view (t)"
            } else {
                "Theater mode (t)"
            }));
        }
    };
    show_theater(app().settings.get().theater);
    follow(root, "watch.theater", move |payload| {
        show_theater(
            payload
                .get("theater")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        );
    });
}

fn on_player_event(controls: &Controls, event: &PlayerEvent, state: &PlayerState) {
    match event {
        PlayerEvent::Loading { video_id } => {
            let streams = app().player.streams().unwrap_or_default();
            let chapters = {
                let page = controls.page_chapters.borrow();
                if page.0 == *video_id {
                    page.1.clone()
                } else {
                    Vec::new()
                }
            };
            show_chapters(controls, chapters);
            *controls.storyboard.borrow_mut() = streams.storyboard;
            controls.sheets.borrow_mut().clear();
            controls.captions.set_visible(!streams.captions.is_empty());
            show_position(controls, state, true);
            show_play(controls, state);
        }
        PlayerEvent::Position(_)
        | PlayerEvent::Duration(_)
        | PlayerEvent::Buffered
        | PlayerEvent::Seeked { .. } => {
            show_position(controls, state, matches!(event, PlayerEvent::Duration(_)));
        }
        PlayerEvent::Paused(_) | PlayerEvent::Ended | PlayerEvent::Loaded => {
            show_play(controls, state)
        }
        PlayerEvent::Volume { volume, muted } => {
            controls.updating.set(true);
            controls
                .volume
                .set_value(if *muted { 0.0 } else { *volume });
            controls.updating.set(false);
            controls.mute.set_icon_name(volume_icon(*volume, *muted));
            controls
                .mute
                .set_tooltip_text(Some(if *muted { "Unmute (m)" } else { "Mute (m)" }));
        }
        PlayerEvent::Settings => {
            if state.captions.is_some() {
                controls.captions.add_css_class("on");
            } else {
                controls.captions.remove_css_class("on");
            }
        }
        _ => {}
    }
}

pub fn volume_icon(volume: f64, muted: bool) -> &'static str {
    match (muted, volume) {
        (true, _) | (_, 0.0) => "yt-volume-off-symbolic",
        (_, v) if v < 50.0 => "yt-volume-down-symbolic",
        _ => "yt-volume-up-symbolic",
    }
}

fn show_play(controls: &Controls, state: &PlayerState) {
    let (icon, tooltip) = if state.ended {
        ("yt-replay-symbolic", "Replay")
    } else if state.paused {
        ("yt-play-symbolic", "Play (k)")
    } else {
        ("yt-pause-symbolic", "Pause (k)")
    };
    controls.play.set_icon_name(icon);
    controls.play.set_tooltip_text(Some(tooltip));
}

fn show_position(controls: &Controls, state: &PlayerState, force: bool) {
    controls.live.set_visible(state.live.is_some());
    if let Some(window) = state.live {
        show_live_position(controls, state, window);
        return;
    }
    controls.time.set_visible(true);
    controls
        .seekbar
        .set_state(state.position, state.duration, state.buffered);
    // The labels only change once a second.
    let second = state.position as i64;
    if second == controls.shown_second.get() && !force {
        return;
    }
    controls.shown_second.set(second);
    controls.time.set_label(&format!(
        "{} / {}",
        format_time(state.position),
        format_time(state.duration)
    ));
    let chapter = chapter_at(&controls.chapters.borrow(), state.position);
    controls.chapter.set_visible(chapter.is_some());
    if let Some(title) = chapter {
        controls.chapter.set_label(&format!("•  {title}"));
    }
}

/// A live stream's bar spans the part that can be watched; behind the edge
/// the time says how far, as on youtube.com.
fn show_live_position(controls: &Controls, state: &PlayerState, window: LiveWindow) {
    controls.seekbar.set_state(
        state.position - window.start,
        (window.edge - window.start).max(1.0),
        (state.buffered - window.start).max(0.0),
    );
    let live = window.at_edge(state.position);
    if live {
        controls.live.remove_css_class("behind");
    } else {
        controls.live.add_css_class("behind");
    }
    controls.time.set_visible(!live);
    controls
        .time
        .set_label(&format!("-{}", format_time(window.edge - state.position)));
    controls.chapter.set_visible(false);
}

fn show_chapters(controls: &Controls, chapters: Vec<Chapter>) {
    controls
        .seekbar
        .set_chapters(chapters.iter().map(|c| c.start).collect());
    *controls.chapters.borrow_mut() = chapters;
}

fn chapter_at(chapters: &[Chapter], time: f64) -> Option<String> {
    chapters
        .iter()
        .rev()
        .find(|c| c.start <= time)
        .map(|c| c.title.clone())
        .filter(|t| !t.is_empty())
}

/// The bubble while hovering the seek bar: the frame (once its sheet has
/// loaded), the chapter and the time, centered over the pointer.
fn show_hover(controls: &Rc<Controls>, hover: Option<(f64, f64)>) {
    controls.hovered.set(hover.map(|(_, t)| t));
    let Some((x, time)) = hover else {
        controls.bubble.set_visible(false);
        return;
    };
    match app().player.state().live {
        Some(window) => {
            let behind = window.edge - (window.start + time);
            controls
                .bubble_time
                .set_label(&if behind < LiveWindow::EDGE {
                    "Live".to_owned()
                } else {
                    format!("-{}", format_time(behind))
                });
        }
        None => controls.bubble_time.set_label(&format_time(time)),
    }
    let chapter = chapter_at(&controls.chapters.borrow(), time);
    controls.bubble_chapter.set_visible(chapter.is_some());
    controls
        .bubble_chapter
        .set_label(chapter.as_deref().unwrap_or_default());

    let board = controls.storyboard.borrow().clone();
    controls.preview.set_visible(board.is_some());
    let width = if board.is_some() { PREVIEW.0 } else { 60 };
    let bar_width = controls.seekbar.width();
    let left = (x as i32 - width / 2).clamp(0, (bar_width - width).max(0));
    controls.bubble.set_margin_start(left + 12);
    controls
        .bubble
        .set_margin_bottom(controls.seekbar.height() + 48);
    controls.bubble.set_visible(true);

    let Some(board) = board else { return };
    let Some((fragment, row, column)) = board.frame_at(time) else {
        return;
    };
    if let Some(sheet) = controls.sheets.borrow().get(&fragment.url) {
        controls
            .preview
            .show(sheet, board.rows, board.columns, row, column);
        return;
    }
    // The sheet loads once, at its own size; the frame shows when it has.
    let url = fragment.url.clone();
    let weak = Rc::downgrade(controls);
    app().images.texture_then(
        &url.clone(),
        (board.width * board.columns) as i32,
        Box::new(move |texture| {
            let (Some(controls), Some(texture)) = (weak.upgrade(), texture) else {
                return;
            };
            controls.sheets.borrow_mut().insert(url, texture);
            if let Some(time) = controls.hovered.get() {
                let x =
                    controls.seekbar.width() as f64 * time / app().player.state().duration.max(1.0);
                show_hover(&controls, Some((x, time)));
            }
        }),
    );
}

fn run(command: &str, args: Value) {
    if let Err(e) = app().ext.commands.run(command, args) {
        app().toast(&e);
    }
}
