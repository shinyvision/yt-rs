//! Lua plugins, running on LuaJIT.
//!
//! At startup the host runs `~/.config/yt-rs/init.lua`, then every plugin in
//! alphabetical order: the ones that come with the app ([`bundled`]) and
//! every `~/.config/yt-rs/plugins/<name>/init.lua`, where a plugin of the
//! same name as a bundled one takes its place. Plugins turned off in
//! Settings don't run. This happens before the window is built, so plugins
//! can replace any built-in component, command or key; changes to the widget
//! tree itself belong in an `app.on("app.ready", ...)` handler.
//!
//! Plugins are trusted code with full access, like Neovim plugins: they get
//! the complete standard library, every introspected library through `gi`
//! (GTK, GLib, Gio, libadwaita, GStreamer, WebKit...; see the `yt-gi`
//! crate), LuaJIT's `ffi` for any other C library, and `require` for Lua
//! and C modules from the plugins directories. Errors in plugins are logged
//! and shown briefly; they never crash the app.

mod api;
mod bundled;
mod convert;
mod hooks;
mod widgets;

use std::cell::Cell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mlua::{Lua, LuaOptions, StdLib};

use crate::app::{App, app};

pub struct PluginHost {
    lua: Lua,
    plugins: Vec<Plugin>,
}

/// A plugin the host found, and what came of it.
#[derive(Clone, Debug)]
pub struct Plugin {
    pub name: String,
    pub dir: PathBuf,
    pub origin: Origin,
    pub state: State,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Comes with the app.
    Bundled,
    /// In the user's plugins directory.
    User,
    /// In the user's plugins directory, in place of a bundled one.
    ReplacesBundled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Loaded,
    /// Turned off in Settings.
    Off,
    /// Its main file failed, with this error.
    Failed(String),
}

impl PluginHost {
    pub fn start(app: &App) -> Self {
        // SAFETY: plugins are trusted code; the unsafe constructor is what
        // enables LuaJIT's ffi library and C modules.
        let lua = unsafe { Lua::unsafe_new_with(StdLib::ALL, LuaOptions::new()) };
        let bundled = bundled::dir()
            .inspect_err(|e| {
                tracing::warn!("couldn't set up the plugins that come with yt-rs: {e}")
            })
            .ok();
        let mut host = Self {
            lua,
            plugins: found(bundled.as_deref()),
        };
        if let Err(e) = install(&host.lua).and_then(|_| host.set_search_path(bundled.as_deref())) {
            report("starting the plugin system", &e);
            for plugin in &mut host.plugins {
                plugin.state = State::Failed(e.to_string());
            }
            return host;
        }
        host.load_all(&app.settings.get().disabled_plugins);
        host
    }

    /// Every plugin found, in the order they run.
    pub fn plugins(&self) -> &[Plugin] {
        &self.plugins
    }

    /// The plugins that loaded without errors.
    pub fn loaded(&self) -> Vec<String> {
        self.plugins
            .iter()
            .filter(|p| p.state == State::Loaded)
            .map(|p| p.name.clone())
            .collect()
    }

    /// Lets plugins `require` modules from the plugins directories, the
    /// user's first.
    fn set_search_path(&self, bundled: Option<&Path>) -> mlua::Result<()> {
        let dirs: Vec<String> = std::iter::once(yt_core::paths::plugins_dir())
            .chain(bundled.map(Path::to_owned))
            .map(|d| d.to_string_lossy().into_owned())
            .collect();
        let package: mlua::Table = self.lua.globals().get("package")?;
        let path: String = package.get("path")?;
        let lua_paths = dirs.iter().map(|d| format!("{d}/?.lua;{d}/?/init.lua;"));
        package.set("path", lua_paths.collect::<String>() + &path)?;
        let cpath: String = package.get("cpath")?;
        let c_paths = dirs.iter().map(|d| format!("{d}/?.so;"));
        package.set("cpath", c_paths.collect::<String>() + &cpath)?;
        Ok(())
    }

    fn load_all(&mut self, disabled: &[String]) {
        let init = yt_core::paths::init_lua();
        if init.is_file()
            && let Err(e) = self.run(&init, "init.lua")
        {
            report("init.lua", &e);
        }
        let mut plugins = std::mem::take(&mut self.plugins);
        for plugin in &mut plugins {
            if disabled.contains(&plugin.name) {
                continue;
            }
            plugin.state = match self.run(&plugin.dir.join("init.lua"), &plugin.name) {
                Ok(()) => State::Loaded,
                Err(e) => {
                    report(&format!("plugin {}", plugin.name), &e);
                    State::Failed(e.to_string())
                }
            };
        }
        self.plugins = plugins;
        let loaded = self.loaded();
        if !loaded.is_empty() {
            tracing::info!(plugins = ?loaded, "loaded plugins");
        }
    }

    /// Runs a plugin's main file. It receives its name and directory as
    /// `...`: `local name, dir = ...`.
    fn run(&self, file: &Path, name: &str) -> mlua::Result<()> {
        let dir = file
            .parent()
            .map(|d| d.to_string_lossy().into_owned())
            .unwrap_or_default();
        let chunk = self.lua.load(file).set_name(format!("@{}", file.display()));
        chunk.call::<()>((name, dir))
    }
}

/// The plugins in the bundled and the user's plugins directories (each a
/// directory with an `init.lua`), by name; the user's replace bundled ones.
fn found(bundled: Option<&Path>) -> Vec<Plugin> {
    let mut plugins = BTreeMap::new();
    let mut add = |dir: &Path, user: bool| {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for dir in entries.filter_map(Result::ok).map(|e| e.path()) {
            if !dir.join("init.lua").is_file() {
                continue;
            }
            let name = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let origin = match (user, plugins.contains_key(&name)) {
                (false, _) => Origin::Bundled,
                (true, false) => Origin::User,
                (true, true) => Origin::ReplacesBundled,
            };
            // Off until it runs.
            let plugin = Plugin {
                name: name.clone(),
                dir,
                origin,
                state: State::Off,
            };
            plugins.insert(name, plugin);
        }
    };
    if let Some(dir) = bundled {
        add(dir, false);
    }
    add(&yt_core::paths::plugins_dir(), true);
    plugins.into_values().collect()
}

/// The Lua environment: `gi` with the toolkit loaded, the widget
/// conveniences, then the `app` and `ui` tables.
fn install(lua: &Lua) -> mlua::Result<()> {
    yt_gi::set_reporter(report);
    yt_gi::install(lua)?;
    yt_gi::require(lua, "Gtk", "4.0")?;
    yt_gi::require(lua, "Adw", "1")?;
    widgets::install(lua)?;
    api::install(lua)
}

/// Reports an error from plugin code: logged in full, and shown as a toast
/// at most every few seconds so a broken plugin can't flood the screen.
pub fn report(context: &str, error: &mlua::Error) {
    tracing::error!(target: "plugin", "{context}: {error}");
    thread_local! {
        static LAST_TOAST: Cell<Option<Instant>> = const { Cell::new(None) };
    }
    let due = LAST_TOAST.with(|last| {
        let due = last
            .get()
            .is_none_or(|t| t.elapsed() > Duration::from_secs(3));
        if due {
            last.set(Some(Instant::now()));
        }
        due
    });
    if due {
        let first_line = error
            .to_string()
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned();
        app().toast(&format!("Plugin error ({context}): {first_line}"));
    }
}
