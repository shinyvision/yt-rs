//! The window layout: sidebar on the left, top bar above the pages.

use gtk::prelude::*;
use serde_json::Value;

use crate::app::app;

pub fn build(_props: &Value) -> gtk::Widget {
    let app = app();
    let sidebar = app.ext.components.build("sidebar", &Value::Null);
    let topbar = app.ext.components.build("topbar", &Value::Null);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.set_widget_name("content");
    content.append(&topbar);
    content.append(&app.page_host());

    let shell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    shell.set_widget_name("shell");
    shell.append(&sidebar);
    shell.append(&content);

    // In fullscreen only the page (the video) is shown.
    let (s, t) = (sidebar.downgrade(), topbar.downgrade());
    app.ext.events.on("window.fullscreen", move |payload| {
        let fullscreen = payload
            .get("fullscreen")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        for widget in [s.upgrade(), t.upgrade()].into_iter().flatten() {
            widget.set_visible(!fullscreen);
        }
    });
    shell.upcast()
}
