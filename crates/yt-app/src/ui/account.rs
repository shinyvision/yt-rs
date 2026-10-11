//! The `account_button` component: the signed-in user's picture in the top
//! bar, with a menu for their channel and signing out.

use adw::prelude::*;
use serde_json::{Value, json};
use yt_core::account::Account;

use super::{avatar, follow};
use crate::app::app;
use crate::social;

pub fn build(_props: &Value) -> gtk::Widget {
    let face = adw::Avatar::new(32, None, true);
    let button = gtk::MenuButton::builder()
        .child(&face)
        .css_classes(["flat", "circular", "account-button"])
        .tooltip_text("Your account")
        .build();
    button.set_valign(gtk::Align::Center);
    show(&button, &face, social::account().as_ref());

    let (weak, weak_face) = (button.downgrade(), face.downgrade());
    follow(&button, "account.loaded", move |payload| {
        let account: Option<Account> = payload
            .get("account")
            .and_then(|a| serde_json::from_value(a.clone()).ok());
        if let (Some(button), Some(face)) = (weak.upgrade(), weak_face.upgrade()) {
            show(&button, &face, account.as_ref());
        }
    });
    let (weak, weak_face) = (button.downgrade(), face.downgrade());
    follow(&button, "auth.signed_out", move |_| {
        if let (Some(button), Some(face)) = (weak.upgrade(), weak_face.upgrade()) {
            show(&button, &face, None);
        }
    });
    button.upcast()
}

fn show(button: &gtk::MenuButton, face: &adw::Avatar, account: Option<&Account>) {
    button.set_visible(account.is_some());
    let Some(account) = account else {
        button.set_popover(None::<&gtk::Popover>);
        return;
    };
    avatar::set(face, &account.name, account.photo.as_deref());
    button.set_popover(Some(&menu(account)));
}

fn menu(account: &Account) -> gtk::Popover {
    let name = gtk::Label::new(Some(&account.name));
    name.add_css_class("account-name");
    name.set_xalign(0.0);
    let names = gtk::Box::new(gtk::Orientation::Vertical, 2);
    names.set_valign(gtk::Align::Center);
    names.append(&name);
    if let Some(handle) = &account.handle {
        let label = gtk::Label::new(Some(handle));
        label.add_css_class("dim");
        label.set_xalign(0.0);
        names.append(&label);
    }
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    header.add_css_class("account-header");
    header.append(&avatar::new(40, &account.name, account.photo.as_deref()));
    header.append(&names);

    // The items go through the `account.menu` filter, like the other menus.
    let mut items = Vec::new();
    if let Some(handle) = &account.handle {
        items.push(json!({
            "label": "Your channel", "icon": "yt-person-symbolic",
            "command": "channel.open", "args": { "handle": handle },
        }));
    }
    items.push(json!({
        "label": "Settings", "icon": "yt-settings-symbolic",
        "command": "app.navigate", "args": { "page": "settings" },
    }));
    items.push(json!({
        "label": "Sign out", "icon": "yt-logout-symbolic", "command": "auth.sign_out",
        "confirm": {
            "title": "Sign out?",
            "body": "yt-rs will forget your sign-in. You can sign in again at any time.",
            "confirm": "Sign out",
        },
    }));
    let account_json = serde_json::to_value(account).unwrap_or_default();
    let items = app()
        .ext
        .filters
        .apply(
            "account.menu",
            json!({ "account": account_json, "items": items }),
        )
        .get("items")
        .cloned()
        .unwrap_or_else(|| json!([]));

    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    column.append(&header);
    column.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    column.append(
        &app()
            .ext
            .components
            .build("menu", &json!({ "items": items })),
    );
    let popover = gtk::Popover::builder()
        .has_arrow(false)
        .child(&column)
        .build();
    popover.add_css_class("yt-menu-popover");
    popover.add_css_class("account-menu");
    popover.set_halign(gtk::Align::End);
    popover
}
