//! "Keyboard shortcuts" (`?`): every key binding, plugins' too, grouped by
//! where it works, each with what it does in words. Built from the keymap
//! when shown, so it is never out of date.

use std::collections::BTreeMap;

use adw::prelude::*;
use serde_json::Value;

use crate::app::app;
use crate::ext::keymap::{GLOBAL, PLAYER};

pub fn present() {
    let app = app();
    let Some(window) = app.window() else { return };
    let mut scopes: BTreeMap<u8, (String, Vec<(String, String)>)> = BTreeMap::new();
    let mut seek_digits = false;
    for (scope, key, binding) in app.ext.keymap.list() {
        // The ten "seek to N %" keys are one line.
        if binding.command == "player.seek_percent" && key.len() == 1 && key != "0" {
            seek_digits = true;
            continue;
        }
        let (order, title) = match scope.as_str() {
            GLOBAL => (0, "Everywhere".to_owned()),
            PLAYER => (1, "Wherever the video shows".to_owned()),
            "watch" => (2, "On the watch page".to_owned()),
            other => (3, format!("On the {other} page")),
        };
        let entry = scopes.entry(order).or_insert((title, Vec::new()));
        let mut keys = key_name(&key);
        let mut what = describe(&binding.command, &binding.args);
        if binding.command == "player.seek_percent" && key == "0" && seek_digits {
            keys = "0 \u{2013} 9".into();
            what = "Seek to 0 \u{2013} 90 %".into();
        }
        entry.1.push((what, keys));
    }
    let column = gtk::Box::new(gtk::Orientation::Vertical, 24);
    column.add_css_class("shortcuts");
    for (_, (title, rows)) in scopes {
        // One line for what a command does, with every key that does it.
        let mut lines: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (what, keys) in rows {
            let line = lines.entry(what).or_default();
            if !line.contains(&keys) {
                line.push(keys);
            }
        }
        let group = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let heading = gtk::Label::new(Some(&title));
        heading.add_css_class("shortcuts-heading");
        heading.set_xalign(0.0);
        group.append(&heading);
        for (what, keys) in lines {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.add_css_class("shortcut");
            let label = gtk::Label::new(Some(&what));
            label.set_xalign(0.0);
            label.set_hexpand(true);
            label.set_margin_end(10);
            row.append(&label);
            for keys in keys {
                let key = gtk::Label::new(Some(&keys));
                key.add_css_class("keycap");
                key.set_valign(gtk::Align::Center);
                row.append(&key);
            }
            group.append(&row);
        }
        column.append(&group);
    }
    let scroller = gtk::ScrolledWindow::builder()
        .child(&column)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(640)
        .build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&scroller));
    let dialog = adw::Dialog::builder()
        .title("Keyboard shortcuts")
        .content_width(520)
        .child(&toolbar)
        .build();
    dialog.add_css_class("yt-dialog");
    dialog.set_widget_name("shortcuts-dialog");
    dialog.present(Some(&window));
}

/// A key as people write it: "Shift+N", "Ctrl+Q", "←", "Space".
fn key_name(accel: &str) -> String {
    let Some((key, mods)) = gtk::accelerator_parse(accel) else {
        return accel.to_owned();
    };
    let mut parts = Vec::new();
    for (flag, name) in [
        (gtk::gdk::ModifierType::CONTROL_MASK, "Ctrl"),
        (gtk::gdk::ModifierType::ALT_MASK, "Alt"),
        (gtk::gdk::ModifierType::SHIFT_MASK, "Shift"),
        (gtk::gdk::ModifierType::SUPER_MASK, "Super"),
    ] {
        if mods.contains(flag) {
            parts.push(name.to_owned());
        }
    }
    let name = match key.name().as_deref() {
        Some("Left") => "\u{2190}".to_owned(),
        Some("Right") => "\u{2192}".to_owned(),
        Some("Up") => "\u{2191}".to_owned(),
        Some("Down") => "\u{2193}".to_owned(),
        Some("space") => "Space".to_owned(),
        Some("Escape") => "Esc".to_owned(),
        Some("greater") => ">".to_owned(),
        Some("less") => "<".to_owned(),
        Some("period") => ".".to_owned(),
        Some("comma") => ",".to_owned(),
        Some("slash") => "/".to_owned(),
        Some("question") => "?".to_owned(),
        // A letter as on the key with a modifier ("Ctrl+Q"), as typed
        // without one ("k"; "Shift+N" says Shift).
        Some(other) if other.chars().count() == 1 && !parts.is_empty() => other.to_uppercase(),
        Some(other) if other.chars().count() == 1 => other.to_lowercase(),
        Some(other) => other.to_owned(),
        None => accel.to_owned(),
    };
    parts.push(name);
    parts.join("+")
}

/// What a command does, in words; its name for ones the app doesn't know.
fn describe(command: &str, args: &Value) -> String {
    let number = |key: &str| args.get(key).and_then(Value::as_f64).unwrap_or(0.0);
    let text = match command {
        "app.back" => "Go back",
        "app.forward" => "Go forward",
        "app.quit" => "Quit",
        "app.shortcuts" => "Keyboard shortcuts",
        "window.toggle_fullscreen" => "Full screen",
        "player.mini" => "Mini player",
        "search.focus" => "Search",
        "feed.refresh" => "Refresh the feed",
        "sidebar.toggle" => "Menu",
        "player.toggle_pause" => "Play or pause",
        "player.toggle_mute" => "Mute",
        "player.toggle_captions" => "Captions on or off",
        "player.next" => "Next video",
        "list.previous" => "Previous video in the list",
        "watch.toggle_theater" => "Theater mode",
        "player.seek_end" => "Go to the end",
        "player.seek_by" => {
            let s = number("seconds");
            return format!(
                "{} {} seconds",
                if s < 0.0 { "Back" } else { "Forward" },
                s.abs()
            );
        }
        "player.change_volume" => {
            return if number("delta") < 0.0 {
                "Volume down"
            } else {
                "Volume up"
            }
            .into();
        }
        "player.change_speed" => {
            return if number("delta") < 0.0 {
                "Slower"
            } else {
                "Faster"
            }
            .into();
        }
        "player.frame_step" => {
            return if args.get("forward").and_then(Value::as_bool) == Some(false) {
                "Previous frame (paused)"
            } else {
                "Next frame (paused)"
            }
            .into();
        }
        "player.seek_percent" => return format!("Seek to {} %", number("percent")),
        other => other,
    };
    text.to_owned()
}
