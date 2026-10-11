//! The app's exported functions for Lua: calling them (`app.api`) and
//! hooking them (`app.hook`). See `yt-export` for what's exported.
//!
//! ```lua
//! -- Calls: positional arguments; async functions take a callback.
//! app.api.youtube.browse("FEwhat_to_watch", function(page, err) ... end)
//! local state = app.api.player.state()
//!
//! -- Hooks run around the function, whoever calls it, and get its
//! -- arguments by name. `next(args)` runs the rest (and the original);
//! -- what the hook returns is the result.
//! local id = app.hook("youtube.browse", function(args, next)
//!   local page = next(args)
//!   page.title = "Home, my way"
//!   return page
//! end)
//! app.unhook(id)
//! ```
//!
//! Hooks always run on the main thread. A call made on another thread (a
//! request on the network threads) waits for them there.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use mlua::thread::ThreadStatus;
use mlua::{Function, Lua, MultiValue, RegistryKey, Table};
use serde_json::Value;
use yt_export::{Call, Interceptor, Remote, Step};

use super::convert::{json_to_lua, lua_to_json};
use super::report;
use crate::runtime;

thread_local! {
    static LUA: RefCell<Option<Lua>> = const { RefCell::new(None) };
    static HOME: Cell<bool> = const { Cell::new(false) };
    /// Hooks by function name, in the order added.
    static HOOKS: RefCell<HashMap<&'static str, Vec<(u64, RegistryKey)>>> = RefCell::new(HashMap::new());
    static CHAIN: RefCell<Option<RegistryKey>> = const { RefCell::new(None) };
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
}

/// Runs a hook chain: the last hook added runs first. The original is
/// `direct(args)` when given (a synchronous call on the main thread);
/// otherwise the chain yields the arguments and is resumed with the
/// original's result (`ok, value`).
const CHAIN_LUA: &str = r#"
local yield = coroutine.yield
return function(hooks, args, direct)
  local function call(i, a)
    if i == 0 then
      if direct then return direct(a) end
      local ok, result = yield(a)
      if not ok then error(result, 0) end
      return result
    end
    return hooks[i](a, function(changed)
      if changed == nil then changed = a end
      return call(i - 1, changed)
    end)
  end
  return call(#hooks, args)
end
"#;

struct LuaHooks;

impl Interceptor for LuaHooks {
    fn is_home(&self) -> bool {
        HOME.try_with(Cell::get).unwrap_or(false)
    }

    fn run(
        &self,
        name: &'static str,
        args: Value,
        next: &mut dyn FnMut(Value) -> Result<Value, String>,
    ) -> Result<Value, String> {
        let Some(lua) = LUA.with(|l| l.borrow().clone()) else {
            return next(args);
        };
        let hooks = match hooks_of(&lua, name) {
            Ok(Some(h)) => h,
            Ok(None) => return next(args),
            Err(e) => return Err(message(&e)),
        };
        let result = lua.scope(|scope| {
            let direct = scope.create_function_mut(|lua, a: mlua::Value| {
                let a = lua_to_json(lua, a)?;
                match next(a) {
                    Ok(v) => json_to_lua(lua, &v),
                    Err(e) => Err(mlua::Error::runtime(e)),
                }
            })?;
            let chain = chain(&lua)?;
            let out: mlua::Value = chain.call((hooks, json_to_lua(&lua, &args)?, direct))?;
            lua_to_json(&lua, out)
        });
        result.map_err(|e| message(&e))
    }

    fn start(&self, name: &'static str, args: Value, remote: Remote) {
        // On the main loop, from wherever the call is.
        gtk::glib::MainContext::default().invoke(move || {
            runtime::spawn_local(async move {
                let mut remote = remote;
                let result = drive(name, args, &mut remote).await;
                (remote.send)(Step::Done(result));
            });
        });
    }
}

/// Runs a chain as a coroutine, asking the caller for the original each
/// time it yields.
async fn drive(name: &'static str, args: Value, remote: &mut Remote) -> Result<Value, String> {
    let Some(lua) = LUA.with(|l| l.borrow().clone()) else {
        return ask(args, remote).await;
    };
    let hooks = match hooks_of(&lua, name).map_err(|e| message(&e))? {
        Some(h) => h,
        None => return ask(args, remote).await,
    };
    let thread = lua
        .create_thread(chain(&lua).map_err(|e| message(&e))?)
        .map_err(|e| message(&e))?;
    let mut input = MultiValue::from_iter([
        mlua::Value::Table(hooks),
        json_to_lua(&lua, &args).map_err(|e| message(&e))?,
        mlua::Value::Nil,
    ]);
    loop {
        let out: MultiValue = thread.resume(input).map_err(|e| message(&e))?;
        let first = out.into_iter().next().unwrap_or(mlua::Value::Nil);
        let value = lua_to_json(&lua, first).map_err(|e| message(&e))?;
        match thread.status() {
            ThreadStatus::Resumable => {
                let result = ask(value, remote).await;
                input = match result {
                    Ok(v) => MultiValue::from_iter([
                        mlua::Value::Boolean(true),
                        json_to_lua(&lua, &v).map_err(|e| message(&e))?,
                    ]),
                    Err(e) => MultiValue::from_iter([
                        mlua::Value::Boolean(false),
                        mlua::Value::String(lua.create_string(&e).map_err(|e| message(&e))?),
                    ]),
                };
            }
            _ => return Ok(value),
        }
    }
}

/// Has the caller run the original.
async fn ask(args: Value, remote: &mut Remote) -> Result<Value, String> {
    (remote.send)(Step::Next(args));
    remote
        .results
        .recv()
        .await
        .unwrap_or_else(|| Err("the call went away".into()))
}

fn chain(lua: &Lua) -> mlua::Result<Function> {
    if let Some(f) = CHAIN.with(|c| {
        c.borrow()
            .as_ref()
            .map(|k| lua.registry_value::<Function>(k))
    }) {
        return f;
    }
    let f: Function = lua.load(CHAIN_LUA).set_name("@hooks").call(())?;
    let key = lua.create_registry_value(f.clone())?;
    CHAIN.with(|c| *c.borrow_mut() = Some(key));
    Ok(f)
}

/// The hooks of `name` as a Lua sequence, if any.
fn hooks_of(lua: &Lua, name: &str) -> mlua::Result<Option<Table>> {
    HOOKS.with(|h| {
        let h = h.borrow();
        let Some(list) = h.get(name).filter(|l| !l.is_empty()) else {
            return Ok(None);
        };
        let t = lua.create_table()?;
        for (_, key) in list {
            t.raw_push(lua.registry_value::<Function>(key)?)?;
        }
        Ok(Some(t))
    })
}

/// The message of a Lua error, without the wrapping and tracebacks.
fn message(error: &mlua::Error) -> String {
    match error {
        mlua::Error::RuntimeError(m) => m.clone(),
        mlua::Error::CallbackError { cause, .. } => message(cause),
        mlua::Error::WithContext { cause, .. } => message(cause),
        other => other.to_string(),
    }
}

pub fn install(lua: &Lua, app_table: &Table) -> mlua::Result<()> {
    LUA.with(|l| *l.borrow_mut() = Some(lua.clone()));
    HOME.with(|h| h.set(true));
    yt_export::set_interceptor(Box::new(LuaHooks));
    yt_export::set_reporter(
        |name, error| tracing::error!(target: "plugin", "hook of {name}: {error}"),
    );

    app_table.set(
        "hook",
        lua.create_function(|lua, (name, f): (String, Function)| {
            let function = yt_export::find(&name).ok_or_else(|| {
                let service = name.split('.').next().unwrap_or_default();
                let mut similar: Vec<&str> = yt_export::functions()
                    .iter()
                    .map(|f| f.name)
                    .filter(|n| n.starts_with(&format!("{service}.")))
                    .collect();
                similar.sort_unstable();
                mlua::Error::runtime(if similar.is_empty() {
                    format!("there's no function {name:?} to hook")
                } else {
                    format!(
                        "there's no function {name:?} to hook ({service} has {})",
                        similar.join(", ")
                    )
                })
            })?;
            let id = NEXT_ID.with(|n| {
                let id = n.get();
                n.set(id + 1);
                id
            });
            let key = lua.create_registry_value(f)?;
            HOOKS.with(|h| {
                h.borrow_mut()
                    .entry(function.name)
                    .or_default()
                    .push((id, key))
            });
            yt_export::set_hooked(function.name, true);
            Ok(id)
        })?,
    )?;
    app_table.set(
        "unhook",
        lua.create_function(|_, id: u64| {
            HOOKS.with(|h| {
                for (name, list) in h.borrow_mut().iter_mut() {
                    let before = list.len();
                    list.retain(|(i, _)| *i != id);
                    if list.len() != before && list.is_empty() {
                        yt_export::set_hooked(name, false);
                    }
                }
            });
            Ok(())
        })?,
    )?;
    // What can be called and hooked: name, doc, params, returns, async.
    app_table.set(
        "functions",
        lua.create_function(|lua, ()| {
            let out = lua.create_table()?;
            let mut all: Vec<_> = yt_export::functions().iter().collect();
            all.sort_by_key(|f| f.name);
            for f in all {
                let entry = lua.create_table()?;
                entry.set("name", f.name)?;
                entry.set("doc", f.doc)?;
                entry.set("returns", f.returns)?;
                entry.set("async", !matches!(f.call, Call::Sync(_)))?;
                let params = lua.create_table()?;
                for p in f.params {
                    let param = lua.create_table()?;
                    param.set("name", p.name)?;
                    param.set("type", p.ty)?;
                    params.raw_push(param)?;
                }
                entry.set("params", params)?;
                out.raw_push(entry)?;
            }
            Ok(out)
        })?,
    )?;
    app_table.set("api", api(lua)?)?;
    Ok(())
}

/// `app.api.<service>.<function>`, for every exported function.
fn api(lua: &Lua) -> mlua::Result<Table> {
    let api = lua.create_table()?;
    for f in yt_export::functions() {
        let Some((service, name)) = f.name.split_once('.') else {
            continue;
        };
        let table: Table = match api.raw_get::<Option<Table>>(service)? {
            Some(t) => t,
            None => {
                let t = lua.create_table()?;
                api.raw_set(service, t.clone())?;
                t
            }
        };
        let function = lua.create_function(move |lua, args: MultiValue| call(lua, f, args))?;
        table.raw_set(name, function)?;
    }
    Ok(api)
}

fn call(lua: &Lua, f: &'static yt_export::Function, args: MultiValue) -> mlua::Result<mlua::Value> {
    let mut values: Vec<mlua::Value> = args.into_iter().collect();
    let callback = match (&f.call, values.last()) {
        (Call::Sync(_), _) => None,
        (_, Some(mlua::Value::Function(cb))) => {
            let cb = cb.clone();
            values.pop();
            Some(cb)
        }
        _ => {
            return Err(mlua::Error::runtime(format!(
                "{} is async: pass a function(result, error) last",
                f.name
            )));
        }
    };
    let json = values
        .into_iter()
        .map(|v| lua_to_json(lua, v))
        .collect::<mlua::Result<Vec<_>>>()?;
    match (&f.call, callback) {
        (Call::Sync(invoke), _) => {
            let result =
                invoke(json).map_err(|e| mlua::Error::runtime(format!("{}: {e}", f.name)))?;
            json_to_lua(lua, &result)
        }
        (Call::Async(invoke), Some(cb)) => {
            let future =
                invoke(json).map_err(|e| mlua::Error::runtime(format!("{}: {e}", f.name)))?;
            let lua = lua.clone();
            runtime::spawn_local(async move {
                let result = runtime::background(future).await;
                deliver(&lua, f.name, &cb, result);
            });
            Ok(mlua::Value::Nil)
        }
        (Call::Local(invoke), Some(cb)) => {
            let future =
                invoke(json).map_err(|e| mlua::Error::runtime(format!("{}: {e}", f.name)))?;
            let lua = lua.clone();
            runtime::spawn_local(async move {
                let result = future.await;
                deliver(&lua, f.name, &cb, result);
            });
            Ok(mlua::Value::Nil)
        }
        _ => unreachable!("async calls have a callback"),
    }
}

fn deliver(lua: &Lua, name: &str, cb: &Function, result: Result<Value, String>) {
    let called = match result {
        Ok(v) => json_to_lua(lua, &v).and_then(|v| cb.call::<()>(v)),
        Err(e) => cb.call::<()>((mlua::Value::Nil, e)),
    };
    if let Err(e) = called {
        report(&format!("{name} callback"), &e);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn exported_names_are_unique() {
        let mut names: Vec<&str> = yt_export::functions().iter().map(|f| f.name).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "two exported functions share a name");
        assert!(count > 200, "only {count} functions are exported");
    }
}
