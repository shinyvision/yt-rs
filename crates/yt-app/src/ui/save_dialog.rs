//! "Save video to...", as on youtube.com: the user's playlists with a box
//! ticked for each one holding the video; ticking or clearing one saves or
//! takes the video out at once. "New playlist" opens a name field and a
//! choice of who may see it, and Create makes the playlist with the video
//! in it.
//!
//! The content is the `save_dialog` component (`{ video_id }`); the
//! `video.save` command shows it.

use std::rc::Rc;

use adw::prelude::*;
use serde_json::{Value, json};
use yt_core::playlist::SaveOption;

use super::{attach, follow};
use crate::app::app;
use crate::runtime;

/// The choices for a new playlist, as YouTube names them.
const PRIVACY: &[(&str, &str)] = &[
    ("Private", "private"),
    ("Unlisted", "unlisted"),
    ("Public", "public"),
];

struct Save {
    video_id: String,
    list: gtk::Box,
    stack: gtk::Stack,
    status: gtk::Label,
    form: gtk::Revealer,
    name: gtk::Entry,
    privacy: gtk::DropDown,
    create: gtk::Button,
    new: gtk::Button,
}

/// Shows the dialog for the video.
pub fn present(video_id: &str) {
    let app = app();
    let Some(window) = app.window() else { return };
    let content = app
        .ext
        .components
        .build("save_dialog", &json!({ "video_id": video_id }));
    // It grows as the playlists arrive.
    let dialog = adw::Dialog::builder()
        .title("Save video to...")
        .follows_content_size(true)
        .child(&content)
        .build();
    dialog.add_css_class("yt-dialog");
    dialog.set_widget_name("save-dialog");
    // Making a playlist closes the dialog, as on youtube.com.
    let weak = dialog.downgrade();
    follow(&content, "playlist.created", move |_| {
        if let Some(dialog) = weak.upgrade() {
            dialog.close();
        }
    });
    dialog.present(Some(&window));
}

/// The `save_dialog` component.
pub fn build(props: &Value) -> gtk::Widget {
    let video_id = props
        .get("video_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let title = gtk::Label::new(Some("Save video to..."));
    title.add_css_class("yt-dialog-title");
    title.set_xalign(0.0);

    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.add_css_class("save-list");
    let scroller = gtk::ScrolledWindow::builder()
        .child(&list)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(320)
        .build();
    let spinner = adw::Spinner::new();
    spinner.set_size_request(32, 32);
    let status = gtk::Label::new(None);
    status.add_css_class("dim");
    status.set_wrap(true);
    // As tall as what it shows: the list once it has loaded.
    let stack = gtk::Stack::new();
    stack.set_vhomogeneous(false);
    stack.set_interpolate_size(true);
    stack.add_named(&spinner, Some("loading"));
    stack.add_named(&scroller, Some("list"));
    stack.add_named(&status, Some("status"));

    let new = gtk::Button::new();
    let new_box = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    new_box.append(&gtk::Image::from_icon_name("yt-add-symbolic"));
    new_box.append(&gtk::Label::new(Some("New playlist")));
    new.set_child(Some(&new_box));
    new.add_css_class("flat");
    new.add_css_class("save-new");
    new.set_widget_name("save-new");

    let name = gtk::Entry::new();
    name.set_placeholder_text(Some("Choose a title"));
    name.set_max_length(150);
    name.set_widget_name("save-name");
    let privacy = gtk::DropDown::from_strings(&PRIVACY.iter().map(|(l, _)| *l).collect::<Vec<_>>());
    privacy.set_widget_name("save-privacy");
    let create = gtk::Button::with_label("Create");
    create.add_css_class("yt-dialog-button");
    create.add_css_class("confirm");
    create.set_halign(gtk::Align::End);
    create.set_sensitive(false);
    create.set_widget_name("save-create");
    let fields = gtk::Box::new(gtk::Orientation::Vertical, 12);
    fields.add_css_class("save-form");
    fields.append(&name);
    fields.append(&privacy);
    fields.append(&create);
    let form = gtk::Revealer::builder()
        .child(&fields)
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .build();

    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.add_css_class("yt-dialog-content");
    root.add_css_class("save-dialog");
    root.append(&title);
    root.append(&stack);
    root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    root.append(&new);
    root.append(&form);

    let save = Rc::new(Save {
        video_id,
        list,
        stack,
        status,
        form,
        name,
        privacy,
        create,
        new,
    });
    attach(&root, save.clone());
    connect(&save, &root);
    load(&save);
    root.upcast()
}

fn connect(save: &Rc<Save>, root: &gtk::Box) {
    let weak = Rc::downgrade(save);
    save.new.connect_clicked(move |_| {
        if let Some(save) = weak.upgrade() {
            save.new.set_visible(false);
            save.form.set_reveal_child(true);
            save.name.grab_focus();
        }
    });
    let weak = Rc::downgrade(save);
    save.name.connect_changed(move |name| {
        if let Some(save) = weak.upgrade() {
            save.create.set_sensitive(!name.text().trim().is_empty());
        }
    });
    let create = {
        let weak = Rc::downgrade(save);
        move || {
            let Some(save) = weak.upgrade() else { return };
            let title = save.name.text().trim().to_owned();
            if title.is_empty() {
                return;
            }
            let privacy = PRIVACY
                .get(save.privacy.selected() as usize)
                .map_or("private", |(_, p)| p);
            save.create.set_sensitive(false);
            let args = json!({ "title": title, "privacy": privacy, "video_id": save.video_id });
            if let Err(e) = app().ext.commands.run("playlist.create", args) {
                app().toast(&e);
                save.create.set_sensitive(true);
            }
        }
    };
    let also = create.clone();
    save.create.connect_clicked(move |_| also());
    save.name.connect_activate(move |_| create());
    // A failed change puts the box back as it was.
    let weak = Rc::downgrade(save);
    follow(root, "playlist.changed.failed", move |payload| {
        let Some(save) = weak.upgrade() else { return };
        if payload.get("video_id").and_then(Value::as_str) != Some(save.video_id.as_str()) {
            return;
        }
        let added = payload
            .get("added")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if let Some(check) = payload
            .get("playlist_id")
            .and_then(Value::as_str)
            .and_then(|id| super::find_widget(save.list.upcast_ref(), &format!("save-{id}")))
            .and_downcast::<gtk::CheckButton>()
        {
            check.set_active(!added);
        }
    });
    let weak = Rc::downgrade(save);
    follow(root, "playlist.created.failed", move |_| {
        if let Some(save) = weak.upgrade() {
            save.create.set_sensitive(true);
        }
    });
}

fn load(save: &Rc<Save>) {
    save.stack.set_visible_child_name("loading");
    let weak = Rc::downgrade(save);
    let video_id = save.video_id.clone();
    runtime::spawn_local(async move {
        let youtube = app().youtube.clone();
        let result =
            runtime::background(async move { youtube.save_options(&video_id).await }).await;
        let Some(save) = weak.upgrade() else { return };
        match result {
            Ok(options) if !options.is_empty() => show(&save, options),
            Ok(_) => {
                save.status.set_label("You have no playlists yet.");
                save.stack.set_visible_child_name("status");
            }
            Err(e) => {
                save.status
                    .set_label(&format!("Couldn't load your playlists: {e}"));
                save.stack.set_visible_child_name("status");
            }
        }
    });
}

fn show(save: &Rc<Save>, options: Vec<SaveOption>) {
    for option in options {
        let check = gtk::CheckButton::new();
        check.set_active(option.contains);
        check.set_widget_name(&format!("save-{}", option.playlist_id));
        check.add_css_class("save-option");
        let label = gtk::Label::new(Some(&option.title));
        label.set_xalign(0.0);
        label.set_hexpand(true);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let icon = match option.privacy.as_deref() {
            Some("private") => Some(("yt-lock-symbolic", "Private")),
            Some("public") => Some(("yt-public-symbolic", "Public")),
            Some("unlisted") => Some(("yt-link-symbolic", "Unlisted")),
            _ => None,
        };
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.append(&label);
        if let Some((icon, tooltip)) = icon {
            let image = gtk::Image::from_icon_name(icon);
            image.set_tooltip_text(Some(tooltip));
            image.add_css_class("dim");
            row.append(&image);
        }
        check.set_child(Some(&row));
        let (video_id, playlist_id, title) = (
            save.video_id.clone(),
            option.playlist_id.clone(),
            option.title.clone(),
        );
        check.connect_toggled(move |check| {
            let command = if check.is_active() {
                "playlist.add"
            } else {
                "playlist.remove"
            };
            let args = json!({
                "playlist_id": playlist_id,
                "video_id": video_id,
                "title": title,
                "undo": false,
            });
            if let Err(e) = app().ext.commands.run(command, args) {
                app().toast(&e);
            }
        });
        save.list.append(&check);
    }
    save.stack.set_visible_child_name("list");
}
