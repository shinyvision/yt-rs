//! The `signin` page: signs in the way a smart TV does. The user approves
//! the app by entering a short code at google.com/device.

use gtk::prelude::*;
use serde_json::Value;

use crate::app::app;
use crate::runtime;

pub fn build(_props: &Value) -> gtk::Widget {
    let status = adw::StatusPage::builder()
        .icon_name("avatar-default-symbolic")
        .title("Sign in to YouTube")
        .description("Your home feed, subscriptions and history come from your account. You approve this app the way you'd sign in a smart TV: by entering a code in your browser.")
        .build();
    status.set_widget_name("page-signin");

    let code = gtk::Label::new(None);
    code.add_css_class("signin-code");
    code.set_selectable(true);
    code.set_visible(false);
    let hint = gtk::Label::new(None);
    hint.add_css_class("dim");
    hint.set_wrap(true);
    hint.set_justify(gtk::Justification::Center);
    let start = gtk::Button::with_label("Sign in");
    start.add_css_class("suggested-action");
    start.add_css_class("pill-button");
    let open = gtk::Button::with_label("Open google.com/device");
    open.add_css_class("pill-button");
    open.set_visible(false);

    let column = gtk::Box::new(gtk::Orientation::Vertical, 18);
    column.set_halign(gtk::Align::Center);
    column.append(&code);
    column.append(&hint);
    column.append(&open);
    column.append(&start);
    status.set_child(Some(&column));

    open.connect_clicked(|button| {
        let url = button
            .tooltip_text()
            .map(|t| t.to_string())
            .unwrap_or_else(|| "https://www.google.com/device".into());
        gtk::UriLauncher::new(&url).launch(
            app().window().as_ref(),
            gtk::gio::Cancellable::NONE,
            |_| {},
        );
    });
    start.connect_clicked(sign_in_flow(&code, &hint, &open));
    status.upcast()
}

/// The sign-in flow, run when "Sign in" is clicked.
fn sign_in_flow(
    code: &gtk::Label,
    hint: &gtk::Label,
    open: &gtk::Button,
) -> impl Fn(&gtk::Button) + 'static {
    let (code, hint, open) = (code.downgrade(), hint.downgrade(), open.downgrade());
    move |start| {
        let (Some(code), Some(hint), Some(open)) = (code.upgrade(), hint.upgrade(), open.upgrade())
        else {
            return;
        };
        start.set_sensitive(false);
        hint.set_label("Asking Google for a code…");
        let start = start.downgrade();
        runtime::spawn_local(async move {
            let session = app().session.clone();
            let oauth = session.oauth().clone();
            let device =
                match runtime::background(async move { oauth.request_device_code().await }).await {
                    Ok(device) => device,
                    Err(e) => {
                        hint.set_label(&format!("Couldn't start signing in: {e}"));
                        if let Some(start) = start.upgrade() {
                            start.set_sensitive(true);
                        }
                        return;
                    }
                };
            code.set_label(&device.user_code);
            code.set_visible(true);
            hint.set_label(&format!(
                "Go to {} and enter this code. Then pick the account or channel you want.",
                device.verification_url
            ));
            open.set_tooltip_text(Some(&device.verification_url));
            open.set_visible(true);
            if let Some(start) = start.upgrade() {
                start.set_visible(false);
            }
            let oauth = session.oauth().clone();
            let result = runtime::background(async move {
                let token = oauth.wait_for_approval(&device).await?;
                session.sign_in(token).await
            })
            .await;
            match result {
                Ok(()) => {
                    let app = app();
                    app.toast("Signed in");
                    app.ext.events.emit("auth.signed_in", &Value::Null);
                    app.navigate("home", Value::Null, false);
                }
                Err(e) => {
                    code.set_visible(false);
                    open.set_visible(false);
                    hint.set_label(&format!("{e}. Try again."));
                    if let Some(start) = start.upgrade() {
                        start.set_visible(true);
                        start.set_sensitive(true);
                    }
                }
            }
        });
    }
}
