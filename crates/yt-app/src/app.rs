//! The application: its services, its extension registries and navigation.
//!
//! There is one [`App`] per process, reachable from anywhere on the main
//! thread with [`app()`]. Startup order matters for moddability: the
//! built-in components, commands and keys are registered first, then the
//! plugins run (and can replace any of them), and only then is the window
//! built from the registries.

use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gtk::glib;
use serde_json::{Value, json};
use yt_core::auth::{OAuth, Session};
use yt_core::innertube::InnerTube;
use yt_core::streams::{Resolver, ResolverOptions};

use crate::channels::ChannelDirectory;
use crate::ext::Extensions;
use crate::images::Images;
use crate::player::Player;
use crate::plugins::PluginHost;
use crate::search::SearchHistory;
use crate::settings::SettingsStore;
use crate::tracker::WatchTracker;
use crate::{commands, runtime, search, social, ui};

pub const APP_ID: &str = "dev.ytrs.YtRs";
/// Pages remembered for going back.
const MAX_HISTORY: usize = 100;

thread_local! {
    static APP: OnceCell<Rc<App>> = const { OnceCell::new() };
}

/// The running app. Panics if called before [`App::start`] or off the main
/// thread.
pub fn app() -> Rc<App> {
    APP.with(|a| a.get().expect("the app has not started").clone())
}

/// The running app, or `None` before it starts or while it is torn down.
pub fn try_app() -> Option<Rc<App>> {
    APP.try_with(|a| a.get().cloned()).ok().flatten()
}

pub struct App {
    pub gtk: adw::Application,
    pub ext: Extensions,
    pub http: reqwest::Client,
    pub session: Arc<Session>,
    pub youtube: Arc<InnerTube>,
    pub streams: Arc<Resolver>,
    pub images: Rc<Images>,
    pub player: Rc<Player>,
    pub tracker: Rc<WatchTracker>,
    pub channels: Rc<ChannelDirectory>,
    pub settings: Rc<SettingsStore>,
    pub search_history: Rc<SearchHistory>,
    window: OnceCell<adw::ApplicationWindow>,
    toasts: crate::ui::toast::Toaster,
    page_host: gtk::Stack,
    nav: RefCell<NavState>,
    plugins: OnceCell<PluginHost>,
}

#[derive(Default)]
struct NavState {
    /// The page shown and its arguments.
    current: Option<(String, Value)>,
    /// Where Back goes, most recent last.
    history: Vec<(String, Value)>,
    /// Where Forward goes after going back, most recent last.
    forward: Vec<(String, Value)>,
    kept: HashMap<String, gtk::Widget>,
}

impl App {
    pub fn start(gtk_app: &adw::Application) -> anyhow::Result<()> {
        let t = std::time::Instant::now();
        let step = |name: &str| tracing::debug!(target: "startup", "{name}: {:?}", t.elapsed());
        let http = yt_core::http_client();
        let session = Arc::new(Session::load(
            OAuth::new(http.clone()),
            yt_core::paths::token_file(),
        ));
        let youtube = Arc::new(InnerTube::new(http.clone(), session.clone()));
        // YT_RS_DRY_RUN=1: nothing that would change the account is sent.
        if std::env::var_os("YT_RS_DRY_RUN").is_some_and(|v| v != "0") {
            tracing::warn!("dry run: changes to the account are not sent");
            youtube.set_dry_run(true);
        }
        let streams = Arc::new(Resolver::new(
            youtube.clone(),
            ResolverOptions::default(),
            runtime::handle(),
        ));
        let images = Images::new(http.clone(), yt_core::paths::cache_dir().join("images"));
        step("services");
        let player = Player::new(http.clone(), streams.clone())
            .map_err(|e| anyhow::anyhow!("could not start mpv: {e}"))?;
        step("mpv");
        let tracker = WatchTracker::new(youtube.clone(), http.clone());

        let page_host = gtk::Stack::new();
        page_host.set_transition_type(gtk::StackTransitionType::Crossfade);
        page_host.set_transition_duration(120);
        page_host.set_widget_name("page-host");
        page_host.set_vexpand(true);
        page_host.set_hexpand(true);

        let app = Rc::new(App {
            gtk: gtk_app.clone(),
            ext: Extensions::default(),
            http,
            session,
            youtube,
            streams,
            images,
            player,
            tracker,
            channels: ChannelDirectory::load(),
            settings: SettingsStore::load(),
            search_history: SearchHistory::load(),
            window: OnceCell::new(),
            toasts: crate::ui::toast::Toaster::new(),
            page_host,
            nav: RefCell::default(),
            plugins: OnceCell::new(),
        });
        APP.with(|a| a.set(app.clone()))
            .map_err(|_| anyhow::anyhow!("the app was started twice"))?;

        step("app");
        adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
        app.ext.theme.add_css(include_str!("ui/style.css"));
        if let Some(display) = gtk::gdk::Display::default() {
            ui::icons::install(&display);
        }
        ui::cursor::install();
        commands::register(&app);
        social::register(&app);
        crate::playlists::register(&app);
        crate::downloads::register(&app);
        search::register(&app);
        ui::register(&app);
        step("theme, commands, components");
        app.tracker.attach(&app.player);
        crate::playback::init();
        ui::player_dock::init();
        crate::media_controls::init();
        app.forward_player_events();

        // Plugins run before the window exists, so they can replace any
        // built-in component, command or key binding.
        let _ = app.plugins.set(PluginHost::start(&app));
        step("plugins");

        app.build_window();
        step("window");
        app.ext.events.emit("app.ready", &Value::Null);

        // Home either way, as youtube.com: signed out, YouTube's own
        // picks, and "Sign in" at the top.
        if crate::social::signed_in() {
            app.load_signed_in();
        }
        app.ext
            .events
            .on("auth.signed_in", |_| crate::app::app().load_signed_in());
        app.navigate("home", Value::Null, false);
        step("first page");
        // Once the first frame is up, get the challenge solver ready for the
        // first click.
        let streams = app.streams.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
            streams.warm_up()
        });
        Ok(())
    }

    fn build_window(self: &Rc<Self>) {
        let window = adw::ApplicationWindow::builder()
            .application(&self.gtk)
            .title("yt-rs")
            .default_width(1440)
            .default_height(920)
            .build();
        window.add_css_class("yt-window");
        window.set_widget_name("main-window");
        self.toasts
            .set_child(&self.ext.components.build("shell", &Value::Null));
        window.set_content(Some(self.toasts.widget()));
        window.add_controller(crate::commands::key_controller());
        window.add_controller(crate::commands::mouse_controller());
        window.connect_fullscreened_notify(|w| {
            app().ext.events.emit(
                "window.fullscreen",
                &json!({ "fullscreen": w.is_fullscreen() }),
            );
        });
        let _ = self.window.set(window.clone());
        window.present();
    }

    /// What the app shows for the signed-in user: their account (name and
    /// picture) and their subscriptions (for channel avatars).
    fn load_signed_in(self: &Rc<Self>) {
        social::load_account();
        self.channels.refresh();
    }

    pub fn window(&self) -> Option<adw::ApplicationWindow> {
        self.window.get().cloned()
    }

    pub fn plugins(&self) -> Option<&PluginHost> {
        self.plugins.get()
    }

    /// The container pages are shown in. A replacement `shell` component
    /// must put this somewhere in its layout.
    pub fn page_host(&self) -> gtk::Stack {
        self.page_host.clone()
    }

    /// Shows a message in a toast.
    pub fn toast(&self, message: &str) {
        self.toasts.show(&serde_json::json!({ "message": message }));
    }

    /// Shows a toast with the `toast` component's properties: a message,
    /// and maybe an action (`{ label, command, args }`) and how long.
    pub fn toast_with(&self, props: &Value) {
        self.toasts.show(props);
    }

    pub fn dismiss_toast(&self) {
        self.toasts.dismiss();
    }

    pub fn current_page(&self) -> Option<String> {
        self.nav.borrow().current.as_ref().map(|(p, _)| p.clone())
    }

    /// The arguments the current page was shown with.
    pub fn current_args(&self) -> Option<Value> {
        self.nav.borrow().current.as_ref().map(|(_, a)| a.clone())
    }

    /// Shows a page. `remember` puts the current page on the back stack
    /// (unless it is this page with these arguments), which also ends any
    /// way forward. Going back to a page shows it with the arguments it had,
    /// so the channel page shows that channel again and the watch page plays
    /// that video again.
    pub fn navigate(&self, page: &str, args: Value, remember: bool) {
        let Some(info) = self.ext.pages.get(page) else {
            self.toast(&format!("Unknown page: {page}"));
            return;
        };
        let args = if args.is_null() {
            info.args.clone()
        } else {
            args
        };
        let component = info.component();

        let widget = {
            let mut nav = self.nav.borrow_mut();
            let entry = (page.to_owned(), args.clone());
            if remember
                && let Some(current) = nav.current.take()
                && current != entry
            {
                nav.history.push(current);
                if nav.history.len() > MAX_HISTORY {
                    nav.history.remove(0);
                }
                nav.forward.clear();
            }
            nav.current = Some(entry);
            nav.kept.get(page).cloned()
        };
        let widget = match widget {
            Some(w) if self.ext.components.update(component, &w, &args) => w,
            existing => {
                if let Some(old) = existing {
                    self.page_host.remove(&old);
                }
                let w = self.ext.components.build(component, &args);
                self.page_host.add_named(&w, Some(page));
                if info.keep {
                    self.nav
                        .borrow_mut()
                        .kept
                        .insert(page.to_owned(), w.clone());
                }
                w
            }
        };
        self.page_host.set_visible_child(&widget);
        // Pages that aren't kept are dropped once they're out of view.
        self.drop_hidden_pages(&widget);
        runtime::release_memory_soon();
        let (back, forward) = (self.can_go_back(), self.can_go_forward());
        self.ext.events.emit(
            "page.changed",
            &json!({ "page": page, "args": args, "back": back, "forward": forward }),
        );
    }

    fn drop_hidden_pages(&self, visible: &gtk::Widget) {
        // After the crossfade, which shows the page being left fading out:
        // without it, the new page fades in from nothing.
        if self.page_host.is_transition_running() {
            let handler: Rc<RefCell<Option<glib::SignalHandlerId>>> = Rc::default();
            let slot = handler.clone();
            let id = self
                .page_host
                .connect_transition_running_notify(move |host| {
                    if host.is_transition_running() {
                        return;
                    }
                    if let Some(id) = slot.borrow_mut().take() {
                        host.disconnect(id);
                    }
                    if let Some(current) = host.visible_child() {
                        app().drop_hidden_pages(&current);
                    }
                });
            *handler.borrow_mut() = Some(id);
            return;
        }
        let kept: Vec<gtk::Widget> = self.nav.borrow().kept.values().cloned().collect();
        let mut child = self.page_host.first_child();
        while let Some(w) = child {
            child = w.next_sibling();
            if &w != visible && !kept.contains(&w) {
                self.page_host.remove(&w);
            }
        }
    }

    pub fn back(&self) -> bool {
        self.step(false)
    }

    pub fn forward(&self) -> bool {
        self.step(true)
    }

    pub fn can_go_back(&self) -> bool {
        !self.nav.borrow().history.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.nav.borrow().forward.is_empty()
    }

    /// Goes one page back, or forward again.
    fn step(&self, forward: bool) -> bool {
        let target = {
            let mut nav = self.nav.borrow_mut();
            let target = if forward {
                nav.forward.pop()
            } else {
                nav.history.pop()
            };
            let Some(target) = target else {
                return false;
            };
            if let Some(current) = nav.current.clone() {
                if forward {
                    nav.history.push(current);
                } else {
                    nav.forward.push(current);
                }
            }
            target
        };
        self.navigate(&target.0, target.1, false);
        true
    }

    pub fn set_fullscreen(&self, fullscreen: bool) {
        if let Some(window) = self.window() {
            window.set_fullscreened(fullscreen);
        }
    }

    /// Re-emits player changes on the event bus as `player.*` events, for
    /// plugins. Position updates are limited to four per second.
    fn forward_player_events(&self) {
        use crate::player::PlayerEvent as E;
        let last_position = std::cell::Cell::new(0.0f64);
        self.player.connect(move |event, state| {
            let events = &app().ext.events;
            let (name, payload) = match event {
                E::Loading { video_id } => ("player.loading", json!({ "video_id": video_id })),
                E::Loaded => (
                    "player.loaded",
                    json!({ "video_id": state.video_id, "live": app().player.is_live() }),
                ),
                E::Position(p) => {
                    if (p - last_position.get()).abs() < 0.25
                        || !events.has_handlers("player.position")
                    {
                        return;
                    }
                    last_position.set(*p);
                    (
                        "player.position",
                        json!({ "position": p, "duration": state.duration }),
                    )
                }
                E::Duration(d) => ("player.duration", json!({ "duration": d })),
                E::Paused(p) => ("player.paused", json!({ "paused": p })),
                E::Buffering(b) => ("player.buffering", json!({ "buffering": b })),
                E::Buffered => return,
                E::Volume { volume, muted } => {
                    ("player.volume", json!({ "volume": volume, "muted": muted }))
                }
                E::Seeking { from, to } => ("player.seeking", json!({ "from": from, "to": to })),
                E::Seeked { position } => ("player.seeked", json!({ "position": position })),
                E::AudioFailed => ("player.audio_failed", json!({ "video_id": state.video_id })),
                E::Ended => ("player.ended", json!({ "video_id": state.video_id })),
                E::Error(message) => ("player.error", json!({ "message": message })),
                E::Stopped => ("player.stopped", Value::Null),
                E::Speed(speed) => ("player.speed", json!({ "speed": speed })),
                E::Settings => (
                    "player.settings",
                    json!({
                        "quality": state.quality,
                        "auto_quality": state.auto_quality,
                        "captions": state.captions,
                        "looping": state.looping,
                        "stable_volume": state.stable_volume,
                    }),
                ),
            };
            events.emit(name, &payload);
        });
    }
}

/// Runs when the app quits: sends the last watch-time report, waiting for
/// it briefly since the main loop has stopped.
pub fn shutdown() {
    let started = APP.with(|a| a.get().cloned());
    if let Some(app) = started {
        app.tracker
            .finish_blocking(std::time::Duration::from_secs(3));
        app.settings.flush();
        // mpv aborts if its renderer outlives it.
        crate::ui::player_dock::shut_down();
        for window in app.gtk.windows() {
            window.destroy();
        }
    }
}
