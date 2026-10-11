//! The top bar: menu button, logo, back and forward, the `search_box`
//! component in the middle with a slot after it, a slot at the end, and the
//! account button. Dragging it moves the window.

use gtk::prelude::*;
use serde_json::{Value, json};

use super::follow;
use super::logo::LogoMark;
use crate::app::app;

pub fn build(_props: &Value) -> gtk::Widget {
    let app = app();
    // A glyph rather than an icon, since icon themes draw "menu" differently.
    let menu = gtk::Button::with_label("☰");
    menu.add_css_class("icon-button");
    menu.set_tooltip_text(Some("Menu"));
    menu.connect_clicked(|_| run("sidebar.toggle", Value::Null));

    let mark = LogoMark::default();
    mark.set_valign(gtk::Align::Center);
    let name = gtk::Label::new(Some("YouTube"));
    name.add_css_class("logo");
    let logo_box = gtk::Box::new(gtk::Orientation::Horizontal, 3);
    logo_box.append(&mark);
    logo_box.append(&name);
    let logo = gtk::Button::builder()
        .child(&logo_box)
        .css_classes(["flat"])
        .build();
    logo.set_tooltip_text(Some("Home"));
    logo.connect_clicked(|_| run("app.navigate", json!({ "page": "home" })));

    let back = nav_button("go-previous-symbolic", "Back (Alt+←)", "app.back");
    let forward = nav_button("go-next-symbolic", "Forward (Alt+→)", "app.forward");
    let history = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    history.set_margin_start(8);
    history.append(&back);
    history.append(&forward);
    let (b, f) = (back.downgrade(), forward.downgrade());
    follow(&history, "page.changed", move |payload| {
        let can = |key: &str| payload.get(key).and_then(Value::as_bool).unwrap_or(false);
        if let (Some(back), Some(forward)) = (b.upgrade(), f.upgrade()) {
            back.set_sensitive(can("back"));
            forward.set_sensitive(can("forward"));
        }
    });

    let start = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    start.append(&menu);
    start.append(&logo);
    start.append(&history);
    start.append(
        &app.ext
            .slots
            .create("topbar.start", gtk::Orientation::Horizontal, 8),
    );

    let end = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    end.append(
        &app.ext
            .slots
            .create("topbar.end", gtk::Orientation::Horizontal, 8),
    );
    end.append(&app.ext.components.build("account_button", &Value::Null));
    end.append(&gtk::WindowControls::new(gtk::PackType::End));

    let center = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    center.set_margin_start(16);
    center.set_margin_end(16);
    center.append(&app.ext.components.build("search_box", &Value::Null));
    center.append(
        &app.ext
            .slots
            .create("topbar.center", gtk::Orientation::Horizontal, 8),
    );
    let bar = gtk::CenterBox::new();
    bar.add_css_class("topbar");
    bar.set_start_widget(Some(&start));
    bar.set_center_widget(Some(&center));
    bar.set_end_widget(Some(&end));

    let handle = gtk::WindowHandle::new();
    handle.set_widget_name("topbar");
    handle.set_child(Some(&bar));
    handle.upcast()
}

fn nav_button(icon: &str, tooltip: &str, command: &'static str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.add_css_class("icon-button");
    button.set_tooltip_text(Some(tooltip));
    button.set_sensitive(false);
    button.connect_clicked(move |_| run(command, Value::Null));
    button
}

fn run(command: &str, args: Value) {
    if let Err(e) = app().ext.commands.run(command, args) {
        app().toast(&e);
    }
}
