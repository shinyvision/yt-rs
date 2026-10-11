//! GObject Introspection for Lua (LuaJIT, through mlua): every library
//! that ships introspection data — GTK, GLib, Gio, libadwaita, Pango,
//! GStreamer, libsoup, WebKit... — from Lua, without bindings written for
//! it.
//!
//! ```lua
//! local Gtk = gi.require("Gtk", "4.0")
//! local button = Gtk.Button.new_with_label("Hi")    -- functions
//! button:set_icon_name("emblem-ok-symbolic")        -- methods
//! button.tooltip_text = "Says hi"                   -- properties
//! button:on("clicked", function() print("hi") end)  -- signals
//! local Swatch = gi.subclass(Gtk.Widget, { ... })   -- new types
//! ```
//!
//! Everything runs on the thread that called [`install`].

mod cairo;
mod call;
mod closure;
mod convert;
mod enums;
mod info;
mod members;
mod namespace;
mod object;
mod record;
mod repo;
mod subclass;
mod sys;
mod types;
mod value;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

use glib::prelude::*;
use glib::translate::FromGlib;
use mlua::{Lua, RegistryKey, Table};

pub use cairo::LuaCairo;
pub use object::{Instance, Object, from_ptr};
pub use record::Record;
pub use repo::add_search_path;
pub use value::{glib_to_lua, lua_to_glib};

/// How errors in callbacks (Lua functions called from C) are reported.
static REPORTER: OnceLock<fn(&str, &mlua::Error)> = OnceLock::new();

/// Sets how errors in callbacks are reported (logged, shown); by default
/// they're printed.
pub fn set_reporter(report: fn(&str, &mlua::Error)) {
    let _ = REPORTER.set(report);
}

pub(crate) fn report(context: &str, error: &mlua::Error) {
    match REPORTER.get() {
        Some(r) => r(context, error),
        None => eprintln!("{context}: {error}"),
    }
}

/// What one Lua state holds on to: its namespace and class tables, and
/// the Lua functions made for GI functions.
#[derive(Default)]
pub(crate) struct State {
    pub functions: RefCell<HashMap<String, RegistryKey>>,
    pub rethrow: RefCell<Option<RegistryKey>>,
    pub namespaces: RefCell<HashMap<String, RegistryKey>>,
    pub classes: RefCell<HashMap<String, RegistryKey>>,
    pub members: RefCell<HashMap<(glib::Type, String), members::Cached>>,
    pub records: RefCell<RecordMembers>,
    /// Lua fields of instances of Lua classes, by instance id.
    pub fields: RefCell<HashMap<u64, RegistryKey>>,
}

/// Methods found for record types, by type and name.
pub(crate) type RecordMembers = HashMap<(String, String), Option<Rc<RegistryKey>>>;

pub(crate) fn state(lua: &Lua) -> mlua::AppDataRef<'_, State> {
    lua.app_data_ref::<State>()
        .expect("gi is installed in this Lua state")
}

/// Installs the `gi` table into `lua`'s globals and returns it.
pub fn install(lua: &Lua) -> mlua::Result<Table> {
    closure::set_main_thread();
    lua.set_app_data(State::default());
    for ns in ["GLib", "GObject", "Gio"] {
        repo::require(ns, None).map_err(mlua::Error::runtime)?;
    }
    let gi = lua.create_table()?;
    gi.set(
        "require",
        lua.create_function(|lua, (ns, version): (String, Option<String>)| {
            namespace::require(lua, &ns, version.as_deref())
        })?,
    )?;
    // The class table of any type, by name, GType or instance: for types
    // without introspection data (defined in Rust) too.
    gi.set(
        "type",
        lua.create_function(|lua, ty: mlua::Value| {
            let gtype = gtype_from_lua(&ty)?;
            namespace::class_table(lua, gtype)
        })?,
    )?;
    gi.set(
        "subclass",
        lua.create_function(|lua, (parent, def): (mlua::Value, Table)| {
            subclass::subclass(lua, &parent, def)
        })?,
    )?;
    gi.set("loaded", lua.create_function(|_, ()| Ok(repo::loaded()))?)?;
    // An instance from a plain pointer: what callbacks typed `gpointer`
    // get (list items in sort and filter functions), or LuaJIT's FFI.
    gi.set(
        "wrap",
        lua.create_function(|lua, p: mlua::LightUserData| {
            if p.0.is_null() {
                return Ok(mlua::Value::Nil);
            }
            // SAFETY: checks the pointer is a type instance first.
            let is_instance = unsafe {
                glib::gobject_ffi::g_type_check_instance(
                    p.0 as *mut glib::gobject_ffi::GTypeInstance,
                ) != 0
            };
            if !is_instance {
                return Err(mlua::Error::runtime(
                    "the pointer isn't an instance of a type",
                ));
            }
            object::wrap(lua, p.0, false, true)
        })?,
    )?;
    // What a type has: its methods (with ancestors' and interfaces'),
    // properties and signals, for finding one's way around.
    gi.set(
        "describe",
        lua.create_function(|lua, ty: mlua::Value| {
            let gtype = gtype_from_lua(&ty)?;
            describe(lua, gtype)
        })?,
    )?;
    lua.globals().set("gi", gi.clone())?;
    Ok(gi)
}

/// Loads a namespace from Rust (e.g. the toolkit the host uses), so its
/// objects have their methods before any plugin asks for it.
pub fn require(lua: &Lua, namespace: &str, version: &str) -> mlua::Result<Table> {
    namespace::require(lua, namespace, Some(version))
}

/// The class table of a type, for adding methods to it from Rust:
/// `class(lua, gtk::Widget::static_type())?.set("find", f)`.
pub fn class(lua: &Lua, gtype: glib::Type) -> mlua::Result<Table> {
    namespace::class_table(lua, gtype)
}

/// A GType from Lua: a number, a type name, a class table, or an instance.
pub fn gtype_from_lua(value: &mlua::Value) -> mlua::Result<glib::Type> {
    let found = match value {
        mlua::Value::Integer(i) => Some(unsafe { glib::Type::from_glib(*i as usize) }),
        mlua::Value::Number(n) => Some(unsafe { glib::Type::from_glib(*n as usize) }),
        mlua::Value::String(s) => {
            let name = s.to_str()?;
            repo::type_by_name(&name).or_else(|| builtin_type(&name))
        }
        // SAFETY (all three): GTypes are checked for validity below.
        mlua::Value::Table(t) => t
            .raw_get::<Option<f64>>("_gtype")?
            .map(|n| unsafe { glib::Type::from_glib(n as usize) }),
        other => object::instance_ptr(other).map(|(_, t)| t),
    };
    match found {
        Some(t) if t.is_valid() => Ok(t),
        _ => Err(mlua::Error::runtime(format!(
            "not a type: {}",
            value
                .to_string()
                .unwrap_or_else(|_| value.type_name().to_owned())
        ))),
    }
}

/// Fundamental types by their C names, which GLib knows by other names
/// (`gint` is "gint", but `int` isn't).
fn builtin_type(name: &str) -> Option<glib::Type> {
    Some(match name {
        "bool" | "boolean" => glib::Type::BOOL,
        "int" | "int32" => glib::Type::I32,
        "uint" | "uint32" => glib::Type::U32,
        "int64" => glib::Type::I64,
        "uint64" => glib::Type::U64,
        "float" => glib::Type::F32,
        "double" | "number" => glib::Type::F64,
        "string" | "utf8" => glib::Type::STRING,
        "object" => glib::Type::OBJECT,
        "pointer" => glib::Type::POINTER,
        "variant" => glib::Type::VARIANT,
        _ => return None,
    })
}

/// A new instance of `gtype`, with properties from `props`.
pub fn new_object(_lua: &Lua, gtype: glib::Type, props: Option<&Table>) -> mlua::Result<Object> {
    if !gtype.is_a(glib::Type::OBJECT) {
        return Err(mlua::Error::runtime(format!(
            "{} isn't a GObject class",
            gtype.name()
        )));
    }
    // SAFETY: reads the type's flags.
    if unsafe {
        glib::gobject_ffi::g_type_test_flags(
            glib::translate::IntoGlib::into_glib(gtype),
            glib::gobject_ffi::G_TYPE_FLAG_ABSTRACT,
        )
    } != 0
    {
        return Err(mlua::Error::runtime(format!(
            "{} is abstract: make one of its subclasses",
            gtype.name()
        )));
    }
    let class = glib::object::ObjectClass::from_type(gtype)
        .ok_or_else(|| mlua::Error::runtime(format!("{} has no class", gtype.name())))?;
    let mut values = Vec::new();
    if let Some(props) = props {
        for pair in props.pairs::<String, mlua::Value>() {
            let (name, value) = pair?;
            let prop = name.replace('_', "-");
            let pspec = class.find_property(&prop).ok_or_else(|| {
                mlua::Error::runtime(format!("{} has no property {name:?}", gtype.name()))
            })?;
            let v = lua_to_glib(&value, pspec.value_type())
                .map_err(|e| mlua::Error::runtime(format!("property {name:?}: {e}")))?;
            values.push((pspec.name(), v));
        }
    }
    let mut refs: Vec<(&str, glib::Value)> = values.into_iter().collect();
    Ok(Object(glib::Object::with_mut_values(gtype, &mut refs)))
}

fn describe(lua: &Lua, gtype: glib::Type) -> mlua::Result<Table> {
    let out = lua.create_table()?;
    let methods = lua.create_table()?;
    let mut seen = std::collections::HashSet::new();
    let mut add = |info: &info::Info, methods: &Table| -> mlua::Result<()> {
        let list: Box<dyn Iterator<Item = info::Info>> = match info.kind() {
            info::Kind::Object => Box::new(info::children(
                info,
                sys::gi_object_info_get_n_methods,
                sys::gi_object_info_get_method,
            )),
            _ => Box::new(std::iter::empty()),
        };
        for m in list {
            if seen.insert(m.name()) {
                methods.raw_set(methods.raw_len() + 1, m.name())?;
            }
        }
        Ok(())
    };
    for t in members::ancestors(gtype) {
        if let Some(info) = repo::find_by_gtype(t) {
            add(&info, &methods)?;
        }
    }
    out.set("methods", methods)?;
    if gtype.is_a(glib::Type::OBJECT)
        && let Some(class) = glib::object::ObjectClass::from_type(gtype)
    {
        let props: Vec<String> = class
            .list_properties()
            .iter()
            .map(|p| p.name().replace('-', "_"))
            .collect();
        out.set("properties", props)?;
    }
    let mut signals = Vec::new();
    for t in members::ancestors(gtype).chain(members::interfaces(gtype)) {
        let mut n = 0;
        // SAFETY: lists a type's signal ids into memory we free.
        unsafe {
            let ids = glib::gobject_ffi::g_signal_list_ids(
                glib::translate::IntoGlib::into_glib(t),
                &mut n,
            );
            for i in 0..n as usize {
                let name = glib::gobject_ffi::g_signal_name(*ids.add(i));
                signals.push(
                    std::ffi::CStr::from_ptr(name)
                        .to_string_lossy()
                        .into_owned(),
                );
            }
            glib::ffi::g_free(ids.cast());
        }
    }
    out.set("signals", signals)?;
    Ok(out)
}
