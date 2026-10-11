//! The Lua API: the `app` table (the registries, HTTP, storage, timers)
//! and the `ui` table (widgets). Lua functions handed to the app are
//! wrapped into the same kind of Rust closures the built-in code registers,
//! so plugins and built-ins are equal citizens. The app's services are
//! `app.api` (see `hooks`), the toolkit is `gi`. See `docs/plugins.md`.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::AdwDialogExt;
use gtk::glib;
use gtk::prelude::*;
use mlua::{Function, Lua, LuaSerdeExt, Table, UserData, UserDataMethods, UserDataRef, Variadic};
use serde_json::Value;

use yt_gi::{LuaCairo, Object};

use super::convert::{json_to_lua, lua_to_json};
use super::report;
use super::widgets::{find, widget, wrap};
use crate::app::app;
use crate::ext::components::Component;
use crate::ext::keymap::GLOBAL;
use crate::ext::pages::{NavItem, Page};
use crate::runtime;

pub fn install(lua: &Lua) -> mlua::Result<()> {
    let app_table = lua.create_table()?;
    install_registries(lua, &app_table)?;
    install_services(lua, &app_table)?;
    super::hooks::install(lua, &app_table)?;
    lua.globals().set("app", app_table)?;
    lua.globals().set("ui", ui_table(lua)?)?;
    lua.load(include_str!("prelude.lua"))
        .set_name("@prelude.lua")
        .exec()
}

fn widget_or_error(value: mlua::Result<mlua::Value>, context: &str) -> gtk::Widget {
    let widget = value.and_then(|v| match v {
        mlua::Value::UserData(ud) => widget(&*ud.borrow::<Object>()?),
        other => Err(mlua::Error::runtime(format!(
            "expected a widget, got {}",
            other.type_name()
        ))),
    });
    widget.unwrap_or_else(|e| {
        report(context, &e);
        let label = gtk::Label::new(Some(&format!("{context}: {e}")));
        label.add_css_class("error");
        label.set_wrap(true);
        label.upcast()
    })
}

/// Component properties for Lua: always a table, even when there are none
/// (cards in a list are first built empty, then filled).
fn props_to_lua(lua: &Lua, props: &Value) -> mlua::Result<mlua::Value> {
    if props.is_null() {
        return Ok(mlua::Value::Table(lua.create_table()?));
    }
    json_to_lua(lua, props)
}

fn lua_widget(lua: &Lua, widget: &gtk::Widget) -> mlua::Result<mlua::Value> {
    Ok(mlua::Value::UserData(
        lua.create_userdata(wrap(widget.clone()))?,
    ))
}

fn install_registries(lua: &Lua, t: &Table) -> mlua::Result<()> {
    // Events.
    t.set(
        "on",
        lua.create_function(|lua, (name, f): (String, Function)| {
            let lua = lua.clone();
            let context = format!("event {name}");
            let id = app().ext.events.on(&name, move |payload| {
                if let Err(e) = json_to_lua(&lua, payload).and_then(|p| f.call::<()>(p)) {
                    report(&context, &e);
                }
            });
            Ok(id.raw())
        })?,
    )?;
    t.set(
        "off",
        lua.create_function(|_, id: u64| {
            app()
                .ext
                .events
                .off(crate::ext::events::HandlerId::from_raw(id));
            Ok(())
        })?,
    )?;
    t.set(
        "emit",
        lua.create_function(|lua, (name, payload): (String, mlua::Value)| {
            app().ext.events.emit(&name, &lua_to_json(lua, payload)?);
            Ok(())
        })?,
    )?;

    // Filters.
    t.set(
        "filter",
        lua.create_function(|lua, (name, f, order): (String, Function, Option<i32>)| {
            let lua = lua.clone();
            let context = format!("filter {name}");
            app()
                .ext
                .filters
                .add(&name, order.unwrap_or(0), move |value| {
                    let result = json_to_lua(&lua, &value)
                        .and_then(|v| f.call::<mlua::Value>(v))
                        .and_then(|r| match r {
                            mlua::Value::Nil => Ok(value.clone()),
                            other => lua_to_json(&lua, other),
                        });
                    result.unwrap_or_else(|e| {
                        report(&context, &e);
                        value
                    })
                });
            Ok(())
        })?,
    )?;

    t.set(
        "apply_filter",
        lua.create_function(|lua, (name, value): (String, mlua::Value)| {
            let result = app().ext.filters.apply(&name, lua_to_json(lua, value)?);
            json_to_lua(lua, &result)
        })?,
    )?;

    // Commands.
    t.set(
        "command",
        lua.create_function(|lua, (name, f): (String, Function)| {
            let lua_ref = lua.clone();
            let previous = app().ext.commands.register(&name, move |args| {
                json_to_lua(&lua_ref, &args)
                    .and_then(|a| f.call::<mlua::Value>(a))
                    .and_then(|r| lua_to_json(&lua_ref, r))
                    .map_err(|e| e.to_string())
            });
            match previous {
                Some(previous) => Ok(mlua::Value::Function(lua.create_function(
                    move |lua, args: mlua::Value| {
                        let result =
                            previous(lua_to_json(lua, args)?).map_err(mlua::Error::runtime)?;
                        json_to_lua(lua, &result)
                    },
                )?)),
                None => Ok(mlua::Value::Nil),
            }
        })?,
    )?;
    t.set(
        "run",
        lua.create_function(|lua, (name, args): (String, mlua::Value)| {
            let result = app()
                .ext
                .commands
                .run(&name, lua_to_json(lua, args)?)
                .map_err(mlua::Error::runtime)?;
            json_to_lua(lua, &result)
        })?,
    )?;
    t.set(
        "commands",
        lua.create_function(|_, ()| Ok(app().ext.commands.names()))?,
    )?;

    // Key bindings.
    let keymap = lua.create_table()?;
    keymap.set(
        "set",
        lua.create_function(
            |lua, (key, command, args, scope): (String, String, mlua::Value, Option<String>)| {
                app()
                    .ext
                    .keymap
                    .set(
                        &key,
                        scope.as_deref().unwrap_or(GLOBAL),
                        &command,
                        lua_to_json(lua, args)?,
                    )
                    .map_err(mlua::Error::runtime)
            },
        )?,
    )?;
    keymap.set(
        "unset",
        lua.create_function(|_, (key, scope): (String, Option<String>)| {
            app()
                .ext
                .keymap
                .unset(&key, scope.as_deref().unwrap_or(GLOBAL))
                .map_err(mlua::Error::runtime)
        })?,
    )?;
    keymap.set(
        "clear",
        lua.create_function(|_, ()| {
            app().ext.keymap.clear();
            Ok(())
        })?,
    )?;
    keymap.set(
        "list",
        lua.create_function(|lua, ()| {
            let list = app().ext.keymap.list();
            let out = lua.create_table()?;
            for (i, (scope, key, binding)) in list.into_iter().enumerate() {
                let entry = lua.create_table()?;
                entry.set("scope", scope)?;
                entry.set("key", key)?;
                entry.set("command", binding.command)?;
                entry.set("args", json_to_lua(lua, &binding.args)?)?;
                out.set(i + 1, entry)?;
            }
            Ok(out)
        })?,
    )?;
    t.set("keymap", keymap)?;

    // Components.
    t.set(
        "component",
        lua.create_function(|lua, (name, spec): (String, Table)| {
            let component = lua_component(lua, &name, &spec)?;
            match app().ext.components.define(&name, component) {
                Some(previous) => component_table(lua, previous).map(mlua::Value::Table),
                None => Ok(mlua::Value::Nil),
            }
        })?,
    )?;
    t.set(
        "wrap",
        lua.create_function(|lua, (name, f): (String, Function)| {
            let lua = lua.clone();
            let context = format!("wrapper for {name}");
            app()
                .ext
                .components
                .wrap(&name, move |widget, props, phase| {
                    let result = (|| {
                        f.call::<()>((
                            lua_widget(&lua, widget)?,
                            props_to_lua(&lua, props)?,
                            phase.as_str(),
                        ))
                    })();
                    if let Err(e) = result {
                        report(&context, &e);
                    }
                });
            Ok(())
        })?,
    )?;
    t.set(
        "build",
        lua.create_function(|lua, (name, props): (String, mlua::Value)| {
            let widget = app().ext.components.build(&name, &lua_to_json(lua, props)?);
            lua_widget(lua, &widget)
        })?,
    )?;
    t.set(
        "components",
        lua.create_function(|_, ()| Ok(app().ext.components.names()))?,
    )?;

    // Slots.
    t.set(
        "slot",
        lua.create_function(|_, (name, f, order): (String, Function, Option<i32>)| {
            let context = format!("slot {name}");
            Ok(app().ext.slots.add(&name, order.unwrap_or(0), move || {
                widget_or_error(f.call::<mlua::Value>(()), &context)
            }))
        })?,
    )?;
    t.set(
        "unslot",
        lua.create_function(|_, id: u64| {
            app().ext.slots.remove(id);
            Ok(())
        })?,
    )?;
    t.set(
        "slots",
        lua.create_function(|_, ()| Ok(app().ext.slots.names()))?,
    )?;

    // Pages and navigation.
    t.set(
        "page",
        lua.create_function(|lua, spec: Table| {
            let name: String = spec.get("name")?;
            let component: Option<String> = spec.get("component")?;
            if spec.contains_key("build")? {
                let component_name = component.clone().unwrap_or_else(|| name.clone());
                let def = lua_component(lua, &component_name, &spec)?;
                app().ext.components.define(&component_name, def);
            }
            let args: mlua::Value = spec.get("args")?;
            app().ext.pages.register(Page {
                title: spec
                    .get::<Option<String>>("title")?
                    .unwrap_or_else(|| name.clone()),
                name,
                component,
                args: lua_to_json(lua, args)?,
                keep: spec.get::<Option<bool>>("keep")?.unwrap_or(true),
            });
            Ok(())
        })?,
    )?;
    t.set(
        "nav",
        lua.create_function(|lua, spec: mlua::Value| {
            let item: NavItem =
                serde_json::from_value(lua_to_json(lua, spec)?).map_err(mlua::Error::external)?;
            app().ext.pages.add_nav(item);
            Ok(())
        })?,
    )?;
    t.set(
        "unnav",
        lua.create_function(|_, id: String| {
            app().ext.pages.remove_nav(&id);
            Ok(())
        })?,
    )?;
    t.set(
        "navigate",
        lua.create_function(|lua, (page, args): (String, mlua::Value)| {
            app().navigate(&page, lua_to_json(lua, args)?, true);
            Ok(())
        })?,
    )?;
    t.set("back", lua.create_function(|_, ()| Ok(app().back()))?)?;
    t.set("forward", lua.create_function(|_, ()| Ok(app().forward()))?)?;
    t.set(
        "current_page",
        lua.create_function(|_, ()| Ok(app().current_page()))?,
    )?;
    t.set(
        "pages",
        lua.create_function(|lua, ()| {
            json_to_lua(
                lua,
                &serde_json::to_value(app().ext.pages.list()).unwrap_or_default(),
            )
        })?,
    )?;

    // Theme.
    t.set(
        "css",
        lua.create_function(|_, css: String| Ok(app().ext.theme.add_css(&css)))?,
    )?;
    t.set(
        "uncss",
        lua.create_function(|_, id: u64| {
            app().ext.theme.remove(id);
            Ok(())
        })?,
    )?;
    Ok(())
}

/// A component defined in Lua: `{ build = function(props) ... end,
/// update = function(widget, props) ... end }`.
fn lua_component(lua: &Lua, name: &str, spec: &Table) -> mlua::Result<Component> {
    let build: Function = spec.get("build")?;
    let update: Option<Function> = spec.get("update")?;
    let (lua_b, context_b) = (lua.clone(), format!("component {name}"));
    let mut component = Component::new(move |props| {
        let result = props_to_lua(&lua_b, props).and_then(|p| build.call::<mlua::Value>(p));
        widget_or_error(result, &context_b)
    });
    if let Some(update) = update {
        let (lua_u, context_u) = (lua.clone(), format!("component {name} (update)"));
        component = component.with_update(move |widget, props| {
            let result = (|| {
                update.call::<()>((lua_widget(&lua_u, widget)?, props_to_lua(&lua_u, props)?))
            })();
            if let Err(e) = result {
                report(&context_u, &e);
            }
        });
    }
    Ok(component)
}

/// A Rust component as a Lua table, so replacements can build on the
/// original.
fn component_table(lua: &Lua, component: Component) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    let build = component.build.clone();
    t.set(
        "build",
        lua.create_function(move |lua, props: mlua::Value| {
            lua_widget(lua, &build(&lua_to_json(lua, props)?))
        })?,
    )?;
    if let Some(update) = component.update {
        t.set(
            "update",
            lua.create_function(
                move |lua, (widget, props): (UserDataRef<Object>, mlua::Value)| {
                    update(&super::widgets::widget(&widget)?, &lua_to_json(lua, props)?);
                    Ok(())
                },
            )?,
        )?;
    }
    Ok(t)
}

struct LuaTimer(Rc<Cell<Option<glib::SourceId>>>);

impl UserData for LuaTimer {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("cancel", |_, timer, ()| {
            if let Some(id) = timer.0.take() {
                id.remove();
            }
            Ok(())
        });
    }
}

fn install_services(lua: &Lua, t: &Table) -> mlua::Result<()> {
    t.set(
        "window",
        lua.create_function(|lua, ()| match app().window() {
            Some(w) => lua_widget(lua, w.upcast_ref()),
            None => Ok(mlua::Value::Nil),
        })?,
    )?;
    // The container pages are shown in, for a replacement `shell`.
    t.set(
        "page_host",
        lua.create_function(|lua, ()| lua_widget(lua, app().page_host().upcast_ref()))?,
    )?;
    t.set(
        "find",
        lua.create_function(|lua, query: String| {
            let found = app().window().and_then(|w| find(w.upcast_ref(), &query));
            match found {
                Some(w) => lua_widget(lua, &w),
                None => Ok(mlua::Value::Nil),
            }
        })?,
    )?;
    t.set(
        "toast",
        lua.create_function(|lua, (message, options): (String, Option<mlua::Table>)| {
            // app.toast(message, { action = { label, command, args }, seconds })
            let mut props = match options {
                Some(options) => super::convert::lua_to_json(lua, mlua::Value::Table(options))?,
                None => serde_json::json!({}),
            };
            props["message"] = message.into();
            app().toast_with(&props);
            Ok(())
        })?,
    )?;
    // app.dialog(content, { title?, width? }) → the dialog: shows a widget
    // in a dialog over the window, as the app's own are.
    t.set(
        "dialog",
        lua.create_function(
            |_, (content, options): (UserDataRef<Object>, Option<Table>)| {
                let content = content
                    .0
                    .clone()
                    .downcast::<gtk::Widget>()
                    .map_err(|_| mlua::Error::runtime("a dialog shows a widget"))?;
                let option = |key: &str| -> Option<mlua::Value> {
                    options
                        .as_ref()
                        .and_then(|o| o.get::<mlua::Value>(key).ok())
                };
                let dialog = adw::Dialog::builder()
                    .child(&content)
                    .follows_content_size(true)
                    .build();
                if let Some(mlua::Value::String(title)) = option("title") {
                    dialog.set_title(&title.to_str()?);
                }
                if let Some(width) = option("width").and_then(|w| w.as_i32()) {
                    dialog.set_content_width(width);
                    dialog.set_follows_content_size(false);
                }
                dialog.add_css_class("yt-dialog");
                if let Some(window) = app().window() {
                    dialog.present(Some(&window));
                }
                Ok(wrap(dialog))
            },
        )?,
    )?;
    t.set(
        "screenshot",
        lua.create_function(|_, path: String| {
            crate::commands::screenshot(&path).map_err(mlua::Error::runtime)
        })?,
    )?;
    t.set(
        "quit",
        lua.create_function(|_, ()| {
            app().gtk.quit();
            Ok(())
        })?,
    )?;
    t.set(
        "log",
        lua.create_function(|_, parts: Variadic<mlua::Value>| {
            let text: Vec<String> = parts
                .iter()
                .map(|v| v.to_string().unwrap_or_else(|_| format!("{v:?}")))
                .collect();
            tracing::info!(target: "plugin", "{}", text.join(" "));
            Ok(())
        })?,
    )?;

    // Timers.
    t.set(
        "now",
        lua.create_function(|_, ()| {
            // Seconds on a clock that only goes forward, for measuring time.
            static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
            Ok(START
                .get_or_init(std::time::Instant::now)
                .elapsed()
                .as_secs_f64())
        })?,
    )?;
    t.set(
        "timer",
        lua.create_function(|lua, (ms, f, repeat): (u64, Function, Option<bool>)| {
            let handle = Rc::new(Cell::new(None));
            let h = handle.clone();
            let repeat = repeat.unwrap_or(false);
            let id = glib::timeout_add_local(Duration::from_millis(ms), move || {
                let keep_going = match f.call::<mlua::Value>(()) {
                    Ok(mlua::Value::Boolean(false)) => false,
                    Ok(_) => repeat,
                    Err(e) => {
                        report("timer", &e);
                        false
                    }
                };
                if keep_going {
                    glib::ControlFlow::Continue
                } else {
                    h.take();
                    glib::ControlFlow::Break
                }
            });
            handle.set(Some(id));
            lua.create_userdata(LuaTimer(handle))
        })?,
    )?;
    t.set(
        "idle",
        lua.create_function(|_, f: Function| {
            glib::idle_add_local_once(move || {
                if let Err(e) = f.call::<()>(()) {
                    report("idle callback", &e);
                }
            });
            Ok(())
        })?,
    )?;

    // HTTP: app.http({ url, method, headers, body }, function(response, err) end)
    t.set(
        "http",
        lua.create_function(|lua, (opts, callback): (Table, Function)| {
            let url: String = opts.get("url")?;
            let method: String = opts
                .get::<Option<String>>("method")?
                .unwrap_or_else(|| "GET".into());
            let headers: Vec<(String, String)> = opts
                .get::<Option<Table>>("headers")?
                .map(|h| h.pairs().collect::<mlua::Result<_>>())
                .transpose()?
                .unwrap_or_default();
            let body = match opts.get::<mlua::Value>("body")? {
                mlua::Value::Nil => None,
                mlua::Value::String(s) => Some(s.as_bytes().to_vec()),
                other => Some(
                    serde_json::to_vec(&lua_to_json(lua, other)?).map_err(mlua::Error::external)?,
                ),
            };
            let lua = lua.clone();
            runtime::spawn_local(async move {
                let http = app().http.clone();
                let result = runtime::background(async move {
                    let method = reqwest::Method::from_bytes(method.as_bytes())
                        .map_err(|e| e.to_string())?;
                    let mut request = http.request(method, &url);
                    for (k, v) in headers {
                        request = request.header(k, v);
                    }
                    if let Some(body) = body {
                        request = request.body(body);
                    }
                    let resp = request.send().await.map_err(|e| e.to_string())?;
                    let status = resp.status().as_u16();
                    let headers: Vec<(String, String)> = resp
                        .headers()
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or_default().to_owned()))
                        .collect();
                    let body = resp.bytes().await.map_err(|e| e.to_string())?;
                    Ok::<_, String>((status, headers, body))
                })
                .await;
                let call = (|| match result {
                    Ok((status, headers, body)) => {
                        let resp = lua.create_table()?;
                        resp.set("status", status)?;
                        resp.set("body", lua.create_string(&body)?)?;
                        let h = lua.create_table()?;
                        for (k, v) in headers {
                            h.set(k, v)?;
                        }
                        resp.set("headers", h)?;
                        callback.call::<()>(resp)
                    }
                    Err(e) => callback.call::<()>((mlua::Value::Nil, e)),
                })();
                if let Err(e) = call {
                    report("http callback", &e);
                }
            });
            Ok(())
        })?,
    )?;

    // YouTube. Each call takes a callback, called with the result or with
    // nil and an error message.

    // The player: its state, and mpv itself for anything else.

    // JSON and paths.
    // app.array(t): marks a table as a list, so an empty one becomes a JSON
    // array rather than an object.
    t.set(
        "array",
        lua.create_function(|lua, list: Table| {
            list.set_metatable(Some(lua.array_metatable()))?;
            Ok(list)
        })?,
    )?;
    let json = lua.create_table()?;
    json.set(
        "encode",
        lua.create_function(|lua, value: mlua::Value| {
            serde_json::to_string(&lua_to_json(lua, value)?).map_err(mlua::Error::external)
        })?,
    )?;
    json.set(
        "decode",
        lua.create_function(|lua, text: String| {
            let value: Value = serde_json::from_str(&text).map_err(mlua::Error::external)?;
            json_to_lua(lua, &value)
        })?,
    )?;
    t.set("json", json)?;
    let hash = lua.create_table()?;
    hash.set(
        "sha256",
        lua.create_function(|_, text: mlua::LuaString| {
            Ok(
                glib::compute_checksum_for_data(glib::ChecksumType::Sha256, &text.as_bytes())
                    .map(|h| h.to_string()),
            )
        })?,
    )?;
    t.set("hash", hash)?;
    // app.storage(name): what a plugin keeps between runs, as a table, and
    // its :save(); a JSON file in the config directory.
    t.set(
        "storage",
        lua.create_function(|lua, name: String| {
            let valid = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if !valid {
                return Err(mlua::Error::runtime(
                    "a storage name is letters, digits, - and _",
                ));
            }
            let path = yt_core::paths::config_dir()
                .join("plugin-data")
                .join(format!("{name}.json"));
            let data: Value = std::fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .filter(Value::is_object)
                .unwrap_or_else(|| serde_json::json!({}));
            let table = match json_to_lua(lua, &data)? {
                mlua::Value::Table(table) => table,
                _ => lua.create_table()?,
            };
            let methods = lua.create_table()?;
            methods.set(
                "save",
                lua.create_function(move |lua, this: Table| {
                    let value = lua_to_json(lua, mlua::Value::Table(this))?;
                    let text = serde_json::to_vec_pretty(&value).map_err(mlua::Error::external)?;
                    let dir = path.parent().expect("a file in a directory");
                    std::fs::create_dir_all(dir).map_err(mlua::Error::external)?;
                    // Written whole, then put in place: never half a file.
                    let partial = path.with_extension("json.partial");
                    std::fs::write(&partial, text).map_err(mlua::Error::external)?;
                    std::fs::rename(&partial, &path).map_err(mlua::Error::external)?;
                    Ok(())
                })?,
            )?;
            let meta = lua.create_table()?;
            meta.set("__index", methods)?;
            table.set_metatable(Some(meta))?;
            Ok(table)
        })?,
    )?;
    Ok(())
}

fn ui_table(lua: &Lua) -> mlua::Result<Table> {
    let ui = lua.create_table()?;
    // ui.new(type_name, props): any GObject type, e.g. "GtkButton"
    // (the same as `Gtk.Button(props)` through `gi`).
    ui.set(
        "new",
        lua.create_function(|lua, (type_name, props): (String, Option<Table>)| {
            let ty = yt_gi::gtype_from_lua(&mlua::Value::String(lua.create_string(&type_name)?))
                .map_err(|_| mlua::Error::runtime(format!("unknown type {type_name:?}")))?;
            yt_gi::new_object(lua, ty, props.as_ref())
        })?,
    )?;
    // ui.wrap(pointer): a GObject from LuaJIT's FFI (lightuserdata or address).
    ui.set(
        "wrap",
        lua.create_function(|_, ptr: mlua::Value| {
            let raw = match ptr {
                mlua::Value::LightUserData(p) => p.0,
                mlua::Value::Integer(i) => i as usize as *mut std::ffi::c_void,
                mlua::Value::Number(n) => n as usize as *mut std::ffi::c_void,
                other => {
                    return Err(mlua::Error::runtime(format!(
                        "expected a pointer, got {}",
                        other.type_name()
                    )));
                }
            };
            yt_gi::from_ptr(raw)
        })?,
    )?;
    // ui.image(url, width): a picture loaded through the app's image cache.
    ui.set(
        "image",
        lua.create_function(|_, (url, width): (Option<String>, Option<i32>)| {
            let picture = gtk::Picture::new();
            picture.set_content_fit(gtk::ContentFit::Cover);
            picture.set_can_shrink(true);
            app()
                .images
                .load_into(&picture, url.as_deref(), width.unwrap_or(0));
            Ok(wrap(picture))
        })?,
    )?;
    // ui.drawing_area(function(cr, width, height) end): draws with Cairo.
    ui.set(
        "drawing_area",
        lua.create_function(|_, draw: Function| {
            let area = gtk::DrawingArea::new();
            area.set_draw_func(move |_, cr, width, height| {
                if let Err(e) = draw.call::<()>((LuaCairo(cr.clone()), width, height)) {
                    report("drawing", &e);
                }
            });
            Ok(wrap(area))
        })?,
    )?;
    ui.set(
        "type_exists",
        lua.create_function(|lua, name: String| {
            Ok(yt_gi::gtype_from_lua(&mlua::Value::String(lua.create_string(&name)?)).is_ok())
        })?,
    )?;
    Ok(ui)
}
