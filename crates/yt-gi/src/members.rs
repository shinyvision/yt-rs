//! Finding what a name means on an instance or a record, cached per type.

use std::rc::Rc;

use glib::prelude::*;
use mlua::{Function, Lua, RegistryKey};

use crate::call;
use crate::info::{Info, Kind};
use crate::namespace;
use crate::record::{self, Field};
use crate::repo;
use crate::sys;

pub enum Member {
    Function(Function),
    Property,
    Field(Rc<Field>),
    Nothing,
}

#[derive(Clone)]
pub(crate) enum Cached {
    Function(Rc<RegistryKey>),
    Property,
    Field(Rc<Field>),
    Nothing,
}

/// Forgets what was found: a class got a new function.
pub fn invalidate(lua: &Lua) {
    let state = crate::state(lua);
    state.members.borrow_mut().clear();
    state.records.borrow_mut().clear();
}

/// A type and its ancestors, nearest first.
pub fn ancestors(gtype: glib::Type) -> impl Iterator<Item = glib::Type> {
    std::iter::successors(Some(gtype), |t| t.parent())
}

/// Every interface a type implements, its ancestors' too.
pub fn interfaces(gtype: glib::Type) -> Vec<glib::Type> {
    let mut out: Vec<glib::Type> = Vec::new();
    for t in ancestors(gtype) {
        for i in t.interfaces().iter() {
            if !out.contains(i) {
                out.push(*i);
            }
        }
    }
    out
}

fn is_method(f: &Info) -> bool {
    // SAFETY: a function info.
    unsafe { sys::gi_function_info_get_flags(f.ptr()) & sys::FUNCTION_IS_METHOD != 0 }
}

fn find_method(info: &Info, name: &str) -> Option<Info> {
    let name = std::ffi::CString::new(name).ok()?;
    // SAFETY: valid infos; returns a new reference or null.
    unsafe {
        Info::from_full(match info.kind() {
            Kind::Object => sys::gi_object_info_find_method(info.ptr(), name.as_ptr()),
            Kind::Interface => sys::gi_interface_info_find_method(info.ptr(), name.as_ptr()),
            Kind::Struct => sys::gi_struct_info_find_method(info.ptr(), name.as_ptr()),
            Kind::Union => sys::gi_union_info_find_method(info.ptr(), name.as_ptr()),
            _ => return None,
        })
    }
}

/// A method (or, with `statics`, any function) named `name` of a type or
/// its ancestors and interfaces, by introspection.
pub fn gi_function(gtype: glib::Type, name: &str, statics: bool) -> Option<Info> {
    let wanted = |f: &Info| statics || is_method(f);
    for t in ancestors(gtype) {
        if let Some(info) = repo::find_by_gtype(t)
            && let Some(f) = find_method(&info, name).filter(wanted)
        {
            return Some(f);
        }
    }
    for i in interfaces(gtype) {
        if let Some(info) = repo::find_by_gtype(i)
            && let Some(f) = find_method(&info, name).filter(wanted)
        {
            return Some(f);
        }
    }
    None
}

fn has_property(gtype: glib::Type, name: &str) -> bool {
    if !gtype.is_a(glib::Type::OBJECT) {
        return false;
    }
    glib::object::ObjectClass::from_type(gtype)
        .is_some_and(|c| c.find_property(&name.replace('_', "-")).is_some())
}

fn compute(lua: &Lua, gtype: glib::Type, key: &str) -> mlua::Result<Cached> {
    // 1. Lua functions added to the class or an ancestor.
    for t in ancestors(gtype).chain(interfaces(gtype)) {
        if let Some(class) = namespace::existing_class(lua, t)?
            && let mlua::Value::Function(f) = class.raw_get::<mlua::Value>(key)?
        {
            return Ok(Cached::Function(Rc::new(lua.create_registry_value(f)?)));
        }
    }
    // 2. Properties.
    if has_property(gtype, key) {
        return Ok(Cached::Property);
    }
    // 3. Methods.
    if let Some(f) = gi_function(gtype, key, false) {
        let f = call::function(lua, &f)?;
        return Ok(Cached::Function(Rc::new(lua.create_registry_value(f)?)));
    }
    // 4. Fields of instance types (`pspec.name`).
    if !gtype.is_a(glib::Type::OBJECT) {
        for t in ancestors(gtype) {
            if let Some(info) = repo::find_by_gtype(t)
                && let Some(f) = record::field(&info, key)
            {
                return Ok(Cached::Field(f));
            }
        }
    }
    Ok(Cached::Nothing)
}

pub fn lookup(lua: &Lua, gtype: glib::Type, key: &str) -> mlua::Result<Member> {
    let cache_key = (gtype, key.to_owned());
    let existing = crate::state(lua).members.borrow().get(&cache_key).cloned();
    let cached = match existing {
        Some(c) => c,
        None => {
            let c = compute(lua, gtype, key)?;
            crate::state(lua)
                .members
                .borrow_mut()
                .insert(cache_key, c.clone());
            c
        }
    };
    Ok(match cached {
        Cached::Function(k) => Member::Function(lua.registry_value(&k)?),
        Cached::Property => Member::Property,
        Cached::Field(f) => Member::Field(f),
        Cached::Nothing => Member::Nothing,
    })
}

/// A method of a record type: one added in Lua, or by introspection.
pub fn record(lua: &Lua, info: &Info, key: &str) -> mlua::Result<Option<Function>> {
    let cache_key = (info.full_name(), key.to_owned());
    let existing = crate::state(lua).records.borrow().get(&cache_key).cloned();
    if let Some(found) = existing {
        return found.map(|k| lua.registry_value(&k)).transpose();
    }
    let mut found = None;
    if let Some(class) = namespace::existing_record_class(lua, info)?
        && let mlua::Value::Function(f) = class.raw_get::<mlua::Value>(key)?
    {
        found = Some(f);
    }
    if found.is_none()
        && let Some(m) = find_method(info, key).filter(is_method)
    {
        found = Some(call::function(lua, &m)?);
    }
    let stored = found
        .as_ref()
        .map(|f| lua.create_registry_value(f.clone()))
        .transpose()?
        .map(Rc::new);
    crate::state(lua)
        .records
        .borrow_mut()
        .insert(cache_key, stored);
    Ok(found)
}
