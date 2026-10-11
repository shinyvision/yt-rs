//! The `subscribe_button` component. Every button for the same channel
//! follows the `channel.subscription` event, so they stay in sync.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use serde_json::{Value, json};
use yt_core::channel::Subscription;

use super::{attach, follow};
use crate::app::app;

struct SubscribeButton {
    /// The button itself, weakly: it owns this.
    button: gtk::glib::WeakRef<gtk::Button>,
    subscription: RefCell<Subscription>,
    channel_name: String,
    busy: Cell<bool>,
}

/// Props: `{ subscription: { channel_id, subscribed, ... }, channel_name }`.
pub fn build(props: &Value) -> gtk::Widget {
    let subscription: Subscription = props
        .get("subscription")
        .and_then(|s| serde_json::from_value(s.clone()).ok())
        .unwrap_or_default();
    let button = gtk::Button::new();
    button.add_css_class("subscribe-button");
    button.add_css_class("pill-button");
    let this = Rc::new(SubscribeButton {
        button: button.downgrade(),
        subscription: RefCell::new(subscription),
        channel_name: props
            .get("channel_name")
            .and_then(Value::as_str)
            .unwrap_or("this channel")
            .to_owned(),
        busy: Cell::new(false),
    });
    show(&this);
    attach(&button, this.clone());

    let weak = Rc::downgrade(&this);
    button.connect_clicked(move |_| {
        if let Some(this) = weak.upgrade() {
            clicked(&this);
        }
    });
    let weak = Rc::downgrade(&this);
    follow(&button, "channel.subscription", move |payload| {
        let Some(this) = weak.upgrade() else { return };
        if payload.get("channel_id").and_then(Value::as_str)
            == Some(this.subscription.borrow().channel_id.as_str())
        {
            this.subscription.borrow_mut().subscribed = payload
                .get("subscribed")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            this.busy.set(false);
            show(&this);
        }
    });
    let weak = Rc::downgrade(&this);
    follow(&button, "channel.subscription.failed", move |payload| {
        if let Some(this) = weak.upgrade()
            && payload.get("channel_id").and_then(Value::as_str)
                == Some(this.subscription.borrow().channel_id.as_str())
        {
            this.busy.set(false);
            show(&this);
        }
    });
    button.upcast()
}

fn show(this: &SubscribeButton) {
    let Some(button) = this.button.upgrade() else {
        return;
    };
    if this.subscription.borrow().subscribed {
        let content = adw::ButtonContent::builder()
            .icon_name("yt-bell-symbolic")
            .label("Subscribed")
            .build();
        button.set_child(Some(&content));
        button.add_css_class("subscribed");
    } else {
        button.set_label("Subscribe");
        button.remove_css_class("subscribed");
    }
    button.set_sensitive(!this.busy.get() && !this.subscription.borrow().channel_id.is_empty());
}

fn clicked(this: &Rc<SubscribeButton>) {
    let Some(button) = this.button.upgrade() else {
        return;
    };
    if this.subscription.borrow().subscribed {
        // Unsubscribing asks first, as on youtube.com.
        let weak = Rc::downgrade(this);
        super::dialog::confirm(
            &button,
            &json!({
                "title": format!("Unsubscribe from {}?", this.channel_name),
                "confirm": "Unsubscribe",
            }),
            move || {
                if let Some(this) = weak.upgrade() {
                    send(&this, "channel.unsubscribe");
                }
            },
        );
    } else {
        send(this, "channel.subscribe");
    }
}

fn send(this: &SubscribeButton, command: &str) {
    this.busy.set(true);
    show(this);
    let args = json!({ "subscription": *this.subscription.borrow() });
    match app().ext.commands.run(command, args) {
        // Signed out: it offered to sign in instead.
        Ok(answer) if answer.get("signed_in") == Some(&Value::Bool(false)) => {
            this.busy.set(false);
            show(this);
        }
        Ok(_) => {}
        Err(e) => {
            this.busy.set(false);
            show(this);
            app().toast(&e);
        }
    }
}
