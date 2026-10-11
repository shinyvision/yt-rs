//! `GValue`s (properties, signal arguments) to and from Lua, by their
//! GType; structs and other instance types through introspection.

use std::ffi::c_void;

use glib::gobject_ffi as go;
use glib::prelude::*;
use glib::translate::{FromGlib, IntoGlib, ToGlibPtr, ToGlibPtrMut};
use mlua::Lua;

use crate::object;
use crate::record::{Keep, Record};

/// The fundamental type `ty` derives from (e.g. `GEnum` for any enum).
fn fundamental(ty: glib::Type) -> glib::Type {
    // SAFETY: g_type_fundamental accepts any type id.
    unsafe { glib::Type::from_glib(go::g_type_fundamental(ty.into_glib())) }
}

/// The introspection data of a struct type, if loaded.
fn struct_info(ty: glib::Type) -> Option<crate::info::Info> {
    crate::repo::find_by_gtype(ty).filter(|i| {
        matches!(
            i.kind(),
            crate::info::Kind::Struct | crate::info::Kind::Union
        )
    })
}

/// Converts a GObject value to Lua.
pub fn glib_to_lua(lua: &Lua, value: &glib::Value) -> mlua::Result<mlua::Value> {
    // SAFETY: a valid GValue.
    unsafe { value_to_lua(lua, value.to_glib_none().0) }
}

/// Converts the GValue at `value` to Lua.
///
/// # Safety
/// `value` is an initialized GValue.
pub unsafe fn value_to_lua(lua: &Lua, value: *const go::GValue) -> mlua::Result<mlua::Value> {
    // SAFETY: as the caller promises; each getter matches the type.
    unsafe {
        let ty = glib::Type::from_glib((*value).g_type);
        let f = fundamental(ty);
        let v = value as *mut go::GValue;
        Ok(match f {
            glib::Type::BOOL => mlua::Value::Boolean(go::g_value_get_boolean(v) != 0),
            glib::Type::I8 => mlua::Value::Integer(go::g_value_get_schar(v) as i64),
            glib::Type::U8 => mlua::Value::Integer(go::g_value_get_uchar(v) as i64),
            glib::Type::I32 => mlua::Value::Integer(go::g_value_get_int(v) as i64),
            glib::Type::U32 => mlua::Value::Integer(go::g_value_get_uint(v) as i64),
            glib::Type::I_LONG => mlua::Value::Number(go::g_value_get_long(v) as f64),
            glib::Type::U_LONG => mlua::Value::Number(go::g_value_get_ulong(v) as f64),
            glib::Type::I64 => mlua::Value::Number(go::g_value_get_int64(v) as f64),
            glib::Type::U64 => mlua::Value::Number(go::g_value_get_uint64(v) as f64),
            glib::Type::F32 => mlua::Value::Number(go::g_value_get_float(v) as f64),
            glib::Type::F64 => mlua::Value::Number(go::g_value_get_double(v)),
            glib::Type::STRING => {
                let s = go::g_value_get_string(v);
                if s.is_null() {
                    mlua::Value::Nil
                } else {
                    mlua::Value::String(lua.create_string(std::ffi::CStr::from_ptr(s).to_bytes())?)
                }
            }
            glib::Type::ENUM => {
                let n = go::g_value_get_enum(v);
                let class = glib::EnumClass::with_type(ty);
                match class.as_ref().and_then(|c| c.value(n)) {
                    Some(e) => mlua::Value::String(lua.create_string(e.nick())?),
                    None => mlua::Value::Integer(n as i64),
                }
            }
            glib::Type::FLAGS => mlua::Value::Integer(go::g_value_get_flags(v) as i64),
            glib::Type::OBJECT | glib::Type::INTERFACE => {
                object::wrap(lua, go::g_value_get_object(v).cast(), false, false)?
            }
            glib::Type::POINTER => {
                let p = go::g_value_get_pointer(v);
                if p.is_null() {
                    mlua::Value::Nil
                } else {
                    mlua::Value::LightUserData(mlua::LightUserData(p))
                }
            }
            glib::Type::VARIANT => {
                let p = go::g_value_get_variant(v);
                match (p.is_null(), crate::repo::find_by_name("GLib", "Variant")) {
                    (false, Some(info)) => mlua::Value::UserData(
                        lua.create_userdata(Record::from_gi(p.cast(), info, false, Keep::Nothing))?,
                    ),
                    _ => mlua::Value::Nil,
                }
            }
            glib::Type::BOXED => {
                let p = go::g_value_get_boxed(v);
                if p.is_null() {
                    mlua::Value::Nil
                } else if ty == crate::types::strv_type() {
                    let strv = p as *const *const std::ffi::c_char;
                    let out = lua.create_table()?;
                    let mut i = 0;
                    while !(*strv.add(i)).is_null() {
                        out.raw_set(
                            i + 1,
                            lua.create_string(std::ffi::CStr::from_ptr(*strv.add(i)).to_bytes())?,
                        )?;
                        i += 1;
                    }
                    mlua::Value::Table(out)
                } else if let Some(info) = struct_info(ty) {
                    if let Some(v) = crate::cairo::to_lua(lua, p, &info, false)? {
                        v
                    } else {
                        mlua::Value::UserData(lua.create_userdata(Record::from_gi(
                            p,
                            info,
                            false,
                            Keep::Nothing,
                        ))?)
                    }
                } else {
                    mlua::Value::LightUserData(mlua::LightUserData(p))
                }
            }
            glib::Type::PARAM_SPEC => {
                object::wrap(lua, go::g_value_get_param(v).cast(), false, false)?
            }
            _ if ty == crate::types::gtype_type() => {
                mlua::Value::Number(glib::gobject_ffi::g_value_get_gtype(v) as f64)
            }
            _ if go::g_type_check_is_value_type(ty.into_glib()) != 0
                && glib::gobject_ffi::g_type_test_flags(
                    f.into_glib(),
                    go::G_TYPE_FLAG_INSTANTIATABLE,
                ) != 0 =>
            {
                // Another instance type (GskRenderNode, GdkEvent...).
                object::wrap(lua, go::g_value_peek_pointer(v), false, false)?
            }
            _ => mlua::Value::String(lua.create_string(ty.name())?),
        })
    }
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
            .or_else(|| {
                value
                    .as_string()
                    .and_then(|s| s.to_str().ok()?.trim().parse().ok())
            })
            .ok_or_else(wrong)
    };
    let f = fundamental(ty);
    let mut out = glib::Value::from_type(ty);
    let v: *mut go::GValue = out.to_glib_none_mut().0;
    // SAFETY: each setter matches the value's type.
    unsafe {
        match f {
            glib::Type::BOOL => go::g_value_set_boolean(
                v,
                !matches!(value, mlua::Value::Nil | mlua::Value::Boolean(false)) as i32,
            ),
            glib::Type::I8 => go::g_value_set_schar(v, number()? as i8),
            glib::Type::U8 => go::g_value_set_uchar(v, number()? as u8),
            glib::Type::I32 => go::g_value_set_int(v, number()? as i32),
            glib::Type::U32 => go::g_value_set_uint(v, number()? as u32),
            glib::Type::I_LONG => go::g_value_set_long(v, number()? as _),
            glib::Type::U_LONG => go::g_value_set_ulong(v, number()? as _),
            glib::Type::I64 => go::g_value_set_int64(v, number()? as i64),
            glib::Type::U64 => go::g_value_set_uint64(v, number()? as u64),
            glib::Type::F32 => go::g_value_set_float(v, number()? as f32),
            glib::Type::F64 => go::g_value_set_double(v, number()?),
            glib::Type::STRING => match value {
                mlua::Value::Nil => {}
                mlua::Value::String(s) => {
                    let s = std::ffi::CString::new(s.as_bytes().to_vec())
                        .map_err(|_| mlua::Error::runtime("a string with a NUL byte"))?;
                    go::g_value_set_string(v, s.as_ptr());
                }
                mlua::Value::Integer(_) | mlua::Value::Number(_) => {
                    let s = std::ffi::CString::new(number()?.to_string()).expect("no NUL");
                    go::g_value_set_string(v, s.as_ptr());
                }
                _ => return Err(wrong()),
            },
            glib::Type::ENUM => {
                let class = glib::EnumClass::with_type(ty).ok_or_else(wrong)?;
                let found = match value {
                    mlua::Value::String(s) => {
                        let s = s.to_str()?;
                        let norm = crate::enums::normalize(&s);
                        class
                            .values()
                            .iter()
                            .find(|e| {
                                crate::enums::normalize(e.nick()) == norm
                                    || e.name().eq_ignore_ascii_case(&s)
                            })
                            .map(|e| e.value())
                    }
                    mlua::Value::Integer(i) => class.value(*i as i32).map(|e| e.value()),
                    mlua::Value::Number(n) => class.value(*n as i32).map(|e| e.value()),
                    _ => None,
                };
                let found = found.ok_or_else(|| {
                    let known: Vec<&str> = class.values().iter().map(|e| e.nick()).collect();
                    mlua::Error::runtime(format!(
                        "{} has no value {} (it has {})",
                        ty.name(),
                        value.to_string().unwrap_or_default(),
                        known.join(", ")
                    ))
                })?;
                go::g_value_set_enum(v, found);
            }
            glib::Type::FLAGS => {
                let class = glib::FlagsClass::with_type(ty).ok_or_else(wrong)?;
                let one = |part: &str| -> mlua::Result<u32> {
                    let norm = crate::enums::normalize(part);
                    class
                        .values()
                        .iter()
                        .find(|f| {
                            crate::enums::normalize(f.nick()) == norm
                                || f.name().eq_ignore_ascii_case(part)
                        })
                        .map(|f| f.value())
                        .ok_or_else(|| {
                            mlua::Error::runtime(format!("{} has no flag {part:?}", ty.name()))
                        })
                };
                let bits = match value {
                    mlua::Value::Nil => 0,
                    mlua::Value::Integer(i) => *i as u32,
                    mlua::Value::Number(n) => *n as u32,
                    mlua::Value::String(s) => {
                        let mut bits = 0;
                        for part in s
                            .to_str()?
                            .split('|')
                            .map(str::trim)
                            .filter(|p| !p.is_empty())
                        {
                            bits |= one(part)?;
                        }
                        bits
                    }
                    mlua::Value::Table(t) => {
                        let mut bits = 0;
                        for part in t.sequence_values::<String>() {
                            bits |= one(&part?)?;
                        }
                        bits
                    }
                    _ => return Err(wrong()),
                };
                go::g_value_set_flags(v, bits);
            }
            glib::Type::OBJECT | glib::Type::INTERFACE => match value {
                mlua::Value::Nil => {}
                other => {
                    let (p, actual) = object::instance_ptr(other).ok_or_else(wrong)?;
                    if !actual.is_a(ty) {
                        return Err(mlua::Error::runtime(format!(
                            "expected a {}, got a {}",
                            ty.name(),
                            actual.name()
                        )));
                    }
                    go::g_value_set_object(v, p.cast());
                }
            },
            glib::Type::POINTER => match value {
                mlua::Value::Nil => {}
                mlua::Value::LightUserData(p) => go::g_value_set_pointer(v, p.0),
                other => {
                    go::g_value_set_pointer(v, object::instance_ptr(other).ok_or_else(wrong)?.0)
                }
            },
            glib::Type::VARIANT => match value {
                mlua::Value::Nil => {}
                mlua::Value::UserData(ud) => {
                    go::g_value_set_variant(v, ud.borrow::<Record>()?.ptr.cast())
                }
                _ => return Err(wrong()),
            },
            glib::Type::PARAM_SPEC => match value {
                mlua::Value::Nil => {}
                other => go::g_value_set_param(
                    v,
                    object::instance_ptr(other).ok_or_else(wrong)?.0.cast(),
                ),
            },
            glib::Type::BOXED => {
                if ty == crate::types::strv_type() {
                    let table = value.as_table().ok_or_else(wrong)?;
                    let items: Vec<String> = table
                        .sequence_values::<String>()
                        .collect::<mlua::Result<_>>()?;
                    out = items.to_value();
                } else {
                    set_boxed(value, ty, v)?;
                }
            }
            _ if ty == crate::types::gtype_type() => {
                go::g_value_set_gtype(v, crate::gtype_from_lua(value)?.into_glib());
            }
            _ => match value {
                mlua::Value::Nil => {}
                other => {
                    let (p, _) = object::instance_ptr(other).ok_or_else(wrong)?;
                    go::g_value_set_instance(v, p.cast());
                }
            },
        }
    }
    Ok(out)
}

/// Sets a boxed value from a record, a table of fields, or a string the
/// struct parses (`"#ff0000"` for a color).
unsafe fn set_boxed(value: &mlua::Value, ty: glib::Type, v: *mut go::GValue) -> mlua::Result<()> {
    let wrong = || {
        mlua::Error::runtime(format!(
            "expected a {}, got {}",
            ty.name(),
            value.type_name()
        ))
    };
    if matches!(value, mlua::Value::Nil) {
        return Ok(());
    }
    let info = struct_info(ty).ok_or_else(wrong)?;
    if let Some(p) = crate::cairo::from_lua(value, &info) {
        // SAFETY: a boxed value of this type.
        unsafe { go::g_value_set_boxed(v, p) };
        return Ok(());
    }
    let lua_value_ptr: *mut c_void = match value {
        mlua::Value::UserData(ud) => ud.borrow::<Record>().map_err(|_| wrong())?.ptr,
        mlua::Value::Table(_) | mlua::Value::String(_) => {
            // Build one through the generic struct conversion.
            let lua = match value {
                mlua::Value::Table(t) => t.weak_lua().upgrade(),
                mlua::Value::String(s) => s.weak_lua().upgrade(),
                _ => unreachable!(),
            };
            let mut scratch = crate::convert::Scratch::default();
            let ty_info = crate::types::Type {
                kind: crate::types::Ty::Struct(info.clone()),
                pointer: true,
                info: info.clone(),
            };
            let arg = crate::convert::from_lua(
                &lua,
                value,
                &ty_info,
                crate::sys::TRANSFER_NOTHING,
                false,
                &mut scratch,
            )
            .map_err(mlua::Error::runtime)?;
            // SAFETY: the struct lives until the scratch goes; set_boxed copies.
            unsafe { go::g_value_set_boxed(v, arg.v_pointer) };
            return Ok(());
        }
        _ => return Err(wrong()),
    };
    // SAFETY: copies the record's struct into the value.
    unsafe { go::g_value_set_boxed(v, lua_value_ptr) };
    Ok(())
}

/// A GValue for a Lua value whose type isn't given (a `GValue` argument):
/// booleans, numbers (whole ones as `gint`), strings, objects, records.
pub fn guess_glib(value: &mlua::Value) -> mlua::Result<glib::Value> {
    Ok(match value {
        mlua::Value::Boolean(b) => b.to_value(),
        mlua::Value::Integer(i) => match i32::try_from(*i) {
            Ok(i) => i.to_value(),
            Err(_) => i.to_value(),
        },
        mlua::Value::Number(n) if n.fract() == 0.0 && n.abs() < i32::MAX as f64 => {
            (*n as i32).to_value()
        }
        mlua::Value::Number(n) => n.to_value(),
        mlua::Value::String(s) => s.to_str()?.to_string().to_value(),
        mlua::Value::Table(t) => {
            let items: Vec<String> = t.sequence_values::<String>().collect::<mlua::Result<_>>()?;
            items.to_value()
        }
        mlua::Value::UserData(ud) => {
            if let Some((_, ty)) = object::instance_ptr(value) {
                return lua_to_glib(value, ty);
            }
            let r = ud.borrow::<Record>()?;
            let ty = r.info.gtype();
            drop(r);
            return lua_to_glib(value, ty);
        }
        other => {
            return Err(mlua::Error::runtime(format!(
                "can't hold a {} in a GValue",
                other.type_name()
            )));
        }
    })
}
