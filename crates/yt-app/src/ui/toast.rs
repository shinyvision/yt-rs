//! Toasts, as on youtube.com: a short message in the bottom-left corner,
//! light on the dark page, with at most one action ("Undo"). A new toast
//! takes the place of the one showing. The `toast` component builds one
//! from `{ message, action? = { label, command, args? }, seconds? }`.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use serde_json::Value;

use crate::app::app;

/// How long a toast stays: longer with an action, to have time for it.
const SHOWN_FOR: Duration = Duration::from_secs(4);
const WITH_ACTION_FOR: Duration = Duration::from_secs(7);

/// Shows toasts over its child (the window's content).
pub struct Toaster {
    overlay: gtk::Overlay,
    revealer: gtk::Revealer,
    /// Hides the toast showing; it clears itself when it fires.
    timer: Rc<RefCell<Option<glib::SourceId>>>,
}

impl Toaster {
    pub fn new() -> Self {
        let revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideUp)
            .transition_duration(200)
            .halign(gtk::Align::Start)
            .valign(gtk::Align::End)
            .build();
        revealer.set_widget_name("toast");
        revealer.add_css_class("toast-place");
        // Gone once hidden, so it takes no clicks.
        revealer.connect_child_revealed_notify(|r| {
            if !r.is_child_revealed() {
                r.set_visible(false);
            }
        });
        revealer.set_visible(false);
        let overlay = gtk::Overlay::new();
        overlay.add_overlay(&revealer);
        Self {
            overlay,
            revealer,
            timer: Rc::default(),
        }
    }

    pub fn widget(&self) -> &gtk::Overlay {
        &self.overlay
    }

    pub fn set_child(&self, child: &impl IsA<gtk::Widget>) {
        self.overlay.set_child(Some(child));
    }

    /// Shows a toast with the `toast` component's properties.
    pub fn show(&self, props: &Value) {
        let toast = app().ext.components.build("toast", props);
        self.revealer.set_child(Some(&toast));
        self.revealer.set_visible(true);
        self.revealer.set_reveal_child(true);
        let shown_for = props
            .get("seconds")
            .and_then(Value::as_f64)
            .map(|s| Duration::from_secs_f64(s.clamp(1.0, 60.0)))
            .unwrap_or(if props.get("action").is_some_and(Value::is_object) {
                WITH_ACTION_FOR
            } else {
                SHOWN_FOR
            });
        if let Some(timer) = self.timer.borrow_mut().take() {
            timer.remove();
        }
        let (revealer, slot) = (self.revealer.downgrade(), self.timer.clone());
        let timer = glib::timeout_add_local_once(shown_for, move || {
            slot.borrow_mut().take();
            if let Some(revealer) = revealer.upgrade() {
                revealer.set_reveal_child(false);
            }
        });
        *self.timer.borrow_mut() = Some(timer);
    }

    /// Hides the toast showing, if any.
    pub fn dismiss(&self) {
        if let Some(timer) = self.timer.borrow_mut().take() {
            timer.remove();
        }
        self.revealer.set_reveal_child(false);
    }
}

impl Default for Toaster {
    fn default() -> Self {
        Self::new()
    }
}

/// The `toast` component.
pub fn build(props: &Value) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 24);
    row.add_css_class("yt-toast");
    let message = gtk::Label::new(props.get("message").and_then(Value::as_str));
    message.set_wrap(true);
    message.set_max_width_chars(56);
    message.set_xalign(0.0);
    message.set_hexpand(true);
    row.append(&message);
    if let Some(action) = props.get("action").filter(|a| a.is_object()) {
        let label = action.get("label").and_then(Value::as_str).unwrap_or("OK");
        let button = gtk::Button::with_label(label);
        button.add_css_class("flat");
        button.add_css_class("toast-action");
        let command = action
            .get("command")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let args = action.get("args").cloned().unwrap_or(Value::Null);
        button.connect_clicked(move |_| {
            let app = app();
            app.dismiss_toast();
            if let Some(command) = &command
                && let Err(e) = app.ext.commands.run(command, args.clone())
            {
                app.toast(&e);
            }
        });
        row.append(&button);
    }
    row.upcast()
}
