//! Menus, styled like youtube.com's: a dark list of items, each an icon and
//! a label, in a popover without an arrow.
//!
//! The `menu` component builds the list from
//! `{ items = [{ label, icon?, command, args?, confirm?, selected? }] }`.
//! Choosing an item runs its command; with `confirm` (the properties of a
//! [confirmation dialog](super::dialog)) it asks first.

use gtk::prelude::*;
use serde_json::{Value, json};

use super::dialog;
use crate::app::app;

/// The `menu` component.
pub fn build(props: &Value) -> gtk::Widget {
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.add_css_class("yt-menu");
    for item in props
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        list.append(&item_button(item));
    }
    list.upcast()
}

/// A popover showing the `menu` component with these items.
pub fn popover(items: Value) -> gtk::Popover {
    let popover = gtk::Popover::builder()
        .has_arrow(false)
        .child(
            &app()
                .ext
                .components
                .build("menu", &json!({ "items": items })),
        )
        .build();
    popover.add_css_class("yt-menu-popover");
    popover
}

/// Shows a menu from `button`, aligned with its right edge, with the items
/// `items` returns at that moment (so filters and state are current).
pub fn attach(button: &gtk::MenuButton, items: impl Fn() -> Value + 'static) {
    button.set_create_popup_func(move |button| {
        let popover = popover(items());
        popover.set_halign(gtk::Align::End);
        button.set_popover(Some(&popover));
    });
}

fn item_button(item: &Value) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    if let Some(icon) = item.get("icon").and_then(Value::as_str) {
        content.append(&gtk::Image::from_icon_name(icon));
    }
    let label = gtk::Label::new(item.get("label").and_then(Value::as_str));
    label.set_xalign(0.0);
    content.append(&label);

    let button = gtk::Button::builder()
        .child(&content)
        .css_classes(["flat", "yt-menu-item"])
        .build();
    if item
        .get("selected")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        button.add_css_class("selected");
    }
    let item = item.clone();
    button.connect_clicked(move |button| choose(button, &item));
    button
}

fn choose(button: &gtk::Button, item: &Value) {
    let command = item
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let args = item.get("args").cloned().unwrap_or_else(|| json!({}));
    let run = move || {
        if let Err(e) = app().ext.commands.run(&command, args) {
            app().toast(&e);
        }
    };
    // Close the menu first; a dialog then belongs to what opened it.
    let popover = button
        .ancestor(gtk::Popover::static_type())
        .and_downcast::<gtk::Popover>();
    let owner = popover.as_ref().and_then(|p| p.parent());
    if let Some(popover) = &popover {
        popover.popdown();
    }
    match item.get("confirm").filter(|c| c.is_object()) {
        Some(confirm) => {
            let parent = owner.unwrap_or_else(|| button.clone().upcast());
            dialog::confirm(&parent, confirm, run);
        }
        None => run(),
    }
}
