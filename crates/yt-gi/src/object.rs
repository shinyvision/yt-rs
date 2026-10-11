//! Instances as Lua values: `Object` for GObjects (widgets, models, files,
//! anything), `Instance` for the other instance types (`GParamSpec`,
//! `GskRenderNode`, `GdkEvent`, `GtkExpression`).
//!
//! Members are looked up in this order:
//!
//! 1. the methods every object has (`on`, `emit`, `get`, `set`...),
//! 2. Lua functions added to its class or an ancestor's
//!    (`function Gtk.Widget:hello() end`),
//! 3. its properties (`button.label`),
//! 4. its methods from introspection (`button:set_label("Hi")`),
//! 5. fields of an instance of a Lua subclass.
//!
//! A property and a method of the same name (`has_focus`) is read as the
//! property; the method stays reachable as `Gtk.Widget.has_focus(w)`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;

use glib::prelude::*;
use glib::translate::FromGlib;
use mlua::{Function, Lua, MetaMethod, UserData, UserDataMethods};

use crate::members::{self, Member};
use crate::value::{glib_to_lua, lua_to_glib};
use crate::{report, subclass};

#[derive(Clone)]
pub struct Object(pub glib::Object);

/// An instance of a non-GObject instance type, holding one reference.
pub struct Instance {
    pub ptr: *mut c_void,
    pub gtype: glib::Type,
    unref: unsafe extern "C" fn(*mut c_void),
}

impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: releases the reference this value owns.
        unsafe { (self.unref)(self.ptr) }
    }
}

/// The ref and unref functions of a fundamental instance type, from its
/// introspection data or a parent's.
#[derive(Clone, Copy)]
struct RefFuncs {
    ref_: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    unref: unsafe extern "C" fn(*mut c_void),
}

thread_local! {
    static REF_FUNCS: RefCell<HashMap<glib::Type, Option<RefFuncs>>> = RefCell::new(HashMap::new());
}

fn ref_funcs(gtype: glib::Type) -> Option<RefFuncs> {
    if let Some(found) = REF_FUNCS.with(|r| r.borrow().get(&gtype).copied()) {
        return found;
    }
    let mut t = Some(gtype);
    let mut found = None;
    while let Some(ty) = t {
        if let Some(info) = crate::repo::find_by_gtype(ty)
            && info.kind() == crate::info::Kind::Object
        {
            // SAFETY: object infos have these names, possibly null.
            let (r, u) = unsafe {
                (
                    crate::info::static_str(crate::sys::gi_object_info_get_ref_function_name(
                        info.ptr(),
                    )),
                    crate::info::static_str(crate::sys::gi_object_info_get_unref_function_name(
                        info.ptr(),
                    )),
                )
            };
            if !r.is_empty()
                && !u.is_empty()
                && let (Some(r), Some(u)) = (info.symbol(r), info.symbol(u))
            {
                // SAFETY: the type's own ref and unref functions.
                found = Some(unsafe {
                    RefFuncs {
                        ref_: std::mem::transmute::<
                            *mut c_void,
                            unsafe extern "C" fn(*mut c_void) -> *mut c_void,
                        >(r),
                        unref: std::mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut c_void)>(
                            u,
                        ),
                    }
                });
                break;
            }
        }
        t = ty.parent();
    }
    REF_FUNCS.with(|r| r.borrow_mut().insert(gtype, found));
    found
}

/// The concrete type of an instance.
///
/// # Safety
/// `ptr` is a valid `GTypeInstance`.
pub unsafe fn instance_type(ptr: *mut c_void) -> glib::Type {
    // SAFETY: as the caller promises.
    unsafe {
        let class = (*(ptr as *mut glib::gobject_ffi::GTypeInstance)).g_class;
        glib::Type::from_glib((*class).g_type)
    }
}

/// Wraps an instance pointer as a Lua value. `full` means the caller hands
/// over a reference; otherwise one is added. A floating reference is sunk
/// and taken over only with `sink` (a function's return value), never for
/// objects passed to callbacks, which may be under construction.
pub fn wrap(lua: &Lua, ptr: *mut c_void, full: bool, sink: bool) -> mlua::Result<mlua::Value> {
    if ptr.is_null() {
        return Ok(mlua::Value::Nil);
    }
    // SAFETY: GI says this is an instance of a type.
    let gtype = unsafe { instance_type(ptr) };
    if gtype.is_a(glib::Type::OBJECT) {
        let raw = ptr as *mut glib::gobject_ffi::GObject;
        // SAFETY: a valid GObject; one reference ends up ours.
        // (Not glib-rs's from_glib_none: it sinks floating references, and
        // sinking one of an object still being constructed would leave
        // nobody owning it once this wrapper goes.)
        let object: glib::Object = unsafe {
            if sink && glib::gobject_ffi::g_object_is_floating(raw) != 0 {
                // The floating reference becomes ours.
                glib::gobject_ffi::g_object_ref_sink(raw);
            } else if !full {
                glib::gobject_ffi::g_object_ref(raw);
            }
            glib::translate::from_glib_full(raw)
        };
        return Ok(mlua::Value::UserData(lua.create_userdata(Object(object))?));
    }
    let Some(funcs) = ref_funcs(gtype) else {
        return Err(mlua::Error::runtime(format!(
            "can't hold a {}: it has no reference counting",
            gtype.name()
        )));
    };
    if !full {
        // SAFETY: the type's own ref function (it also sinks floating ones).
        unsafe { (funcs.ref_)(ptr) };
    }
    Ok(mlua::Value::UserData(lua.create_userdata(Instance {
        ptr,
        gtype,
        unref: funcs.unref,
    })?))
}

/// The instance pointer behind a Lua value, if it's an object or instance.
pub fn instance_ptr(value: &mlua::Value) -> Option<(*mut c_void, glib::Type)> {
    let mlua::Value::UserData(ud) = value else {
        return None;
    };
    if let Ok(o) = ud.borrow::<Object>() {
        return Some((o.0.as_ptr().cast(), o.0.type_()));
    }
    if let Ok(i) = ud.borrow::<Instance>() {
        return Some((i.ptr, i.gtype));
    }
    None
}

/// Adds a reference to an instance (for handing it over to C).
///
/// # Safety
/// `ptr` is a valid instance of `gtype`.
pub unsafe fn add_ref(ptr: *mut c_void, gtype: glib::Type) {
    // SAFETY: as the caller promises.
    unsafe {
        if gtype.is_a(glib::Type::OBJECT) {
            glib::gobject_ffi::g_object_ref(ptr.cast());
        } else if let Some(f) = ref_funcs(gtype) {
            (f.ref_)(ptr);
        }
    }
}

impl Object {
    fn set(&self, name: &str, value: &mlua::Value) -> mlua::Result<()> {
        let name = &name.replace('_', "-");
        let pspec = self.0.find_property(name).ok_or_else(|| {
            mlua::Error::runtime(format!(
                "{} has no property {name:?}",
                self.0.type_().name()
            ))
        })?;
        if !pspec.flags().contains(glib::ParamFlags::WRITABLE) {
            return Err(mlua::Error::runtime(format!(
                "property {name:?} is read-only"
            )));
        }
        let converted = lua_to_glib(value, pspec.value_type())
            .map_err(|e| mlua::Error::runtime(format!("property {name:?}: {e}")))?;
        self.0.set_property_from_value(name, &converted);
        Ok(())
    }

    fn get(&self, lua: &Lua, name: &str) -> mlua::Result<mlua::Value> {
        let name = &name.replace('_', "-");
        if self.0.find_property(name).is_none() {
            return Ok(mlua::Value::Nil);
        }
        glib_to_lua(lua, &self.0.property_value(name))
    }

    /// Connects a Lua function to a signal. The function gets the object
    /// and the signal's arguments; its return value is used if the signal
    /// expects one (e.g. `true` to stop an event).
    fn connect(&self, lua: &Lua, signal: &str, f: Function, after: bool) -> mlua::Result<u64> {
        let (id, _) = glib::subclass::signal::SignalId::parse_name(signal, self.0.type_(), true)
            .ok_or_else(|| {
                mlua::Error::runtime(format!(
                    "{} has no signal {signal:?}",
                    self.0.type_().name()
                ))
            })?;
        let return_type = id.query().return_type().type_();
        let lua = lua.clone();
        let handler = self.0.connect_closure(
            signal,
            after,
            glib::RustClosure::new_local(move |args| {
                let lua_args: mlua::Result<Vec<mlua::Value>> =
                    args.iter().map(|a| glib_to_lua(&lua, a)).collect();
                let result =
                    lua_args.and_then(|a| f.call::<mlua::Value>(mlua::MultiValue::from_iter(a)));
                let result = match result {
                    Ok(v) => v,
                    Err(e) => {
                        report(&format!("signal {:?} handler", id.name()), &e);
                        mlua::Value::Nil
                    }
                };
                if return_type == glib::Type::UNIT {
                    None
                } else {
                    Some(
                        lua_to_glib(&result, return_type)
                            .unwrap_or_else(|_| glib::Value::from_type(return_type)),
                    )
                }
            }),
        );
        // SAFETY: only reads the id of a handler that was just connected.
        let id = unsafe { handler.as_raw() };
        // A C `unsigned long`: 64 bits here, 32 on Windows.
        #[allow(clippy::useless_conversion)]
        Ok(u64::from(id))
    }

    /// Emits a signal with Lua arguments, returning its result.
    fn emit(&self, lua: &Lua, signal: &str, args: &[mlua::Value]) -> mlua::Result<mlua::Value> {
        let (id, _) = glib::subclass::signal::SignalId::parse_name(signal, self.0.type_(), true)
            .ok_or_else(|| {
                mlua::Error::runtime(format!(
                    "{} has no signal {signal:?}",
                    self.0.type_().name()
                ))
            })?;
        let query = id.query();
        let types = query.param_types();
        if args.len() != types.len() {
            return Err(mlua::Error::runtime(format!(
                "signal {signal:?} takes {} arguments, not {}",
                types.len(),
                args.len()
            )));
        }
        let values = args
            .iter()
            .zip(types)
            .map(|(a, t)| lua_to_glib(a, t.type_()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| mlua::Error::runtime(format!("signal {signal:?}: {e}")))?;
        match self.0.emit_by_name_with_values(signal, &values) {
            Some(result) => glib_to_lua(lua, &result),
            None => Ok(mlua::Value::Nil),
        }
    }

    fn disconnect(&self, id: u64) {
        // SAFETY: g_signal_handler_disconnect ignores ids that aren't
        // connected to this instance (with a warning).
        unsafe {
            glib::gobject_ffi::g_signal_handler_disconnect(self.0.as_ptr() as *mut _, id as _)
        };
    }
}

fn index(
    lua: &Lua,
    ud: &mlua::AnyUserData,
    gtype: glib::Type,
    key: &mlua::Value,
) -> mlua::Result<mlua::Value> {
    let Some(key) = key.as_string().and_then(|s| s.to_str().ok()) else {
        return Ok(mlua::Value::Nil);
    };
    match members::lookup(lua, gtype, &key)? {
        Member::Function(f) => Ok(mlua::Value::Function(f)),
        Member::Property => {
            let this = ud.borrow::<Object>()?;
            this.get(lua, &key)
        }
        Member::Field(field) => {
            let (ptr, _) = instance_ptr(&mlua::Value::UserData(ud.clone()))
                .ok_or_else(|| mlua::Error::runtime("not an instance"))?;
            field.get(lua, ptr.cast(), &mlua::Value::UserData(ud.clone()))
        }
        Member::Nothing => subclass::get_field(lua, ud, &key),
    }
}

impl UserData for Object {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("set", |_, this, (name, value): (String, mlua::Value)| {
            this.set(&name, &value)
        });
        methods.add_method("get", |lua, this, name: String| this.get(lua, &name));
        methods.add_method("on", |lua, this, (signal, f): (String, Function)| {
            this.connect(lua, &signal, f, false)
        });
        methods.add_method("on_after", |lua, this, (signal, f): (String, Function)| {
            this.connect(lua, &signal, f, true)
        });
        methods.add_method(
            "emit",
            |lua, this, (signal, args): (String, mlua::Variadic<mlua::Value>)| {
                this.emit(lua, &signal, &args)
            },
        );
        methods.add_method("off", |_, this, id: u64| {
            this.disconnect(id);
            Ok(())
        });
        methods.add_method("type_name", |_, this, ()| {
            Ok(this.0.type_().name().to_owned())
        });
        methods.add_method("is_a", |_, this, ty: mlua::Value| {
            Ok(crate::gtype_from_lua(&ty).is_ok_and(|t| this.0.type_().is_a(t)))
        });
        methods.add_method("properties", |_, this, ()| {
            Ok(this
                .0
                .list_properties()
                .iter()
                .map(|p| p.name().to_owned())
                .collect::<Vec<_>>())
        });
        // The C pointer, for LuaJIT's FFI: `ffi.cast("GtkWidget*", w:ptr())`.
        methods.add_method("ptr", |_, this, ()| {
            Ok(mlua::LightUserData(this.0.as_ptr().cast()))
        });

        methods.add_meta_function(
            MetaMethod::Index,
            |lua, (ud, key): (mlua::AnyUserData, mlua::Value)| {
                let gtype = ud.borrow::<Object>()?.0.type_();
                index(lua, &ud, gtype, &key)
            },
        );
        methods.add_meta_function(
            MetaMethod::NewIndex,
            |lua, (ud, key, value): (mlua::AnyUserData, String, mlua::Value)| {
                let this = ud.borrow::<Object>()?.clone();
                if this.0.find_property(&key.replace('_', "-")).is_some() {
                    return this.set(&key, &value);
                }
                subclass::set_field(lua, &ud, &this.0, &key, value)
            },
        );
        methods.add_meta_method(MetaMethod::Eq, |_, this, other: mlua::Value| {
            Ok(instance_ptr(&other).is_some_and(|(p, _)| p == this.0.as_ptr().cast()))
        });
        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            let name = this
                .0
                .find_property("name")
                .filter(|p| p.value_type() == glib::Type::STRING)
                .and_then(|_| this.0.property::<Option<String>>("name"))
                .unwrap_or_default();
            Ok(if name.is_empty() {
                this.0.type_().name().to_owned()
            } else {
                format!("{}({name})", this.0.type_().name())
            })
        });
    }
}

impl UserData for Instance {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("type_name", |_, this, ()| Ok(this.gtype.name().to_owned()));
        methods.add_method("is_a", |_, this, ty: mlua::Value| {
            Ok(crate::gtype_from_lua(&ty).is_ok_and(|t| this.gtype.is_a(t)))
        });
        methods.add_method("ptr", |_, this, ()| Ok(mlua::LightUserData(this.ptr)));
        methods.add_meta_function(
            MetaMethod::Index,
            |lua, (ud, key): (mlua::AnyUserData, mlua::Value)| {
                let gtype = ud.borrow::<Instance>()?.gtype;
                index(lua, &ud, gtype, &key)
            },
        );
        methods.add_meta_method(MetaMethod::Eq, |_, this, other: mlua::Value| {
            Ok(instance_ptr(&other).is_some_and(|(p, _)| p == this.ptr))
        });
        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(this.gtype.name().to_owned())
        });
    }
}

/// Wraps a raw GObject pointer (e.g. from LuaJIT's FFI), checking that it
/// really is a GObject.
pub fn from_ptr(ptr: *mut c_void) -> mlua::Result<Object> {
    if ptr.is_null() {
        return Err(mlua::Error::runtime("null pointer"));
    }
    let instance = ptr as *mut glib::gobject_ffi::GTypeInstance;
    // SAFETY: checks the instance's type before treating it as a GObject.
    let is_object = unsafe {
        glib::gobject_ffi::g_type_check_instance_is_a(instance, glib::gobject_ffi::G_TYPE_OBJECT)
    } != 0;
    if !is_object {
        return Err(mlua::Error::runtime("the pointer is not a GObject"));
    }
    let raw = ptr as *mut glib::gobject_ffi::GObject;
    // SAFETY: a valid GObject. New widgets from C constructors carry a
    // floating reference; sink it and take it over, otherwise add one.
    let object: glib::Object = unsafe {
        if glib::gobject_ffi::g_object_is_floating(raw) != glib::ffi::GFALSE {
            glib::gobject_ffi::g_object_ref_sink(raw);
            glib::translate::from_glib_full(raw)
        } else {
            glib::translate::from_glib_none(raw)
        }
    };
    Ok(Object(object))
}
