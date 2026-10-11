//! The player's settings panel (the gear), as on youtube.com: stable volume,
//! captions, sleep timer, playback speed and quality, each opening a list
//! of choices.
//!
//! The panel is built from data, `{ entries = [...] }`, after the
//! `player.settings` filter, so plugins can add their own. An entry is a
//! switch, `{ label, icon, toggle = true, active, command }` (run with
//! `{ enabled }`), or a list, `{ label, icon, value, options = [{ label,
//! command, args?, selected? }] }`.

use gtk::prelude::*;
use serde_json::{Value, json};

use crate::app::app;
use crate::playback::{self, SleepTimer};

/// Speeds offered, as on youtube.com.
const SPEEDS: [f64; 8] = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];
/// Sleep timer choices, in minutes.
const SLEEP_MINUTES: [u32; 7] = [10, 15, 20, 30, 45, 60, 90];

/// The entries for the current video and state.
pub fn entries() -> Value {
    let app = app();
    let state = app.player.state();
    let streams = app.player.streams().unwrap_or_default();
    let mut entries = vec![json!({
        "label": "Stable volume", "icon": "yt-stable-volume-symbolic", "toggle": true,
        "active": state.stable_volume, "command": "player.set_stable_volume",
    })];

    if !streams.captions.is_empty() {
        let mut options = vec![json!({
            "label": "Off", "command": "player.set_captions", "args": { "lang": null },
            "selected": state.captions.is_none(),
        })];
        options.extend(streams.captions.iter().map(|c| {
            json!({
                "label": c.label(), "command": "player.set_captions", "args": { "lang": c.lang },
                "selected": state.captions.as_deref() == Some(c.lang.as_str()),
            })
        }));
        let value = state
            .captions
            .as_deref()
            .and_then(|lang| playback::best_caption(&streams.captions, lang))
            .map_or("Off".to_owned(), |c| c.label());
        entries.push(json!({
            "label": format!("Subtitles/CC ({})", streams.captions.len()),
            "icon": "yt-subtitles-symbolic", "value": value, "options": options,
        }));
    }

    let sleep = playback::sleep_timer();
    let mut options = vec![json!({
        "label": "Off", "command": "player.sleep_timer", "args": { "minutes": null },
        "selected": matches!(sleep, SleepTimer::Off),
    })];
    options.extend(SLEEP_MINUTES.iter().map(|m| {
        json!({ "label": format!("{m} minutes"), "command": "player.sleep_timer", "args": { "minutes": m } })
    }));
    options.push(json!({
        "label": "End of video", "command": "player.sleep_timer", "args": { "minutes": "end" },
        "selected": matches!(sleep, SleepTimer::EndOfVideo),
    }));
    entries.push(json!({
        "label": "Sleep timer", "icon": "yt-timer-symbolic", "options": options,
        "value": match sleep {
            SleepTimer::Off => "Off".to_owned(),
            SleepTimer::EndOfVideo => "End of video".to_owned(),
            SleepTimer::Minutes(m) => format!("{} min", m.ceil()),
        },
    }));

    let options: Vec<Value> = SPEEDS
        .iter()
        .map(|s| {
            json!({
                "label": speed_label(*s), "command": "player.set_speed", "args": { "speed": s },
                "selected": (state.speed - s).abs() < 0.01,
            })
        })
        .collect();
    entries.push(json!({
        "label": "Playback speed", "icon": "yt-speed-symbolic",
        "value": speed_label(state.speed), "options": options,
    }));

    if !streams.qualities.is_empty() {
        let mut options: Vec<Value> = streams
            .qualities
            .iter()
            .enumerate()
            .map(|(i, q)| {
                let label = q.quality_label().unwrap_or_default();
                json!({
                    "label": label, "badge": badge(q.height), "command": "player.set_quality",
                    "args": { "index": i },
                    "selected": !state.auto_quality && state.quality.as_deref() == Some(label.as_str()),
                })
            })
            .collect();
        options.push(json!({
            "label": "Auto", "command": "player.set_quality", "args": { "index": null },
            "selected": state.auto_quality,
        }));
        let current = state.quality.clone().unwrap_or_default();
        entries.push(json!({
            "label": "Quality", "icon": "yt-tune-symbolic", "options": options,
            "value": if state.auto_quality { format!("Auto ({current})") } else { current },
        }));
    }
    app.ext
        .filters
        .apply("player.settings", json!({ "entries": entries }))
}

/// "Normal", "1.5"
fn speed_label(speed: f64) -> String {
    if (speed - 1.0).abs() < 0.01 {
        "Normal".into()
    } else {
        format!("{speed}")
    }
}

/// "HD" from 720p, "4K" from 2160p, as YouTube marks them.
fn badge(height: Option<u32>) -> Option<&'static str> {
    match height? {
        h if h >= 4320 => Some("8K"),
        h if h >= 2160 => Some("4K"),
        h if h >= 720 => Some("HD"),
        _ => None,
    }
}

/// The `player_settings` component: the panel's pages.
pub fn build(props: &Value) -> gtk::Widget {
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
    stack.set_transition_duration(180);
    stack.set_hhomogeneous(false);
    stack.set_vhomogeneous(false);
    stack.set_interpolate_size(true);
    stack.add_css_class("player-settings");

    let main = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let entries = props
        .get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for (i, entry) in entries.iter().enumerate() {
        let label = entry
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let icon = entry.get("icon").and_then(Value::as_str);
        if entry
            .get("toggle")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            main.append(&toggle_row(label, icon, entry));
            continue;
        }
        let page = format!("page-{i}");
        let value = entry
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default();
        main.append(&list_row(label, icon, value, &stack, &page));
        stack.add_named(&options_page(label, entry, &stack), Some(&page));
    }
    stack.add_named(&main, Some("main"));
    stack.set_visible_child_name("main");
    stack.upcast()
}

fn row_content(icon: Option<&str>, label: &str) -> gtk::Box {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let image = gtk::Image::new();
    if let Some(icon) = icon {
        image.set_icon_name(Some(icon));
    }
    image.add_css_class("settings-icon");
    content.append(&image);
    let title = gtk::Label::new(Some(label));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    content.append(&title);
    content
}

fn toggle_row(label: &str, icon: Option<&str>, entry: &Value) -> gtk::Widget {
    let content = row_content(icon, label);
    let switch = gtk::Switch::new();
    switch.set_active(
        entry
            .get("active")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    );
    switch.set_valign(gtk::Align::Center);
    switch.set_can_focus(false);
    switch.set_can_target(false);
    content.append(&switch);
    let button = gtk::Button::builder()
        .child(&content)
        .css_classes(["flat", "settings-row"])
        .build();
    let command = entry
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    button.connect_clicked(move |_| {
        let enabled = !switch.is_active();
        switch.set_active(enabled);
        run(&command, json!({ "enabled": enabled }));
    });
    button.upcast()
}

fn list_row(
    label: &str,
    icon: Option<&str>,
    value: &str,
    stack: &gtk::Stack,
    page: &str,
) -> gtk::Widget {
    let content = row_content(icon, label);
    let value = gtk::Label::new(Some(value));
    value.add_css_class("settings-value");
    content.append(&value);
    content.append(&gtk::Image::from_icon_name("yt-chevron-right-symbolic"));
    let button = gtk::Button::builder()
        .child(&content)
        .css_classes(["flat", "settings-row"])
        .build();
    let (stack, page) = (stack.downgrade(), page.to_owned());
    button.connect_clicked(move |_| {
        if let Some(stack) = stack.upgrade() {
            stack.set_visible_child_name(&page);
        }
    });
    button.upcast()
}

fn options_page(title: &str, entry: &Value, stack: &gtk::Stack) -> gtk::Widget {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let back_content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    back_content.append(&gtk::Image::from_icon_name("yt-chevron-left-symbolic"));
    let heading = gtk::Label::new(Some(title));
    heading.set_xalign(0.0);
    back_content.append(&heading);
    let back = gtk::Button::builder()
        .child(&back_content)
        .css_classes(["flat", "settings-back"])
        .build();
    let weak = stack.downgrade();
    back.connect_clicked(move |_| {
        if let Some(stack) = weak.upgrade() {
            stack.set_visible_child_name("main");
        }
    });
    page.append(&back);
    page.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    for option in entry
        .get("options")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let check = gtk::Image::from_icon_name("yt-check-symbolic");
        check.add_css_class("settings-check");
        check.set_opacity(
            if option
                .get("selected")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                1.0
            } else {
                0.0
            },
        );
        content.append(&check);
        let label = gtk::Label::new(option.get("label").and_then(Value::as_str));
        label.set_xalign(0.0);
        content.append(&label);
        if let Some(badge) = option.get("badge").and_then(Value::as_str) {
            let badge = gtk::Label::new(Some(badge));
            badge.add_css_class("settings-badge");
            badge.set_valign(gtk::Align::Start);
            content.append(&badge);
        }
        let button = gtk::Button::builder()
            .child(&content)
            .css_classes(["flat", "settings-option"])
            .build();
        let command = option
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let args = option.get("args").cloned().unwrap_or_else(|| json!({}));
        button.connect_clicked(move |button| {
            if let Some(popover) = button
                .ancestor(gtk::Popover::static_type())
                .and_downcast::<gtk::Popover>()
            {
                popover.popdown();
            }
            run(&command, args.clone());
        });
        list.append(&button);
    }
    // Long lists (qualities, captions) scroll.
    let scroller = gtk::ScrolledWindow::builder()
        .child(&list)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(320)
        .build();
    page.append(&scroller);
    page.upcast()
}

fn run(command: &str, args: Value) {
    if let Err(e) = app().ext.commands.run(command, args) {
        app().toast(&e);
    }
}
