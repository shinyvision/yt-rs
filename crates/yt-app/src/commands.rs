//! The built-in commands and default key bindings.
//!
//! Buttons, menus and keys in the built-in UI all run these commands, so a
//! plugin that replaces one changes every way of triggering it.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
use serde_json::{Value, json};
use yt_core::feed::Video;

use crate::app::{App, app};
use crate::ext::keymap::GLOBAL;
use crate::{playback, ui};

pub fn register(this: &Rc<App>) {
    let c = &this.ext.commands;

    c.register("app.navigate", |args| {
        let page = str_arg(&args, "page")?;
        app().navigate(page, args.get("args").cloned().unwrap_or_default(), true);
        Ok(Value::Null)
    });
    c.register("app.back", |_| Ok(json!(app().back())));
    c.register("app.forward", |_| Ok(json!(app().forward())));
    c.register("app.quit", |_| {
        app().gtk.quit();
        Ok(Value::Null)
    });
    // { title, body?, confirm, cancel?, command, args? }: asks, then runs
    // the command if the user agrees.
    c.register("app.confirm", |args| {
        let command = str_arg(&args, "command")?.to_owned();
        let command_args = args.get("args").cloned().unwrap_or_else(|| json!({}));
        let window = app().window().ok_or("the window isn't open yet")?;
        ui::dialog::confirm(&window, &args, move || {
            if let Err(e) = app().ext.commands.run(&command, command_args) {
                app().toast(&e);
            }
        });
        Ok(Value::Null)
    });
    c.register("app.toast", |args| {
        app().toast(str_arg(&args, "message")?);
        Ok(Value::Null)
    });
    c.register("app.plugins", |_| {
        Ok(json!(
            app()
                .plugins()
                .map(|p| p.loaded().to_vec())
                .unwrap_or_default()
        ))
    });
    c.register("app.screenshot", |args| {
        screenshot(str_arg(&args, "path")?).map(|_| Value::Null)
    });

    c.register("sidebar.toggle", |_| {
        ui::sidebar::toggle();
        Ok(Value::Null)
    });
    c.register("window.toggle_fullscreen", |_| {
        let app = app();
        let on = app.window().is_some_and(|w| !w.is_fullscreen());
        app.set_fullscreen(on);
        Ok(json!(on))
    });
    c.register("window.fullscreen", |args| {
        app().set_fullscreen(
            args.get("fullscreen")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        );
        Ok(Value::Null)
    });
    c.register("feed.refresh", |_| {
        app().ext.events.emit("feed.refresh", &Value::Null);
        Ok(Value::Null)
    });

    c.register("video.play", |args| {
        playback::play(video_arg(&args)?, args.get("start").and_then(Value::as_f64));
        Ok(Value::Null)
    });
    c.register("video.prefetch", |args| {
        app().streams.prefetch(&video_arg(&args)?.id);
        Ok(Value::Null)
    });
    c.register("video.copy_link", |args| {
        let url = video_arg(&args)?.url();
        if let Some(display) = gdk::Display::default() {
            display.clipboard().set_text(&url);
            app().toast("Link copied");
        }
        Ok(json!(url))
    });
    // { text, toast? }
    c.register("app.copy_text", |args| {
        let text = str_arg(&args, "text")?;
        if let Some(display) = gdk::Display::default() {
            display.clipboard().set_text(text);
            if let Some(toast) = args.get("toast").and_then(Value::as_str) {
                app().toast(toast);
            }
        }
        Ok(Value::Null)
    });
    c.register("video.open_in_browser", |args| {
        let url = video_arg(&args)?.url();
        gtk::UriLauncher::new(&url).launch(
            app().window().as_ref(),
            gtk::gio::Cancellable::NONE,
            |_| {},
        );
        Ok(Value::Null)
    });

    c.register("player.toggle_pause", |_| {
        app().player.toggle_pause();
        Ok(Value::Null)
    });
    c.register("player.play", |_| {
        app().player.set_paused(false);
        Ok(Value::Null)
    });
    c.register("player.pause", |_| {
        app().player.set_paused(true);
        Ok(Value::Null)
    });
    c.register("player.seek", |args| {
        let position = num_arg(&args, "position")?;
        app().player.seek(
            position,
            args.get("exact").and_then(Value::as_bool).unwrap_or(true),
        );
        Ok(Value::Null)
    });
    c.register("player.seek_by", |args| {
        let seconds = num_arg(&args, "seconds")?;
        app().player.seek_by(seconds);
        let icon = if seconds < 0.0 {
            "yt-rewind-symbolic"
        } else {
            "yt-forward-symbolic"
        };
        bezel(icon, &format!("{} seconds", seconds.abs().round()));
        Ok(Value::Null)
    });
    c.register("player.set_volume", |args| {
        app().player.set_volume(num_arg(&args, "volume")?);
        Ok(Value::Null)
    });
    c.register("player.change_volume", |args| {
        let player = &app().player;
        let volume = (player.state().volume + num_arg(&args, "delta")?).clamp(0.0, 100.0);
        player.set_volume(volume);
        player.set_muted(false);
        bezel(
            ui::controls::volume_icon(volume, false),
            &format!("{volume:.0}%"),
        );
        Ok(Value::Null)
    });
    c.register("player.toggle_mute", |_| {
        let player = &app().player;
        let state = player.state();
        player.set_muted(!state.muted);
        app().ext.events.emit(
            "player.bezel",
            &json!({ "icon": ui::controls::volume_icon(state.volume, !state.muted) }),
        );
        Ok(Value::Null)
    });
    // Plays any URL mpv can open, with an optional separate audio URL.
    // Passing `video_id` lets the app look that video's streams up again if
    // a stream is refused.
    c.register("player.open", |args| {
        let url = str_arg(&args, "url")?.to_owned();
        let stream = |url: String| yt_core::streams::Stream {
            url,
            ..Default::default()
        };
        let streams = yt_core::streams::StreamSet {
            title: args.get("title").and_then(Value::as_str).map(str::to_owned),
            video: stream(url.clone()),
            audio: args
                .get("audio_url")
                .and_then(Value::as_str)
                .map(|a| stream(a.to_owned())),
            ..Default::default()
        };
        let id = args
            .get("video_id")
            .and_then(Value::as_str)
            .unwrap_or(&url)
            .to_owned();
        let start = args.get("start").and_then(Value::as_f64).unwrap_or(0.0);
        app()
            .player
            .load(&id, &streams, start)
            .map_err(|e| e.to_string())?;
        Ok(Value::Null)
    });
    c.register("player.set_speed", |args| {
        app().player.set_speed(num_arg(&args, "speed")?);
        Ok(Value::Null)
    });
    // { delta }: YouTube's steps of 0.25.
    c.register("player.change_speed", |args| {
        let player = &app().player;
        let speed = (player.state().speed + num_arg(&args, "delta")?).clamp(0.25, 4.0);
        player.set_speed(speed);
        bezel("yt-speed-symbolic", &format!("{speed}x"));
        Ok(json!(speed))
    });
    // { index }: one of the streams' `qualities`, or null for automatic.
    c.register("player.set_quality", |args| {
        let index = args
            .get("index")
            .and_then(Value::as_u64)
            .map(|i| i as usize);
        app().player.set_quality(index)?;
        Ok(Value::Null)
    });
    c.register("player.qualities", |_| {
        let streams = app().player.streams();
        Ok(json!(streams.map(|s| {
            s.qualities
                .iter()
                .map(|q| q.quality_label())
                .collect::<Vec<_>>()
        })))
    });
    // { lang }, or { lang = null } to turn captions off.
    c.register("player.set_captions", |args| {
        let lang = args.get("lang").and_then(Value::as_str).map(str::to_owned);
        playback::set_captions(lang)?;
        Ok(Value::Null)
    });
    c.register("player.toggle_captions", |_| {
        let app = app();
        if app.player.state().captions.is_some() {
            bezel("yt-cc-symbolic", "Subtitles off");
            return playback::set_captions(None).map(|()| json!(false));
        }
        let captions = app.player.streams().map(|s| s.captions).unwrap_or_default();
        // The language chosen before, else English, else the first track.
        let wanted = app.settings.get().captions.unwrap_or_else(|| "en".into());
        let lang = playback::best_caption(&captions, &wanted)
            .or(captions.first())
            .map(|c| c.lang.clone())
            .ok_or("this video has no captions")?;
        bezel("yt-cc-symbolic", "Subtitles on");
        playback::set_captions(Some(lang)).map(|()| json!(true))
    });
    c.register("player.set_looping", |args| {
        let looping = args.get("looping").and_then(Value::as_bool).unwrap_or(true);
        app().player.set_looping(looping);
        Ok(Value::Null)
    });
    c.register("player.toggle_loop", |_| {
        let player = &app().player;
        player.set_looping(!player.state().looping);
        Ok(json!(player.state().looping))
    });
    c.register("player.set_stable_volume", |args| {
        let on = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
        app().player.set_stable_volume(on);
        Ok(Value::Null)
    });
    // { minutes }, { minutes = "end" } for the end of the video, or
    // { minutes = null } to turn it off.
    c.register("player.sleep_timer", |args| {
        match args.get("minutes") {
            Some(Value::String(s)) if s == "end" => playback::sleep_at_end(),
            Some(Value::Number(n)) => playback::set_sleep_timer(n.as_f64()),
            _ => playback::set_sleep_timer(None),
        }
        Ok(Value::Null)
    });
    c.register("player.frame_step", |args| {
        let forward = args.get("forward").and_then(Value::as_bool).unwrap_or(true);
        app().player.frame_step(forward);
        Ok(Value::Null)
    });
    // { percent }: 0 to 100.
    // Of the video, or of the part of a live stream that can be watched.
    c.register("player.seek_percent", |args| {
        let player = &app().player;
        let percent = num_arg(&args, "percent")?.clamp(0.0, 100.0);
        let (start, end) = player.timeline();
        player.seek(start + (end - start) * percent / 100.0, true);
        Ok(Value::Null)
    });
    c.register("player.seek_end", |_| {
        let player = &app().player;
        if player.state().live.is_some() {
            player.go_live();
        } else {
            let (_, end) = player.timeline();
            player.seek((end - 1.0).max(0.0), true);
        }
        Ok(Value::Null)
    });
    c.register("player.go_live", |_| {
        app().player.go_live();
        Ok(Value::Null)
    });
    c.register("player.next", |_| playback::next().map(|()| Value::Null));
    c.register("player.cancel_up_next", |_| {
        playback::cancel_up_next();
        Ok(Value::Null)
    });
    c.register("player.set_autoplay", |args| {
        let on = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
        app().settings.update(|s| s.autoplay = on);
        app()
            .ext
            .events
            .emit("player.autoplay", &json!({ "enabled": on }));
        Ok(Value::Null)
    });
    c.register("player.stats", |_| {
        Ok(app()
            .player
            .stats()
            .into_iter()
            .map(|(k, v)| json!([k, v]))
            .collect())
    });
    // The settings panel's entries; see `ui::player_settings`.
    c.register("player.settings", |_| Ok(ui::player_settings::entries()));
    c.register("player.toggle_stats", |_| {
        app().ext.events.emit("player.toggle_stats", &Value::Null);
        Ok(Value::Null)
    });
    // { at_time }: the link starts at the current moment.
    c.register("player.copy_url", |args| {
        let state = app().player.state();
        let id = state.video_id.ok_or("no video is playing")?;
        let mut url = format!("https://youtu.be/{id}");
        if args
            .get("at_time")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            url.push_str(&format!("?t={}", state.position as u64));
        }
        if let Some(display) = gdk::Display::default() {
            display.clipboard().set_text(&url);
            app().toast("Link copied");
        }
        Ok(json!(url))
    });
    c.register("watch.toggle_theater", |_| {
        let app = app();
        let theater = !app.settings.get().theater;
        app.settings.update(|s| s.theater = theater);
        app.ext
            .events
            .emit("watch.theater", &json!({ "theater": theater }));
        Ok(json!(theater))
    });
    c.register("settings.get", |_| {
        Ok(serde_json::to_value(app().settings.get()).unwrap_or_default())
    });
    c.register("player.stop", |_| {
        playback::stop();
        Ok(Value::Null)
    });

    c.register("watch.set_tracking", |args| {
        let enabled = args
            .get("enabled")
            .and_then(Value::as_bool)
            .ok_or("missing boolean argument \"enabled\"")?;
        app().tracker.set_enabled(enabled);
        app()
            .ext
            .events
            .emit("watch.tracking", &json!({ "enabled": enabled }));
        Ok(json!(enabled))
    });
    c.register("auth.sign_out", |_| {
        crate::runtime::spawn_local(async {
            let app = app();
            let session = app.session.clone();
            match crate::runtime::background(async move { session.sign_out().await }).await {
                Ok(()) => {
                    app.ext.events.emit("auth.signed_out", &Value::Null);
                    app.navigate("signin", Value::Null, false);
                }
                Err(e) => app.toast(&format!("Could not sign out: {e}")),
            }
        });
        Ok(Value::Null)
    });

    default_keys(this);
}

/// The default key bindings. Plugins can change or clear them.
fn default_keys(app: &App) {
    let keys = &app.ext.keymap;
    let bind = |key: &str, scope: &str, command: &str, args: Value| {
        keys.set(key, scope, command, args)
            .expect("built-in key bindings are valid");
    };
    bind("Escape", GLOBAL, "app.back", Value::Null);
    bind("alt+Left", GLOBAL, "app.back", Value::Null);
    bind("alt+Right", GLOBAL, "app.forward", Value::Null);
    bind("Back", GLOBAL, "app.back", Value::Null);
    bind("Forward", GLOBAL, "app.forward", Value::Null);
    bind("F11", GLOBAL, "window.toggle_fullscreen", Value::Null);
    // As on youtube.com.
    bind("slash", GLOBAL, "search.focus", Value::Null);
    bind("ctrl+r", GLOBAL, "feed.refresh", Value::Null);
    bind("F5", GLOBAL, "feed.refresh", Value::Null);
    bind("ctrl+q", GLOBAL, "app.quit", Value::Null);

    for key in ["space", "k"] {
        bind(key, "watch", "player.toggle_pause", Value::Null);
    }
    bind("f", "watch", "window.toggle_fullscreen", Value::Null);
    bind("m", "watch", "player.toggle_mute", Value::Null);
    bind("Left", "watch", "player.seek_by", json!({ "seconds": -5 }));
    bind("Right", "watch", "player.seek_by", json!({ "seconds": 5 }));
    bind("j", "watch", "player.seek_by", json!({ "seconds": -10 }));
    bind("l", "watch", "player.seek_by", json!({ "seconds": 10 }));
    bind("Up", "watch", "player.change_volume", json!({ "delta": 5 }));
    bind(
        "Down",
        "watch",
        "player.change_volume",
        json!({ "delta": -5 }),
    );
    // As on youtube.com.
    bind("c", "watch", "player.toggle_captions", Value::Null);
    bind(
        "greater",
        "watch",
        "player.change_speed",
        json!({ "delta": 0.25 }),
    );
    bind(
        "less",
        "watch",
        "player.change_speed",
        json!({ "delta": -0.25 }),
    );
    bind(
        "period",
        "watch",
        "player.frame_step",
        json!({ "forward": true }),
    );
    bind(
        "comma",
        "watch",
        "player.frame_step",
        json!({ "forward": false }),
    );
    for digit in 0..10 {
        let args = json!({ "percent": digit * 10 });
        bind(&digit.to_string(), "watch", "player.seek_percent", args);
    }
    bind(
        "Home",
        "watch",
        "player.seek_percent",
        json!({ "percent": 0 }),
    );
    bind("End", "watch", "player.seek_end", Value::Null);
    bind("t", "watch", "watch.toggle_theater", Value::Null);
    bind("shift+n", "watch", "player.next", Value::Null);
}

/// Runs key bindings for key presses anywhere in the window, except plain
/// typing (and Escape) in text fields.
pub fn key_controller() -> gtk::EventControllerKey {
    let controller = gtk::EventControllerKey::new();
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    controller.connect_key_pressed(|controller, key, _code, modifiers| {
        let app = app();
        let page = app.current_page().unwrap_or_default();
        let Some(binding) = app.ext.keymap.lookup(key, modifiers, &page) else {
            return glib::Propagation::Proceed;
        };
        let typing = controller
            .widget()
            .and_then(|w| w.root())
            .and_then(|r| r.focus())
            .is_some_and(|f| f.is::<gtk::Text>() || f.is::<gtk::TextView>());
        let has_command_modifier =
            modifiers.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK);
        // Typing goes to the text field, Escape included (it cancels a
        // comment, say); only keys with Ctrl or Alt run commands there.
        if typing && !has_command_modifier {
            return glib::Propagation::Proceed;
        }
        if let Err(e) = app.ext.commands.run(&binding.command, binding.args) {
            app.toast(&e);
        }
        glib::Propagation::Stop
    });
    controller
}

/// Confirms an action in the player's bezel.
fn bezel(icon: &str, text: &str) {
    app()
        .ext
        .events
        .emit("player.bezel", &json!({ "icon": icon, "text": text }));
}

/// The mouse's back and forward buttons run `app.back` and `app.forward`,
/// anywhere in the window.
pub fn mouse_controller() -> gtk::GestureClick {
    // GDK numbers the side buttons 8 (back) and 9 (forward).
    const BACK: u32 = 8;
    const FORWARD: u32 = 9;
    let click = gtk::GestureClick::new();
    click.set_button(0);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    click.connect_pressed(|gesture, _, _, _| {
        let command = match gesture.current_button() {
            BACK => "app.back",
            FORWARD => "app.forward",
            _ => return,
        };
        gesture.set_state(gtk::EventSequenceState::Claimed);
        if let Err(e) = app().ext.commands.run(command, Value::Null) {
            app().toast(&e);
        }
    });
    click
}

/// Saves a PNG of the window's contents once the next frame is painted
/// (only then is the whole widget tree's rendering available, e.g. while a
/// video plays). Emits `app.screenshot` with `{ path, ok, error }`.
pub fn screenshot(path: &str) -> Result<(), String> {
    let window = app().window().ok_or("no window")?;
    let clock = window.frame_clock().ok_or("the window isn't shown")?;
    let path = path.to_owned();
    let handler: Rc<std::cell::RefCell<Option<glib::SignalHandlerId>>> = Rc::default();
    let h = handler.clone();
    let id = clock.connect_after_paint(move |clock| {
        if let Some(id) = h.borrow_mut().take() {
            clock.disconnect(id);
        }
        let result = capture(&path);
        if let Err(e) = &result {
            tracing::warn!("screenshot failed: {e}");
        }
        app().ext.events.emit(
            "app.screenshot",
            &json!({ "path": path, "ok": result.is_ok(), "error": result.err() }),
        );
    });
    *handler.borrow_mut() = Some(id);
    window.queue_draw();
    Ok(())
}

fn capture(path: &str) -> Result<(), String> {
    let window = app().window().ok_or("no window")?;
    let (width, height) = (window.width() as f32, window.height() as f32);
    let paintable = gtk::WidgetPaintable::new(Some(&window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, width as f64, height as f64);
    let node = snapshot.to_node().ok_or("nothing to draw")?;
    let renderer = window.renderer().ok_or("the window has no renderer")?;
    let viewport = gtk::graphene::Rect::new(0.0, 0.0, width, height);
    renderer
        .render_texture(&node, Some(&viewport))
        .save_to_png(path)
        .map_err(|e| e.to_string())
}

pub fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string argument {key:?}"))
}

pub fn num_arg(args: &Value, key: &str) -> Result<f64, String> {
    args.get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("missing number argument {key:?}"))
}

/// The `video` argument: a video object, or just `{ "id": ... }`.
pub fn video_arg(args: &Value) -> Result<Video, String> {
    let video = args.get("video").unwrap_or(args);
    let mut video: Video = match video {
        Value::String(id) => Video {
            id: id.clone(),
            ..Video::default()
        },
        other => {
            serde_json::from_value(other.clone()).map_err(|e| format!("invalid video: {e}"))?
        }
    };
    if video.id.is_empty() {
        return Err("the video has no id".into());
    }
    if video.title.is_empty() {
        video.title = "YouTube video".into();
    }
    Ok(video)
}
