//! The `settings` component (the Settings page): playback choices, what
//! plugins add (the `settings.sections` slot, for their own groups of
//! rows), the account, and about: the version and the keyboard shortcuts.

use adw::prelude::*;
use serde_json::{Value, json};

use crate::app::app;

pub fn build(_props: &Value) -> gtk::Widget {
    let page = adw::PreferencesPage::new();
    page.set_widget_name("page-settings");
    page.add_css_class("settings-page");
    page.add(&playback());
    page.add(&downloads());
    let sections = app()
        .ext
        .slots
        .create("settings.sections", gtk::Orientation::Vertical, 24);
    let group = adw::PreferencesGroup::new();
    group.add(&sections);
    page.add(&group);
    page.add(&account());
    page.add(&about());
    page.upcast()
}

fn playback() -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("Playback").build();
    let settings = app().settings.get();
    let switch = |title: &str, subtitle: &str, on: bool, change: fn(bool)| {
        let row = adw::SwitchRow::builder()
            .title(title)
            .subtitle(subtitle)
            .active(on)
            .build();
        row.connect_active_notify(move |row| change(row.is_active()));
        group.add(&row);
    };
    switch(
        "Autoplay",
        "When a video ends, play YouTube's next suggestion",
        settings.autoplay,
        |on| run("player.set_autoplay", json!({ "enabled": on })),
    );
    switch(
        "Theater mode",
        "The wide player, with the details and \u{201c}Up next\u{201d} below it",
        settings.theater,
        |on| {
            if app().settings.get().theater != on {
                run("watch.toggle_theater", Value::Null);
            }
        },
    );
    switch(
        "Stable volume",
        "Even out loud and quiet parts",
        settings.stable_volume,
        |on| run("player.set_stable_volume", json!({ "enabled": on })),
    );
    switch(
        "Captions",
        "Show captions when a video has them, in your language",
        settings.captions.is_some(),
        |on| {
            let lang = on.then(locale_language);
            app().settings.update(|s| s.captions = lang.clone());
            if app().player.state().video_id.is_some() {
                run("player.set_captions", json!({ "lang": lang }));
            }
        },
    );
    group
}

/// The desktop's language, as YouTube names captions: "en", "nl", "pt-BR".
fn locale_language() -> String {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|key| {
            std::env::var(key)
                .ok()
                .filter(|v| !v.is_empty() && v != "C")
        })
        .unwrap_or_default();
    let tag = locale.split(['.', '@']).next().unwrap_or_default();
    match tag.split_once('_') {
        // YouTube tells some variants apart; for the rest the language is enough.
        Some((lang @ ("pt" | "zh"), region)) => format!("{lang}-{region}"),
        Some((lang, _)) => lang.to_owned(),
        None if tag.is_empty() || tag == "POSIX" => "en".to_owned(),
        None => tag.to_owned(),
    }
}

/// The qualities offered for downloads: (name, the highest height).
const DOWNLOAD_QUALITIES: &[(&str, Option<u32>)] = &[
    ("Highest (up to 1080p)", None),
    ("720p", Some(720)),
    ("480p", Some(480)),
    ("360p (least space)", Some(360)),
];

fn downloads() -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("Downloads").build();
    let names: Vec<&str> = DOWNLOAD_QUALITIES.iter().map(|(n, _)| *n).collect();
    let row = adw::ComboRow::builder()
        .title("Quality")
        .subtitle("How sharp downloaded videos are; the sharper, the more space they take")
        .model(&gtk::StringList::new(&names))
        .build();
    let current = app().settings.get().download_height;
    row.set_selected(
        DOWNLOAD_QUALITIES
            .iter()
            .position(|(_, h)| *h == current)
            .unwrap_or(0) as u32,
    );
    row.connect_selected_notify(|row| {
        let height = DOWNLOAD_QUALITIES
            .get(row.selected() as usize)
            .and_then(|(_, h)| *h);
        app().settings.update(|s| s.download_height = height);
    });
    group.add(&row);
    group
}

fn account() -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("Account").build();
    let row = adw::ActionRow::new();
    let button = gtk::Button::new();
    button.add_css_class("pill-button");
    button.add_css_class("outline");
    button.set_valign(gtk::Align::Center);
    match crate::social::account() {
        Some(account) => {
            row.set_title(&account.name);
            row.set_subtitle(account.handle.as_deref().unwrap_or_default());
            let avatar = adw::Avatar::new(40, Some(&account.name), true);
            super::avatar::set(&avatar, &account.name, account.photo.as_deref());
            row.add_prefix(&avatar);
            button.set_label("Sign out");
            button.connect_clicked(|b| {
                let props = json!({
                    "title": "Sign out?",
                    "body": "yt-rs forgets your sign-in on this computer.",
                    "confirm": "Sign out",
                });
                super::dialog::confirm(b, &props, || run("auth.sign_out", Value::Null));
            });
        }
        None => {
            row.set_title("Not signed in");
            row.set_subtitle("Sign in for your feed, playlists, history and comments");
            button.set_label("Sign in");
            button.connect_clicked(|_| app().navigate("signin", Value::Null, true));
        }
    }
    row.add_suffix(&button);
    group.add(&row);
    group
}

fn about() -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("About").build();
    let version = adw::ActionRow::builder()
        .title("yt-rs")
        .subtitle(format!(
            "Version {}: a YouTube client of its own, not made by YouTube",
            env!("CARGO_PKG_VERSION")
        ))
        .build();
    group.add(&version);
    let shortcuts = adw::ActionRow::builder()
        .title("Keyboard shortcuts")
        .subtitle("?")
        .activatable(true)
        .build();
    shortcuts.add_suffix(&gtk::Image::from_icon_name("yt-chevron-right-symbolic"));
    shortcuts.connect_activated(|_| run("app.shortcuts", Value::Null));
    group.add(&shortcuts);
    let plugins = app()
        .plugins()
        .map(|p| p.loaded().to_vec())
        .unwrap_or_default();
    let row = adw::ActionRow::builder()
        .title("Plugins")
        .subtitle(if plugins.is_empty() {
            format!(
                "None. Lua plugins go in {}",
                yt_core::paths::plugins_dir().display()
            )
        } else {
            plugins.join(", ")
        })
        .build();
    group.add(&row);
    group
}

fn run(command: &str, args: Value) {
    if let Err(e) = app().ext.commands.run(command, args) {
        app().toast(&e);
    }
}
