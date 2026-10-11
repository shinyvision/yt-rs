//! The `list_panel` component: on the watch page, above "Up next", the
//! list playing (a playlist or the queue), as on youtube.com: its title,
//! whose it is and where playback is ("3 / 59"), buttons to loop and
//! shuffle it and to fold the panel, and its videos with the one playing
//! marked. Clicking a video plays it; the queue's have a button to take
//! them out. Hidden while no list plays.

use std::cell::Cell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use serde_json::{Value, json};
use yt_core::feed::Video;

use super::{attach, follow};
use crate::app::app;
use crate::lists::List;

/// The list's height before it scrolls.
const MAX_HEIGHT: i32 = 400;

struct Panel {
    root: gtk::Box,
    title: gtk::Label,
    subtitle: gtk::Label,
    looping: gtk::ToggleButton,
    shuffled: gtk::ToggleButton,
    fold: gtk::Button,
    body: gtk::Revealer,
    model: gtk::gio::ListStore,
    list: gtk::ListView,
    /// Set while the buttons show the list, so it isn't sent back.
    showing: Cell<bool>,
}

/// A row: a video, where it is in the order, whether it plays, and
/// whether it can be taken out (the queue's).
struct Row {
    video: Video,
    position: usize,
    current: bool,
    removable: bool,
}

pub fn build(_props: &Value) -> gtk::Widget {
    let title = gtk::Label::new(None);
    title.add_css_class("list-title");
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let subtitle = gtk::Label::new(None);
    subtitle.add_css_class("dim");
    subtitle.add_css_class("list-subtitle");
    subtitle.set_xalign(0.0);
    let toggle = |icon: &str, tooltip: &str| {
        let b = gtk::ToggleButton::new();
        b.set_icon_name(icon);
        b.set_tooltip_text(Some(tooltip));
        b.add_css_class("icon-button");
        b
    };
    let looping = toggle("yt-loop-symbolic", "Loop playlist");
    let shuffled = toggle("yt-shuffle-symbolic", "Shuffle playlist");
    let fold = gtk::Button::from_icon_name("yt-collapse-symbolic");
    fold.add_css_class("icon-button");
    fold.set_tooltip_text(Some("Hide"));
    fold.set_valign(gtk::Align::Start);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    text.append(&title);
    text.append(&subtitle);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    buttons.append(&looping);
    buttons.append(&shuffled);
    text.append(&buttons);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.add_css_class("list-header");
    header.append(&text);
    header.append(&fold);

    let model = gtk::gio::ListStore::new::<glib::BoxedAnyObject>();
    let list = gtk::ListView::new(
        Some(gtk::NoSelection::new(Some(model.clone()))),
        Some(factory()),
    );
    list.add_css_class("list-videos");
    list.set_can_focus(false);
    let scroller = gtk::ScrolledWindow::builder()
        .child(&list)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(MAX_HEIGHT)
        .build();
    let body = gtk::Revealer::builder()
        .child(&scroller)
        .reveal_child(true)
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .build();

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("list-panel");
    root.set_widget_name("list-panel");
    root.append(&header);
    root.append(&body);

    let panel = Rc::new(Panel {
        root: root.clone(),
        title,
        subtitle,
        looping,
        shuffled,
        fold,
        body,
        model,
        list,
        showing: Cell::new(false),
    });
    attach(&root, panel.clone());
    connect(&panel);
    show(&panel, crate::lists::current().as_ref());
    root.upcast()
}

fn connect(panel: &Rc<Panel>) {
    let weak = Rc::downgrade(panel);
    follow(&panel.root, "list.changed", move |_| {
        if let Some(panel) = weak.upgrade() {
            show(&panel, crate::lists::current().as_ref());
        }
    });
    let weak = Rc::downgrade(panel);
    panel.looping.connect_toggled(move |b| {
        if let Some(panel) = weak.upgrade()
            && !panel.showing.get()
        {
            run("list.set_looping", json!({ "looping": b.is_active() }));
        }
    });
    let weak = Rc::downgrade(panel);
    panel.shuffled.connect_toggled(move |b| {
        if let Some(panel) = weak.upgrade()
            && !panel.showing.get()
        {
            run("list.set_shuffled", json!({ "shuffled": b.is_active() }));
        }
    });
    let weak = Rc::downgrade(panel);
    panel.fold.connect_clicked(move |b| {
        let Some(panel) = weak.upgrade() else { return };
        let open = !panel.body.reveals_child();
        panel.body.set_reveal_child(open);
        b.set_icon_name(if open {
            "yt-collapse-symbolic"
        } else {
            "yt-expand-symbolic"
        });
        b.set_tooltip_text(Some(if open { "Hide" } else { "Show" }));
    });
}

fn show(panel: &Panel, list: Option<&List>) {
    let Some(list) = list else {
        panel.root.set_visible(false);
        return;
    };
    panel.root.set_visible(true);
    panel.showing.set(true);
    panel.title.set_label(&list.title);
    let place = format!("{} / {}", list.position + 1, list.order.len());
    panel.subtitle.set_label(&match &list.owner {
        Some(owner) => format!("{owner} \u{2022} {place}"),
        None => place,
    });
    panel.looping.set_active(list.looping);
    panel.shuffled.set_active(list.shuffled);
    // The queue plays in the order it was made.
    panel.shuffled.set_visible(list.playlist_id.is_some());
    panel.showing.set(false);

    let removable = list.playlist_id.is_none();
    let rows: Vec<glib::BoxedAnyObject> = list
        .order
        .iter()
        .enumerate()
        .filter_map(|(position, &index)| {
            Some(glib::BoxedAnyObject::new(Row {
                video: list.videos.get(index)?.clone(),
                position,
                current: position == list.position,
                removable,
            }))
        })
        .collect();
    panel.model.splice(0, panel.model.n_items(), &rows);
    let position = list.position as u32;
    let view = panel.list.downgrade();
    glib::idle_add_local_once(move || {
        if let Some(view) = view.upgrade() {
            view.scroll_to(position, gtk::ListScrollFlags::NONE, None);
        }
    });
}

/// Rows: ▶ or the number, a small picture, the title and channel, and for
/// the queue a button to take the video out.
fn factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
        let mark = gtk::Label::new(None);
        mark.add_css_class("list-mark");
        mark.add_css_class("dim");
        let picture = gtk::Picture::new();
        picture.set_content_fit(gtk::ContentFit::Cover);
        picture.set_can_shrink(true);
        let thumb = super::aspect::AspectBin::new(9.0 / 16.0, &picture);
        thumb.add_css_class("thumb");
        thumb.set_overflow(gtk::Overflow::Hidden);
        thumb.set_natural_width(100);
        thumb.set_size_request(100, -1);
        // Its own height for its width, not the row's.
        thumb.set_valign(gtk::Align::Center);
        thumb.set_hexpand(false);
        let title = gtk::Label::new(None);
        title.add_css_class("list-video-title");
        title.set_xalign(0.0);
        title.set_wrap(true);
        title.set_lines(2);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_hexpand(true);
        let channel = gtk::Label::new(None);
        channel.add_css_class("dim");
        channel.add_css_class("meta");
        channel.set_xalign(0.0);
        channel.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.set_valign(gtk::Align::Center);
        text.set_hexpand(true);
        text.append(&title);
        text.append(&channel);
        let remove = gtk::Button::from_icon_name("yt-close-symbolic");
        remove.add_css_class("icon-button");
        remove.set_tooltip_text(Some("Remove from queue"));
        remove.set_valign(gtk::Align::Center);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("list-row");
        row.set_cursor_from_name(Some("pointer"));
        for w in [
            mark.upcast_ref::<gtk::Widget>(),
            thumb.upcast_ref(),
            text.upcast_ref(),
            remove.upcast_ref(),
        ] {
            row.append(w);
        }
        // Rows are in the list's order, so a row's place is the video's.
        let click = gtk::GestureClick::new();
        let at = item.downgrade();
        click.connect_released(move |gesture, n, _, _| {
            if n == 1
                && let Some(item) = at.upgrade()
            {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                run("list.play_at", json!({ "position": item.position() }));
            }
        });
        row.add_controller(click);
        let at = item.downgrade();
        remove.connect_clicked(move |_| {
            if let Some(item) = at.upgrade() {
                run("list.remove", json!({ "position": item.position() }));
            }
        });
        item.set_child(Some(&row));
        item.set_activatable(false);
        item.set_focusable(false);
    });
    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
        let (Some(row), Some(object)) = (
            item.child(),
            item.item().and_downcast::<glib::BoxedAnyObject>(),
        ) else {
            return;
        };
        let entry = object.borrow::<Row>();
        let parts: Vec<gtk::Widget> =
            std::iter::successors(row.first_child(), |w| w.next_sibling()).collect();
        let [mark, thumb, text, remove] = parts.as_slice() else {
            return;
        };
        if let Some(mark) = mark.downcast_ref::<gtk::Label>() {
            mark.set_label(&if entry.current {
                "\u{25B6}".to_owned()
            } else {
                (entry.position + 1).to_string()
            });
        }
        if entry.current {
            row.add_css_class("current");
        } else {
            row.remove_css_class("current");
        }
        if let Some(picture) = thumb.first_child().and_downcast::<gtk::Picture>() {
            app().images.load_into(
                &picture,
                entry.video.thumbnail.as_deref(),
                200 * row.scale_factor(),
            );
        }
        let labels: Vec<gtk::Widget> =
            std::iter::successors(text.first_child(), |w| w.next_sibling()).collect();
        if let [title, channel] = labels.as_slice() {
            if let Some(title) = title.downcast_ref::<gtk::Label>() {
                title.set_label(&entry.video.title);
            }
            if let Some(channel) = channel.downcast_ref::<gtk::Label>() {
                channel.set_label(entry.video.channel.as_deref().unwrap_or_default());
            }
        }
        remove.set_visible(entry.removable && !entry.current);
    });
    factory
}

fn run(command: &str, args: Value) {
    if let Err(e) = app().ext.commands.run(command, args) {
        app().toast(&e);
    }
}
