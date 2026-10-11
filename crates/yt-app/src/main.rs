//! yt-rs: a fast native YouTube client for the desktop, moddable with Lua.

mod app;
mod channels;
mod commands;
mod ext;
mod images;
mod playback;
mod player;
mod plugins;
mod runtime;
mod search;
mod settings;
mod social;
mod tracker;
mod ui;

use adw::prelude::*;

fn main() -> gtk::glib::ExitCode {
    // GTK's OpenGL renderer: starts faster than its Vulkan one, and mpv
    // draws video with OpenGL, so frames need no conversion. GSK_RENDERER
    // still wins if set.
    if std::env::var_os("GSK_RENDERER").is_none() {
        // SAFETY: no other threads exist yet.
        unsafe { std::env::set_var("GSK_RENDERER", "gl") };
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("YT_RS_LOG")
                .unwrap_or_else(|_| "warn,yt_app=info,yt_core=info".into()),
        )
        .init();

    // One window per profile: a second launch focuses the running app, but a
    // separate profile (YT_RS_CONFIG_DIR) runs as its own instance.
    let flags = if std::env::var_os("YT_RS_CONFIG_DIR").is_some() {
        gtk::gio::ApplicationFlags::NON_UNIQUE
    } else {
        gtk::gio::ApplicationFlags::default()
    };
    // Another ID (and so window class) lets window-manager rules tell an
    // instance apart, e.g. a test run's.
    let app_id = std::env::var("YT_RS_APP_ID").unwrap_or_else(|_| app::APP_ID.to_owned());
    let application = adw::Application::builder()
        .application_id(app_id)
        .flags(flags)
        .build();
    application.connect_activate(|application| {
        if let Some(window) = application.active_window() {
            window.present();
            return;
        }
        if let Err(e) = app::App::start(application) {
            tracing::error!("{e:#}");
            eprintln!("yt-rs: {e:#}");
            application.quit();
        }
    });
    application.connect_shutdown(|_| app::shutdown());
    application.run()
}
