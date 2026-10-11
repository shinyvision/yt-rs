//! The user's own pages, as on youtube.com:
//!
//! - `library` (the "You" page): who is signed in, then rows of their
//!   history, playlists, Watch later and liked videos, each with "View
//!   all". Rows load on their own, so one slow list doesn't hold the rest.
//! - `playlists`: all their playlists, as a grid of cards.

use std::rc::Rc;

use adw::prelude::*;
use serde_json::{Value, json};
use yt_core::playlist::{LIKED, WATCH_LATER};

use super::shelves::{Items, row};
use super::{attach, attached, follow};
use crate::app::app;
use crate::runtime;

/// How many videos a row of the "You" page shows.
const ROW_LENGTH: usize = 20;

/// The `library` component.
pub fn build_library(_props: &Value) -> gtk::Widget {
    // Signed out, youtube.com's prompt: it is all the account's.
    if !crate::social::signed_in() {
        let prompt = super::signin::prompt("library");
        prompt.set_widget_name("page-library");
        return prompt;
    }
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    column.add_css_class("library");
    column.append(&account_header());
    for section in ["history", "playlists", "watch_later", "liked"] {
        let place = gtk::Box::new(gtk::Orientation::Vertical, 0);
        place.set_widget_name(&format!("library-{section}"));
        column.append(&place);
        load_section(&place, section);
    }
    column.append(
        &app()
            .ext
            .slots
            .create("library.bottom", gtk::Orientation::Vertical, 0),
    );
    let scroller = gtk::ScrolledWindow::builder()
        .child(&column)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    scroller.set_widget_name("page-library");
    // Lists change as the user saves and removes videos.
    for event in ["playlist.changed", "playlist.created", "playlist.deleted"] {
        let weak = column.downgrade();
        follow(&scroller, event, move |_| {
            if let Some(column) = weak.upgrade() {
                refresh(&column);
            }
        });
    }
    scroller.upcast()
}

/// The user's picture, name and handle, and a way to their channel.
fn account_header() -> gtk::Widget {
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    header.add_css_class("library-header");
    let account = crate::social::account();
    let name = account.as_ref().map(|a| a.name.clone()).unwrap_or_default();
    let avatar = adw::Avatar::new(120, Some(&name), true);
    if let Some(account) = &account {
        super::avatar::set(&avatar, &account.name, account.photo.as_deref());
    }
    let title = gtk::Label::new(Some(&name));
    title.add_css_class("library-name");
    title.set_xalign(0.0);
    let handle = gtk::Label::new(account.as_ref().and_then(|a| a.handle.as_deref()));
    handle.add_css_class("dim");
    handle.set_xalign(0.0);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 4);
    text.set_valign(gtk::Align::Center);
    text.append(&title);
    text.append(&handle);
    if let Some(handle) = account.as_ref().and_then(|a| a.handle.clone()) {
        let view = gtk::Button::with_label("View channel");
        view.add_css_class("flat");
        view.add_css_class("library-view-channel");
        view.set_halign(gtk::Align::Start);
        view.connect_clicked(move |_| {
            if let Err(e) = app()
                .ext
                .commands
                .run("channel.open", json!({ "handle": handle }))
            {
                app().toast(&e);
            }
        });
        text.append(&view);
    }
    header.append(&avatar);
    header.append(&text);
    header.set_visible(account.is_some());
    header.upcast()
}

fn refresh(column: &gtk::Box) {
    for section in ["history", "playlists", "watch_later", "liked"] {
        if let Some(place) = super::find_widget(column.upcast_ref(), &format!("library-{section}"))
            .and_downcast::<gtk::Box>()
        {
            load_section(&place, section);
        }
    }
}

/// Fills `place` with a row of the section, once it has loaded; leaves it
/// empty if the list is (or failed to load: the rest of the page stays).
fn load_section(place: &gtk::Box, section: &'static str) {
    let weak = place.downgrade();
    runtime::spawn_local(async move {
        let youtube = app().youtube.clone();
        let loaded = runtime::background(async move {
            let videos = |list: yt_core::Result<yt_core::playlist::PlaylistPage>| {
                list.map(|p| {
                    let count = p.playlist.count;
                    (
                        Items::Videos(p.videos.into_iter().take(ROW_LENGTH).collect()),
                        count,
                    )
                })
            };
            match section {
                "history" => youtube.browse("FEhistory").await.map(|page| {
                    let videos = page
                        .shelves
                        .into_iter()
                        .flat_map(|s| s.videos)
                        .filter(|v| !v.short)
                        .take(ROW_LENGTH)
                        .collect();
                    (Items::Videos(videos), None)
                }),
                "playlists" => youtube
                    .playlists()
                    .await
                    .map(|p| (Items::Playlists(p), None)),
                "watch_later" => videos(youtube.playlist(WATCH_LATER).await),
                _ => videos(youtube.playlist(LIKED).await),
            }
        })
        .await;
        let Some(place) = weak.upgrade() else { return };
        while let Some(old) = place.first_child() {
            place.remove(&old);
        }
        let (items, count) = match loaded {
            Ok(loaded) => loaded,
            Err(e) => {
                tracing::warn!("You page, {section}: {e}");
                return;
            }
        };
        let empty = match &items {
            Items::Videos(v) => v.is_empty(),
            Items::Playlists(p) => p.is_empty(),
        };
        if empty {
            return;
        }
        let (title, page, args) = match section {
            "history" => ("History", "history", Value::Null),
            "playlists" => ("Playlists", "playlists", Value::Null),
            "watch_later" => (
                "Watch later",
                "playlist",
                json!({ "playlist_id": WATCH_LATER }),
            ),
            _ => ("Liked videos", "playlist", json!({ "playlist_id": LIKED })),
        };
        let title = match count {
            Some(count) => format!("{title} \u{2022} {}", count.trim_end_matches(" videos")),
            None => title.to_owned(),
        };
        place.append(&row(&title, items, false, Some((page, args))));
    });
}

/// The `playlists` component: playlists as a grid; the user's, or with
/// `{ browse_id, params }` a page's (a channel's Playlists tab), which has
/// no title.
pub fn build_playlists(props: &Value) -> gtk::Widget {
    // The user's playlists need them signed in; a channel's don't.
    if props.get("browse_id").is_none() && !crate::social::signed_in() {
        let prompt = super::signin::prompt("playlists");
        prompt.set_widget_name("page-playlists");
        return prompt;
    }
    let title = gtk::Label::new(Some("Playlists"));
    title.add_css_class("page-title");
    title.set_xalign(0.0);
    title.set_visible(props.get("browse_id").is_none());
    let grid = gtk::FlowBox::new();
    grid.add_css_class("playlist-grid");
    grid.set_selection_mode(gtk::SelectionMode::None);
    grid.set_homogeneous(true);
    grid.set_min_children_per_line(1);
    grid.set_max_children_per_line(6);
    grid.set_column_spacing(16);
    grid.set_row_spacing(32);
    grid.set_valign(gtk::Align::Start);
    let column = gtk::Box::new(gtk::Orientation::Vertical, 16);
    column.add_css_class("playlists-page");
    column.append(&title);
    column.append(&grid);
    let scroller = gtk::ScrolledWindow::builder()
        .child(&column)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    scroller.set_widget_name("page-playlists");
    let page = Rc::new(Grid {
        grid,
        source: std::cell::RefCell::new(source(props)),
    });
    attach(&scroller, page.clone());
    for event in ["playlist.created", "playlist.deleted", "playlist.changed"] {
        let weak = Rc::downgrade(&page);
        follow(&scroller, event, move |_| {
            if let Some(page) = weak.upgrade() {
                load_playlists(&page);
            }
        });
    }
    load_playlists(&page);
    scroller.upcast()
}

pub fn update_playlists(widget: &gtk::Widget, props: &Value) {
    if let Some(page) = attached::<Grid>(widget) {
        *page.source.borrow_mut() = source(props);
        load_playlists(&page);
    }
}

struct Grid {
    grid: gtk::FlowBox,
    /// A browse page and its params; `None` for the user's playlists.
    source: std::cell::RefCell<Option<(String, String)>>,
}

fn source(props: &Value) -> Option<(String, String)> {
    let text = |key: &str| props.get(key).and_then(Value::as_str).map(str::to_owned);
    Some((text("browse_id")?, text("params").unwrap_or_default()))
}

fn load_playlists(page: &Grid) {
    let weak = page.grid.downgrade();
    let source = page.source.borrow().clone();
    runtime::spawn_local(async move {
        let youtube = app().youtube.clone();
        let result = runtime::background(async move {
            match source {
                Some((browse_id, params)) => youtube
                    .browse_with(&browse_id, &params)
                    .await
                    .map(|page| page.shelves.into_iter().flat_map(|s| s.playlists).collect()),
                None => youtube.playlists().await,
            }
        })
        .await;
        let Some(grid) = weak.upgrade() else { return };
        match result {
            Ok(playlists) => {
                grid.remove_all();
                for playlist in playlists {
                    let props = serde_json::to_value(&playlist).unwrap_or_default();
                    grid.append(&app().ext.components.build("playlist_card", &props));
                }
            }
            Err(e) => app().toast(&format!("Couldn't load your playlists: {e}")),
        }
    });
}
