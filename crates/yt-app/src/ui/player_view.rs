//! The `player` component: mpv's video (`video::VideoView`), with the
//! controls over it.
//!
//! The controls show on mouse movement and fade out after a moment without
//! it while the video plays, as on youtube.com: not while the pointer is on
//! them or a menu is open, and the pointer hides with them. Clicking the video pauses it; double-clicking toggles
//! fullscreen; right-clicking opens a menu (loop, copy link, stats for
//! nerds). Over the video: the bezel that confirms play, pause, seeking and
//! volume (`player.bezel` shows any icon and text), "stats for nerds" and
//! the "Up next" countdown when a video ends.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;
use serde_json::Value;

use serde_json::json;

use super::bezel::Bezel;
use super::video::VideoView;
use super::{attach, follow};
use crate::app::app;
use crate::player::PlayerEvent;

const HIDE_CONTROLS_AFTER: Duration = Duration::from_millis(2500);
/// The controls fade in quickly and out a little slower, as on youtube.com.
const FADE_IN_MS: u32 = 100;
const FADE_OUT_MS: u32 = 250;
/// How long the bezel's text (volume, speed) stays.
const BEZEL_TEXT_FOR: Duration = Duration::from_millis(900);
const STATS_EVERY: Duration = Duration::from_millis(500);

struct PlayerView {
    area: VideoView,
    controls: gtk::Box,
    /// Whether the controls are shown (or fading in).
    controls_shown: Cell<bool>,
    /// In the mini player, which has controls of its own.
    mini: Cell<bool>,
    /// The pointer is over the controls, so they stay.
    over_controls: Cell<bool>,
    fade: RefCell<Option<adw::TimedAnimation>>,
    spinner: adw::Spinner,
    message: gtk::Label,
    /// Under the message, what to do about it ("Sign in", "Join", ...): a
    /// command and its arguments.
    action: gtk::Button,
    action_command: RefCell<Option<(String, Value)>>,
    hide_timer: RefCell<Option<glib::SourceId>>,
    bezel: Bezel,
    bezel_text: gtk::Revealer,
    bezel_timer: RefCell<Option<glib::SourceId>>,
    stats: gtk::Box,
    stats_grid: gtk::Grid,
    stats_timer: RefCell<Option<glib::SourceId>>,
    up_next: gtk::Box,
    up_next_title: gtk::Label,
    up_next_channel: gtk::Label,
    up_next_picture: gtk::Picture,
    up_next_count: gtk::Label,
    up_next_timer: RefCell<Option<glib::SourceId>>,
    /// Counts down while YouTube holds the streams back (`player.waiting`).
    waiting: RefCell<Option<glib::SourceId>>,
}

pub fn build(_props: &Value) -> gtk::Widget {
    let area = VideoView::new();
    area.set_widget_name("video");
    area.set_hexpand(true);
    area.set_vexpand(true);

    let spinner = adw::Spinner::new();
    spinner.set_size_request(64, 64);
    spinner.set_halign(gtk::Align::Center);
    spinner.set_valign(gtk::Align::Center);
    spinner.set_visible(false);
    spinner.set_can_target(false);

    let message = gtk::Label::new(None);
    message.add_css_class("player-status");
    message.set_halign(gtk::Align::Center);
    message.set_valign(gtk::Align::Center);
    message.set_wrap(true);
    message.set_max_width_chars(60);
    message.set_visible(false);

    // Faded in and out by changing its opacity; see `set_controls_shown`.
    let controls = gtk::Box::new(gtk::Orientation::Vertical, 0);
    controls.set_valign(gtk::Align::End);
    controls.append(&app().ext.components.build("player_controls", &Value::Null));
    // The bubble above the seek bar rises out of the controls.
    controls.set_overflow(gtk::Overflow::Visible);

    let bezel = Bezel::default();
    let bezel_label = gtk::Label::new(None);
    bezel_label.add_css_class("bezel-text");
    let bezel_text = gtk::Revealer::builder()
        .child(&bezel_label)
        .transition_type(gtk::RevealerTransitionType::Crossfade)
        .transition_duration(150)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Start)
        .can_target(false)
        .build();

    let (stats, stats_grid) = stats_panel();
    // The video dims behind the card; the controls stay usable below it.
    let up_next = gtk::Box::new(gtk::Orientation::Vertical, 0);
    up_next.add_css_class("up-next-backdrop");
    up_next.set_visible(false);
    let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
    card.add_css_class("up-next");
    card.set_halign(gtk::Align::Center);
    card.set_valign(gtk::Align::Center);
    card.set_vexpand(true);
    up_next.append(&card);
    let up_next_count = gtk::Label::new(None);
    up_next_count.add_css_class("up-next-count");
    up_next_count.set_xalign(0.0);
    let up_next_picture = gtk::Picture::new();
    up_next_picture.set_content_fit(gtk::ContentFit::Cover);
    up_next_picture.set_can_shrink(true);
    let up_next_thumb = super::aspect::AspectBin::new(9.0 / 16.0, &up_next_picture);
    up_next_thumb.set_natural_width(256);
    up_next_thumb.set_size_request(256, -1);
    up_next_thumb.set_overflow(gtk::Overflow::Hidden);
    up_next_thumb.add_css_class("up-next-picture");
    let up_next_title = gtk::Label::new(None);
    up_next_title.add_css_class("up-next-title");
    up_next_title.set_xalign(0.0);
    up_next_title.set_wrap(true);
    up_next_title.set_lines(2);
    up_next_title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    up_next_title.set_max_width_chars(30);
    let up_next_channel = gtk::Label::new(None);
    up_next_channel.add_css_class("dim");
    up_next_channel.set_xalign(0.0);
    let cancel = gtk::Button::with_label("Cancel");
    cancel.add_css_class("up-next-cancel");
    cancel.connect_clicked(|_| run("player.cancel_up_next", Value::Null));
    let play_now = gtk::Button::with_label("Play now");
    play_now.add_css_class("up-next-play");
    play_now.connect_clicked(|_| run("player.next", Value::Null));
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_homogeneous(true);
    buttons.append(&cancel);
    buttons.append(&play_now);
    card.append(&up_next_count);
    card.append(&up_next_thumb);
    card.append(&up_next_title);
    card.append(&up_next_channel);
    card.append(&buttons);

    let overlay = gtk::Overlay::new();
    overlay.set_widget_name("player");
    overlay.add_css_class("player");
    overlay.set_overflow(gtk::Overflow::Hidden);
    overlay.set_child(Some(&area));
    let action = gtk::Button::new();
    action.add_css_class("pill-button");
    action.add_css_class("player-action");
    action.set_halign(gtk::Align::Center);
    action.set_valign(gtk::Align::Center);
    action.set_margin_top(120);
    action.set_visible(false);
    overlay.add_overlay(&spinner);
    overlay.add_overlay(&message);
    overlay.add_overlay(&action);
    overlay.add_overlay(&bezel);
    overlay.add_overlay(&bezel_text);
    overlay.add_overlay(&stats);
    overlay.add_overlay(&up_next);
    overlay.add_overlay(&controls);
    overlay.add_overlay(&app().ext.slots.create_layer("player.overlay"));

    let view = Rc::new(PlayerView {
        area: area.clone(),

        controls,
        controls_shown: Cell::new(true),
        mini: Cell::new(false),
        over_controls: Cell::new(false),
        fade: RefCell::new(None),
        spinner,
        message,
        action,
        action_command: RefCell::new(None),
        hide_timer: RefCell::new(None),
        bezel,
        bezel_text,
        bezel_timer: RefCell::new(None),
        stats,
        stats_grid,
        stats_timer: RefCell::new(None),
        up_next,
        up_next_title,
        up_next_channel,
        up_next_picture,
        up_next_count,
        up_next_timer: RefCell::new(None),
        waiting: RefCell::new(None),
    });
    attach(&overlay, view.clone());
    connect_rendering(&view);
    connect_input(&overlay, &view);
    connect_player(&view);
    connect_overlays(&overlay, &view);

    let o = overlay.downgrade();
    app().ext.events.on("window.fullscreen", move |payload| {
        let fullscreen = payload
            .get("fullscreen")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if let Some(o) = o.upgrade() {
            if fullscreen {
                o.add_css_class("fullscreen")
            } else {
                o.remove_css_class("fullscreen")
            }
        }
    });
    overlay.upcast()
}

fn connect_rendering(view: &Rc<PlayerView>) {
    let weak = Rc::downgrade(view);
    view.area.connect_error(move |message| {
        if let Some(view) = weak.upgrade() {
            show_message(&view, message);
        }
    });
}

fn connect_input(overlay: &gtk::Overlay, view: &Rc<PlayerView>) {
    let click = gtk::GestureClick::new();
    // A click pauses or resumes; a double click toggles fullscreen (and
    // undoes the pause its first click caused).
    click.connect_released(|_, n_press, _, _| {
        let commands = &app().ext.commands;
        match n_press {
            1 => {
                let _ = commands.run("player.toggle_pause", Value::Null);
            }
            2 => {
                let _ = commands.run("player.toggle_pause", Value::Null);
                let _ = commands.run("window.toggle_fullscreen", Value::Null);
            }
            _ => {}
        }
    });
    view.area.add_controller(click);

    // The controls stay while the pointer is on them.
    let hover = gtk::EventControllerMotion::new();
    let weak = Rc::downgrade(view);
    hover.connect_enter(move |_, _, _| {
        if let Some(view) = weak.upgrade() {
            view.over_controls.set(true);
        }
    });
    let weak = Rc::downgrade(view);
    hover.connect_leave(move |_| {
        if let Some(view) = weak.upgrade() {
            view.over_controls.set(false);
            schedule_hide(&view);
        }
    });
    view.controls.add_controller(hover);

    let motion = gtk::EventControllerMotion::new();
    let weak = Rc::downgrade(view);
    let last = std::cell::Cell::new((0.0, 0.0));
    motion.connect_motion(move |_, x, y| {
        // Wayland repeats motion events when nothing moved; ignore those.
        if last.replace((x, y)) == (x, y) {
            return;
        }
        if let Some(view) = weak.upgrade() {
            show_controls(&view);
        }
    });
    overlay.add_controller(motion);
}

fn connect_player(view: &Rc<PlayerView>) {
    let weak = Rc::downgrade(view);
    app().player.connect(move |event, _| {
        let Some(view) = weak.upgrade() else { return };
        match event {
            PlayerEvent::Loading { .. } => {
                if let Some(timer) = view.waiting.borrow_mut().take() {
                    timer.remove();
                }
                view.message.set_visible(false);
                show_action(&view, None);
                view.spinner.set_visible(true);
                show_controls(&view);
            }
            PlayerEvent::Loaded => view.spinner.set_visible(false),
            PlayerEvent::Buffering(buffering) => view.spinner.set_visible(*buffering),
            PlayerEvent::Paused(true) | PlayerEvent::Ended => show_controls(&view),
            PlayerEvent::Paused(false) => schedule_hide(&view),
            PlayerEvent::Error(message) => show_message(&view, message),
            PlayerEvent::Stopped => view.spinner.set_visible(false),
            _ => {}
        }
    });
    // YouTube holds the streams back for as long as a pre-roll ad would
    // play; count that down.
    let weak = Rc::downgrade(view);
    app().ext.events.on("player.waiting", move |payload| {
        let Some(view) = weak.upgrade() else { return };
        let left = std::rc::Rc::new(std::cell::Cell::new(
            payload
                .get("seconds")
                .and_then(Value::as_f64)
                .unwrap_or(0.0) as u32,
        ));
        let show = |view: &PlayerView, seconds: u32| {
            view.message.set_label(&format!("Starting in {seconds} s"));
            view.message.set_visible(true);
            view.spinner.set_visible(true);
        };
        show(&view, left.get());
        let target = Rc::downgrade(&view);
        let timer = glib::timeout_add_seconds_local(1, move || {
            let Some(view) = target.upgrade() else {
                return glib::ControlFlow::Break;
            };
            left.set(left.get().saturating_sub(1));
            if left.get() == 0 {
                view.waiting.borrow_mut().take();
                view.message.set_visible(false);
                return glib::ControlFlow::Break;
            }
            show(&view, left.get());
            glib::ControlFlow::Continue
        });
        if let Some(old) = view.waiting.borrow_mut().replace(timer) {
            old.remove();
        }
    });
    // In the mini player, the controls and the "Up next" card stay away.
    let weak = Rc::downgrade(view);
    app().ext.events.on("player.docked", move |payload| {
        let Some(view) = weak.upgrade() else { return };
        let mini = payload.get("slot").and_then(Value::as_str) == Some("mini");
        view.mini.set(mini);
        if mini {
            set_controls_shown(&view, false);
            view.area.set_cursor_from_name(None);
            view.up_next.set_visible(false);
        } else {
            show_controls(&view);
        }
    });
    // Errors from looking up streams arrive as events, not player events,
    // with what to do about them: `{ message, action? = { label, command,
    // args? } }`.
    let weak = Rc::downgrade(view);
    app().ext.events.on("player.error", move |payload| {
        if let Some(view) = weak.upgrade() {
            show_message(
                &view,
                payload
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("Playback failed"),
            );
            show_action(&view, payload.get("action"));
        }
    });
    // A live stream or premiere that hasn't started: it plays once it does.
    let weak = Rc::downgrade(view);
    app().ext.events.on("player.upcoming", move |payload| {
        if let Some(view) = weak.upgrade() {
            show_message(
                &view,
                payload
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            );
            show_action(&view, None);
            view.spinner.set_visible(true);
        }
    });
    let weak = Rc::downgrade(view);
    view.action.connect_clicked(move |_| {
        let Some(view) = weak.upgrade() else { return };
        if let Some((command, args)) = view.action_command.borrow().clone()
            && let Err(e) = app().ext.commands.run(&command, args)
        {
            app().toast(&e);
        }
    });
}

fn show_message(view: &PlayerView, message: &str) {
    view.spinner.set_visible(false);
    view.message.set_label(message);
    view.message.set_visible(true);
}

fn show_action(view: &PlayerView, action: Option<&Value>) {
    let command = action.and_then(|a| {
        let label = a.get("label")?.as_str()?;
        let command = a.get("command")?.as_str()?;
        view.action.set_label(label);
        Some((
            command.to_owned(),
            a.get("args").cloned().unwrap_or_else(|| json!({})),
        ))
    });
    view.action.set_visible(command.is_some());
    *view.action_command.borrow_mut() = command;
}

fn show_controls(view: &Rc<PlayerView>) {
    if view.mini.get() {
        return;
    }
    set_controls_shown(view, true);
    schedule_hide(view);
}

/// Fades the controls in or out; the pointer shows and hides with them.
/// Hidden controls take no clicks, which go to the video.
fn set_controls_shown(view: &Rc<PlayerView>, shown: bool) {
    view.area
        .set_cursor_from_name(if shown { None } else { Some("none") });
    if view.controls_shown.replace(shown) == shown {
        return;
    }
    if let Some(old) = view.fade.borrow_mut().take() {
        old.pause();
    }
    view.controls.set_can_target(shown);
    let controls = view.controls.downgrade();
    let target = adw::CallbackAnimationTarget::new(move |opacity| {
        if let Some(controls) = controls.upgrade() {
            controls.set_opacity(opacity);
        }
    });
    let fade = adw::TimedAnimation::builder()
        .widget(&view.controls)
        .value_from(view.controls.opacity())
        .value_to(if shown { 1.0 } else { 0.0 })
        .duration(if shown { FADE_IN_MS } else { FADE_OUT_MS })
        .easing(if shown {
            adw::Easing::EaseOutCubic
        } else {
            adw::Easing::EaseInCubic
        })
        .target(&target)
        .build();
    fade.play();
    *view.fade.borrow_mut() = Some(fade);
}

/// Whether one of the player's menus (settings, right-click) is open.
fn menu_open(root: &gtk::Widget) -> bool {
    let mut child = root.first_child();
    while let Some(widget) = child {
        if (widget.is::<gtk::Popover>() && widget.is_visible()) || menu_open(&widget) {
            return true;
        }
        child = widget.next_sibling();
    }
    false
}

fn schedule_hide(view: &Rc<PlayerView>) {
    if let Some(timer) = view.hide_timer.borrow_mut().take() {
        timer.remove();
    }
    let weak = Rc::downgrade(view);
    let timer = glib::timeout_add_local_once(HIDE_CONTROLS_AFTER, move || {
        let Some(view) = weak.upgrade() else { return };
        view.hide_timer.borrow_mut().take();
        let state = app().player.state();
        let playing = !state.paused && !state.ended && state.video_id.is_some();
        let busy =
            view.over_controls.get() || view.area.parent().is_some_and(|player| menu_open(&player));
        if playing && busy {
            // Try again once the pointer leaves or the menu closes.
            schedule_hide(&view);
        } else if playing {
            set_controls_shown(&view, false);
        }
    });
    *view.hide_timer.borrow_mut() = Some(timer);
}

fn run(command: &str, args: Value) {
    if let Err(e) = app().ext.commands.run(command, args) {
        app().toast(&e);
    }
}

/// Flashes the bezel with `icon`, and shows `text` above it for a moment.
fn flash(view: &Rc<PlayerView>, icon: Option<&str>, text: Option<&str>) {
    // The mini player's own buttons say it.
    if view.mini.get() {
        return;
    }
    if let Some(icon) = icon {
        view.bezel.flash(icon);
    }
    let Some(text) = text else { return };
    if let Some(label) = view.bezel_text.child().and_downcast::<gtk::Label>() {
        label.set_label(text);
    }
    view.bezel_text.set_reveal_child(true);
    if let Some(timer) = view.bezel_timer.borrow_mut().take() {
        timer.remove();
    }
    let weak = Rc::downgrade(view);
    let timer = glib::timeout_add_local_once(BEZEL_TEXT_FOR, move || {
        if let Some(view) = weak.upgrade() {
            view.bezel_timer.borrow_mut().take();
            view.bezel_text.set_reveal_child(false);
        }
    });
    *view.bezel_timer.borrow_mut() = Some(timer);
}

fn stats_panel() -> (gtk::Box, gtk::Grid) {
    let grid = gtk::Grid::new();
    grid.set_column_spacing(12);
    grid.set_row_spacing(2);
    let close = gtk::Button::from_icon_name("yt-close-symbolic");
    close.add_css_class("stats-close");
    close.set_halign(gtk::Align::End);
    close.set_tooltip_text(Some("Close"));
    close.connect_clicked(|_| run("player.toggle_stats", Value::Null));
    let panel = gtk::Box::new(gtk::Orientation::Vertical, 4);
    panel.add_css_class("stats-for-nerds");
    panel.set_halign(gtk::Align::Start);
    panel.set_valign(gtk::Align::Start);
    panel.append(&close);
    panel.append(&grid);
    panel.set_visible(false);
    (panel, grid)
}

fn show_stats(view: &PlayerView) {
    while let Some(child) = view.stats_grid.first_child() {
        view.stats_grid.remove(&child);
    }
    for (row, (name, value)) in app().player.stats().into_iter().enumerate() {
        let name = gtk::Label::new(Some(name));
        name.set_xalign(1.0);
        name.add_css_class("stats-name");
        let value = gtk::Label::new(Some(&value));
        value.set_xalign(0.0);
        view.stats_grid.attach(&name, 0, row as i32, 1, 1);
        view.stats_grid.attach(&value, 1, row as i32, 1, 1);
    }
}

fn toggle_stats(view: &Rc<PlayerView>) {
    let open = !view.stats.is_visible();
    view.stats.set_visible(open);
    if let Some(timer) = view.stats_timer.borrow_mut().take() {
        timer.remove();
    }
    if open {
        show_stats(view);
        let weak = Rc::downgrade(view);
        let timer = glib::timeout_add_local(STATS_EVERY, move || match weak.upgrade() {
            Some(view) => {
                show_stats(&view);
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
        *view.stats_timer.borrow_mut() = Some(timer);
    }
}

/// The video's right-click menu, through the `player.menu` filter.
fn context_menu(overlay: &gtk::Overlay, x: f64, y: f64) {
    let state = app().player.state();
    if state.video_id.is_none() {
        return;
    }
    let items = json!([
        { "label": "Loop", "icon": "yt-loop-symbolic", "command": "player.toggle_loop", "selected": state.looping },
        { "label": "Copy video URL", "icon": "yt-link-symbolic", "command": "player.copy_url" },
        { "label": "Copy video URL at current time", "icon": "yt-link-symbolic",
          "command": "player.copy_url", "args": { "at_time": true } },
        { "label": "Stats for nerds", "icon": "yt-info-symbolic", "command": "player.toggle_stats" },
    ]);
    let items = app()
        .ext
        .filters
        .apply("player.menu", json!({ "items": items }))
        .get("items")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let popover = super::menu::popover(items);
    popover.set_parent(overlay);
    popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    popover.set_position(gtk::PositionType::Bottom);
    popover.set_halign(gtk::Align::Start);
    popover.connect_closed(|popover| {
        let popover = popover.clone();
        // After the item it closed for has run.
        glib::idle_add_local_once(move || popover.unparent());
    });
    popover.popup();
}

fn connect_overlays(overlay: &gtk::Overlay, view: &Rc<PlayerView>) {
    // Play and pause confirm themselves in the bezel (not when a video
    // loads or ends).
    let weak = Rc::downgrade(view);
    app().player.connect(move |event, state| {
        let Some(view) = weak.upgrade() else { return };
        match event {
            PlayerEvent::Paused(paused) if !state.ended && state.duration > 0.0 => {
                flash(
                    &view,
                    Some(if *paused {
                        "yt-pause-symbolic"
                    } else {
                        "yt-play-symbolic"
                    }),
                    None,
                );
            }
            PlayerEvent::Loading { .. } => hide_up_next(&view),
            _ => {}
        }
    });
    let weak = Rc::downgrade(view);
    follow(overlay, "player.bezel", move |payload| {
        if let Some(view) = weak.upgrade() {
            let text = |key: &str| payload.get(key).and_then(Value::as_str);
            flash(&view, text("icon"), text("text"));
        }
    });
    let weak = Rc::downgrade(view);
    follow(overlay, "player.toggle_stats", move |_| {
        if let Some(view) = weak.upgrade() {
            toggle_stats(&view);
        }
    });
    let weak = Rc::downgrade(view);
    follow(overlay, "player.up_next", move |payload| {
        if let Some(view) = weak.upgrade() {
            show_up_next(&view, payload);
        }
    });
    let weak = Rc::downgrade(view);
    follow(overlay, "player.up_next_cancelled", move |_| {
        if let Some(view) = weak.upgrade() {
            hide_up_next(&view);
        }
    });

    let menu = gtk::GestureClick::new();
    menu.set_button(gtk::gdk::BUTTON_SECONDARY);
    let target = overlay.downgrade();
    menu.connect_pressed(move |gesture, _, x, y| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        if let Some(overlay) = target.upgrade() {
            context_menu(&overlay, x, y);
        }
    });
    view.area.add_controller(menu);
}

/// The countdown to the next video: its picture, title and channel, with
/// Cancel and Play now.
fn show_up_next(view: &Rc<PlayerView>, payload: &Value) {
    let video = payload.get("video").cloned().unwrap_or_default();
    let text = |key: &str| {
        video
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    view.up_next_title.set_label(&text("title"));
    view.up_next_channel.set_label(&text("channel"));
    app().images.load_into(
        &view.up_next_picture,
        video.get("thumbnail").and_then(Value::as_str),
        640,
    );
    let left = std::rc::Rc::new(std::cell::Cell::new(
        payload.get("seconds").and_then(Value::as_u64).unwrap_or(8),
    ));
    view.up_next_count
        .set_label(&format!("Up next in {}", left.get()));
    // Too big for the mini player, where the next video simply follows.
    view.up_next.set_visible(!view.mini.get());
    if let Some(timer) = view.up_next_timer.borrow_mut().take() {
        timer.remove();
    }
    let weak = Rc::downgrade(view);
    let timer = glib::timeout_add_seconds_local(1, move || {
        let Some(view) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        left.set(left.get().saturating_sub(1));
        view.up_next_count
            .set_label(&format!("Up next in {}", left.get()));
        if left.get() == 0 {
            view.up_next_timer.borrow_mut().take();
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
    *view.up_next_timer.borrow_mut() = Some(timer);
}

fn hide_up_next(view: &PlayerView) {
    view.up_next.set_visible(false);
    if let Some(timer) = view.up_next_timer.borrow_mut().take() {
        timer.remove();
    }
}
