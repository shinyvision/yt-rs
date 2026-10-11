//! Lua plugins, running on LuaJIT.
//!
//! At startup the host runs `~/.config/yt-rs/init.lua`, then every
//! `~/.config/yt-rs/plugins/<name>/init.lua` in alphabetical order. This
//! happens before the window is built, so plugins can replace any built-in
//! component, command or key; changes to the widget tree itself belong in an
//! `app.on("app.ready", ...)` handler.
//!
//! Plugins are trusted code with full access, like Neovim plugins: they get
//! the complete standard library, every introspected library through `gi`
//! (GTK, GLib, Gio, libadwaita, GStreamer, WebKit...; see the `yt-gi`
//! crate), LuaJIT's `ffi` for any other C library, and `require` for Lua
//! and C modules from the plugins directory. Errors in plugins are logged
//! and shown briefly; they never crash the app.

mod api;
mod convert;
mod hooks;
mod widgets;

use std::cell::Cell;
use std::path::Path;
use std::time::{Duration, Instant};

use mlua::{Lua, LuaOptions, StdLib};

use crate::app::{App, app};

pub struct PluginHost {
    lua: Lua,
    loaded: Vec<String>,
}

impl PluginHost {
    pub fn start(_app: &App) -> Self {
        // SAFETY: plugins are trusted code; the unsafe constructor is what
        // enables LuaJIT's ffi library and C modules.
        let lua = unsafe { Lua::unsafe_new_with(StdLib::ALL, LuaOptions::new()) };
        let mut host = Self {
            lua,
            loaded: Vec::new(),
        };
        if let Err(e) = install(&host.lua).and_then(|_| host.set_search_path()) {
            report("starting the plugin system", &e);
            return host;
        }
        host.load_all();
        host
    }

    /// The plugins that loaded without errors.
    pub fn loaded(&self) -> &[String] {
        &self.loaded
    }

    /// Lets plugins `require` modules from the plugins directory.
    fn set_search_path(&self) -> mlua::Result<()> {
        let dir = yt_core::paths::plugins_dir();
        let dir = dir.to_string_lossy();
        let package: mlua::Table = self.lua.globals().get("package")?;
        let path: String = package.get("path")?;
        package.set("path", format!("{dir}/?.lua;{dir}/?/init.lua;{path}"))?;
        let cpath: String = package.get("cpath")?;
        package.set("cpath", format!("{dir}/?.so;{cpath}"))?;
        Ok(())
    }

    fn load_all(&mut self) {
        let init = yt_core::paths::init_lua();
        if init.is_file() {
            self.run(&init, "init.lua");
        }
        let Ok(entries) = std::fs::read_dir(yt_core::paths::plugins_dir()) else {
            return;
        };
        let mut plugins: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.join("init.lua").is_file())
            .collect();
        plugins.sort();
        for dir in plugins {
            let name = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.run(&dir.join("init.lua"), &name);
        }
        if !self.loaded.is_empty() {
            tracing::info!(plugins = ?self.loaded, "loaded plugins");
        }
    }

    /// Runs a plugin's main file. It receives its name and directory as
    /// `...`: `local name, dir = ...`.
    fn run(&mut self, file: &Path, name: &str) {
        let dir = file
            .parent()
            .map(|d| d.to_string_lossy().into_owned())
            .unwrap_or_default();
        let chunk = self.lua.load(file).set_name(format!("@{}", file.display()));
        match chunk.call::<()>((name, dir)) {
            Ok(()) => self.loaded.push(name.to_owned()),
            Err(e) => report(&format!("plugin {name}"), &e),
        }
    }
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
