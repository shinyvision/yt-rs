//! Conversions between Lua values, JSON and GObject values.

use gtk::glib;
use gtk::prelude::*;
use mlua::serde::SerializeOptions;
use mlua::{Lua, LuaSerdeExt};

use super::object::LuaObject;

/// JSON to Lua. JSON null becomes nil, so `if video.channel then` works.
pub fn json_to_lua(lua: &Lua, value: &serde_json::Value) -> mlua::Result<mlua::Value> {
    lua.to_value_with(
        value,
        SerializeOptions::new()
            .serialize_none_to_null(false)
            .serialize_unit_to_null(false),
    )
}

/// Lua to JSON. Userdata (such as widgets) can't be represented and becomes
/// null.
pub fn lua_to_json(lua: &Lua, value: mlua::Value) -> mlua::Result<serde_json::Value> {
    match value {
        mlua::Value::UserData(_) | mlua::Value::Function(_) | mlua::Value::Thread(_) => {
            Ok(serde_json::Value::Null)
        }
        other => lua.from_value(other),
    }
}

/// The fundamental type `ty` derives from (e.g. `GEnum` for any enum).
fn fundamental(ty: glib::Type) -> glib::Type {
    use glib::translate::{FromGlib, IntoGlib};
    // SAFETY: g_type_fundamental accepts any type id.
    unsafe { glib::Type::from_glib(glib::gobject_ffi::g_type_fundamental(ty.into_glib())) }
}

/// Converts a Lua value into a GObject value of type `ty`, for setting
/// properties and returning from signal handlers.
pub fn lua_to_glib(value: &mlua::Value, ty: glib::Type) -> mlua::Result<glib::Value> {
    let wrong = || {
        mlua::Error::runtime(format!(
            "expected a value of type {}, got {}",
            ty.name(),
            value.type_name()
        ))
    };
    let number = || {
        value
            .as_f64()
            .or_else(|| value.as_i64().map(|i| i as f64))
            .ok_or_else(wrong)
    };
    let fundamental = fundamental(ty);
    let converted = if fundamental == glib::Type::BOOL {
        (!matches!(value, mlua::Value::Nil | mlua::Value::Boolean(false))).to_value()
    } else if fundamental == glib::Type::I32 {
        (number()? as i32).to_value()
    } else if fundamental == glib::Type::U32 {
        (number()? as u32).to_value()
    } else if fundamental == glib::Type::I64 || fundamental == glib::Type::I_LONG {
        (number()? as i64).to_value()
    } else if fundamental == glib::Type::U64 || fundamental == glib::Type::U_LONG {
        (number()? as u64).to_value()
    } else if fundamental == glib::Type::F32 {
        (number()? as f32).to_value()
    } else if fundamental == glib::Type::F64 {
        number()?.to_value()
    } else if fundamental == glib::Type::STRING {
        match value {
            mlua::Value::Nil => None::<String>.to_value(),
            mlua::Value::String(s) => s.to_str()?.to_string().to_value(),
            mlua::Value::Integer(_) | mlua::Value::Number(_) => number()?.to_string().to_value(),
            _ => return Err(wrong()),
        }
    } else if fundamental == glib::Type::ENUM {
        let class = glib::EnumClass::with_type(ty).ok_or_else(wrong)?;
        let found = match value {
            mlua::Value::String(s) => {
                let s = s.to_str()?;
                class.value_by_nick(&s).or_else(|| class.value_by_name(&s))
            }
            mlua::Value::Integer(i) => class.value(*i as i32),
            _ => None,
        };
        let found = found
            .ok_or_else(|| mlua::Error::runtime(format!("{} has no value {value:?}", ty.name())))?;
        class.to_value(found.value()).ok_or_else(wrong)?
    } else if fundamental == glib::Type::FLAGS {
        let class = glib::FlagsClass::with_type(ty).ok_or_else(wrong)?;
        let bits = match value {
            mlua::Value::Integer(i) => *i as u32,
            mlua::Value::String(s) => {
                let mut bits = 0;
                for part in s
                    .to_str()?
                    .split('|')
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                {
                    let flag = class
                        .value_by_nick(part)
                        .or_else(|| class.value_by_name(part))
                        .ok_or_else(|| {
                            mlua::Error::runtime(format!("{} has no flag {part:?}", ty.name()))
                        })?;
                    bits |= flag.value();
                }
                bits
            }
            _ => return Err(wrong()),
        };
        class.to_value(bits).ok_or_else(wrong)?
    } else if fundamental == glib::Type::OBJECT || fundamental == glib::Type::INTERFACE {
        match value {
            mlua::Value::Nil => glib::Value::from_type(ty),
            mlua::Value::UserData(ud) => {
                let object = ud.borrow::<LuaObject>()?.0.clone();
                if !object.type_().is_a(ty) {
                    return Err(mlua::Error::runtime(format!(
                        "expected a {}, got a {}",
                        ty.name(),
                        object.type_().name()
                    )));
                }
                object.to_value()
            }
            _ => return Err(wrong()),
        }
    } else if ty == glib::StrV::static_type() {
        // A list of strings, e.g. `css_classes`.
        let table = value.as_table().ok_or_else(wrong)?;
        let items: Vec<String> = table
            .sequence_values::<String>()
            .collect::<mlua::Result<_>>()?;
        items.to_value()
    } else if ty == gtk::gdk::RGBA::static_type() {
        let text = value.as_string().ok_or_else(wrong)?.to_str()?.to_string();
        gtk::gdk::RGBA::parse(&text)
            .map_err(|_| mlua::Error::runtime(format!("invalid color {text:?}")))?
            .to_value()
    } else {
        return Err(mlua::Error::runtime(format!(
            "can't convert a Lua value to {}",
            ty.name()
        )));
    };
    Ok(converted)
}

/// Converts a GObject value to Lua. Types without a Lua equivalent become
/// their type name as a string.
pub fn glib_to_lua(lua: &Lua, value: &glib::Value) -> mlua::Result<mlua::Value> {
    let ty = value.type_();
    let fundamental = fundamental(ty);
    Ok(if fundamental == glib::Type::BOOL {
        mlua::Value::Boolean(value.get::<bool>().unwrap_or(false))
    } else if fundamental == glib::Type::I32 {
        mlua::Value::Integer(value.get::<i32>().unwrap_or(0) as _)
    } else if fundamental == glib::Type::U32 {
        mlua::Value::Integer(value.get::<u32>().unwrap_or(0) as _)
    } else if fundamental == glib::Type::I64 || fundamental == glib::Type::I_LONG {
        mlua::Value::Number(value.get::<i64>().unwrap_or(0) as f64)
    } else if fundamental == glib::Type::U64 || fundamental == glib::Type::U_LONG {
        mlua::Value::Number(value.get::<u64>().unwrap_or(0) as f64)
    } else if fundamental == glib::Type::F32 {
        mlua::Value::Number(value.get::<f32>().unwrap_or(0.0) as f64)
    } else if fundamental == glib::Type::F64 {
        mlua::Value::Number(value.get::<f64>().unwrap_or(0.0))
    } else if fundamental == glib::Type::STRING {
        match value.get::<Option<String>>().ok().flatten() {
            Some(s) => mlua::Value::String(lua.create_string(&s)?),
            None => mlua::Value::Nil,
        }
    } else if fundamental == glib::Type::ENUM {
        let nick = glib::EnumValue::from_value(value).map(|(_, v)| v.nick().to_owned());
        match nick {
            Some(n) => mlua::Value::String(lua.create_string(&n)?),
            None => mlua::Value::Nil,
        }
    } else if fundamental == glib::Type::FLAGS {
        let bits = glib::FlagsValue::from_value(value)
            .map(|(_, v)| v.iter().map(|f| f.value()).fold(0, |a, b| a | b))
            .unwrap_or(0);
        mlua::Value::Integer(bits as _)
    } else if fundamental == glib::Type::OBJECT || fundamental == glib::Type::INTERFACE {
        match value.get::<Option<glib::Object>>().ok().flatten() {
            Some(object) => mlua::Value::UserData(lua.create_userdata(LuaObject(object))?),
            None => mlua::Value::Nil,
        }
    } else if ty == glib::StrV::static_type() {
        let items = value.get::<Vec<String>>().unwrap_or_default();
        mlua::Value::Table(lua.create_sequence_from(items)?)
    } else if ty == gtk::gdk::RGBA::static_type() {
        let rgba = value
            .get::<gtk::gdk::RGBA>()
            .map(|c| c.to_string())
            .unwrap_or_default();
        mlua::Value::String(lua.create_string(&rgba)?)
    } else {
        mlua::Value::String(lua.create_string(ty.name())?)
    })
}
