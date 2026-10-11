//! Namespaces (`gi.require("Gtk", "4.0")`) and the tables for their
//! classes, records and enums, filled in lazily as names are used.
//!
//! A class table holds the class's functions (`Gtk.IconTheme.get_for_display`,
//! constructors, and methods to call as `Gtk.Widget.show(w)`), constants,
//! `vfunc_<name>` to call a class's implementation of a virtual function
//! (chaining up), and `_gtype`. Calling it makes an instance with
//! properties: `Gtk.Label { label = "Hi" }`. Functions set on it in Lua
//! become methods of its instances and its subclasses' instances.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use glib::translate::IntoGlib;
use mlua::{Function, Lua, MultiValue, RegistryKey, Table};

use crate::call::{self, Callable};
use crate::convert;
use crate::enums;
use crate::info::{Info, Kind};
use crate::members;
use crate::record::{self, Record};
use crate::repo;
use crate::sys::{self, GIArgument};
use crate::types::Type;

thread_local! {
    /// The names class tables are kept under (in `State::classes`): GI
    /// names (`Gtk.Button`) or, for types without introspection data,
    /// GType names.
    static KEYS: RefCell<HashMap<glib::Type, String>> = RefCell::new(HashMap::new());
}

fn get_table(
    lua: &Lua,
    map: &RefCell<HashMap<String, RegistryKey>>,
    key: &str,
) -> mlua::Result<Option<Table>> {
    map.borrow()
        .get(key)
        .map(|k| lua.registry_value::<Table>(k))
        .transpose()
}

/// The name a type's class table is kept under.
fn class_key(gtype: glib::Type) -> String {
    if let Some(k) = KEYS.with(|k| k.borrow().get(&gtype).cloned()) {
        return k;
    }
    let key = match repo::find_by_gtype(gtype) {
        Some(info) if matches!(info.kind(), Kind::Object | Kind::Interface) => info.full_name(),
        _ => gtype.name().to_owned(),
    };
    KEYS.with(|k| k.borrow_mut().insert(gtype, key.clone()));
    key
}

/// The class table of a type, if one was made.
pub fn existing_class(lua: &Lua, gtype: glib::Type) -> mlua::Result<Option<Table>> {
    get_table(lua, &crate::state(lua).classes, &class_key(gtype))
}

pub fn existing_record_class(lua: &Lua, info: &Info) -> mlua::Result<Option<Table>> {
    get_table(lua, &crate::state(lua).classes, &info.full_name())
}

/// Loads a namespace and returns its table.
pub fn require(lua: &Lua, namespace: &str, version: Option<&str>) -> mlua::Result<Table> {
    let existing = get_table(lua, &crate::state(lua).namespaces, namespace)?;
    if let Some(t) = existing {
        if let Some(wanted) = version {
            let loaded: String = t.raw_get("_version")?;
            if loaded != wanted {
                return Err(mlua::Error::runtime(format!(
                    "{namespace} {loaded} is loaded already; can't load {namespace} {wanted} too"
                )));
            }
        }
        return Ok(t);
    }
    let loaded = repo::require(namespace, version).map_err(mlua::Error::runtime)?;
    let table = lua.create_table()?;
    table.raw_set("_namespace", namespace)?;
    table.raw_set("_version", loaded)?;
    let meta = lua.create_table()?;
    let ns = namespace.to_owned();
    meta.raw_set(
        "__index",
        lua.create_function(move |lua, (table, key): (Table, mlua::Value)| {
            let Some(key) = key
                .as_string()
                .and_then(|s| s.to_str().ok())
                .map(|s| s.to_string())
            else {
                return Ok(mlua::Value::Nil);
            };
            let value = namespace_member(lua, &ns, &key)?;
            if !value.is_nil() {
                table.raw_set(key, value.clone())?;
            }
            Ok(value)
        })?,
    )?;
    let name = namespace.to_owned();
    meta.raw_set(
        "__tostring",
        lua.create_function(move |_, _: mlua::Value| Ok(format!("namespace {name}")))?,
    )?;
    table.set_metatable(Some(meta))?;
    let key = lua.create_registry_value(table.clone())?;
    crate::state(lua)
        .namespaces
        .borrow_mut()
        .insert(namespace.to_owned(), key);
    Ok(table)
}

fn namespace_member(lua: &Lua, namespace: &str, key: &str) -> mlua::Result<mlua::Value> {
    let Some(info) = repo::find_by_name(namespace, key) else {
        return Ok(mlua::Value::Nil);
    };
    Ok(match info.kind() {
        Kind::Function => mlua::Value::Function(call::function(lua, &info)?),
        Kind::Object | Kind::Interface => {
            mlua::Value::Table(class_table(lua, info.gtype(), Some(info))?)
        }
        Kind::Struct | Kind::Union => mlua::Value::Table(record_class(lua, info)?),
        Kind::Enum | Kind::Flags => mlua::Value::Table(enum_table(lua, &info)?),
        Kind::Constant => constant(lua, &info)?,
        _ => mlua::Value::Nil,
    })
}

fn constant(lua: &Lua, info: &Info) -> mlua::Result<mlua::Value> {
    let mut value = GIArgument::default();
    // SAFETY: a constant info; the value is freed after converting.
    unsafe {
        let ty = Type::new(
            Info::from_full(sys::gi_constant_info_get_type_info(info.ptr()))
                .expect("constant type"),
        );
        sys::gi_constant_info_get_value(info.ptr(), &mut value);
        let out = convert::to_lua(lua, &value, &ty, sys::TRANSFER_NOTHING, None);
        sys::gi_constant_info_free_value(info.ptr(), &mut value);
        out
    }
}

fn enum_table(lua: &Lua, info: &Info) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    let flags = info.kind() == Kind::Flags;
    for (upper, nick, value) in &enums::values(info).list {
        let v = if flags {
            mlua::Value::Integer(*value)
        } else {
            mlua::Value::String(lua.create_string(&**nick)?)
        };
        t.raw_set(upper.as_str(), v.clone())?;
        t.raw_set(upper.to_ascii_lowercase(), v)?;
    }
    // Functions of the enum (`Gio.IOErrorEnum.from_errno`), on demand.
    let meta = lua.create_table()?;
    let enum_info = info.clone();
    meta.raw_set(
        "__index",
        lua.create_function(move |lua, (_, key): (Table, String)| {
            for m in crate::info::children(
                &enum_info,
                sys::gi_enum_info_get_n_methods,
                sys::gi_enum_info_get_method,
            ) {
                if m.name() == key {
                    return Ok(mlua::Value::Function(call::function(lua, &m)?));
                }
            }
            Ok(mlua::Value::Nil)
        })?,
    )?;
    let name = info.full_name();
    meta.raw_set(
        "__tostring",
        lua.create_function(move |_, _: mlua::Value| Ok(name.clone()))?,
    )?;
    t.set_metatable(Some(meta))?;
    Ok(t)
}

/// The class table of `gtype` (made on first use).
pub fn class_table(lua: &Lua, gtype: glib::Type, info: Option<Info>) -> mlua::Result<Table> {
    let key = class_key(gtype);
    let existing = get_table(lua, &crate::state(lua).classes, &key)?;
    if let Some(t) = existing {
        return Ok(t);
    }
    let info = info
        .or_else(|| repo::find_by_gtype(gtype))
        .filter(|i| matches!(i.kind(), Kind::Object | Kind::Interface));
    let table = lua.create_table()?;
    table.raw_set("_gtype", gtype.into_glib() as f64)?;
    table.raw_set("_name", key.clone())?;
    let meta = lua.create_table()?;
    meta.raw_set(
        "__index",
        lua.create_function(move |lua, (table, key): (Table, mlua::Value)| {
            let Some(key) = key
                .as_string()
                .and_then(|s| s.to_str().ok())
                .map(|s| s.to_string())
            else {
                return Ok(mlua::Value::Nil);
            };
            let value = class_member(lua, gtype, &key)?;
            // Cache what's found where instances don't look for Lua
            // functions: in the metatable.
            if !value.is_nil()
                && let Some(meta) = table.metatable()
            {
                let cache: Table = meta.raw_get("__cache")?;
                cache.raw_set(key, value.clone())?;
            }
            Ok(value)
        })?,
    )?;
    meta.raw_set("__cache", lua.create_table()?)?;
    meta.raw_set(
        "__newindex",
        lua.create_function(
            |lua, (table, key, value): (Table, mlua::Value, mlua::Value)| {
                table.raw_set(key, value)?;
                members::invalidate(lua);
                Ok(())
            },
        )?,
    )?;
    meta.raw_set(
        "__call",
        lua.create_function(move |lua, (_, props): (Table, Option<Table>)| {
            lua.create_userdata(crate::new_object(lua, gtype, props.as_ref())?)
        })?,
    )?;
    let name = key.clone();
    meta.raw_set(
        "__tostring",
        lua.create_function(move |_, _: mlua::Value| Ok(name.clone()))?,
    )?;
    table.set_metatable(Some(meta))?;
    // Look in the cache before computing.
    let index: Function = lua
        .load(
            r#"
local compute, cache = ...
return function(t, k)
  local v = cache[k]
  if v ~= nil then return v end
  return compute(t, k)
end
"#,
        )
        .set_name("@gi-class")
        .call((
            table
                .metatable()
                .expect("meta")
                .raw_get::<Function>("__index")?,
            table
                .metatable()
                .expect("meta")
                .raw_get::<Table>("__cache")?,
        ))?;
    table.metatable().expect("meta").raw_set("__index", index)?;
    let _ = info;
    let stored = lua.create_registry_value(table.clone())?;
    crate::state(lua).classes.borrow_mut().insert(key, stored);
    Ok(table)
}

fn class_member(lua: &Lua, gtype: glib::Type, key: &str) -> mlua::Result<mlua::Value> {
    if let Some(name) = key.strip_prefix("vfunc_") {
        return vfunc_caller(lua, gtype, name);
    }
    if key == "_parent" {
        return match gtype.parent() {
            Some(p) => Ok(mlua::Value::Table(class_table(lua, p, None)?)),
            None => Ok(mlua::Value::Nil),
        };
    }
    // Lua functions on an ancestor's table, then functions of this class
    // or an ancestor (`Gtk.Button.show`).
    for t in members::ancestors(gtype).skip(1) {
        if let Some(class) = existing_class(lua, t)?
            && let mlua::Value::Function(f) = class.raw_get::<mlua::Value>(key)?
        {
            return Ok(mlua::Value::Function(f));
        }
    }
    if let Some(f) = members::gi_function(gtype, key, true) {
        return Ok(mlua::Value::Function(call::function(lua, &f)?));
    }
    // Constants of the class.
    if let Some(info) = repo::find_by_gtype(gtype) {
        let constants: Box<dyn Iterator<Item = Info>> = match info.kind() {
            Kind::Object => Box::new(crate::info::children(
                &info,
                sys::gi_object_info_get_n_constants,
                sys::gi_object_info_get_constant,
            )),
            Kind::Interface => Box::new(crate::info::children(
                &info,
                sys::gi_interface_info_get_n_constants,
                sys::gi_interface_info_get_constant,
            )),
            _ => Box::new(std::iter::empty()),
        };
        for c in constants {
            if c.name() == key {
                return constant(lua, &c);
            }
        }
    }
    Ok(mlua::Value::Nil)
}

/// The virtual function `name` as declared by `gtype` or an ancestor (or
/// one of its interfaces), with the type that declares it.
pub fn find_vfunc(gtype: glib::Type, name: &str) -> Option<(Info, glib::Type)> {
    let cname = std::ffi::CString::new(name).ok()?;
    for t in members::ancestors(gtype) {
        if let Some(info) = repo::find_by_gtype(t).filter(|i| i.kind() == Kind::Object) {
            // SAFETY: an object info; returns a new reference or null.
            if let Some(v) = unsafe {
                Info::from_full(sys::gi_object_info_find_vfunc(info.ptr(), cname.as_ptr()))
            } {
                return Some((v, t));
            }
        }
    }
    for i in members::interfaces(gtype) {
        if let Some(info) = repo::find_by_gtype(i).filter(|i| i.kind() == Kind::Interface) {
            // SAFETY: an interface info; returns a new reference or null.
            if let Some(v) = unsafe {
                Info::from_full(sys::gi_interface_info_find_vfunc(
                    info.ptr(),
                    cname.as_ptr(),
                ))
            } {
                return Some((v, i));
            }
        }
    }
    None
}

/// `Gtk.Widget.vfunc_snapshot(self, snapshot)`: calls `gtype`'s own
/// implementation of a virtual function, for chaining up from a subclass.
fn vfunc_caller(lua: &Lua, gtype: glib::Type, name: &str) -> mlua::Result<mlua::Value> {
    let name = name.to_owned();
    let Some((info, _declarer)) = find_vfunc(gtype, &name) else {
        return Ok(mlua::Value::Nil);
    };
    let callable = Rc::new(Callable::new(info.clone()));
    let f = call::wrap(lua, callable, move |_: &MultiValue| {
        let mut error = std::ptr::null_mut();
        // SAFETY: looks the implementation up in the class structure.
        let address =
            unsafe { sys::gi_vfunc_info_get_address(info.ptr(), gtype.into_glib(), &mut error) };
        if address.is_null() {
            if !error.is_null() {
                // SAFETY: GI set the error.
                let e: glib::Error = unsafe { glib::translate::from_glib_full(error) };
                return Err(mlua::Error::runtime(e.message().to_owned()));
            }
            return Err(mlua::Error::runtime(format!(
                "{} has no {name} implementation",
                gtype.name()
            )));
        }
        Ok(address)
    })?;
    Ok(mlua::Value::Function(f))
}

/// The table of a struct or union type: its functions, and calling it
/// makes one (`Gdk.RGBA { red = 1 }`, `Gdk.RGBA("#ff0000")`).
fn record_class(lua: &Lua, info: Info) -> mlua::Result<Table> {
    let key = info.full_name();
    let existing = get_table(lua, &crate::state(lua).classes, &key)?;
    if let Some(t) = existing {
        return Ok(t);
    }
    let table = lua.create_table()?;
    table.raw_set("_name", key.clone())?;
    if info.gtype().is_valid() && info.gtype() != glib::Type::UNIT {
        table.raw_set("_gtype", info.gtype().into_glib() as f64)?;
    }
    let meta = lua.create_table()?;
    let lookup = info.clone();
    meta.raw_set(
        "__index",
        lua.create_function(move |lua, (table, key): (Table, String)| {
            let name = std::ffi::CString::new(key.as_str()).map_err(mlua::Error::external)?;
            // SAFETY: valid infos; returns a new reference or null.
            let f = unsafe {
                Info::from_full(match lookup.kind() {
                    Kind::Union => sys::gi_union_info_find_method(lookup.ptr(), name.as_ptr()),
                    _ => sys::gi_struct_info_find_method(lookup.ptr(), name.as_ptr()),
                })
            };
            match f {
                Some(f) => {
                    let f = call::function(lua, &f)?;
                    // Cached in the metatable: instances only look for Lua
                    // functions in the table itself.
                    let cache: Table = table.metatable().expect("meta").raw_get("__cache")?;
                    cache.raw_set(key, f.clone())?;
                    Ok(mlua::Value::Function(f))
                }
                None => Ok(mlua::Value::Nil),
            }
        })?,
    )?;
    meta.raw_set("__cache", lua.create_table()?)?;
    meta.raw_set(
        "__newindex",
        lua.create_function(
            |lua, (table, key, value): (Table, mlua::Value, mlua::Value)| {
                table.raw_set(key, value)?;
                members::invalidate(lua);
                Ok(())
            },
        )?,
    )?;
    let make = info.clone();
    meta.raw_set(
        "__call",
        lua.create_function(move |lua, (_, init): (Table, mlua::Value)| {
            let ud = lua.create_userdata(Record::new_zeroed(make.clone())?)?;
            let owner = mlua::Value::UserData(ud.clone());
            match &init {
                mlua::Value::Nil => {}
                mlua::Value::Table(t) => {
                    let r = ud.borrow::<Record>()?;
                    record::fill(lua, &r, &owner, t)?;
                }
                mlua::Value::String(_) => {
                    let parse = members::record(lua, &make, "parse")?.ok_or_else(|| {
                        mlua::Error::runtime(format!(
                            "a {} can't be made from a string",
                            make.full_name()
                        ))
                    })?;
                    if !parse.call::<bool>((owner.clone(), init.clone()))? {
                        return Err(mlua::Error::runtime(format!(
                            "{:?} isn't a valid {}",
                            init.to_string().unwrap_or_default(),
                            make.full_name()
                        )));
                    }
                }
                other => {
                    return Err(mlua::Error::runtime(format!(
                        "a {} is made from a table of fields, not {}",
                        make.full_name(),
                        other.type_name()
                    )));
                }
            }
            Ok(owner)
        })?,
    )?;
    let name = key.clone();
    meta.raw_set(
        "__tostring",
        lua.create_function(move |_, _: mlua::Value| Ok(name.clone()))?,
    )?;
    table.set_metatable(Some(meta))?;
    let index: Function = lua
        .load(
            r#"
local compute, cache = ...
return function(t, k)
  local v = cache[k]
  if v ~= nil then return v end
  return compute(t, k)
end
"#,
        )
        .set_name("@gi-record")
        .call((
            table
                .metatable()
                .expect("meta")
                .raw_get::<Function>("__index")?,
            table
                .metatable()
                .expect("meta")
                .raw_get::<Table>("__cache")?,
        ))?;
    table.metatable().expect("meta").raw_set("__index", index)?;
    let stored = lua.create_registry_value(table.clone())?;
    crate::state(lua).classes.borrow_mut().insert(key, stored);
    Ok(table)
}
