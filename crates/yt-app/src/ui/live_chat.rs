//! The `live_chat` component: a live stream's chat beside the video, as on
//! youtube.com. Top chat or Live chat; messages as they come, with their
//! authors' badges (the channel itself, moderators, members); Super Chats,
//! Super Stickers, memberships and gifts in their colors; the pinned
//! message; channels' own emoji as pictures.
//!
//! It follows the video on screen by itself (`video.opened`,
//! `watch.loaded`, `player.loaded`, `player.upcoming`) and shows only for a
//! live stream or one about to start. It asks for messages only while it
//! is on screen, and starts afresh when it is back. Each page's messages
//! come in one by one over the wait for the next, as youtube.com shows
//! them, and only the newest are kept.
//!
//! Plugins: the `chat.messages` filter (`{ video_id, messages }`, each page's
//! new messages before they show: drop or change them), the `chat.message`
//! event as each shows, `live_chat.header` (a slot for buttons), and the
//! `chat.*` commands (see the plugin docs).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::{Rc, Weak};
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;
use serde_json::{Value, json};
use yt_core::live_chat::{self, ChatAction, ChatAuthor, ChatKind, ChatMessage, ChatView};
use yt_core::text::RichText;

use super::{attach, avatar, follow};
use crate::app::app;
use crate::runtime;

/// Messages kept; older ones go (unless the user scrolled up to read).
const KEPT: usize = 150;
/// How tall the chat is where nothing sets it (below the details).
const HEIGHT: i32 = 560;
const MIN_HEIGHT: i32 = 400;
/// Waits after failed pages, growing to the last.
const RETRY: [u64; 4] = [2_000, 5_000, 10_000, 30_000];

#[derive(Default)]
struct State {
    video_id: String,
    channel_id: Option<String>,
    live: bool,
    view: ChatView,
    /// Bumped to stop the pages being asked for (another video, another
    /// view, off screen).
    generation: u64,
    running: bool,
    /// Messages still to come in, one by one, and the timer bringing them.
    queue: VecDeque<ChatAction>,
    tick: Option<glib::SourceId>,
    /// The messages shown, oldest first: (ID, author's channel, row).
    rows: VecDeque<(String, Option<String>, gtk::Widget)>,
    pinned: Option<String>,
    /// Follows the newest messages (not scrolled up).
    stick: bool,
}

pub struct Chat {
    state: RefCell<State>,
    /// The component itself, weakly: it owns this.
    root: gtk::glib::WeakRef<gtk::Box>,
    frame: gtk::Box,
    show_button: gtk::Button,
    view_label: gtk::Label,
    pinned: gtk::Revealer,
    content: gtk::Stack,
    status: gtk::Label,
    list: gtk::Box,
    scroller: gtk::ScrolledWindow,
    jump: gtk::Button,
    viewer: gtk::Label,
}

pub fn build(_props: &Value) -> gtk::Widget {
    let view_label = gtk::Label::new(Some(ChatView::Top.label()));
    let view_content = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    view_content.append(&view_label);
    view_content.append(&gtk::Image::from_icon_name("yt-expand-symbolic"));
    let view_button = gtk::MenuButton::builder()
        .child(&view_content)
        .css_classes(["flat", "live-chat-view"])
        .tooltip_text("Which messages")
        .build();
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    header.add_css_class("live-chat-header");
    header.append(&view_button);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    header.append(&spacer);
    header.append(
        &app()
            .ext
            .slots
            .create("live_chat.header", gtk::Orientation::Horizontal, 4),
    );
    let hide = gtk::Button::from_icon_name("yt-close-symbolic");
    hide.add_css_class("icon-button");
    hide.set_tooltip_text(Some("Hide chat"));
    header.append(&hide);

    let pinned = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .build();
    pinned.add_css_class("live-chat-pinned");

    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.add_css_class("live-chat-messages");
    list.set_valign(gtk::Align::End);
    let scroller = gtk::ScrolledWindow::builder()
        .child(&list)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    let jump = gtk::Button::from_icon_name("yt-arrow-down-symbolic");
    jump.add_css_class("live-chat-jump");
    jump.set_tooltip_text(Some("Newest messages"));
    jump.set_halign(gtk::Align::Center);
    jump.set_valign(gtk::Align::End);
    jump.set_visible(false);
    let body = gtk::Overlay::new();
    body.set_child(Some(&scroller));
    body.add_overlay(&jump);

    let spinner = adw::Spinner::new();
    spinner.set_size_request(32, 32);
    spinner.set_halign(gtk::Align::Center);
    spinner.set_valign(gtk::Align::Center);
    let status = gtk::Label::new(None);
    status.add_css_class("dim");
    status.set_wrap(true);
    status.set_justify(gtk::Justification::Center);
    status.set_margin_start(24);
    status.set_margin_end(24);
    let content = gtk::Stack::new();
    content.add_named(&spinner, Some("loading"));
    content.add_named(&status, Some("status"));
    content.add_named(&body, Some("messages"));
    content.set_vexpand(true);

    // Chatting needs youtube.com: the TV client can't post to a chat.
    let viewer = gtk::Label::new(None);
    viewer.add_css_class("live-chat-viewer");
    viewer.set_xalign(0.0);
    viewer.set_hexpand(true);
    viewer.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let chat_there = gtk::Button::builder()
        .css_classes(["pill-button", "live-chat-open"])
        .tooltip_text("Chat in the browser, on youtube.com")
        .build();
    let chat_there_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    chat_there_content.append(&gtk::Image::from_icon_name("yt-open-symbolic"));
    chat_there_content.append(&gtk::Label::new(Some("Chat on YouTube")));
    chat_there.set_child(Some(&chat_there_content));
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    footer.add_css_class("live-chat-footer");
    footer.append(&viewer);
    footer.append(&chat_there);

    let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
    frame.add_css_class("live-chat");
    frame.set_widget_name("live-chat");
    frame.set_size_request(-1, HEIGHT);
    frame.append(&header);
    frame.append(&pinned);
    frame.append(&content);
    frame.append(&footer);

    let show_button = gtk::Button::builder()
        .css_classes(["pill-button", "live-chat-show"])
        .build();
    let show_content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    show_content.set_halign(gtk::Align::Center);
    show_content.append(&gtk::Image::from_icon_name("yt-chat-symbolic"));
    show_content.append(&gtk::Label::new(Some("Show chat")));
    show_button.set_child(Some(&show_content));

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.set_widget_name("live-chat-panel");
    root.append(&frame);
    root.append(&show_button);
    root.set_visible(false);

    let chat = Rc::new(Chat {
        state: RefCell::new(State {
            stick: true,
            ..State::default()
        }),
        root: root.downgrade(),
        frame,
        show_button: show_button.clone(),
        view_label,
        pinned,
        content,
        status,
        list,
        scroller: scroller.clone(),
        jump: jump.clone(),
        viewer,
    });
    attach(&root, chat.clone());
    show_hidden(&chat);

    view_button.set_popover(Some(&view_menu(&chat)));
    let weak = Rc::downgrade(&chat);
    hide.connect_clicked(move |_| {
        if let Some(chat) = weak.upgrade() {
            set_hidden(&chat, true);
        }
    });
    let weak = Rc::downgrade(&chat);
    show_button.connect_clicked(move |_| {
        if let Some(chat) = weak.upgrade() {
            set_hidden(&chat, false);
        }
    });
    let weak = Rc::downgrade(&chat);
    chat_there.connect_clicked(move |_| {
        if let Some(chat) = weak.upgrade() {
            let url = format!(
                "https://www.youtube.com/live_chat?is_popout=1&v={}",
                chat.state.borrow().video_id
            );
            if let Err(e) = app()
                .ext
                .commands
                .run("app.open_url", json!({ "url": url }))
            {
                app().toast(&e);
            }
        }
    });

    // Following the newest messages, unless scrolled up to read.
    let adjustment = scroller.vadjustment();
    let weak = Rc::downgrade(&chat);
    adjustment.connect_changed(move |adj| {
        if let Some(chat) = weak.upgrade()
            && chat.state.borrow().stick
        {
            adj.set_value(adj.upper() - adj.page_size());
            // Again once the new rows are laid out: the scrolled window
            // clamps the value as they grow.
            let adj = adj.clone();
            glib::idle_add_local_once(move || adj.set_value(adj.upper() - adj.page_size()));
        }
    });
    let weak = Rc::downgrade(&chat);
    adjustment.connect_value_changed(move |adj| {
        if let Some(chat) = weak.upgrade() {
            let at_end = adj.value() >= adj.upper() - adj.page_size() - 4.0;
            chat.state.borrow_mut().stick = at_end;
            chat.jump.set_visible(!at_end);
        }
    });
    let weak = Rc::downgrade(&chat);
    jump.connect_clicked(move |_| {
        if let Some(chat) = weak.upgrade() {
            chat.state.borrow_mut().stick = true;
            let adj = chat.scroller.vadjustment();
            adj.set_value(adj.upper() - adj.page_size());
            chat.jump.set_visible(false);
            trim(&chat);
        }
    });

    // Off screen, it asks for nothing; back, it starts afresh.
    let weak = Rc::downgrade(&chat);
    root.connect_map(move |_| {
        if let Some(chat) = weak.upgrade() {
            start(&chat);
        }
    });
    let weak = Rc::downgrade(&chat);
    root.connect_unmap(move |_| {
        if let Some(chat) = weak.upgrade() {
            stop(&chat);
        }
    });

    follow_video(&chat, &root);
    // The video playing when the page was made.
    if let Some(video) = crate::playback::current() {
        let mut state = chat.state.borrow_mut();
        state.video_id = video.id;
        state.live = video.live;
    }
    refresh_visible(&chat);
    root.upcast()
}

/// Follows the video on screen: which it is, its channel, whether it is
/// live.
fn follow_video(chat: &Rc<Chat>, root: &gtk::Box) {
    let weak = Rc::downgrade(chat);
    follow(root, "video.opened", move |payload| {
        let Some(chat) = weak.upgrade() else { return };
        let video = payload.get("video").cloned().unwrap_or_default();
        let id = video.get("id").and_then(Value::as_str).unwrap_or_default();
        if id == chat.state.borrow().video_id {
            return;
        }
        stop(&chat);
        {
            let mut state = chat.state.borrow_mut();
            state.video_id = id.to_owned();
            // From the watch page's details (`watch.loaded`).
            state.channel_id = None;
            state.live = video.get("live").and_then(Value::as_bool) == Some(true);
            state.pinned = None;
        }
        clear(&chat);
        refresh_visible(&chat);
        start(&chat);
    });
    let weak = Rc::downgrade(chat);
    follow(root, "watch.loaded", move |payload| {
        let Some(chat) = weak.upgrade() else { return };
        if payload.get("video_id").and_then(Value::as_str)
            != Some(chat.state.borrow().video_id.as_str())
        {
            return;
        }
        let channel = payload
            .pointer("/page/owner/id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if channel.is_some() {
            chat.state.borrow_mut().channel_id = channel;
            start(&chat);
        }
    });
    // A live stream plays (or is about to start): it has a chat.
    for event in ["player.loaded", "player.upcoming"] {
        let weak = Rc::downgrade(chat);
        let upcoming = event == "player.upcoming";
        follow(root, event, move |payload| {
            let Some(chat) = weak.upgrade() else { return };
            let ours = payload.get("video_id").and_then(Value::as_str)
                == Some(chat.state.borrow().video_id.as_str());
            let live = payload.get("live").and_then(Value::as_bool) == Some(true);
            if ours && (upcoming || live) && !chat.state.borrow().live {
                chat.state.borrow_mut().live = true;
                refresh_visible(&chat);
                start(&chat);
            }
        });
    }
}

/// Widgets are never changed with the state borrowed: showing one maps it,
/// and mapping starts the chat.
fn refresh_visible(chat: &Chat) {
    let live = chat.state.borrow().live;
    if let Some(root) = chat.root.upgrade() {
        root.set_visible(live);
    }
}

fn hidden() -> bool {
    app().settings.get().live_chat_hidden
}

fn show_hidden(chat: &Chat) {
    let hidden = hidden();
    chat.frame.set_visible(!hidden);
    chat.show_button.set_visible(hidden);
}

/// Folds the chat away ("Hide chat") or brings it back, remembered.
pub fn set_hidden(chat: &Rc<Chat>, hide: bool) {
    app().settings.update(|s| s.live_chat_hidden = hide);
    show_hidden(chat);
    if hide {
        stop(chat);
    } else {
        start(chat);
    }
}

/// "Top chat" or "Live chat", each with what it shows.
fn view_menu(chat: &Rc<Chat>) -> gtk::Popover {
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.add_css_class("yt-menu");
    let popover = gtk::Popover::builder()
        .has_arrow(false)
        .child(&list)
        .css_classes(["yt-menu-popover"])
        .build();
    let mut checks = Vec::new();
    for view in ChatView::ALL {
        let title = gtk::Label::new(Some(view.label()));
        title.set_xalign(0.0);
        let description = gtk::Label::new(Some(view.description()));
        description.add_css_class("dim");
        description.add_css_class("live-chat-view-description");
        description.set_xalign(0.0);
        description.set_wrap(true);
        description.set_max_width_chars(28);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.set_hexpand(true);
        text.append(&title);
        text.append(&description);
        let check = gtk::Image::from_icon_name("yt-check-symbolic");
        checks.push((view, check.clone()));
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        content.append(&text);
        content.append(&check);
        let item = gtk::Button::builder()
            .child(&content)
            .css_classes(["flat", "yt-menu-item"])
            .build();
        let (weak, popover) = (Rc::downgrade(chat), popover.clone());
        item.connect_clicked(move |_| {
            popover.popdown();
            if let Some(chat) = weak.upgrade() {
                set_view(&chat, view);
            }
        });
        list.append(&item);
    }
    let weak = Rc::downgrade(chat);
    popover.connect_show(move |_| {
        if let Some(chat) = weak.upgrade() {
            let current = chat.state.borrow().view;
            for (view, check) in &checks {
                check.set_opacity(if *view == current { 1.0 } else { 0.0 });
            }
        }
    });
    popover
}

pub fn set_view(chat: &Rc<Chat>, view: ChatView) {
    if chat.state.borrow().view == view {
        return;
    }
    chat.state.borrow_mut().view = view;
    chat.view_label.set_label(view.label());
    stop(chat);
    start(chat);
}

/// Starts asking for messages, from now, if the chat can show.
fn start(chat: &Rc<Chat>) {
    let (token, generation) = {
        let mut state = chat.state.borrow_mut();
        if state.running
            || !state.live
            || hidden()
            || !chat.root.upgrade().is_some_and(|r| r.is_mapped())
        {
            return;
        }
        let Some(channel) = state.channel_id.clone() else {
            return;
        };
        state.generation += 1;
        state.running = true;
        state.stick = true;
        (
            live_chat::token(&channel, &state.video_id, state.view),
            state.generation,
        )
    };
    clear(chat);
    chat.content.set_visible_child_name("loading");
    runtime::spawn_local(poll(Rc::downgrade(chat), token, generation));
}

fn stop(chat: &Chat) {
    let mut state = chat.state.borrow_mut();
    state.generation += 1;
    state.running = false;
    state.queue.clear();
    if let Some(tick) = state.tick.take() {
        tick.remove();
    }
}

fn clear(chat: &Chat) {
    let rows: Vec<gtk::Widget> = {
        let mut state = chat.state.borrow_mut();
        state.pinned = None;
        state.rows.drain(..).map(|(_, _, row)| row).collect()
    };
    for row in rows {
        chat.list.remove(&row);
    }
    chat.pinned.set_reveal_child(false);
    chat.jump.set_visible(false);
}

/// Asks for pages of the chat one after another, until it ends or is
/// stopped.
async fn poll(weak: Weak<Chat>, mut token: String, generation: u64) {
    let mut first = true;
    let mut failures = 0;
    loop {
        let youtube = app().youtube.clone();
        let asked = token.clone();
        let result = runtime::background(async move { youtube.live_chat(&asked).await }).await;
        let wait = {
            let Some(chat) = weak.upgrade() else { return };
            if chat.state.borrow().generation != generation {
                return;
            }
            match result {
                Ok(mut page) => {
                    failures = 0;
                    if let Some(viewer) = &page.viewer {
                        chat.viewer.set_label(viewer);
                    }
                    filter(&chat, &mut page.actions);
                    let ended = page.continuation.is_none();
                    if first {
                        if ended && page.actions.is_empty() {
                            show_status(&chat, "Chat is disabled for this live stream.");
                            chat.state.borrow_mut().running = false;
                            return;
                        }
                        // What was said before shows at once.
                        for action in page.actions {
                            apply(&chat, action);
                        }
                        chat.content.set_visible_child_name("messages");
                    } else {
                        trickle(&chat, page.actions, page.wait_ms);
                    }
                    match page.continuation {
                        Some(next) => token = next,
                        None => {
                            add_notice(&chat, "Chat ended");
                            chat.state.borrow_mut().running = false;
                            return;
                        }
                    }
                    page.wait_ms
                }
                Err(e) => {
                    tracing::warn!("live chat: {e}");
                    if first {
                        show_status(&chat, &format!("Couldn't load the chat: {e}"));
                        chat.state.borrow_mut().running = false;
                        return;
                    }
                    failures += 1;
                    RETRY[(failures - 1).min(RETRY.len() - 1)]
                }
            }
        };
        first = false;
        glib::timeout_future(Duration::from_millis(wait)).await;
    }
}

fn show_status(chat: &Chat, text: &str) {
    chat.status.set_label(text);
    chat.content.set_visible_child_name("status");
}

/// Plugins see each page's new messages first, and may drop or change them.
fn filter(chat: &Chat, actions: &mut Vec<ChatAction>) {
    let filters = &app().ext.filters;
    if filters.is_empty("chat.messages") {
        return;
    }
    let mut messages: Vec<ChatMessage> = actions
        .iter()
        .filter_map(|a| match a {
            ChatAction::Add { message } => Some(message.clone()),
            _ => None,
        })
        .collect();
    let before: Vec<String> = messages.iter().map(|m| m.id.clone()).collect();
    let context = json!({ "video_id": chat.state.borrow().video_id });
    if let Err(e) = filters.apply_to("chat.messages", context, "messages", &mut messages) {
        tracing::warn!("chat.messages filter: {e}");
        return;
    }
    let mut kept: HashMap<String, ChatMessage> =
        messages.into_iter().map(|m| (m.id.clone(), m)).collect();
    let known: HashSet<String> = before.into_iter().collect();
    actions.retain_mut(|action| match action {
        ChatAction::Add { message } if known.contains(&message.id) => {
            match kept.remove(&message.id) {
                Some(changed) => {
                    *message = changed;
                    true
                }
                None => false,
            }
        }
        _ => true,
    });
}

/// Shows a page's messages one by one over the wait for the next page,
/// as youtube.com does; what is left of the last page shows at once.
fn trickle(chat: &Rc<Chat>, actions: Vec<ChatAction>, wait_ms: u64) {
    flush(chat);
    let adds = actions
        .iter()
        .filter(|a| matches!(a, ChatAction::Add { .. }))
        .count();
    if adds <= 1 {
        for action in actions {
            apply(chat, action);
        }
        return;
    }
    let interval = (wait_ms * 9 / 10 / adds as u64).clamp(40, 1_000);
    chat.state.borrow_mut().queue.extend(actions);
    let weak = Rc::downgrade(chat);
    let tick = glib::timeout_add_local(Duration::from_millis(interval), move || {
        let Some(chat) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        // The next message, and whatever else comes before the one after.
        loop {
            let next = chat.state.borrow_mut().queue.pop_front();
            let Some(action) = next else {
                chat.state.borrow_mut().tick = None;
                return glib::ControlFlow::Break;
            };
            let added = matches!(action, ChatAction::Add { .. });
            apply(&chat, action);
            if added {
                return glib::ControlFlow::Continue;
            }
        }
    });
    chat.state.borrow_mut().tick = Some(tick);
}

fn flush(chat: &Chat) {
    if let Some(tick) = chat.state.borrow_mut().tick.take() {
        tick.remove();
    }
    let queued: Vec<ChatAction> = chat.state.borrow_mut().queue.drain(..).collect();
    for action in queued {
        apply(chat, action);
    }
}

fn apply(chat: &Chat, action: ChatAction) {
    match action {
        ChatAction::Add { message } => {
            if app().ext.events.has_handlers("chat.message") {
                // Built first: handlers may change the chat.
                let payload =
                    json!({ "video_id": chat.state.borrow().video_id, "message": message });
                app().ext.events.emit("chat.message", &payload);
            }
            let row = row(&message);
            chat.list.append(&row);
            chat.state
                .borrow_mut()
                .rows
                .push_back((message.id, message.author.channel_id, row));
            trim(chat);
        }
        ChatAction::Replace { id, message } => {
            let new = row(&message);
            let old = {
                let mut state = chat.state.borrow_mut();
                state
                    .rows
                    .iter_mut()
                    .find(|(i, _, _)| *i == id)
                    .map(|slot| std::mem::replace(&mut slot.2, new.clone()))
            };
            if let Some(old) = old {
                chat.list.insert_child_after(&new, Some(&old));
                chat.list.remove(&old);
            }
        }
        ChatAction::Remove { id } => remove_where(chat, |(i, _, _)| *i == id),
        ChatAction::RemoveByAuthor { channel_id } => {
            remove_where(chat, |(_, author, _)| {
                author.as_deref() == Some(channel_id.as_str())
            });
        }
        ChatAction::Pin { id, message } => {
            let banner = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            banner.append(&gtk::Image::from_icon_name("yt-pin-symbolic"));
            let content = row(&message);
            content.set_hexpand(true);
            banner.append(&content);
            chat.pinned.set_child(Some(&banner));
            chat.pinned.set_reveal_child(true);
            chat.state.borrow_mut().pinned = Some(id);
        }
        ChatAction::Unpin { id } => {
            if chat.state.borrow().pinned.as_deref() == Some(id.as_str()) {
                chat.state.borrow_mut().pinned = None;
                chat.pinned.set_reveal_child(false);
            }
        }
    }
}

fn remove_where(chat: &Chat, matches: impl Fn(&(String, Option<String>, gtk::Widget)) -> bool) {
    let mut gone = Vec::new();
    chat.state.borrow_mut().rows.retain(|entry| {
        let keep = !matches(entry);
        if !keep {
            gone.push(entry.2.clone());
        }
        keep
    });
    for row in gone {
        chat.list.remove(&row);
    }
}

/// Keeps the newest messages; while scrolled up, twice as many, so what is
/// being read doesn't move away.
fn trim(chat: &Chat) {
    let gone: Vec<gtk::Widget> = {
        let mut state = chat.state.borrow_mut();
        let keep = if state.stick { KEPT } else { KEPT * 2 };
        let extra = state.rows.len().saturating_sub(keep);
        state.rows.drain(..extra).map(|(_, _, row)| row).collect()
    };
    for row in gone {
        chat.list.remove(&row);
    }
}

fn add_notice(chat: &Chat, text: &str) {
    let message = ChatMessage {
        id: format!("notice-{}", glib::monotonic_time()),
        text: RichText::from_plain(text),
        kind: ChatKind::Notice,
        ..ChatMessage::default()
    };
    apply(chat, ChatAction::Add { message });
}

/// A message's row, as youtube.com draws each kind.
fn row(message: &ChatMessage) -> gtk::Widget {
    match &message.kind {
        ChatKind::Text => plain_row(message, &message.text),
        ChatKind::Notice => {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            row.add_css_class("live-chat-notice");
            row.append(&gtk::Image::from_icon_name("yt-info-symbolic"));
            let text = text_view(None, &message.text);
            row.append(&text);
            row.upcast()
        }
        ChatKind::SuperChat {
            amount,
            header_color,
            body_color,
        } => {
            let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
            card.add_css_class("live-chat-card");
            let header = card_header(&message.author, Some(amount), None);
            header.add_css_class(&color_class(header_color));
            card.append(&header);
            if !message.text.runs.is_empty() {
                let body = text_view(None, &message.text);
                body.add_css_class("live-chat-card-body");
                let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
                frame.add_css_class("live-chat-card-body");
                frame.add_css_class(&color_class(body_color));
                frame.append(&body);
                card.append(&frame);
            }
            card.upcast()
        }
        ChatKind::Sticker {
            amount,
            sticker,
            color,
        } => {
            let header = card_header(&message.author, Some(amount), None);
            header.add_css_class(&color_class(color));
            header.add_css_class("live-chat-card");
            if let Some(url) = sticker {
                let picture = picture(url, 72);
                picture.set_halign(gtk::Align::End);
                picture.set_hexpand(true);
                header.append(&picture);
            }
            header.upcast()
        }
        ChatKind::Membership { header, subtext } => {
            let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
            card.add_css_class("live-chat-card");
            let top = card_header(&message.author, None, Some((header, subtext.as_deref())));
            top.add_css_class("live-chat-membership");
            card.append(&top);
            if !message.text.runs.is_empty() {
                let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
                frame.add_css_class("live-chat-card-body");
                frame.add_css_class("live-chat-membership-body");
                frame.append(&text_view(None, &message.text));
                card.append(&frame);
            }
            card.upcast()
        }
        ChatKind::Gift { image } => {
            let row = plain_row(message, &message.text);
            if let (Some(url), Some(row)) = (image, row.downcast_ref::<gtk::Box>()) {
                row.append(&picture(url, 40));
            }
            row.add_css_class("live-chat-gift");
            row
        }
    }
}

/// An avatar, then the author and the text in one flow.
fn plain_row(message: &ChatMessage, text: &RichText) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("live-chat-message");
    let avatar = avatar::new(24, &message.author.name, message.author.avatar.as_deref());
    avatar.set_valign(gtk::Align::Start);
    row.append(&avatar);
    row.append(&text_view(Some(&message.author), text));
    row.upcast()
}

/// The top of a colored card: avatar, author, and the amount or what the
/// membership says.
fn card_header(
    author: &ChatAuthor,
    amount: Option<&str>,
    membership: Option<(&String, Option<&str>)>,
) -> gtk::Box {
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    header.add_css_class("live-chat-card-header");
    let avatar = avatar::new(32, &author.name, author.avatar.as_deref());
    avatar.set_valign(gtk::Align::Center);
    header.append(&avatar);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_valign(gtk::Align::Center);
    let name = gtk::Label::new(Some(&author.name));
    name.add_css_class("live-chat-card-name");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&name);
    if let Some(amount) = amount {
        let label = gtk::Label::new(Some(amount));
        label.add_css_class("live-chat-amount");
        label.set_xalign(0.0);
        text.append(&label);
    }
    if let Some((primary, sub)) = membership {
        let label = gtk::Label::new(Some(primary));
        label.add_css_class("live-chat-amount");
        label.set_xalign(0.0);
        label.set_wrap(true);
        text.append(&label);
        if let Some(sub) = sub {
            let label = gtk::Label::new(Some(sub));
            label.set_xalign(0.0);
            text.append(&label);
        }
    }
    header.append(&text);
    header
}

fn picture(url: &str, size: i32) -> gtk::Picture {
    let picture = gtk::Picture::new();
    picture.set_size_request(size, size);
    picture.set_can_shrink(true);
    picture.set_content_fit(gtk::ContentFit::Contain);
    app().images.load_into(&picture, Some(url), size * 2);
    picture
}

/// The author (with badges) and the message, wrapping as one text, with
/// channels' emoji as pictures in it.
fn text_view(author: Option<&ChatAuthor>, text: &RichText) -> gtk::Widget {
    use super::inline_label::Piece;
    let mut pieces = Vec::new();
    if let Some(author) = author.filter(|a| !a.name.is_empty()) {
        // The author's name colored as youtube.com colors it: the channel
        // on yellow, moderators blue, members green.
        let (color, background) = if author.owner {
            ("#0f0f0f", Some("#ffd600"))
        } else if author.moderator {
            ("#5e84f1", None)
        } else if author.member.is_some() {
            ("#2ba640", None)
        } else {
            ("rgba(255, 255, 255, 0.7)", None)
        };
        pieces.push(Piece::Text {
            text: &author.name,
            color: Some(color),
            background,
            bold: true,
        });
        if author.verified {
            pieces.push(Piece::text(" "));
            pieces.push(Piece::Icon {
                name: "yt-verified-symbolic",
                size: 14,
                class: "live-chat-badge",
                tooltip: "Verified",
            });
        }
        if author.moderator {
            pieces.push(Piece::text(" "));
            pieces.push(Piece::Icon {
                name: "yt-wrench-symbolic",
                size: 14,
                class: "live-chat-moderator",
                tooltip: "Moderator",
            });
        }
        if let Some(url) = &author.member_badge {
            pieces.push(Piece::text(" "));
            pieces.push(Piece::Picture {
                url,
                size: 16,
                tooltip: author.member.as_deref(),
            });
        }
        pieces.push(Piece::text("  "));
    }
    for run in &text.runs {
        pieces.push(match &run.emoji {
            Some(url) => Piece::Picture {
                url,
                size: 24,
                tooltip: Some(&run.text),
            },
            None => Piece::Text {
                text: &run.text,
                color: None,
                background: None,
                bold: run.bold,
            },
        });
    }
    let label = super::inline_label::build(&pieces);
    label.add_css_class("live-chat-text");
    label
}

thread_local! {
    /// The CSS classes for the colors YouTube gives paid messages (a few
    /// tiers), and the style sheet defining them.
    static COLORS: RefCell<(HashMap<String, String>, Option<gtk::CssProvider>)> =
        RefCell::default();
}

/// A class with `color` as its background, defined the first time.
fn color_class(color: &str) -> String {
    COLORS.with(|colors| {
        let (classes, provider) = &mut *colors.borrow_mut();
        if let Some(class) = classes.get(color) {
            return class.clone();
        }
        let class = format!("live-chat-color-{}", classes.len());
        classes.insert(color.to_owned(), class.clone());
        let css: String = classes
            .iter()
            .map(|(color, class)| format!(".{class} {{ background-color: {color}; }}\n"))
            .collect();
        let provider = provider.get_or_insert_with(|| {
            let provider = gtk::CssProvider::new();
            if let Some(display) = gtk::gdk::Display::default() {
                gtk::style_context_add_provider_for_display(
                    &display,
                    &provider,
                    gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
                );
            }
            provider
        });
        provider.load_from_string(&css);
        class
    })
}

/// The chat on screen, for the `chat.*` commands.
pub fn on_screen() -> Option<Rc<Chat>> {
    let root = super::find_widget(app().window()?.upcast_ref(), "live-chat-panel")?;
    root.is_mapped()
        .then(|| super::attached::<Chat>(&root))
        .flatten()
}

/// What `chat.state` answers: the video, the view, whether it is shown and
/// asking, and the messages on screen.
pub fn describe(chat: &Chat) -> Value {
    let state = chat.state.borrow();
    json!({
        "video_id": state.video_id,
        "view": state.view,
        "live": state.live,
        "hidden": hidden(),
        "running": state.running,
        "messages": state.rows.len(),
        "pinned": state.pinned,
        "following": state.stick,
        "status": chat.content.visible_child_name().map(|n| n.to_string()),
    })
}

/// Makes the chat as tall as `height` (the player beside it), or its own
/// height for `None`.
pub fn set_height(panel: &gtk::Widget, height: Option<i32>) {
    if let Some(chat) = super::attached::<Chat>(panel) {
        let height = height.map_or(HEIGHT, |h| h.max(MIN_HEIGHT));
        if chat.frame.height_request() != height {
            chat.frame.set_size_request(-1, height);
        }
    }
}
