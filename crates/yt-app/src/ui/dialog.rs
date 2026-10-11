//! Confirmation dialogs, styled like youtube.com's: a title, an optional
//! message, and text buttons at the bottom right, the action in blue.
//!
//! The content is the `confirm_dialog` component, so plugins can restyle or
//! replace it; a replacement needs buttons named `dialog-cancel` and
//! `dialog-confirm`. The `app.confirm` command shows one from Lua.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use serde_json::Value;

use super::find_widget;
use crate::app::app;

/// Asks before doing something; runs `on_confirm` if the user agrees.
///
/// `props`: `{ title, body?, confirm = "Delete", cancel? = "Cancel" }`.
pub fn confirm(parent: &impl IsA<gtk::Widget>, props: &Value, on_confirm: impl FnOnce() + 'static) {
    let content = app().ext.components.build("confirm_dialog", props);
    let dialog = adw::Dialog::builder()
        .title(text(props, "title").unwrap_or_default())
        .child(&content)
        .build();
    dialog.add_css_class("yt-dialog");

    let button = |name: &str| find_widget(&content, name).and_downcast::<gtk::Button>();
    if let Some(cancel) = button("dialog-cancel") {
        let weak = dialog.downgrade();
        cancel.connect_clicked(move |_| {
            if let Some(dialog) = weak.upgrade() {
                dialog.close();
            }
        });
        dialog.set_focus(Some(&cancel));
    }
    if let Some(confirm) = button("dialog-confirm") {
        let weak = dialog.downgrade();
        let on_confirm = Rc::new(Cell::new(Some(on_confirm)));
        confirm.connect_clicked(move |_| {
            if let Some(dialog) = weak.upgrade() {
                dialog.close();
            }
            if let Some(action) = on_confirm.take() {
                action();
            }
        });
    }
    dialog.present(Some(parent));
}

/// The `confirm_dialog` component: the dialog's content.
pub fn build(props: &Value) -> gtk::Widget {
    let title = gtk::Label::new(text(props, "title").as_deref());
    title.add_css_class("yt-dialog-title");
    title.set_xalign(0.0);
    title.set_wrap(true);
    title.set_max_width_chars(40);

    let cancel = gtk::Button::with_label(text(props, "cancel").as_deref().unwrap_or("Cancel"));
    cancel.set_widget_name("dialog-cancel");
    let confirm = gtk::Button::with_label(text(props, "confirm").as_deref().unwrap_or("OK"));
    confirm.set_widget_name("dialog-confirm");
    confirm.add_css_class("confirm");
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.add_css_class("yt-dialog-actions");
    actions.set_halign(gtk::Align::End);
    for button in [&cancel, &confirm] {
        button.add_css_class("yt-dialog-button");
        actions.append(button);
    }

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("yt-dialog-content");
    content.append(&title);
    if let Some(body) = text(props, "body") {
        let body = gtk::Label::new(Some(&body));
        body.add_css_class("yt-dialog-body");
        body.set_xalign(0.0);
        body.set_wrap(true);
        body.set_max_width_chars(40);
        content.append(&body);
    }
    content.append(&actions);
    content.upcast()
}

fn text(props: &Value, key: &str) -> Option<String> {
    props
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}
