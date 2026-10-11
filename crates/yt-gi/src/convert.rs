//! Values between Lua and C, as GI describes them.
//!
//! | C                         | Lua                                     |
//! |---------------------------|-----------------------------------------|
//! | numbers, gboolean         | numbers, booleans                       |
//! | `char*`                   | strings                                 |
//! | enums / flags             | nick strings / numbers                  |
//! | objects, instances        | objects                                 |
//! | structs, unions           | records (or a table of fields, in)      |
//! | arrays, lists             | sequences (`guint8` arrays: strings)    |
//! | hash tables               | tables                                  |
//! | `GValue`                  | the value it holds                      |
//! | callbacks                 | functions (see `closure`)               |

use std::ffi::{c_char, c_void};
use std::ptr;

use glib::ffi as g;
use glib::gobject_ffi as go;
use glib::translate::IntoGlib;
use mlua::Lua;

use crate::enums;
use crate::info::Info;
use crate::object;
use crate::record::{Keep, Owner, Record};
use crate::sys::{self, GIArgument, GITransfer};
use crate::types::{Array, Int, Ty, Type};

/// What a call allocated for its arguments, freed when it returns.
#[derive(Default)]
pub struct Scratch {
    items: Vec<Temp>,
}

pub enum Temp {
    Free(*mut c_void),
    List(*mut g::GList),
    SList(*mut g::GSList),
    Hash(*mut g::GHashTable),
    Array(*mut g::GArray),
    PtrArray(*mut g::GPtrArray),
    ByteArray(*mut g::GByteArray),
    Bytes(*mut g::GBytes),
    Value(Box<go::GValue>),
    /// A Lua value that must outlive the call (a record made for it).
    Keep(#[expect(dead_code, reason = "held until the call returns")] mlua::Value),
    /// A callback only valid during the call.
    Closure(
        #[expect(dead_code, reason = "held until the call returns")] Box<crate::closure::Closure>,
    ),
}

impl Scratch {
    pub fn push(&mut self, t: Temp) {
        self.items.push(t);
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        for item in self.items.drain(..).rev() {
            // SAFETY: each item is something this call allocated.
            unsafe {
                match item {
                    Temp::Free(p) => g::g_free(p),
                    Temp::List(l) => g::g_list_free(l),
                    Temp::SList(l) => g::g_slist_free(l),
                    Temp::Hash(h) => g::g_hash_table_unref(h),
                    Temp::Array(a) => {
                        g::g_array_unref(a);
                    }
                    Temp::PtrArray(a) => g::g_ptr_array_unref(a),
                    Temp::ByteArray(a) => g::g_byte_array_unref(a),
                    Temp::Bytes(b) => g::g_bytes_unref(b),
                    Temp::Value(mut v) => go::g_value_unset(&mut *v),
                    Temp::Keep(_) | Temp::Closure(_) => {}
                }
            }
        }
    }
}

type R<T> = Result<T, String>;

fn number(v: &mlua::Value) -> R<f64> {
    match v {
        mlua::Value::Integer(i) => Ok(*i as f64),
        mlua::Value::Number(n) => Ok(*n),
        mlua::Value::String(s) => s
            .to_str()
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .ok_or_else(|| format!("expected a number, got {:?}", s.to_string_lossy())),
        other => Err(format!("expected a number, got {}", other.type_name())),
    }
}

fn integer(v: &mlua::Value) -> R<i64> {
    match v {
        mlua::Value::Integer(i) => Ok(*i),
        other => {
            let n = number(other)?;
            if n.fract() != 0.0 || !n.is_finite() {
                return Err(format!("expected a whole number, got {n}"));
            }
            Ok(n as i64)
        }
    }
}

/// Stores an integer in the argument field of its width.
fn set_int(arg: &mut GIArgument, int: Int, v: i64) {
    match int {
        Int::I8 => arg.v_int8 = v as i8,
        Int::U8 => arg.v_uint8 = v as u8,
        Int::I16 => arg.v_int16 = v as i16,
        Int::U16 => arg.v_uint16 = v as u16,
        Int::I32 => arg.v_int32 = v as i32,
        Int::U32 => arg.v_uint32 = v as u32,
        Int::I64 => arg.v_int64 = v,
        Int::U64 => arg.v_uint64 = v as u64,
    }
}

/// Reads an integer from the argument field of its width.
pub fn get_int(arg: &GIArgument, int: Int) -> i64 {
    // SAFETY: reads the union field that was written for this width.
    unsafe {
        match int {
            Int::I8 => arg.v_int8 as i64,
            Int::U8 => arg.v_uint8 as i64,
            Int::I16 => arg.v_int16 as i64,
            Int::U16 => arg.v_uint16 as i64,
            Int::I32 => arg.v_int32 as i64,
            Int::U32 => arg.v_uint32 as i64,
            Int::I64 => arg.v_int64,
            Int::U64 => arg.v_uint64 as i64,
        }
    }
}

/// The integer kind an argument of `ty` is stored as, for array lengths.
pub fn int_kind(ty: &Type) -> Int {
    match ty.kind {
        Ty::Int(i) => i,
        _ => Int::U64,
    }
}

fn strdup(s: &[u8]) -> *mut c_char {
    // SAFETY: copies the bytes into a new NUL-terminated string.
    unsafe { g::g_strndup(s.as_ptr().cast(), s.len()) }
}

/// A Lua value as an argument of type `ty`. With `transfer` other than
/// nothing, the callee takes what's made; otherwise `scratch` frees it.
pub fn from_lua(
    lua: &Lua,
    value: &mlua::Value,
    ty: &Type,
    transfer: GITransfer,
    nullable: bool,
    scratch: &mut Scratch,
) -> R<GIArgument> {
    let mut arg = GIArgument::default();
    let full = transfer == sys::TRANSFER_EVERYTHING;
    match &ty.kind {
        Ty::Void if ty.pointer => arg.v_pointer = pointer(value)?,
        Ty::Void => {}
        Ty::Bool => {
            arg.v_boolean = !matches!(value, mlua::Value::Nil | mlua::Value::Boolean(false)) as i32
        }
        Ty::Int(int) => set_int(&mut arg, *int, integer(value)?),
        Ty::Float => arg.v_float = number(value)? as f32,
        Ty::Double => arg.v_double = number(value)?,
        Ty::GType => {
            arg.v_size = crate::gtype_from_lua(value)
                .map_err(|e| e.to_string())?
                .into_glib()
        }
        Ty::Unichar => {
            arg.v_uint32 = match value {
                mlua::Value::String(s) => {
                    let s = s.to_str().map_err(|e| e.to_string())?;
                    s.chars().next().map_or(0, u32::from)
                }
                other => integer(other)? as u32,
            }
        }
        Ty::Utf8 | Ty::Filename => {
            arg.v_pointer = match value {
                mlua::Value::Nil if nullable => ptr::null_mut(),
                mlua::Value::String(s) => strdup(&s.as_bytes()).cast(),
                mlua::Value::Integer(_) | mlua::Value::Number(_) => {
                    strdup(number(value)?.to_string().as_bytes()).cast()
                }
                other => return Err(format!("expected a string, got {}", other.type_name())),
            };
            if !full && !arg_null(&arg) {
                // SAFETY: reading the pointer just stored.
                scratch.push(Temp::Free(unsafe { arg.v_pointer }));
            }
        }
        Ty::Array(array) => {
            return array_from_lua(lua, value, ty, array, transfer, nullable, scratch)
                .map(|(a, _)| a);
        }
        Ty::Object(info) => {
            arg.v_pointer = match value {
                mlua::Value::Nil if nullable => ptr::null_mut(),
                v => {
                    let (p, actual) = object::instance_ptr(v).ok_or_else(|| {
                        format!("expected a {}, got {}", info.full_name(), describe(v))
                    })?;
                    let expected = info.gtype();
                    if expected.is_valid() && expected != glib::Type::UNIT && !actual.is_a(expected)
                    {
                        return Err(format!(
                            "expected a {}, got a {}",
                            info.full_name(),
                            actual.name()
                        ));
                    }
                    if full {
                        // SAFETY: a valid instance of `actual`.
                        unsafe { object::add_ref(p, actual) };
                    }
                    p
                }
            }
        }
        Ty::Struct(info) | Ty::Union(info) => {
            arg.v_pointer = record_from_lua(lua, value, info, full, nullable, scratch)?
        }
        Ty::Enum {
            info,
            flags,
            storage,
        } => set_int(&mut arg, *storage, enums::from_lua(info, *flags, value)?),
        Ty::Callback(info) => {
            return Err(format!(
                "a {} callback can't be passed here",
                info.full_name()
            ));
        }
        Ty::List { single, item } => {
            let mlua::Value::Table(t) = value else {
                if matches!(value, mlua::Value::Nil) {
                    return Ok(arg);
                }
                return Err(format!(
                    "expected a list (a table), got {}",
                    value.type_name()
                ));
            };
            let item_transfer = if full {
                sys::TRANSFER_EVERYTHING
            } else {
                sys::TRANSFER_NOTHING
            };
            let mut list: *mut c_void = ptr::null_mut();
            for v in t.sequence_values::<mlua::Value>() {
                let v = v.map_err(|e| e.to_string())?;
                let mut item_arg = from_lua(lua, &v, item, item_transfer, true, scratch)?;
                // SAFETY: lists hold pointers (or integers in pointers).
                let data = unsafe {
                    sys::gi_type_info_hash_pointer_from_argument(item.info.ptr(), &mut item_arg)
                };
                // SAFETY: appends to a list we're building.
                list = unsafe {
                    if *single {
                        g::g_slist_append(list.cast(), data).cast()
                    } else {
                        g::g_list_append(list.cast(), data).cast()
                    }
                };
            }
            if transfer == sys::TRANSFER_NOTHING {
                scratch.push(if *single {
                    Temp::SList(list.cast())
                } else {
                    Temp::List(list.cast())
                });
            }
            arg.v_pointer = list;
        }
        Ty::Hash { key, value: vty } => {
            let mlua::Value::Table(t) = value else {
                if matches!(value, mlua::Value::Nil) {
                    return Ok(arg);
                }
                return Err(format!("expected a table, got {}", value.type_name()));
            };
            let strings = matches!(key.kind, Ty::Utf8 | Ty::Filename);
            // SAFETY: a new table hashing strings by contents, others by
            // pointer value.
            let hash = unsafe {
                if strings {
                    g::g_hash_table_new(Some(g::g_str_hash), Some(g::g_str_equal))
                } else {
                    g::g_hash_table_new(Some(g::g_direct_hash), Some(g::g_direct_equal))
                }
            };
            for pair in t.pairs::<mlua::Value, mlua::Value>() {
                let (k, v) = pair.map_err(|e| e.to_string())?;
                let mut ka = from_lua(lua, &k, key, sys::TRANSFER_NOTHING, false, scratch)?;
                let mut va = from_lua(lua, &v, vty, sys::TRANSFER_NOTHING, true, scratch)?;
                // SAFETY: inserts pointers that live as long as the call.
                unsafe {
                    let kp = sys::gi_type_info_hash_pointer_from_argument(key.info.ptr(), &mut ka);
                    let vp = sys::gi_type_info_hash_pointer_from_argument(vty.info.ptr(), &mut va);
                    g::g_hash_table_insert(hash, kp, vp);
                }
            }
            scratch.push(Temp::Hash(hash));
            if full {
                // SAFETY: the callee gets its own reference.
                unsafe { g::g_hash_table_ref(hash) };
            }
            arg.v_pointer = hash.cast();
        }
        Ty::Error => {
            arg.v_pointer = match value {
                mlua::Value::Nil => ptr::null_mut(),
                mlua::Value::UserData(ud) => ud.borrow::<Record>().map_err(|e| e.to_string())?.ptr,
                other => return Err(format!("expected a GLib.Error, got {}", other.type_name())),
            }
        }
        Ty::Unresolved(name) => return Err(format!("the type {name} isn't loaded")),
    }
    Ok(arg)
}

fn arg_null(arg: &GIArgument) -> bool {
    // SAFETY: pointer arguments are read as pointers.
    unsafe { arg.v_pointer.is_null() }
}

fn describe(v: &mlua::Value) -> String {
    match v {
        mlua::Value::UserData(ud) => {
            if let Ok(r) = ud.borrow::<Record>() {
                return format!("a {}", r.info.full_name());
            }
            "userdata".into()
        }
        other => other.type_name().into(),
    }
}

/// A `gpointer`: nil, a light userdata, or any wrapped value's pointer.
fn pointer(value: &mlua::Value) -> R<*mut c_void> {
    Ok(match value {
        mlua::Value::Nil => ptr::null_mut(),
        mlua::Value::LightUserData(p) => p.0,
        mlua::Value::UserData(ud) => {
            if let Some((p, _)) = object::instance_ptr(value) {
                p
            } else if let Ok(r) = ud.borrow::<Record>() {
                r.ptr
            } else {
                return Err("expected a pointer".into());
            }
        }
        other => return Err(format!("expected a pointer, got {}", other.type_name())),
    })
}

fn record_from_lua(
    lua: &Lua,
    value: &mlua::Value,
    info: &Info,
    full: bool,
    nullable: bool,
    scratch: &mut Scratch,
) -> R<*mut c_void> {
    let gtype = info.gtype();
    // A GValue: whatever value it should hold.
    if gtype == crate::types::value_type() {
        if let mlua::Value::UserData(ud) = value
            && let Ok(r) = ud.borrow::<Record>()
        {
            return Ok(r.ptr);
        }
        let v = crate::value::guess_glib(value).map_err(|e| e.to_string())?;
        let mut boxed = Box::new(unsafe { std::mem::zeroed::<go::GValue>() });
        // SAFETY: copies the value into a zeroed GValue we own.
        unsafe {
            go::g_value_init(&mut *boxed, v.type_().into_glib());
            go::g_value_copy(glib::translate::ToGlibPtr::to_glib_none(&v).0, &mut *boxed);
        }
        let p: *mut go::GValue = &mut *boxed;
        scratch.push(Temp::Value(boxed));
        return Ok(p.cast());
    }
    if gtype == glib::Type::from_name("GBytes").unwrap_or(glib::Type::INVALID)
        && let mlua::Value::String(s) = value
    {
        let bytes = s.as_bytes();
        // SAFETY: copies the bytes into new GBytes.
        let b = unsafe { g::g_bytes_new(bytes.as_ptr().cast(), bytes.len()) };
        if !full {
            scratch.push(Temp::Bytes(b));
        }
        return Ok(b.cast());
    }
    if let Some(p) = crate::cairo::from_lua(value, info) {
        return Ok(p);
    }
    match value {
        mlua::Value::Nil if nullable => Ok(ptr::null_mut()),
        mlua::Value::UserData(ud) => {
            let r = ud
                .borrow::<Record>()
                .map_err(|_| format!("expected a {}, got {}", info.full_name(), describe(value)))?;
            let same = r.info.full_name() == info.full_name()
                || (gtype.is_valid() && r.info.gtype().is_valid() && r.info.gtype().is_a(gtype));
            if !same {
                return Err(format!(
                    "expected a {}, got a {}",
                    info.full_name(),
                    r.info.full_name()
                ));
            }
            if full {
                if r.info.gtype() == glib::Type::VARIANT {
                    // SAFETY: a GVariant.
                    unsafe { g::g_variant_ref(r.ptr.cast()) };
                } else if crate::record::is_boxed(gtype) {
                    // SAFETY: a boxed value of this type.
                    return Ok(unsafe { go::g_boxed_copy(gtype.into_glib(), r.ptr) });
                }
            }
            Ok(r.ptr)
        }
        // A table of fields, or a string for structs that parse one
        // (`"#ff0000"` for a Gdk.RGBA).
        mlua::Value::Table(_) | mlua::Value::String(_) => {
            let record = Record::new_zeroed(info.clone()).map_err(|e| e.to_string())?;
            let ud = lua.create_userdata(record).map_err(|e| e.to_string())?;
            let owner = mlua::Value::UserData(ud.clone());
            match value {
                mlua::Value::Table(t) => {
                    let r = ud.borrow::<Record>().map_err(|e| e.to_string())?;
                    crate::record::fill(lua, &r, &owner, t).map_err(|e| e.to_string())?;
                }
                _ => {
                    let parse = crate::members::record(lua, info, "parse")
                        .map_err(|e| e.to_string())?
                        .ok_or_else(|| format!("expected a {}, got a string", info.full_name()))?;
                    let ok: bool = parse
                        .call((owner.clone(), value.clone()))
                        .map_err(|e| e.to_string())?;
                    if !ok {
                        return Err(format!(
                            "{:?} isn't a valid {}",
                            value.to_string().unwrap_or_default(),
                            info.full_name()
                        ));
                    }
                }
            }
            let p = ud.borrow::<Record>().map_err(|e| e.to_string())?.ptr;
            scratch.push(Temp::Keep(owner));
            if full && crate::record::is_boxed(gtype) {
                // SAFETY: a boxed value of this type.
                return Ok(unsafe { go::g_boxed_copy(gtype.into_glib(), p) });
            }
            Ok(p)
        }
        other => Err(format!(
            "expected a {}, got {}",
            info.full_name(),
            other.type_name()
        )),
    }
}

/// A Lua table (or a string, for byte arrays) as an array. Returns the
/// array and its length.
pub fn array_from_lua(
    lua: &Lua,
    value: &mlua::Value,
    ty: &Type,
    array: &Array,
    transfer: GITransfer,
    nullable: bool,
    scratch: &mut Scratch,
) -> R<(GIArgument, usize)> {
    let mut arg = GIArgument::default();
    let item = &array.item;
    let bytes_item = matches!(item.kind, Ty::Int(Int::U8 | Int::I8)) && !item.pointer;
    let items: Vec<mlua::Value> = match value {
        mlua::Value::Nil if nullable || array.length_arg.is_some() => return Ok((arg, 0)),
        mlua::Value::String(s) if bytes_item => {
            let bytes = s.as_bytes();
            return Ok((
                bytes_array(&bytes, array, ty, transfer, scratch),
                bytes.len(),
            ));
        }
        mlua::Value::Table(t) => t
            .sequence_values::<mlua::Value>()
            .collect::<mlua::Result<_>>()
            .map_err(|e| e.to_string())?,
        other => return Err(format!("expected a table, got {}", other.type_name())),
    };
    let n = items.len();
    let size = item.size().max(1);
    let item_transfer = if transfer == sys::TRANSFER_EVERYTHING {
        sys::TRANSFER_EVERYTHING
    } else {
        sys::TRANSFER_NOTHING
    };
    let embedded = !item.pointer && matches!(item.kind, Ty::Struct(_) | Ty::Union(_));
    let extra = usize::from(array.zero_terminated);
    let count = array.fixed_size.map_or(n, |f| f.max(n));
    // SAFETY: zeroed memory for the items (and the terminator).
    let data = unsafe { g::g_malloc0((count + extra) * size) } as *mut u8;
    for (i, v) in items.iter().enumerate() {
        let a = from_lua(lua, v, item, item_transfer, true, scratch)?;
        // SAFETY: within the allocation; embedded structs are copied.
        unsafe {
            if embedded {
                if !a.v_pointer.is_null() {
                    ptr::copy_nonoverlapping(a.v_pointer as *const u8, data.add(i * size), size);
                }
            } else {
                store(data.add(i * size), item, &a);
            }
        }
    }
    let keep = transfer == sys::TRANSFER_NOTHING;
    match array.kind {
        sys::ARRAY_C => {
            if keep {
                scratch.push(Temp::Free(data.cast()));
            }
            arg.v_pointer = data.cast();
        }
        sys::ARRAY_PTR_ARRAY => {
            // SAFETY: builds a pointer array of the items.
            unsafe {
                let a = g::g_ptr_array_sized_new(n as u32);
                for i in 0..n {
                    g::g_ptr_array_add(a, *(data.add(i * size) as *const *mut c_void));
                }
                g::g_free(data.cast());
                if keep {
                    scratch.push(Temp::PtrArray(a));
                }
                arg.v_pointer = a.cast();
            }
        }
        _ => {
            // SAFETY: builds a GArray of the items.
            unsafe {
                let a =
                    g::g_array_sized_new(array.zero_terminated as i32, 1, size as u32, n as u32);
                g::g_array_append_vals(a, data.cast(), n as u32);
                g::g_free(data.cast());
                if keep {
                    scratch.push(Temp::Array(a));
                }
                arg.v_pointer = a.cast();
            }
        }
    }
    let _ = ty;
    Ok((arg, n))
}

fn bytes_array(
    bytes: &[u8],
    array: &Array,
    _ty: &Type,
    transfer: GITransfer,
    scratch: &mut Scratch,
) -> GIArgument {
    let mut arg = GIArgument::default();
    let keep = transfer == sys::TRANSFER_NOTHING;
    // SAFETY: copies the bytes into the array kind the callee wants.
    unsafe {
        match array.kind {
            sys::ARRAY_BYTE_ARRAY => {
                let a = g::g_byte_array_sized_new(bytes.len() as u32);
                g::g_byte_array_append(a, bytes.as_ptr(), bytes.len() as u32);
                if keep {
                    scratch.push(Temp::ByteArray(a));
                }
                arg.v_pointer = a.cast();
            }
            sys::ARRAY_ARRAY => {
                let a =
                    g::g_array_sized_new(array.zero_terminated as i32, 0, 1, bytes.len() as u32);
                g::g_array_append_vals(a, bytes.as_ptr().cast(), bytes.len() as u32);
                if keep {
                    scratch.push(Temp::Array(a));
                }
                arg.v_pointer = a.cast();
            }
            _ => {
                let extra = usize::from(array.zero_terminated);
                let data = g::g_malloc0(bytes.len() + extra) as *mut u8;
                ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len());
                if keep {
                    scratch.push(Temp::Free(data.cast()));
                }
                arg.v_pointer = data.cast();
            }
        }
    }
    arg
}

/// Loads a value of `ty` stored at `at` (an array item, a struct field, an
/// out argument). Embedded structs load as their address.
///
/// # Safety
/// `at` points to a value of `ty`.
pub unsafe fn load(at: *const u8, ty: &Type) -> GIArgument {
    let mut arg = GIArgument::default();
    // SAFETY: as the caller promises.
    unsafe {
        if ty.pointer {
            arg.v_pointer = ptr::read_unaligned(at as *const *mut c_void);
            return arg;
        }
        match &ty.kind {
            Ty::Bool => arg.v_int32 = ptr::read_unaligned(at as *const i32),
            Ty::Int(i) | Ty::Enum { storage: i, .. } => match i {
                Int::I8 => arg.v_int8 = ptr::read_unaligned(at as *const i8),
                Int::U8 => arg.v_uint8 = ptr::read_unaligned(at),
                Int::I16 => arg.v_int16 = ptr::read_unaligned(at as *const i16),
                Int::U16 => arg.v_uint16 = ptr::read_unaligned(at as *const u16),
                Int::I32 => arg.v_int32 = ptr::read_unaligned(at as *const i32),
                Int::U32 => arg.v_uint32 = ptr::read_unaligned(at as *const u32),
                Int::I64 => arg.v_int64 = ptr::read_unaligned(at as *const i64),
                Int::U64 => arg.v_uint64 = ptr::read_unaligned(at as *const u64),
            },
            Ty::Float => arg.v_float = ptr::read_unaligned(at as *const f32),
            Ty::Double => arg.v_double = ptr::read_unaligned(at as *const f64),
            Ty::GType => arg.v_size = ptr::read_unaligned(at as *const usize),
            Ty::Unichar => arg.v_uint32 = ptr::read_unaligned(at as *const u32),
            Ty::Struct(_) | Ty::Union(_) => arg.v_pointer = at as *mut c_void,
            Ty::Void => {}
            _ => arg.v_pointer = ptr::read_unaligned(at as *const *mut c_void),
        }
    }
    arg
}

/// Stores a value of `ty` at `at`; embedded structs are copied in.
///
/// # Safety
/// `at` has room for a value of `ty`.
pub unsafe fn store(at: *mut u8, ty: &Type, arg: &GIArgument) {
    // SAFETY: as the caller promises.
    unsafe {
        if ty.pointer {
            ptr::write_unaligned(at as *mut *mut c_void, arg.v_pointer);
            return;
        }
        match &ty.kind {
            Ty::Bool => ptr::write_unaligned(at as *mut i32, arg.v_int32),
            Ty::Int(i) | Ty::Enum { storage: i, .. } => match i {
                Int::I8 => ptr::write_unaligned(at as *mut i8, arg.v_int8),
                Int::U8 => ptr::write_unaligned(at, arg.v_uint8),
                Int::I16 => ptr::write_unaligned(at as *mut i16, arg.v_int16),
                Int::U16 => ptr::write_unaligned(at as *mut u16, arg.v_uint16),
                Int::I32 => ptr::write_unaligned(at as *mut i32, arg.v_int32),
                Int::U32 => ptr::write_unaligned(at as *mut u32, arg.v_uint32),
                Int::I64 => ptr::write_unaligned(at as *mut i64, arg.v_int64),
                Int::U64 => ptr::write_unaligned(at as *mut u64, arg.v_uint64),
            },
            Ty::Float => ptr::write_unaligned(at as *mut f32, arg.v_float),
            Ty::Double => ptr::write_unaligned(at as *mut f64, arg.v_double),
            Ty::GType => ptr::write_unaligned(at as *mut usize, arg.v_size),
            Ty::Unichar => ptr::write_unaligned(at as *mut u32, arg.v_uint32),
            Ty::Struct(_) | Ty::Union(_) => {
                if !arg.v_pointer.is_null() {
                    ptr::copy_nonoverlapping(arg.v_pointer as *const u8, at, ty.size());
                }
            }
            Ty::Void => {}
            _ => ptr::write_unaligned(at as *mut *mut c_void, arg.v_pointer),
        }
    }
}

fn string(lua: &Lua, p: *mut c_char, full: bool) -> mlua::Result<mlua::Value> {
    if p.is_null() {
        return Ok(mlua::Value::Nil);
    }
    // SAFETY: a NUL-terminated string; freed if it's ours.
    let s = unsafe { std::ffi::CStr::from_ptr(p) };
    let out = lua.create_string(s.to_bytes())?;
    if full {
        // SAFETY: ours to free.
        unsafe { g::g_free(p.cast()) };
    }
    Ok(mlua::Value::String(out))
}

/// A C value as Lua. With `transfer` other than nothing, what it points to
/// is ours and is freed (or taken over). `length` is the item count for
/// arrays whose length GI passes separately.
pub fn to_lua(
    lua: &Lua,
    arg: &GIArgument,
    ty: &Type,
    transfer: GITransfer,
    length: Option<usize>,
) -> mlua::Result<mlua::Value> {
    let full = transfer == sys::TRANSFER_EVERYTHING;
    // SAFETY: each arm reads the union field GI wrote for this type.
    unsafe {
        Ok(match &ty.kind {
            Ty::Void if ty.pointer => {
                if arg.v_pointer.is_null() {
                    mlua::Value::Nil
                } else {
                    mlua::Value::LightUserData(mlua::LightUserData(arg.v_pointer))
                }
            }
            Ty::Void => mlua::Value::Nil,
            Ty::Bool => mlua::Value::Boolean(arg.v_int32 != 0),
            Ty::Int(Int::I64) => mlua::Value::Number(arg.v_int64 as f64),
            Ty::Int(Int::U64) => mlua::Value::Number(arg.v_uint64 as f64),
            Ty::Int(i) => mlua::Value::Integer(get_int(arg, *i)),
            Ty::Float => mlua::Value::Number(arg.v_float as f64),
            Ty::Double => mlua::Value::Number(arg.v_double),
            Ty::GType => mlua::Value::Number(arg.v_size as f64),
            Ty::Unichar => match char::from_u32(arg.v_uint32) {
                Some(c) => mlua::Value::String(lua.create_string(c.to_string())?),
                None => mlua::Value::Nil,
            },
            Ty::Utf8 | Ty::Filename => string(lua, arg.v_string, full)?,
            Ty::Array(array) => array_to_lua(lua, arg.v_pointer, array, transfer, length)?,
            Ty::Object(_) => object::wrap(lua, arg.v_pointer, full, true)?,
            Ty::Struct(info) | Ty::Union(info) => {
                record_to_lua(lua, arg.v_pointer, info, ty.pointer, full)?
            }
            Ty::Enum {
                info,
                flags,
                storage,
            } => enums::to_lua(lua, info, *flags, get_int(arg, *storage))?,
            Ty::Callback(_) => {
                if arg.v_pointer.is_null() {
                    mlua::Value::Nil
                } else {
                    mlua::Value::LightUserData(mlua::LightUserData(arg.v_pointer))
                }
            }
            Ty::List { single, item } => {
                let out = lua.create_table()?;
                let mut node = arg.v_pointer;
                let item_transfer = if full {
                    sys::TRANSFER_EVERYTHING
                } else {
                    sys::TRANSFER_NOTHING
                };
                let mut i = 1;
                while !node.is_null() {
                    let (data, next) = if *single {
                        let n = node as *mut g::GSList;
                        ((*n).data, (*n).next as *mut c_void)
                    } else {
                        let n = node as *mut g::GList;
                        ((*n).data, (*n).next as *mut c_void)
                    };
                    let mut item_arg = GIArgument::default();
                    sys::gi_type_info_argument_from_hash_pointer(
                        item.info.ptr(),
                        data,
                        &mut item_arg,
                    );
                    out.raw_set(i, to_lua(lua, &item_arg, item, item_transfer, None)?)?;
                    i += 1;
                    node = next;
                }
                if transfer != sys::TRANSFER_NOTHING {
                    if *single {
                        g::g_slist_free(arg.v_pointer.cast());
                    } else {
                        g::g_list_free(arg.v_pointer.cast());
                    }
                }
                mlua::Value::Table(out)
            }
            Ty::Hash { key, value } => {
                if arg.v_pointer.is_null() {
                    return Ok(mlua::Value::Nil);
                }
                let out = lua.create_table()?;
                let hash = arg.v_pointer as *mut g::GHashTable;
                let mut iter = std::mem::zeroed::<g::GHashTableIter>();
                g::g_hash_table_iter_init(&mut iter, hash);
                let (mut k, mut v) = (ptr::null_mut(), ptr::null_mut());
                while g::g_hash_table_iter_next(&mut iter, &mut k, &mut v) != 0 {
                    let (mut ka, mut va) = (GIArgument::default(), GIArgument::default());
                    sys::gi_type_info_argument_from_hash_pointer(key.info.ptr(), k, &mut ka);
                    sys::gi_type_info_argument_from_hash_pointer(value.info.ptr(), v, &mut va);
                    out.raw_set(
                        to_lua(lua, &ka, key, sys::TRANSFER_NOTHING, None)?,
                        to_lua(lua, &va, value, sys::TRANSFER_NOTHING, None)?,
                    )?;
                }
                if transfer != sys::TRANSFER_NOTHING {
                    g::g_hash_table_unref(hash);
                }
                mlua::Value::Table(out)
            }
            Ty::Error => {
                if arg.v_pointer.is_null() {
                    return Ok(mlua::Value::Nil);
                }
                error_to_lua(lua, arg.v_pointer.cast(), full)?
            }
            Ty::Unresolved(_) => mlua::Value::Nil,
        })
    }
}

/// A `GError` as a Lua record with `message`, `code` and `domain`.
pub fn error_to_lua(lua: &Lua, error: *mut g::GError, full: bool) -> mlua::Result<mlua::Value> {
    let info = crate::repo::find_by_name("GLib", "Error")
        .ok_or_else(|| mlua::Error::runtime("GLib isn't loaded"))?;
    // SAFETY: a GError, ours with `full`.
    let record = unsafe { Record::from_gi(error.cast(), info, full, Keep::Nothing) };
    Ok(mlua::Value::UserData(lua.create_userdata(record)?))
}

fn record_to_lua(
    lua: &Lua,
    p: *mut c_void,
    info: &Info,
    pointer: bool,
    full: bool,
) -> mlua::Result<mlua::Value> {
    if p.is_null() {
        return Ok(mlua::Value::Nil);
    }
    let gtype = info.gtype();
    if gtype == crate::types::value_type() {
        // SAFETY: a GValue; freed if ours.
        let value = unsafe { crate::value::value_to_lua(lua, p.cast()) };
        if full {
            // SAFETY: ours.
            unsafe {
                go::g_value_unset(p.cast());
                g::g_free(p);
            }
        }
        return value;
    }
    if let Some(v) = crate::cairo::to_lua(lua, p, info, full)? {
        return Ok(v);
    }
    let record = if pointer {
        // SAFETY: GI says it's a struct of this type, ours with `full`.
        unsafe { Record::from_gi(p, info.clone(), full, Keep::Nothing) }
    } else {
        Record::copy_of(p, info.clone())
    };
    Ok(mlua::Value::UserData(lua.create_userdata(record)?))
}

/// How many items an array has: fixed, passed separately, or up to its
/// zero terminator.
///
/// # Safety
/// `data` is an array as `array` describes.
unsafe fn count(data: *const u8, array: &Array, length: Option<usize>) -> usize {
    if let Some(n) = length {
        return n;
    }
    if let Some(n) = array.fixed_size {
        return n;
    }
    if array.zero_terminated && !data.is_null() {
        let size = array.item.size().max(1);
        let mut n = 0;
        // SAFETY: the array ends with an all-zero item.
        unsafe {
            while std::slice::from_raw_parts(data.add(n * size), size)
                .iter()
                .any(|b| *b != 0)
            {
                n += 1;
            }
        }
        return n;
    }
    0
}

fn array_to_lua(
    lua: &Lua,
    p: *mut c_void,
    array: &Array,
    transfer: GITransfer,
    length: Option<usize>,
) -> mlua::Result<mlua::Value> {
    if p.is_null() {
        return Ok(if length.is_some() {
            mlua::Value::Table(lua.create_table()?)
        } else {
            mlua::Value::Nil
        });
    }
    let item = &array.item;
    let size = item.size().max(1);
    // SAFETY: GI describes the array; its kind says where the data is.
    let (data, n) = unsafe {
        match array.kind {
            sys::ARRAY_ARRAY => {
                let a = p as *mut g::GArray;
                ((*a).data as *const u8, (*a).len as usize)
            }
            sys::ARRAY_PTR_ARRAY => {
                let a = p as *mut g::GPtrArray;
                ((*a).pdata as *const u8, (*a).len as usize)
            }
            sys::ARRAY_BYTE_ARRAY => {
                let a = p as *mut g::GByteArray;
                ((*a).data as *const u8, (*a).len as usize)
            }
            _ => (p as *const u8, count(p as *const u8, array, length)),
        }
    };
    let bytes = matches!(item.kind, Ty::Int(Int::U8 | Int::I8)) && !item.pointer;
    let out = if bytes {
        // SAFETY: `n` bytes of data.
        mlua::Value::String(lua.create_string(unsafe { std::slice::from_raw_parts(data, n) })?)
    } else {
        let t = lua.create_table_with_capacity(n, 0)?;
        let item_transfer = if transfer == sys::TRANSFER_EVERYTHING {
            sys::TRANSFER_EVERYTHING
        } else {
            sys::TRANSFER_NOTHING
        };
        for i in 0..n {
            // SAFETY: item `i` of the array.
            let a = unsafe { load(data.add(i * size), item) };
            t.raw_set(i + 1, to_lua(lua, &a, item, item_transfer, None)?)?;
        }
        mlua::Value::Table(t)
    };
    if transfer != sys::TRANSFER_NOTHING {
        // SAFETY: the container is ours; its items were taken above.
        unsafe {
            match array.kind {
                sys::ARRAY_ARRAY => {
                    g::g_array_set_clear_func(p.cast(), None);
                    g::g_array_unref(p.cast());
                }
                sys::ARRAY_PTR_ARRAY => {
                    g::g_ptr_array_set_free_func(p.cast(), None);
                    g::g_ptr_array_unref(p.cast());
                }
                sys::ARRAY_BYTE_ARRAY => g::g_byte_array_unref(p.cast()),
                _ => g::g_free(p),
            }
        }
    }
    Ok(out)
}

/// A record holding a GI-made struct, for out arguments the caller
/// allocates.
pub fn caller_allocated(lua: &Lua, p: *mut c_void, info: &Info) -> mlua::Result<mlua::Value> {
    if info.gtype() == crate::types::value_type() {
        // SAFETY: a GValue we allocated; unset and freed after reading.
        let value = unsafe { crate::value::value_to_lua(lua, p.cast()) };
        // SAFETY: as above.
        unsafe {
            go::g_value_unset(p.cast());
            g::g_free(p);
        }
        return value;
    }
    Ok(mlua::Value::UserData(lua.create_userdata(
        Record::with_owner(p, info.clone(), Owner::Malloc),
    )?))
}
