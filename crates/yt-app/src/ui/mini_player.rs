//! The `mini_player` component: while a video plays and another page is
//! shown, the video carries on in the bottom-right corner, as on
//! youtube.com. Below it, the title and channel; along its bottom, how far
//! it has played; and while the pointer is over it, buttons to go back to
//! the watch page, to close it (which stops the video), to play or pause,
//! and to play the next video. Clicking the video goes back to the watch
//! page. `i` opens and closes it (`player.mini`).
//!
//! The video is the player itself, moved here (see `player_dock`).

use std::rc::Rc;

use gtk::prelude::*;
use serde_json::Value;

use super::aspect::AspectBin;
use super::follow;
use crate::app::app;
use crate::player::{PlayerEvent, PlayerState};

/// youtube.com's size; smaller on a narrow window.
const WIDTH: i32 = 400;
const NARROW_WIDTH: i32 = 320;

struct Mini {
    /// The component itself, weakly: it owns this.
    root: gtk::glib::WeakRef<gtk::Box>,
    previous: gtk::Button,
    balance: gtk::Box,
    list: gtk::Label,
    frame: AspectBin,
    title: gtk::Label,
    channel: gtk::Label,
    play: gtk::Button,
    next: gtk::Button,
    progress: gtk::ProgressBar,
}

pub fn build(_props: &Value) -> gtk::Widget {
    let frame = AspectBin::empty(9.0 / 16.0);
    frame.set_natural_width(WIDTH);
    frame.set_size_request(WIDTH, -1);
    super::player_dock::register("mini", &frame);

    let button = |icon: &str, tooltip: &str, command: &'static str| {
        let b = gtk::Button::from_icon_name(icon);
        b.set_tooltip_text(Some(tooltip));
        b.add_css_class("mini-button");
        b.connect_clicked(move |_| run(command));
        b
    };
    let expand = button("yt-open-full-symbolic", "Expand (i)", "player.expand");
    let close = button("yt-close-symbolic", "Close", "player.stop");
    let play = button("yt-pause-symbolic", "Pause (k)", "player.toggle_pause");
    play.add_css_class("mini-play");
    let next = button("yt-next-symbolic", "Next (Shift+N)", "player.next");
    let previous = button(
        "yt-previous-symbolic",
        "Previous (Shift+P)",
        "list.previous",
    );
    previous.set_visible(false);
    let top = gtk::CenterBox::new();
    top.set_start_widget(Some(&expand));
    top.set_end_widget(Some(&close));
    top.set_valign(gtk::Align::Start);
    // As wide as "next", so "play" is in the middle.
    let balance = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    balance.add_css_class("mini-button");
    balance.set_can_target(false);
    let middle = gtk::Box::new(gtk::Orientation::Horizontal, 24);
    middle.set_halign(gtk::Align::Center);
    middle.set_valign(gtk::Align::Center);
    middle.set_vexpand(true);
    middle.append(&previous);
    middle.append(&balance);
    middle.append(&play);
    middle.append(&next);
    let controls = gtk::Box::new(gtk::Orientation::Vertical, 0);
    controls.add_css_class("mini-controls");
    controls.append(&top);
    controls.append(&middle);

    let progress = gtk::ProgressBar::new();
    progress.add_css_class("mini-progress");
    progress.set_valign(gtk::Align::End);
    progress.set_can_target(false);

    let video = gtk::Overlay::new();
    video.add_css_class("mini-video");
    video.set_overflow(gtk::Overflow::Hidden);
    video.set_child(Some(&frame));
    video.add_overlay(&controls);
    video.add_overlay(&progress);
    video.set_cursor_from_name(Some("pointer"));
    // A click on the video (not a button) goes back to the watch page.
    let click = gtk::GestureClick::new();
    click.connect_released(|gesture, n, _, _| {
        if n == 1 {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            run("player.expand");
        }
    });
    controls.add_controller(click);
    let hover = gtk::EventControllerMotion::new();
    let shown = video.downgrade();
    hover.connect_enter(move |_, _, _| {
        if let Some(video) = shown.upgrade() {
            video.add_css_class("hovered");
        }
    });
    let shown = video.downgrade();
    hover.connect_leave(move |_| {
        if let Some(video) = shown.upgrade() {
            video.remove_css_class("hovered");
        }
    });
    video.add_controller(hover);

    let label = |class: &str, lines: i32| {
        let l = gtk::Label::new(None);
        l.add_css_class(class);
        l.set_xalign(0.0);
        l.set_wrap(true);
        l.set_lines(lines);
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        l.set_max_width_chars(1);
        l.set_hexpand(true);
        l
    };
    let title = label("mini-title", 1);
    let channel = label("mini-channel", 1);
    // The list playing, and where: "Watch later • 2 / 15".
    let list = label("mini-channel", 1);
    list.add_css_class("mini-list");
    list.set_visible(false);
    let info = gtk::Box::new(gtk::Orientation::Vertical, 2);
    info.add_css_class("mini-info");
    info.append(&title);
    info.append(&channel);
    info.append(&list);
    info.append(
        &app()
            .ext
            .slots
            .create("mini_player.info", gtk::Orientation::Horizontal, 8),
    );

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.set_widget_name("mini-player");
    root.add_css_class("mini-player");
    root.set_halign(gtk::Align::End);
    root.set_valign(gtk::Align::End);
    root.set_visible(false);
    root.append(&video);
    root.append(&info);

    let mini = Rc::new(Mini {
        root: root.downgrade(),
        previous,
        balance,
        list,
        frame: frame.clone(),
        title,
        channel,
        play,
        next,
        progress,
    });
    super::attach(&root, mini.clone());
    connect(&mini, &root);
    root.upcast()
}

fn connect(mini: &Rc<Mini>, root: &gtk::Box) {
    let weak = Rc::downgrade(mini);
    follow(root, "list.changed", move |_| {
        if let Some(mini) = weak.upgrade() {
            show_list(&mini);
        }
    });
    let weak = Rc::downgrade(mini);
    follow(root, "shell.layout", move |payload| {
        if let Some(mini) = weak.upgrade() {
            let narrow = payload.get("size").and_then(Value::as_str) == Some("narrow");
            let width = if narrow { NARROW_WIDTH } else { WIDTH };
            mini.frame.set_natural_width(width);
            mini.frame.set_size_request(width, -1);
        }
    });
    let weak = Rc::downgrade(mini);
    follow(root, "player.docked", move |payload| {
        if let Some(mini) = weak.upgrade() {
            let here = payload.get("slot").and_then(Value::as_str) == Some("mini");
            if let Some(root) = mini.root.upgrade() {
                root.set_visible(here);
            }
            if here {
                show_state(&mini, &app().player.state());
                show_list(&mini);
            }
        }
    });
    // The title and channel: the card's at first, then the watch page's.
    let weak = Rc::downgrade(mini);
    follow(root, "video.opened", move |payload| {
        if let Some(mini) = weak.upgrade() {
            let text = |key: &str| {
                payload
                    .pointer(&format!("/video/{key}"))
                    .and_then(Value::as_str)
            };
            mini.title.set_label(text("title").unwrap_or_default());
            mini.channel.set_label(text("channel").unwrap_or_default());
        }
    });
    let weak = Rc::downgrade(mini);
    follow(root, "watch.loaded", move |payload| {
        let Some(mini) = weak.upgrade() else { return };
        if let Some(title) = payload.pointer("/page/title").and_then(Value::as_str) {
            mini.title.set_label(title);
            mini.title.set_tooltip_text(Some(title));
        }
        if let Some(name) = payload.pointer("/page/owner/name").and_then(Value::as_str) {
            mini.channel.set_label(name);
        }
        show_list(&mini);
    });
    let weak = Rc::downgrade(mini);
    app().player.connect(move |event, state| {
        let Some(mini) = weak.upgrade() else { return };
        if !mini.root.upgrade().is_some_and(|r| r.is_visible()) {
            return;
        }
        match event {
            PlayerEvent::Position(_)
            | PlayerEvent::Duration(_)
            | PlayerEvent::Paused(_)
            | PlayerEvent::Ended
            | PlayerEvent::Loaded => show_state(&mini, state),
            _ => {}
        }
    });
}

/// Previous and next, and the list's name and place; with no list, "play"
/// stays in the middle with a spacer the size of "previous".
fn show_list(mini: &Mini) {
    let list = crate::lists::current();
    let next = crate::playback::up_next().is_some();
    mini.next.set_visible(next);
    mini.previous.set_visible(list.is_some());
    mini.previous
        .set_sensitive(list.as_ref().is_some_and(|l| l.position > 0));
    mini.balance.set_visible(list.is_none() && next);
    match &list {
        Some(list) => {
            mini.list.set_label(&format!(
                "{} \u{2022} {} / {}",
                list.title,
                list.position + 1,
                list.order.len()
            ));
            mini.list.set_visible(true);
        }
        None => mini.list.set_visible(false),
    }
}

fn show_state(mini: &Mini, state: &PlayerState) {
    let fraction = match state.live {
        Some(window) => (state.position - window.start) / (window.edge - window.start).max(1.0),
        None if state.duration > 0.0 => state.position / state.duration,
        None => 0.0,
    };
    mini.progress.set_fraction(fraction.clamp(0.0, 1.0));
    let (icon, tooltip) = if state.paused || state.ended {
        ("yt-play-symbolic", "Play (k)")
    } else {
        ("yt-pause-symbolic", "Pause (k)")
    };
    mini.play.set_icon_name(icon);
    mini.play.set_tooltip_text(Some(tooltip));
}

fn run(command: &str) {
    if let Err(e) = app().ext.commands.run(command, Value::Null) {
        app().toast(&e);
    }
}
