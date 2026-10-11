//! The comments section (`comments`), single comments (`comment`) and the
//! box to write one (`comment_composer`).
//!
//! The section loads more comments as its scrolled window nears the end.
//! Actions go through commands (`comment.post`, `comment.vote`, ...) and the
//! UI follows their events, so a plugin that posts a comment or deletes one
//! sees the section update the same way.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::glib;
use serde_json::{Value, json};
use yt_core::comments::{Comment, Vote};
use yt_core::watch::CommentSort;

use super::{attach, attached, avatar, follow, richtext};
use crate::app::app;
use crate::{runtime, social};

/// Load more when this close (in pixels) to the end.
const PRELOAD_DISTANCE: f64 = 900.0;
/// Longer comments are folded, with "Read more".
const FOLDED_LINES: usize = 5;
const FOLDED_CHARS: usize = 500;

fn run(command: &str, args: Value) {
    if let Err(e) = app().ext.commands.run(command, args) {
        app().toast(&e);
    }
}

// The section.

struct Section {
    video_id: RefCell<String>,
    sorts: RefCell<Vec<CommentSort>>,
    selected: Cell<usize>,
    next: RefCell<Option<String>>,
    loading: Cell<bool>,
    generation: Cell<u64>,
    title: gtk::Label,
    sort_button: gtk::MenuButton,
    composer: gtk::Box,
    list: gtk::Box,
    footer: gtk::Stack,
    status: gtk::Label,
}

/// Props: `{ video_id, count, sorts: [{ title, token, selected }] }`.
pub fn build(props: &Value) -> gtk::Widget {
    let title = gtk::Label::new(None);
    title.add_css_class("comments-title");
    title.set_xalign(0.0);
    let sort_button = gtk::MenuButton::new();
    sort_button.set_child(Some(
        &adw::ButtonContent::builder()
            .icon_name("yt-sort-symbolic")
            .label("Sort by")
            .build(),
    ));
    sort_button.add_css_class("flat");
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 24);
    header.append(&title);
    header.append(&sort_button);
    header.append(
        &app()
            .ext
            .slots
            .create("comments.header", gtk::Orientation::Horizontal, 8),
    );

    let composer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let list = gtk::Box::new(gtk::Orientation::Vertical, 20);
    list.add_css_class("comment-list");

    let spinner = adw::Spinner::new();
    spinner.set_size_request(32, 32);
    let status = gtk::Label::new(None);
    status.add_css_class("dim");
    status.set_wrap(true);
    let footer = gtk::Stack::new();
    footer.add_named(&spinner, Some("loading"));
    footer.add_named(&status, Some("status"));
    footer.add_named(&gtk::Box::new(gtk::Orientation::Vertical, 0), Some("idle"));
    footer.set_margin_top(12);
    footer.set_margin_bottom(24);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 20);
    root.add_css_class("comments");
    root.set_widget_name("comments");
    root.append(&header);
    root.append(&composer);
    root.append(&list);
    root.append(&footer);

    let section = Rc::new(Section {
        video_id: RefCell::default(),
        sorts: RefCell::default(),
        selected: Cell::new(0),
        next: RefCell::default(),
        loading: Cell::new(false),
        generation: Cell::new(0),
        title,
        sort_button,
        composer,
        list,
        footer,
        status,
    });
    attach(&root, section.clone());
    connect_sorting(&root, &section);
    connect_events(&root, &section);
    connect_scrolling(&root, &section);
    configure(&section, props);
    root.upcast()
}

pub fn update(widget: &gtk::Widget, props: &Value) {
    if let Some(section) = attached::<Section>(widget) {
        configure(&section, props);
    }
}

fn configure(section: &Rc<Section>, props: &Value) {
    let video_id = props
        .get("video_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let sorts: Vec<CommentSort> = props
        .get("sorts")
        .and_then(|s| serde_json::from_value(s.clone()).ok())
        .unwrap_or_default();
    let count = props
        .get("count")
        .and_then(Value::as_str)
        .unwrap_or_default();
    section.title.set_label(&if count.is_empty() {
        "Comments".to_owned()
    } else {
        format!("{count} Comments")
    });
    section
        .selected
        .set(sorts.iter().position(|s| s.selected).unwrap_or(0));
    section.sort_button.set_visible(sorts.len() > 1);
    *section.sorts.borrow_mut() = sorts;
    *section.video_id.borrow_mut() = video_id.clone();

    while let Some(child) = section.composer.first_child() {
        section.composer.remove(&child);
    }
    // Signed out, no "Add a comment...", as on youtube.com.
    if social::signed_in() {
        section.composer.append(
            &app()
                .ext
                .components
                .build("comment_composer", &json!({ "video_id": video_id })),
        );
    }
    reload(section);
}

/// The sort menu runs `comments.sort`, whose `comments.sort_selected` event
/// the section follows.
fn connect_sorting(root: &gtk::Box, section: &Rc<Section>) {
    let weak = Rc::downgrade(section);
    super::menu::attach(&section.sort_button, move || {
        let Some(section) = weak.upgrade() else {
            return json!([]);
        };
        let video_id = section.video_id.borrow().clone();
        let items: Vec<Value> = section
            .sorts
            .borrow()
            .iter()
            .enumerate()
            .map(|(i, sort)| {
                json!({
                    "label": sort.title,
                    "command": "comments.sort",
                    "args": { "video_id": video_id, "index": i },
                    "selected": i == section.selected.get(),
                })
            })
            .collect();
        Value::Array(items)
    });
    let weak = Rc::downgrade(section);
    follow(root, "comments.sort_selected", move |payload| {
        let Some(section) = weak.upgrade() else {
            return;
        };
        let index = payload.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
        if payload.get("video_id").and_then(Value::as_str)
            == Some(section.video_id.borrow().as_str())
            && index != section.selected.get()
            && index < section.sorts.borrow().len()
        {
            section.selected.set(index);
            reload(&section);
        }
    });
}

fn connect_events(root: &gtk::Box, section: &Rc<Section>) {
    let weak = Rc::downgrade(section);
    follow(root, "comment.posted", move |payload| {
        let Some(section) = weak.upgrade() else {
            return;
        };
        let same_video = payload.get("video_id").and_then(Value::as_str)
            == Some(section.video_id.borrow().as_str());
        if same_video && payload.get("parent_id").is_none_or(Value::is_null) {
            let comment = payload.get("comment").cloned().unwrap_or_default();
            section
                .list
                .prepend(&comment_widget(&section.video_id.borrow(), comment, false));
            section.footer.set_visible_child_name("idle");
        }
    });
    let weak = Rc::downgrade(section);
    follow(root, "comment.deleted", move |payload| {
        let Some(section) = weak.upgrade() else {
            return;
        };
        let id = payload
            .get("comment_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if let Some(widget) =
            super::find_widget(section.list.upcast_ref(), &format!("comment:{id}"))
        {
            widget.unparent();
        }
    });
}

fn connect_scrolling(root: &gtk::Box, section: &Rc<Section>) {
    let weak = Rc::downgrade(section);
    root.connect_map(move |root| {
        let Some(scroller) = root
            .ancestor(gtk::ScrolledWindow::static_type())
            .and_downcast::<gtk::ScrolledWindow>()
        else {
            return;
        };
        if attached::<Hooked>(&scroller).is_some() {
            return;
        }
        attach(&scroller, Rc::new(Hooked));
        let weak = weak.clone();
        scroller.vadjustment().connect_value_changed(move |adj| {
            if adj.value() + adj.page_size() > adj.upper() - PRELOAD_DISTANCE
                && let Some(section) = weak.upgrade()
            {
                load_more(&section);
            }
        });
    });
}

/// Marks a scrolled window that already loads more comments.
struct Hooked;

fn reload(section: &Rc<Section>) {
    section.generation.set(section.generation.get() + 1);
    while let Some(child) = section.list.first_child() {
        section.list.remove(&child);
    }
    let token = section
        .sorts
        .borrow()
        .get(section.selected.get())
        .map(|s| s.token.clone());
    *section.next.borrow_mut() = token;
    section.loading.set(false);
    load_more(section);
}

fn load_more(section: &Rc<Section>) {
    if section.loading.get() {
        return;
    }
    let Some(token) = section.next.borrow_mut().take() else {
        return;
    };
    section.loading.set(true);
    section.footer.set_visible_child_name("loading");
    let (generation, video_id, weak) = (
        section.generation.get(),
        section.video_id.borrow().clone(),
        Rc::downgrade(section),
    );
    runtime::spawn_local(async move {
        let result = social::load_comments(&video_id, &token).await;
        let Some(section) = weak.upgrade().filter(|s| s.generation.get() == generation) else {
            return;
        };
        section.loading.set(false);
        match result {
            Ok(page) => {
                for comment in page.comments {
                    let props = serde_json::to_value(&comment).unwrap_or_default();
                    section
                        .list
                        .append(&comment_widget(&video_id, props, false));
                }
                *section.next.borrow_mut() = page.next;
                let empty = section.list.first_child().is_none();
                section.status.set_label("No comments yet. Be the first!");
                section
                    .footer
                    .set_visible_child_name(if empty { "status" } else { "idle" });
            }
            Err(e) => {
                section
                    .status
                    .set_label(&format!("Couldn't load comments: {e}"));
                section.footer.set_visible_child_name("status");
                *section.next.borrow_mut() = Some(token);
            }
        }
    });
}

/// Builds a comment through the `comment` component, after the `comment`
/// filter. The filter also sees `by_creator`: whether the video's channel
/// wrote it.
fn comment_widget(video_id: &str, mut comment: Value, reply: bool) -> gtk::Widget {
    let app = app();
    if comment.is_object() {
        let author = comment.pointer("/author/handle").and_then(Value::as_str);
        comment["by_creator"] = json!(social::is_by_creator(video_id, author));
    }
    let mut props = app.ext.filters.apply("comment", comment);
    if props.is_object() {
        props["video_id"] = json!(video_id);
        props["reply"] = json!(reply);
    }
    app.ext.components.build("comment", &props)
}

// A comment.

struct CommentView {
    video_id: String,
    comment: RefCell<Comment>,
    vote: Cell<Vote>,
    like: gtk::Button,
    likes: gtk::Label,
    dislike: gtk::Button,
    reply_area: gtk::Revealer,
    replies_toggle: gtk::Button,
    replies: gtk::Box,
    more_replies: gtk::Button,
    replies_next: RefCell<Option<String>>,
    replies_loaded: Cell<bool>,
    loading: Cell<bool>,
}

/// Props: a comment plus `video_id`, `reply` (whether it's a reply) and
/// `by_creator`.
pub fn build_comment(props: &Value) -> gtk::Widget {
    let comment: Comment = serde_json::from_value(props.clone()).unwrap_or_default();
    let video_id = props
        .get("video_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let reply = props.get("reply").and_then(Value::as_bool).unwrap_or(false);

    // The avatar and the name both open the author's channel.
    let face = gtk::Button::builder()
        .child(&avatar::new(
            if reply { 24 } else { 40 },
            &comment.author.name,
            comment.author.avatar.as_deref(),
        ))
        .css_classes(["flat", "circular", "comment-avatar"])
        .tooltip_text(&comment.author.name)
        .build();
    face.set_valign(gtk::Align::Start);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
    content.set_hexpand(true);

    if let Some(pinned) = &comment.pinned {
        let badge = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        badge.add_css_class("comment-pinned");
        badge.append(&gtk::Image::from_icon_name("yt-pin-symbolic"));
        badge.append(&gtk::Label::new(Some(pinned)));
        content.append(&badge);
    }
    let author = gtk::Button::with_label(&comment.author.name);
    author.add_css_class("comment-author");
    author.add_css_class("flat");
    if props
        .get("by_creator")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        author.add_css_class("by-creator");
        author.set_tooltip_text(Some("The video's creator"));
    }
    let channel = comment.author.clone();
    let open = move |_: &gtk::Button| {
        run(
            "channel.open",
            json!({ "channel_id": channel.id, "handle": channel.handle, "name": channel.name }),
        );
    };
    author.connect_clicked(open.clone());
    face.connect_clicked(open);
    let time = gtk::Label::new(Some(&format!(
        "{}{}",
        comment.published,
        if comment.edited { " (edited)" } else { "" }
    )));
    time.add_css_class("dim");
    time.add_css_class("comment-time");
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.append(&author);
    header.append(&time);
    content.append(&header);
    content.append(&comment_text(&comment));

    let like = gtk::Button::new();
    let likes = gtk::Label::new(None);
    likes.add_css_class("dim");
    let dislike = gtk::Button::new();
    for b in [&like, &dislike] {
        b.add_css_class("flat");
        b.add_css_class("circular");
        b.add_css_class("vote-button");
    }
    like.set_tooltip_text(Some("Like"));
    dislike.set_tooltip_text(Some("Dislike"));
    let reply_button = gtk::Button::with_label("Reply");
    reply_button.add_css_class("flat");
    reply_button.add_css_class("pill-button");
    let menu = gtk::MenuButton::new();
    menu.set_icon_name("yt-more-symbolic");
    menu.add_css_class("flat");
    menu.add_css_class("circular");
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    actions.add_css_class("comment-actions");
    actions.append(&like);
    actions.append(&likes);
    actions.append(&dislike);
    actions.append(&reply_button);
    actions.append(&spacer);
    actions.append(&menu);
    content.append(&actions);

    let reply_area = gtk::Revealer::new();
    reply_area.set_transition_type(gtk::RevealerTransitionType::SlideDown);
    content.append(&reply_area);

    let replies_toggle = gtk::Button::new();
    replies_toggle.add_css_class("replies-toggle");
    replies_toggle.add_css_class("flat");
    replies_toggle.add_css_class("pill-button");
    replies_toggle.set_halign(gtk::Align::Start);
    replies_toggle.set_visible(!reply && comment.reply_count > 0);
    let replies = gtk::Box::new(gtk::Orientation::Vertical, 16);
    replies.add_css_class("replies");
    replies.set_visible(false);
    let more_replies = gtk::Button::with_label("Show more replies");
    more_replies.add_css_class("replies-toggle");
    more_replies.add_css_class("flat");
    more_replies.add_css_class("pill-button");
    more_replies.set_halign(gtk::Align::Start);
    more_replies.set_visible(false);
    content.append(&replies_toggle);
    content.append(&replies);
    content.append(&more_replies);

    let root = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    root.add_css_class(if reply { "reply" } else { "comment" });
    root.set_widget_name(&format!("comment:{}", comment.id));
    root.append(&face);
    root.append(&content);

    let view = Rc::new(CommentView {
        video_id,
        vote: Cell::new(comment.vote),
        comment: RefCell::new(comment),
        like,
        likes,
        dislike,
        reply_area,
        replies_toggle,
        replies,
        more_replies,
        replies_next: RefCell::default(),
        replies_loaded: Cell::new(false),
        loading: Cell::new(false),
    });
    attach(&root, view.clone());
    show_vote(&view);
    show_replies_toggle(&view, false);
    connect_comment(&root, &view, &reply_button, &menu, reply);
    root.upcast()
}

/// The comment's text; a long one folded, with "Read more".
fn comment_text(comment: &Comment) -> gtk::Widget {
    let Some(folded) = comment.text.truncated(FOLDED_LINES, FOLDED_CHARS) else {
        return richtext::label(&comment.text).upcast();
    };
    let label = richtext::label(&folded);
    let toggle = gtk::Button::with_label("Read more");
    toggle.add_css_class("flat");
    toggle.add_css_class("read-more");
    toggle.set_halign(gtk::Align::Start);
    let (full, expanded) = (comment.text.clone(), Cell::new(false));
    let weak = label.downgrade();
    toggle.connect_clicked(move |toggle| {
        let Some(label) = weak.upgrade() else { return };
        let open = !expanded.get();
        expanded.set(open);
        richtext::set(&label, if open { &full } else { &folded });
        toggle.set_label(if open { "Show less" } else { "Read more" });
    });
    let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
    column.append(&label);
    column.append(&toggle);
    column.upcast()
}

fn show_vote(view: &CommentView) {
    let vote = view.vote.get();
    view.like.set_icon_name(if vote == Vote::Like {
        "yt-liked-symbolic"
    } else {
        "yt-like-symbolic"
    });
    view.dislike.set_icon_name(if vote == Vote::Dislike {
        "yt-disliked-symbolic"
    } else {
        "yt-dislike-symbolic"
    });
    let comment = view.comment.borrow();
    let base = comment.likes.clone().unwrap_or_default();
    // Show the user's own like in the count when it is an exact number.
    let shown = match (base.replace(',', "").parse::<i64>(), comment.vote, vote) {
        (Ok(n), Vote::Like, v) if v != Vote::Like => (n - 1).to_string(),
        (Ok(n), v, Vote::Like) if v != Vote::Like => (n + 1).to_string(),
        (Err(_), v, Vote::Like) if v != Vote::Like && base.is_empty() => "1".into(),
        _ => base,
    };
    view.likes.set_label(if shown == "0" { "" } else { &shown });
}

fn show_replies_toggle(view: &CommentView, open: bool) {
    let count = view.comment.borrow().reply_count;
    let label = if count == 1 {
        "1 reply".to_owned()
    } else {
        format!("{count} replies")
    };
    let content = adw::ButtonContent::builder()
        .icon_name(if open {
            "yt-collapse-symbolic"
        } else {
            "yt-expand-symbolic"
        })
        .label(label)
        .build();
    view.replies_toggle.set_child(Some(&content));
}

fn connect_comment(
    root: &gtk::Box,
    view: &Rc<CommentView>,
    reply_button: &gtk::Button,
    menu: &gtk::MenuButton,
    is_reply: bool,
) {
    let vote_on = |target: Vote| {
        let weak = Rc::downgrade(view);
        move |_: &gtk::Button| {
            let Some(view) = weak.upgrade() else { return };
            let from = view.vote.get();
            let to = if from == target { Vote::None } else { target };
            view.vote.set(to);
            show_vote(&view);
            let comment = view.comment.borrow();
            run(
                "comment.vote",
                json!({ "video_id": view.video_id, "comment_id": comment.id, "thread": comment.thread, "from": from, "to": to }),
            );
        }
    };
    view.like.connect_clicked(vote_on(Vote::Like));
    view.dislike.connect_clicked(vote_on(Vote::Dislike));

    let weak = Rc::downgrade(view);
    follow(root, "comment.vote.failed", move |payload| {
        let Some(view) = weak.upgrade() else { return };
        if payload.get("comment_id").and_then(Value::as_str)
            == Some(view.comment.borrow().id.as_str())
        {
            let from = payload
                .get("from")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            view.vote.set(from);
            show_vote(&view);
        }
    });

    // Replying: a composer under the comment. Replies to a reply go to the
    // top-level thread with the author mentioned, as on youtube.com.
    let weak = Rc::downgrade(view);
    reply_button.connect_clicked(move |_| {
        let Some(view) = weak.upgrade() else { return };
        if !social::signed_in() {
            return social::offer_sign_in("Sign in to comment");
        }
        let comment = view.comment.borrow();
        let parent_id = if is_reply { comment.id.split('.').next().unwrap_or_default().to_owned() } else { comment.id.clone() };
        let mention = if is_reply { comment.author.handle.clone().map(|h| format!("{h} ")) } else { None };
        let composer = app().ext.components.build(
            "comment_composer",
            &json!({ "video_id": view.video_id, "parent_id": parent_id, "mention": mention, "autofocus": true }),
        );
        view.reply_area.set_child(Some(&composer));
        view.reply_area.set_reveal_child(true);
    });

    if !is_reply {
        let weak = Rc::downgrade(view);
        view.replies_toggle.connect_clicked(move |_| {
            let Some(view) = weak.upgrade() else { return };
            let open = !view.replies.is_visible();
            view.replies.set_visible(open);
            view.more_replies
                .set_visible(open && view.replies_next.borrow().is_some());
            show_replies_toggle(&view, open);
            if open && !view.replies_loaded.get() {
                let token = view.comment.borrow().thread.clone();
                load_replies(&view, token);
            }
        });
        let weak = Rc::downgrade(view);
        view.more_replies.connect_clicked(move |_| {
            if let Some(view) = weak.upgrade() {
                let token = view.replies_next.borrow_mut().take();
                load_replies(&view, token);
            }
        });
        let weak = Rc::downgrade(view);
        follow(root, "comment.posted", move |payload| {
            let Some(view) = weak.upgrade() else { return };
            let comment_id = view.comment.borrow().id.clone();
            if payload.get("parent_id").and_then(Value::as_str) == Some(comment_id.as_str()) {
                let reply = payload.get("comment").cloned().unwrap_or_default();
                view.replies
                    .append(&comment_widget(&view.video_id, reply, true));
                view.replies.set_visible(true);
                view.reply_area.set_reveal_child(false);
                view.comment.borrow_mut().reply_count += 1;
                view.replies_toggle.set_visible(true);
                show_replies_toggle(&view, true);
            }
        });
    }
    let weak = Rc::downgrade(view);
    follow(root, "comment.posted", move |payload| {
        // Close this comment's own reply box once its reply is posted.
        if let Some(view) = weak.upgrade()
            && payload.get("parent_id").is_some_and(|p| !p.is_null())
            && view.reply_area.reveals_child()
        {
            view.reply_area.set_reveal_child(false);
        }
    });

    connect_menu(view, menu);
}

fn load_replies(view: &Rc<CommentView>, token: Option<String>) {
    let Some(token) = token else { return };
    if view.loading.replace(true) {
        return;
    }
    view.more_replies.set_sensitive(false);
    let weak: Weak<CommentView> = Rc::downgrade(view);
    let video_id = view.video_id.clone();
    runtime::spawn_local(async move {
        let result = social::load_thread(&video_id, &token).await;
        let Some(view) = weak.upgrade() else { return };
        view.loading.set(false);
        view.more_replies.set_sensitive(true);
        match result {
            Ok(thread) => {
                for reply in thread.replies {
                    let props = serde_json::to_value(&reply).unwrap_or_default();
                    view.replies.append(&comment_widget(&video_id, props, true));
                }
                view.replies_loaded.set(true);
                view.more_replies.set_visible(thread.next.is_some());
                *view.replies_next.borrow_mut() = thread.next;
            }
            Err(e) => app().toast(&format!("Couldn't load replies: {e}")),
        }
    });
}

/// The comment's menu, through the `comment.menu` filter. Each command
/// runs with `{ video_id, comment_id, comment }` added to its arguments.
fn connect_menu(view: &Rc<CommentView>, menu: &gtk::MenuButton) {
    let weak = Rc::downgrade(view);
    super::menu::attach(menu, move || {
        let Some(view) = weak.upgrade() else {
            return json!([]);
        };
        let comment = view.comment.borrow().clone();
        let mut defaults = vec![
            json!({ "label": "Copy text", "icon": "yt-copy-symbolic", "command": "comment.copy" }),
        ];
        if social::is_mine(comment.author.handle.as_deref()) {
            defaults.push(json!({
                "label": "Delete", "icon": "yt-delete-symbolic", "command": "comment.delete",
                "confirm": {
                    "title": "Delete comment",
                    "body": "Delete your comment permanently?",
                    "confirm": "Delete",
                },
            }));
        }
        let comment_json = serde_json::to_value(&comment).unwrap_or_default();
        let result = app().ext.filters.apply(
            "comment.menu",
            json!({ "comment": comment_json, "items": defaults }),
        );
        let mut items: Vec<Value> = result
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for item in &mut items {
            if !item.get("args").is_some_and(Value::is_object) {
                item["args"] = json!({});
            }
            item["args"]["video_id"] = json!(view.video_id);
            item["args"]["comment_id"] = json!(comment.id);
            item["args"]["comment"] = comment_json.clone();
        }
        Value::Array(items)
    });
}

// The composer.

struct Composer {
    video_id: String,
    parent_id: Option<String>,
    input: gtk::TextView,
    buttons: gtk::Revealer,
    submit: gtk::Button,
    face: adw::Avatar,
    busy: Cell<bool>,
}

/// Props: `{ video_id, parent_id?, mention?, autofocus? }`.
pub fn build_composer(props: &Value) -> gtk::Widget {
    let parent_id = props
        .get("parent_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let reply = parent_id.is_some();
    let account = social::account();
    let face = avatar::new(
        if reply { 24 } else { 40 },
        account.as_ref().map_or("", |a| a.name.as_str()),
        account.as_ref().and_then(|a| a.photo.as_deref()),
    );
    face.set_valign(gtk::Align::Start);

    let input = gtk::TextView::new();
    input.set_wrap_mode(gtk::WrapMode::WordChar);
    input.add_css_class("composer-input");
    input.set_accepts_tab(false);
    if let Some(mention) = props.get("mention").and_then(Value::as_str) {
        input.buffer().set_text(mention);
    }
    let placeholder = gtk::Label::new(Some(if reply {
        "Add a reply…"
    } else {
        "Add a comment…"
    }));
    placeholder.add_css_class("dim");
    placeholder.add_css_class("composer-placeholder");
    placeholder.set_halign(gtk::Align::Start);
    placeholder.set_valign(gtk::Align::Start);
    placeholder.set_can_target(false);
    // The field draws the underline, around both the text and the
    // placeholder, and is as tall as whichever is taller.
    let field = gtk::Overlay::new();
    field.add_css_class("composer-field");
    field.set_child(Some(&input));
    field.add_overlay(&placeholder);
    field.set_measure_overlay(&placeholder, true);

    let cancel = gtk::Button::with_label("Cancel");
    cancel.add_css_class("flat");
    cancel.add_css_class("pill-button");
    let submit = gtk::Button::with_label(if reply { "Reply" } else { "Comment" });
    submit.add_css_class("suggested-action");
    submit.add_css_class("pill-button");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_halign(gtk::Align::End);
    row.append(&cancel);
    row.append(&submit);
    let buttons = gtk::Revealer::new();
    buttons.set_child(Some(&row));
    buttons.set_transition_type(gtk::RevealerTransitionType::SlideDown);

    let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
    column.set_hexpand(true);
    column.append(&field);
    column.append(&buttons);
    let root = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    root.add_css_class("comment-composer");
    root.append(&face);
    root.append(&column);

    let composer = Rc::new(Composer {
        video_id: props
            .get("video_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        parent_id,
        input: input.clone(),
        buttons,
        submit: submit.clone(),
        face,
        busy: Cell::new(false),
    });
    attach(&root, composer.clone());

    let buffer = input.buffer();
    let (weak, ph) = (Rc::downgrade(&composer), placeholder.downgrade());
    let refresh = move |buffer: &gtk::TextBuffer| {
        let (Some(c), Some(ph)) = (weak.upgrade(), ph.upgrade()) else {
            return;
        };
        let empty = buffer.char_count() == 0;
        ph.set_visible(empty);
        c.submit
            .set_sensitive(!empty && !c.busy.get() && !buffer_text(buffer).trim().is_empty());
    };
    refresh(&buffer);
    buffer.connect_changed(refresh);

    let focus = gtk::EventControllerFocus::new();
    let weak = Rc::downgrade(&composer);
    focus.connect_enter(move |_| {
        if let Some(c) = weak.upgrade() {
            c.buttons.set_reveal_child(true);
        }
    });
    input.add_controller(focus);

    let keys = gtk::EventControllerKey::new();
    let weak = Rc::downgrade(&composer);
    keys.connect_key_pressed(move |_, key, _, mods| {
        let Some(c) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        match key {
            gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter
                if mods.contains(gtk::gdk::ModifierType::CONTROL_MASK) =>
            {
                send(&c);
                glib::Propagation::Stop
            }
            gtk::gdk::Key::Escape => {
                close(&c);
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    input.add_controller(keys);

    let weak = Rc::downgrade(&composer);
    cancel.connect_clicked(move |_| {
        if let Some(c) = weak.upgrade() {
            close(&c);
        }
    });
    let weak = Rc::downgrade(&composer);
    submit.connect_clicked(move |_| {
        if let Some(c) = weak.upgrade() {
            send(&c);
        }
    });

    let weak = Rc::downgrade(&composer);
    follow(&root, "comment.posted", move |payload| {
        if let Some(c) = weak.upgrade().filter(|c| is_for(c, payload)) {
            c.busy.set(false);
            c.input.set_sensitive(true);
            c.input.buffer().set_text("");
            c.buttons.set_reveal_child(false);
        }
    });
    let weak = Rc::downgrade(&composer);
    follow(&root, "comment.posted.failed", move |payload| {
        if let Some(c) = weak.upgrade().filter(|c| is_for(c, payload)) {
            c.busy.set(false);
            c.input.set_sensitive(true);
            let buffer = c.input.buffer();
            c.submit
                .set_sensitive(!buffer_text(&buffer).trim().is_empty());
        }
    });
    let weak = Rc::downgrade(&composer);
    follow(&root, "account.loaded", move |payload| {
        if let Some(c) = weak.upgrade() {
            let account = payload.get("account");
            let name = account
                .and_then(|a| a.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            avatar::set(
                &c.face,
                name,
                account.and_then(|a| a.get("photo")).and_then(Value::as_str),
            );
        }
    });

    if props
        .get("autofocus")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        input.connect_map(|input| {
            input.grab_focus();
            let buffer = input.buffer();
            buffer.place_cursor(&buffer.end_iter());
        });
    }
    root.upcast()
}

fn is_for(c: &Composer, payload: &Value) -> bool {
    payload.get("video_id").and_then(Value::as_str) == Some(c.video_id.as_str())
        && payload.get("parent_id").and_then(Value::as_str) == c.parent_id.as_deref()
}

fn buffer_text(buffer: &gtk::TextBuffer) -> String {
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), false)
        .to_string()
}

fn send(c: &Composer) {
    let text = buffer_text(&c.input.buffer());
    if text.trim().is_empty() || c.busy.get() {
        return;
    }
    c.busy.set(true);
    c.submit.set_sensitive(false);
    c.input.set_sensitive(false);
    let result = match &c.parent_id {
        Some(parent) => app().ext.commands.run(
            "comment.reply",
            json!({ "video_id": c.video_id, "parent_id": parent, "text": text }),
        ),
        None => app().ext.commands.run(
            "comment.post",
            json!({ "video_id": c.video_id, "text": text }),
        ),
    };
    if let Err(e) = result {
        c.busy.set(false);
        c.input.set_sensitive(true);
        c.submit.set_sensitive(true);
        app().toast(&e);
    }
}

/// Cancel: clears the text; a reply box also folds away.
fn close(c: &Composer) {
    c.input.buffer().set_text("");
    c.buttons.set_reveal_child(false);
    if let Some(revealer) = c
        .input
        .ancestor(gtk::Revealer::static_type())
        .and_downcast::<gtk::Revealer>()
        && c.parent_id.is_some()
    {
        revealer.set_reveal_child(false);
    }
}
