//! The `search_box` component in the top bar, as on youtube.com: a field
//! with a clear button, the search button, and below the field while it has
//! the focus, earlier searches and YouTube's suggestions for what is typed.
//! Up and Down pick one (the field shows it), Enter searches, Escape closes
//! the list, and `/` anywhere puts the cursor in the field (`search.focus`).
//!
//! The list's rows go through the `search.box_rows` filter:
//! `{ typed, rows = [{ text, history?, image?, description?, icon?,
//! command?, args? }] }`. Choosing a row searches for its text, or with a
//! `command`, runs that instead (a plugin's "Play this video" for a pasted
//! link, say).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use serde_json::{Value, json};

use super::follow;
use crate::app::app;

/// Earlier searches shown with an empty field, and above suggestions.
const HISTORY_ALONE: usize = 12;
const HISTORY_WITH_SUGGESTIONS: usize = 3;
const MAX_ROWS: usize = 12;
/// The size of a person's or topic's picture in the list.
const SUGGESTION_IMAGE: i32 = 40;
/// Suggestions are asked for once typing pauses this long.
const SUGGEST_AFTER: Duration = Duration::from_millis(110);

struct SearchBox {
    entry: gtk::Entry,
    field: gtk::Box,
    clear: gtk::Button,
    popover: gtk::Popover,
    list: gtk::Box,
    /// The rows shown: `{ text, history?, image?, description? }`.
    rows: RefCell<Vec<Value>>,
    suggestions: RefCell<(String, Vec<Value>)>,
    /// The row picked with the arrow keys.
    selected: Cell<Option<usize>>,
    /// What the user typed, which the field shows again when no row is
    /// picked.
    typed: RefCell<String>,
    timer: RefCell<Option<glib::SourceId>>,
    /// The field's text is being set by the box itself.
    quiet: Cell<bool>,
    /// The pointer is over the list, so a click on it is on its way.
    hovered: Cell<bool>,
}

pub fn build(_props: &Value) -> gtk::Widget {
    let entry = gtk::Entry::builder()
        .placeholder_text("Search")
        .has_frame(false)
        .hexpand(true)
        // Room for about this many characters, if the window has it.
        .width_chars(8)
        .max_width_chars(52)
        .build();
    entry.add_css_class("search-entry");
    let clear = gtk::Button::from_icon_name("yt-close-symbolic");
    clear.add_css_class("search-clear");
    clear.set_tooltip_text(Some("Clear search query"));
    clear.set_visible(false);
    clear.set_focus_on_click(false);
    let field = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    field.add_css_class("search-field");
    field.set_hexpand(true);
    field.append(&entry);
    field.append(&clear);
    let button = gtk::Button::from_icon_name("yt-search-symbolic");
    button.add_css_class("search-button");
    button.set_tooltip_text(Some("Search"));
    button.set_focus_on_click(false);
    let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    root.add_css_class("search-box");
    root.set_widget_name("search-box");
    root.append(&field);
    root.append(&button);

    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.add_css_class("search-suggestions");
    // Not autohide: it must not take the keyboard from the field.
    let popover = gtk::Popover::builder()
        .child(&list)
        .has_arrow(false)
        .autohide(false)
        .can_focus(false)
        .position(gtk::PositionType::Bottom)
        .halign(gtk::Align::Start)
        .build();
    popover.add_css_class("search-popover");
    popover.set_parent(&field);
    popover.set_offset(0, 4);
    let attached = popover.clone();
    field.connect_destroy(move |_| attached.unparent());

    let this = Rc::new(SearchBox {
        entry,
        field,
        clear,
        popover,
        list,
        rows: RefCell::default(),
        suggestions: RefCell::default(),
        selected: Cell::new(None),
        typed: RefCell::default(),
        timer: RefCell::default(),
        quiet: Cell::new(false),
        hovered: Cell::new(false),
    });
    super::attach(&root, this.clone());
    connect(&this, &root, &button);
    root.upcast()
}

fn connect(this: &Rc<SearchBox>, root: &gtk::Box, button: &gtk::Button) {
    let weak = Rc::downgrade(this);
    this.entry.connect_changed(move |entry| {
        let Some(this) = weak.upgrade() else { return };
        this.clear.set_visible(!entry.text().is_empty());
        if this.quiet.get() {
            return;
        }
        *this.typed.borrow_mut() = entry.text().to_string();
        this.selected.set(None);
        schedule_suggestions(&this);
        refresh(&this);
    });
    let weak = Rc::downgrade(this);
    this.entry.connect_activate(move |entry| {
        let Some(this) = weak.upgrade() else { return };
        let picked = this
            .selected
            .get()
            .and_then(|i| this.rows.borrow().get(i).cloned());
        match picked {
            Some(row) => choose(&this, &row),
            None => search(&this, &entry.text()),
        }
    });
    let weak = Rc::downgrade(this);
    button.connect_clicked(move |_| {
        if let Some(this) = weak.upgrade() {
            let text = this.entry.text();
            if text.trim().is_empty() {
                this.entry.grab_focus();
            } else {
                search(&this, &text);
            }
        }
    });
    let weak = Rc::downgrade(this);
    this.clear.connect_clicked(move |_| {
        if let Some(this) = weak.upgrade() {
            this.entry.set_text("");
            this.entry.grab_focus();
        }
    });

    let focus = gtk::EventControllerFocus::new();
    let weak = Rc::downgrade(this);
    focus.connect_enter(move |_| {
        if let Some(this) = weak.upgrade() {
            *this.typed.borrow_mut() = this.entry.text().to_string();
            this.field.add_css_class("focused");
            refresh(&this);
        }
    });
    let weak = Rc::downgrade(this);
    focus.connect_leave(move |_| {
        if let Some(this) = weak.upgrade() {
            this.field.remove_css_class("focused");
            // A click on a row closes the list itself.
            if !this.hovered.get() {
                this.popover.popdown();
            }
        }
    });
    this.entry.add_controller(focus);
    let motion = gtk::EventControllerMotion::new();
    let weak = Rc::downgrade(this);
    motion.connect_enter(move |_, _, _| {
        if let Some(this) = weak.upgrade() {
            this.hovered.set(true);
        }
    });
    let weak = Rc::downgrade(this);
    motion.connect_leave(move |_| {
        if let Some(this) = weak.upgrade() {
            this.hovered.set(false);
            if !has_focus(&this) {
                this.popover.popdown();
            }
        }
    });
    this.list.add_controller(motion);

    let keys = gtk::EventControllerKey::new();
    let weak = Rc::downgrade(this);
    keys.connect_key_pressed(move |_, key, _, _| {
        let Some(this) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        match key {
            gtk::gdk::Key::Down => step(&this, 1),
            gtk::gdk::Key::Up => step(&this, -1),
            gtk::gdk::Key::Escape if this.popover.is_visible() => this.popover.popdown(),
            gtk::gdk::Key::Escape => {
                // Out of the field, so the page's keys work again.
                if let Some(window) = this.entry.root().and_downcast::<gtk::Window>() {
                    GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
                }
            }
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    this.entry.add_controller(keys);

    let weak = Rc::downgrade(this);
    follow(root, "search.focus", move |_| {
        if let Some(this) = weak.upgrade() {
            this.entry.grab_focus();
        }
    });
    let weak = Rc::downgrade(this);
    follow(root, "search.suggestions", move |payload| {
        let Some(this) = weak.upgrade() else { return };
        let query = payload
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if query == this.typed.borrow().trim() {
            let rows = payload
                .get("suggestions")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            *this.suggestions.borrow_mut() = (query.to_owned(), rows);
            refresh(&this);
        }
    });
    let weak = Rc::downgrade(this);
    follow(root, "search.history_changed", move |_| {
        if let Some(this) = weak.upgrade()
            && this.popover.is_visible()
        {
            refresh(&this);
        }
    });
    // The field shows the query of the results on screen, and empties on
    // the feeds.
    let weak = Rc::downgrade(this);
    follow(root, "page.changed", move |payload| {
        let Some(this) = weak.upgrade() else { return };
        let page = payload
            .get("page")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let text = match page {
            "search" => payload.pointer("/args/query").and_then(Value::as_str),
            "home" | "subscriptions" | "history" => Some(""),
            _ => None,
        };
        if let Some(text) = text {
            set_text(&this, text);
        }
    });
}

/// Sets the field's text without asking for suggestions.
fn set_text(this: &SearchBox, text: &str) {
    this.quiet.set(true);
    this.entry.set_text(text);
    this.entry.set_position(-1);
    this.quiet.set(false);
}

/// Runs a row's command, or searches for its text.
fn choose(this: &SearchBox, row: &Value) {
    let Some(command) = row.get("command").and_then(Value::as_str) else {
        search(this, &text_of(row));
        return;
    };
    this.popover.popdown();
    set_text(this, "");
    if let Some(window) = this.entry.root().and_downcast::<gtk::Window>() {
        GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
    }
    let args = row.get("args").cloned().unwrap_or_else(|| json!({}));
    if let Err(e) = app().ext.commands.run(command, args) {
        app().toast(&e);
    }
}

fn search(this: &SearchBox, text: &str) {
    let query = text.trim();
    if query.is_empty() {
        return;
    }
    this.popover.popdown();
    set_text(this, query);
    if let Some(window) = this.entry.root().and_downcast::<gtk::Window>() {
        GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
    }
    if let Err(e) = app()
        .ext
        .commands
        .run("search.open", json!({ "query": query }))
    {
        app().toast(&e);
    }
}

fn schedule_suggestions(this: &Rc<SearchBox>) {
    if let Some(timer) = this.timer.borrow_mut().take() {
        timer.remove();
    }
    let typed = this.typed.borrow().trim().to_owned();
    if typed.is_empty() {
        return;
    }
    let weak = Rc::downgrade(this);
    let timer = glib::timeout_add_local_once(SUGGEST_AFTER, move || {
        if let Some(this) = weak.upgrade() {
            this.timer.borrow_mut().take();
            let _ = app()
                .ext
                .commands
                .run("search.suggest", json!({ "query": typed }));
        }
    });
    *this.timer.borrow_mut() = Some(timer);
}

/// Moves the picked row; the field shows it (or what was typed again).
fn step(this: &SearchBox, by: i32) {
    let count = this.rows.borrow().len() as i32;
    if count == 0 {
        return;
    }
    if !this.popover.is_visible() {
        this.popover.popup();
    }
    let next = match this.selected.get() {
        None if by > 0 => Some(0),
        None => Some(count - 1),
        Some(i) => Some(i as i32 + by).filter(|n| (0..count).contains(n)),
    };
    this.selected.set(next.map(|n| n as usize));
    let text = match next {
        Some(n) => text_of(&this.rows.borrow()[n as usize]),
        None => this.typed.borrow().clone(),
    };
    set_text(this, &text);
    show_selection(this);
}

/// Whether the keyboard is in the field (in the text inside the entry).
fn has_focus(this: &SearchBox) -> bool {
    this.entry
        .root()
        .and_then(|root| root.focus())
        .is_some_and(|focus| {
            focus == *this.entry.upcast_ref::<gtk::Widget>() || focus.is_ancestor(&this.entry)
        })
}

fn text_of(row: &Value) -> String {
    row.get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// Shows earlier searches and suggestions for what is typed.
fn refresh(this: &Rc<SearchBox>) {
    let typed = this.typed.borrow().trim().to_owned();
    let history = &app().search_history;
    let mut rows: Vec<Value> = history
        .matching(
            &typed,
            if typed.is_empty() {
                HISTORY_ALONE
            } else {
                HISTORY_WITH_SUGGESTIONS
            },
        )
        .into_iter()
        .map(|q| json!({ "text": q, "history": true }))
        .collect();
    let (asked, suggestions) = this.suggestions.borrow().clone();
    // Suggestions for an earlier prefix still help until the new ones come.
    if !typed.is_empty() && typed.to_lowercase().starts_with(&asked.to_lowercase()) {
        for s in suggestions {
            let text = text_of(&s);
            let starts = text.to_lowercase().starts_with(&typed.to_lowercase());
            if starts && !rows.iter().any(|r| text_of(r).eq_ignore_ascii_case(&text)) {
                rows.push(s);
            }
        }
    }
    rows.truncate(MAX_ROWS);
    let rows = app()
        .ext
        .filters
        .apply("search.box_rows", json!({ "typed": typed, "rows": rows }))
        .get("rows")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    *this.rows.borrow_mut() = rows.clone();
    this.selected.set(None);
    while let Some(child) = this.list.first_child() {
        this.list.remove(&child);
    }
    for row in &rows {
        this.list.append(&row_widget(this, row, &typed));
    }
    if rows.is_empty() || !has_focus(this) {
        this.popover.popdown();
    } else {
        this.popover.set_size_request(this.field.width(), -1);
        this.popover.popup();
    }
}

fn row_widget(this: &Rc<SearchBox>, row: &Value, typed: &str) -> gtk::Widget {
    let text = text_of(row);
    let history = row.get("history").and_then(Value::as_bool).unwrap_or(false);
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    match row.get("image").and_then(Value::as_str) {
        Some(url) => {
            // A person or topic: their picture, a fixed size whatever the
            // image's.
            let image = gtk::Image::new();
            image.set_pixel_size(SUGGESTION_IMAGE);
            image.add_css_class("suggestion-image");
            image.set_overflow(gtk::Overflow::Hidden);
            let weak = image.downgrade();
            app().images.texture_then(
                url,
                SUGGESTION_IMAGE * 2,
                Box::new(move |texture| {
                    if let (Some(image), Some(texture)) = (weak.upgrade(), texture) {
                        image.set_paintable(Some(&texture));
                    }
                }),
            );
            content.append(&image);
        }
        None => {
            let icon = row
                .get("icon")
                .and_then(Value::as_str)
                .unwrap_or(if history {
                    "yt-history-symbolic"
                } else {
                    "yt-search-symbolic"
                });
            let icon = gtk::Image::from_icon_name(icon);
            icon.add_css_class("suggestion-icon");
            content.append(&icon);
        }
    }
    let words = gtk::Box::new(gtk::Orientation::Vertical, 0);
    words.set_valign(gtk::Align::Center);
    words.set_hexpand(true);
    let label = gtk::Label::new(None);
    label.set_markup(&completion_markup(&text, typed));
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    words.append(&label);
    if let Some(description) = row.get("description").and_then(Value::as_str) {
        let label = gtk::Label::new(Some(description));
        label.add_css_class("suggestion-description");
        label.set_xalign(0.0);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        words.append(&label);
    }
    content.append(&words);
    if history {
        let remove = gtk::Button::with_label("Remove");
        remove.add_css_class("suggestion-remove");
        remove.set_focus_on_click(false);
        let query = text.clone();
        remove.connect_clicked(move |_| {
            let _ = app()
                .ext
                .commands
                .run("search.history_remove", json!({ "query": query }));
        });
        content.append(&remove);
    }
    let button = gtk::Button::builder()
        .child(&content)
        .css_classes(["flat", "suggestion"])
        .focus_on_click(false)
        .can_focus(false)
        .build();
    let weak = Rc::downgrade(this);
    let row = row.clone();
    button.connect_clicked(move |_| {
        if let Some(this) = weak.upgrade() {
            choose(&this, &row);
        }
    });
    button.upcast()
}

/// YouTube's way: what was typed as typed, the rest of the suggestion bold.
fn completion_markup(text: &str, typed: &str) -> String {
    let escape = |s: &str| glib::markup_escape_text(s).to_string();
    let n = typed.len();
    if n > 0 && text.len() >= n && text.is_char_boundary(n) && text[..n].eq_ignore_ascii_case(typed)
    {
        format!("{}<b>{}</b>", escape(&text[..n]), escape(&text[n..]))
    } else {
        format!("<b>{}</b>", escape(text))
    }
}

fn show_selection(this: &SearchBox) {
    let mut child = this.list.first_child();
    let mut i = 0;
    while let Some(row) = child {
        if Some(i) == this.selected.get() {
            row.add_css_class("selected");
        } else {
            row.remove_css_class("selected");
        }
        child = row.next_sibling();
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::completion_markup;

    #[test]
    fn completion_is_bold() {
        assert_eq!(
            completion_markup("hello world", "hel"),
            "hel<b>lo world</b>"
        );
        assert_eq!(completion_markup("Hello", "hel"), "Hel<b>lo</b>");
        assert_eq!(completion_markup("a & b", "x"), "<b>a &amp; b</b>");
        assert_eq!(completion_markup("héllo", "h"), "h<b>éllo</b>");
    }
}
