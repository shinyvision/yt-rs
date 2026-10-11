//! The `downloads` component (the Downloads page): the videos kept for
//! watching offline, newest first, each with its picture, title, channel,
//! quality and size, how far along a download under way is, and buttons to
//! stop or delete it. A click plays it from its files. With none, it says
//! how to download one.

use std::rc::Rc;

use adw::prelude::*;
use serde_json::{Value, json};
use yt_core::downloads::Download;

use super::{attach, attached, follow};
use crate::app::app;

struct Page {
    list: gtk::Box,
    /// The page itself, weakly: it owns this.
    stack: gtk::glib::WeakRef<gtk::Stack>,
}

pub fn build(_props: &Value) -> gtk::Widget {
    let title = gtk::Label::new(Some("Downloads"));
    title.add_css_class("page-title");
    title.set_xalign(0.0);
    let list = gtk::Box::new(gtk::Orientation::Vertical, 8);
    list.add_css_class("downloads");
    let column = gtk::Box::new(gtk::Orientation::Vertical, 16);
    column.add_css_class("downloads-page");
    column.append(&title);
    column.append(&list);
    let clamp = adw::Clamp::builder()
        .child(&column)
        .maximum_size(1000)
        .tightening_threshold(1000)
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .child(&clamp)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    let empty = adw::StatusPage::builder()
        .icon_name("yt-download-symbolic")
        .title("No downloads")
        .description("Videos you download play here, offline too. \u{201c}Download\u{201d} is in a video\u{2019}s \u{22ee} menu and beside its likes.")
        .build();
    let stack = gtk::Stack::new();
    stack.add_named(&scroller, Some("list"));
    stack.add_named(&empty, Some("empty"));
    stack.set_widget_name("page-downloads");
    let page = Rc::new(Page {
        list,
        stack: stack.downgrade(),
    });
    attach(&stack, page.clone());
    let weak = Rc::downgrade(&page);
    follow(&stack, "download.changed", move |payload| {
        let Some(page) = weak.upgrade() else { return };
        // A row's progress changes in place; anything else rebuilds.
        let id = payload
            .get("video_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !update_row(&page, id, payload) {
            fill(&page);
        }
    });
    fill(&page);
    stack.upcast()
}

pub fn update(widget: &gtk::Widget, _props: &Value) {
    if let Some(page) = attached::<Page>(widget) {
        fill(&page);
    }
}

fn fill(page: &Page) {
    while let Some(child) = page.list.first_child() {
        page.list.remove(&child);
    }
    let downloads = crate::downloads::list();
    if let Some(stack) = page.stack.upgrade() {
        stack.set_visible_child_name(if downloads.is_empty() {
            "empty"
        } else {
            "list"
        });
    }
    for download in downloads {
        page.list.append(&row(&download));
    }
}

/// Shows a download's progress in its row, if it has one.
fn update_row(page: &Page, video_id: &str, payload: &Value) -> bool {
    let Some(download) = payload
        .get("download")
        .and_then(|d| serde_json::from_value::<Download>(d.clone()).ok())
    else {
        return false;
    };
    let Some(row) = super::find_widget(page.list.upcast_ref(), &format!("download-{video_id}"))
    else {
        return false;
    };
    let running = payload
        .get("running")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if let Some(bar) =
        super::find_widget(&row, "download-progress").and_downcast::<gtk::ProgressBar>()
    {
        bar.set_fraction(download.progress());
        bar.set_visible(!download.complete);
    }
    if let Some(label) = super::find_widget(&row, "download-status").and_downcast::<gtk::Label>() {
        label.set_label(&status(&download, running));
    }
    // From under way to done (or stopped): the buttons change.
    let was_running = super::find_widget(&row, "download-cancel").is_some_and(|b| b.is_visible());
    was_running == running
}

fn status(download: &Download, running: bool) -> String {
    let size = |bytes: u64| format!("{:.1} MB", bytes as f64 / 1e6);
    let mut parts: Vec<String> = Vec::new();
    if let Some(q) = &download.quality {
        parts.push(q.clone());
    }
    if download.complete {
        parts.push(size(download.total.max(download.bytes)));
    } else if running {
        parts.push(format!(
            "Downloading {:.0} % of {}",
            download.progress() * 100.0,
            if download.total > 0 {
                size(download.total)
            } else {
                "…".into()
            }
        ));
    } else {
        parts.push(format!("Stopped at {:.0} %", download.progress() * 100.0));
    }
    parts.join(" \u{2022} ")
}

fn row(download: &Download) -> gtk::Widget {
    let id = download.video.id.clone();
    let running = crate::downloads::is_running(&id);
    let picture = gtk::Picture::new();
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.set_can_shrink(true);
    let thumbnail = download.thumbnail_path();
    if thumbnail.is_file() {
        picture.set_filename(Some(&thumbnail));
    }
    let thumb = super::aspect::AspectBin::new(9.0 / 16.0, &picture);
    thumb.add_css_class("thumb");
    thumb.set_overflow(gtk::Overflow::Hidden);
    thumb.set_natural_width(240);
    thumb.set_size_request(240, -1);
    thumb.set_valign(gtk::Align::Start);

    let label = |text: &str, class: &str| {
        let l = gtk::Label::new(Some(text));
        l.add_css_class(class);
        l.set_xalign(0.0);
        l.set_wrap(true);
        l.set_lines(2);
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        l
    };
    let title = label(&download.video.title, "download-title");
    let channel = label(
        download.video.channel.as_deref().unwrap_or_default(),
        "meta",
    );
    channel.add_css_class("dim");
    let state = label(&status(download, running), "meta");
    state.add_css_class("dim");
    state.set_widget_name("download-status");
    let bar = gtk::ProgressBar::new();
    bar.set_widget_name("download-progress");
    bar.add_css_class("download-progress");
    bar.set_fraction(download.progress());
    bar.set_visible(!download.complete);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 4);
    text.set_hexpand(true);
    text.append(&title);
    text.append(&channel);
    text.append(&state);
    text.append(&bar);

    let button = |icon: &str, tooltip: &str, name: &str| {
        let b = gtk::Button::from_icon_name(icon);
        b.add_css_class("icon-button");
        b.set_tooltip_text(Some(tooltip));
        b.set_widget_name(name);
        b.set_valign(gtk::Align::Start);
        b
    };
    let cancel = button("yt-close-symbolic", "Stop downloading", "download-cancel");
    cancel.set_visible(running);
    let resume = button(
        "yt-download-symbolic",
        "Carry on downloading",
        "download-resume",
    );
    resume.set_visible(!running && !download.complete);
    let delete = button("yt-delete-symbolic", "Delete", "download-delete");
    let run = |command: &'static str, args: Value| {
        move |_: &gtk::Button| {
            if let Err(e) = app().ext.commands.run(command, args.clone()) {
                app().toast(&e);
            }
        }
    };
    cancel.connect_clicked(run("download.cancel", json!({ "video_id": id })));
    resume.connect_clicked(run(
        "download.start",
        json!({ "video": download.video, "quality": download.quality }),
    ));
    let video_id = id.clone();
    delete.connect_clicked(move |b| {
        let props = json!({
            "title": "Delete download?",
            "body": "It won't play offline any more.",
            "confirm": "Delete",
        });
        let id = video_id.clone();
        super::dialog::confirm(b, &props, move || {
            if let Err(e) = app()
                .ext
                .commands
                .run("download.delete", json!({ "video_id": id }))
            {
                app().toast(&e);
            }
        });
    });

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.add_css_class("download-row");
    row.set_widget_name(&format!("download-{id}"));
    row.set_cursor_from_name(Some("pointer"));
    row.append(&thumb);
    row.append(&text);
    row.append(&cancel);
    row.append(&resume);
    row.append(&delete);
    let video = download.video.clone();
    let complete = download.complete;
    let click = gtk::GestureClick::new();
    click.connect_released(move |gesture, n, _, _| {
        if n == 1 && complete {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            if let Err(e) = app()
                .ext
                .commands
                .run("video.play", json!({ "video": video }))
            {
                app().toast(&e);
            }
        }
    });
    thumb.add_controller(click);
    row.upcast()
}
