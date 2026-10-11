//! The `channel` page: banner, avatar, name, stats, description and the
//! subscribe button, then the channel's videos in tabs (Home, Videos,
//! Shorts, Live), shown by a `feed` component. The banner and description
//! fold away once the videos scroll, giving the grid the room.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use serde_json::{Value, json};
use yt_core::channel::{ChannelPage, ChannelTab};

use super::aspect::AspectBin;
use super::{attach, attached, avatar};
use crate::app::app;
use crate::runtime;
use crate::social::{self, ChannelKey};

/// Banners are about 6:1.
const BANNER_RATIO: f64 = 1.0 / 6.2;

struct ChannelView {
    key: RefCell<ChannelKey>,
    page: RefCell<Option<ChannelPage>>,
    generation: Cell<u64>,
    /// The page itself, weakly: it owns this view.
    content: gtk::glib::WeakRef<gtk::Stack>,
    status: adw::StatusPage,
    banner: gtk::Picture,
    banner_frame: gtk::Revealer,
    avatar: adw::Avatar,
    name: gtk::Label,
    verified: gtk::Image,
    meta: gtk::Label,
    description: gtk::Button,
    description_frame: gtk::Revealer,
    subscribe: gtk::Box,
    tabs: gtk::Box,
    feed: gtk::Box,
    /// The component showing the tab: `shelves` for Home, else `feed`.
    tab_component: Cell<&'static str>,
}

/// Props: any of `{ channel_id, handle, name, video_id }`; see
/// [`ChannelKey`].
pub fn build(props: &Value) -> gtk::Widget {
    let banner = gtk::Picture::new();
    banner.set_content_fit(gtk::ContentFit::Cover);
    banner.set_can_shrink(true);
    let banner_bin = AspectBin::new(BANNER_RATIO, &banner);
    banner_bin.add_css_class("channel-banner");
    banner_bin.set_overflow(gtk::Overflow::Hidden);
    let banner_frame = gtk::Revealer::builder()
        .child(&banner_bin)
        .transition_type(gtk::RevealerTransitionType::SlideUp)
        .reveal_child(true)
        .build();

    let avatar = adw::Avatar::new(112, None, true);
    avatar.set_valign(gtk::Align::Start);
    let name = gtk::Label::new(None);
    name.add_css_class("channel-name");
    name.set_xalign(0.0);
    let verified = gtk::Image::from_icon_name("yt-verified-symbolic");
    verified.add_css_class("verified");
    verified.set_visible(false);
    let title = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    title.append(&name);
    title.append(&verified);
    let meta = gtk::Label::new(None);
    meta.add_css_class("dim");
    meta.set_xalign(0.0);
    meta.set_wrap(true);
    let description_label = gtk::Label::new(None);
    description_label.set_xalign(0.0);
    description_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    description_label.set_lines(1);
    description_label.set_max_width_chars(90);
    let description = gtk::Button::builder()
        .child(&description_label)
        .css_classes(["flat", "channel-description"])
        .build();
    description.set_halign(gtk::Align::Start);
    let description_frame = gtk::Revealer::builder()
        .child(&description)
        .reveal_child(true)
        .build();
    let subscribe = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.append(&subscribe);
    actions.append(&app().ext.slots.create(
        "channel.header.actions",
        gtk::Orientation::Horizontal,
        8,
    ));
    let text = gtk::Box::new(gtk::Orientation::Vertical, 6);
    text.set_valign(gtk::Align::Center);
    text.append(&title);
    text.append(&meta);
    text.append(&description_frame);
    text.append(&actions);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 24);
    header.add_css_class("channel-header");
    header.append(&avatar);
    header.append(&text);

    let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    tabs.add_css_class("channel-tabs");
    let feed = gtk::Box::new(gtk::Orientation::Vertical, 0);
    feed.set_vexpand(true);

    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    column.append(&banner_frame);
    column.append(&header);
    column.append(&tabs);
    column.append(&feed);

    let spinner = adw::Spinner::new();
    spinner.set_size_request(48, 48);
    spinner.set_halign(gtk::Align::Center);
    spinner.set_valign(gtk::Align::Center);
    let status = adw::StatusPage::new();
    status.set_icon_name(Some("dialog-warning-symbolic"));
    let content = gtk::Stack::new();
    content.add_named(&spinner, Some("loading"));
    content.add_named(&status, Some("status"));
    content.add_named(&column, Some("channel"));
    content.set_widget_name("page-channel");
    content.add_css_class("channel-page");

    let view = Rc::new(ChannelView {
        key: RefCell::default(),
        page: RefCell::default(),
        generation: Cell::new(0),
        content: content.downgrade(),
        status,
        banner,
        banner_frame,
        avatar,
        name,
        verified,
        meta,
        description,
        description_frame,
        subscribe,
        tabs,
        feed,
        tab_component: Cell::new(""),
    });
    attach(&content, view.clone());
    let weak = Rc::downgrade(&view);
    view.description.connect_clicked(move |button| {
        if let Some(view) = weak.upgrade() {
            show_about(&view, button);
        }
    });
    load(&view, props);
    content.upcast()
}

pub fn update(widget: &gtk::Widget, props: &Value) {
    if let Some(view) = attached::<ChannelView>(widget) {
        load(&view, props);
    }
}

fn load(view: &Rc<ChannelView>, props: &Value) {
    let key = ChannelKey::from_args(props);
    let same = view.page.borrow().as_ref().is_some_and(|p| {
        let c = &p.channel;
        key.id.is_some() && key.id == c.id || key.handle.is_some() && key.handle == c.handle
    });
    let loading = *view.key.borrow() == key
        && view
            .content
            .upgrade()
            .and_then(|c| c.visible_child_name())
            .as_deref()
            == Some("loading");
    if same || loading {
        return;
    }
    *view.key.borrow_mut() = key.clone();
    view.page.borrow_mut().take();
    view.generation.set(view.generation.get() + 1);
    let generation = view.generation.get();
    show_content(view, "loading");
    let weak = Rc::downgrade(view);
    runtime::spawn_local(async move {
        let result = social::load_channel(key).await;
        let Some(view) = weak.upgrade().filter(|v| v.generation.get() == generation) else {
            return;
        };
        match result {
            Ok(page) => fill(&view, page),
            Err(e) => {
                view.status.set_title("Couldn't load the channel");
                view.status.set_description(Some(&e));
                show_content(&view, "status");
            }
        }
    });
}

fn fill(view: &Rc<ChannelView>, page: ChannelPage) {
    let channel = &page.channel;
    view.name.set_label(&channel.name);
    view.verified.set_visible(channel.verified);
    avatar::set(&view.avatar, &channel.name, channel.avatar.as_deref());
    let videos = page.about.iter().find(|a| a.ends_with(" videos"));
    let meta: Vec<&str> = [
        channel.handle.as_deref(),
        channel.subscribers.as_deref(),
        videos.map(String::as_str),
    ]
    .into_iter()
    .flatten()
    .collect();
    view.meta.set_label(&meta.join("  •  "));
    let description = page.description.clone().unwrap_or_default();
    if let Some(label) = view.description.child().and_downcast::<gtk::Label>() {
        let first = description
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("More about this channel");
        label.set_markup(&format!(
            "{} <b>…more</b>",
            gtk::glib::markup_escape_text(first)
        ));
    }
    view.description_frame
        .set_visible(!description.is_empty() || !page.about.is_empty());
    app()
        .images
        .load_into(&view.banner, page.banner.as_deref(), 2120);
    view.banner_frame.set_visible(page.banner.is_some());

    while let Some(child) = view.subscribe.first_child() {
        view.subscribe.remove(&child);
    }
    if social::is_mine(channel.handle.as_deref()) {
        // Your own channel: you can't subscribe, but you can manage it.
        let manage = gtk::Button::with_label("Manage videos");
        manage.add_css_class("pill-button");
        manage.add_css_class("manage-button");
        manage.set_tooltip_text(Some("Open YouTube Studio in your browser"));
        let url = format!(
            "https://studio.youtube.com/channel/{}/videos",
            channel.id.as_deref().unwrap_or_default()
        );
        manage.connect_clicked(move |_| {
            if let Err(e) = app()
                .ext
                .commands
                .run("app.open_url", json!({ "url": url }))
            {
                app().toast(&e);
            }
        });
        view.subscribe.append(&manage);
    } else if let Some(subscription) = &page.subscription {
        view.subscribe.append(&app().ext.components.build(
            "subscribe_button",
            &json!({ "subscription": subscription, "channel_name": channel.name }),
        ));
    }

    let channel_id = channel.id.clone().unwrap_or_default();
    let tabs: Vec<ChannelTab> = std::iter::once(ChannelTab {
        title: "Home".into(),
        params: String::new(),
    })
    .chain(page.tabs.iter().cloned())
    .collect();
    while let Some(child) = view.tabs.first_child() {
        view.tabs.remove(&child);
    }
    let mut group: Option<gtk::ToggleButton> = None;
    for (i, tab) in tabs.iter().enumerate() {
        let chip = gtk::ToggleButton::with_label(&tab.title);
        chip.add_css_class("channel-tab");
        chip.set_active(i == 0);
        match &group {
            Some(first) => chip.set_group(Some(first)),
            None => group = Some(chip.clone()),
        }
        let (weak, id, tab) = (Rc::downgrade(view), channel_id.clone(), tab.clone());
        chip.connect_toggled(move |chip| {
            if chip.is_active()
                && let Some(view) = weak.upgrade()
            {
                show_tab(&view, &id, &tab);
            }
        });
        view.tabs.append(&chip);
    }
    *view.page.borrow_mut() = Some(page);
    // Shown first: the feed must be on screen when it fills, or the grid
    // keeps a stale scroll position.
    show_content(view, "channel");
    show_tab(view, &channel_id, &tabs[0]);
}

/// Shows a tab: Home's featured shelves as rows (`shelves`), or a tab's
/// grid (`feed`).
fn show_tab(view: &Rc<ChannelView>, channel_id: &str, tab: &ChannelTab) {
    let mut props = json!({
        "browse_id": channel_id,
        "params": tab.params,
        "untitled": tab.title,
        "on_channel": true,
    });
    let home = tab.params.is_empty();
    let playlists = tab.params == yt_core::channel::PLAYLISTS_TAB;
    // Videos come in youtube.com's orders: Latest, Popular, Oldest.
    if tab.params == yt_core::channel::VIDEOS_TAB {
        props["sortable"] = true.into();
    }
    // Home came with the channel page; the other tabs load when chosen.
    if home && let Some(page) = view.page.borrow().as_ref() {
        props["page"] = serde_json::to_value(&page.home).unwrap_or_default();
    }
    let component = match (home, playlists) {
        (true, _) => "shelves",
        (_, true) => "playlists",
        _ => "feed",
    };
    let components = &app().ext.components;
    match view.feed.first_child() {
        Some(feed)
            if view.tab_component.get() == component
                && components.update(component, &feed, &props) => {}
        existing => {
            if let Some(old) = existing {
                view.feed.remove(&old);
            }
            view.tab_component.set(component);
            let feed = components.build(component, &props);
            feed.set_vexpand(true);
            view.feed.append(&feed);
            connect_folding(view, &feed);
        }
    }
    set_folded(view, false);
}

fn show_content(view: &ChannelView, name: &str) {
    if let Some(content) = view.content.upgrade() {
        content.set_visible_child_name(name);
    }
}

/// Folds the banner and description away when the user scrolls the videos
/// down, and back when they scroll up at the top.
///
/// Only scrolling by the user counts: folding gives the grid more room,
/// which moves its scroll position, so following the position would fold
/// and unfold in a loop.
fn connect_folding(view: &Rc<ChannelView>, feed: &gtk::Widget) {
    let scroller = super::find_widget(feed, "shelves")
        .or_else(|| super::find_widget(feed, "page-playlists"))
        .or_else(|| super::find_widget(feed, "video-grid").and_then(|g| g.parent()));
    let Some(scroller) = scroller.and_downcast::<gtk::ScrolledWindow>() else {
        return;
    };
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    // Before the scrolled window takes the event.
    scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
    let (weak, adjustment) = (Rc::downgrade(view), scroller.vadjustment());
    scroll.connect_scroll(move |_, _, dy| {
        if let Some(view) = weak.upgrade() {
            let scrollable = adjustment.upper() > adjustment.page_size() + 1.0;
            if dy > 0.0 && scrollable {
                set_folded(&view, true);
            } else if dy < 0.0 && adjustment.value() <= 0.0 {
                set_folded(&view, false);
            }
        }
        gtk::glib::Propagation::Proceed
    });
    scroller.add_controller(scroll);
}

fn set_folded(view: &ChannelView, folded: bool) {
    if view.banner_frame.reveals_child() == folded {
        view.banner_frame.set_reveal_child(!folded);
        view.description_frame.set_reveal_child(!folded);
    }
}

/// The full description and the about facts, in a dialog.
fn show_about(view: &ChannelView, parent: &gtk::Button) {
    let Some(page) = view.page.borrow().clone() else {
        return;
    };
    let description = gtk::Label::new(page.description.as_deref());
    description.set_wrap(true);
    description.set_xalign(0.0);
    description.set_selectable(true);
    let facts = gtk::Box::new(gtk::Orientation::Vertical, 6);
    for fact in &page.about {
        let label = gtk::Label::new(Some(fact));
        label.set_xalign(0.0);
        label.add_css_class("dim");
        facts.append(&label);
    }
    let column = gtk::Box::new(gtk::Orientation::Vertical, 18);
    column.set_margin_top(12);
    column.set_margin_bottom(24);
    column.set_margin_start(24);
    column.set_margin_end(24);
    column.append(&description);
    column.append(&facts);
    let scroller = gtk::ScrolledWindow::builder()
        .child(&column)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(560)
        .build();
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&scroller));
    let dialog = adw::Dialog::builder()
        .title(format!("About {}", page.channel.name))
        .content_width(560)
        .child(&toolbar)
        .build();
    dialog.present(Some(parent));
}
