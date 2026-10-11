//! Types of arguments, return values, fields and constants, read once from
//! a `GITypeInfo` into a form that's quick to convert values with.

use std::mem::size_of;
use std::rc::Rc;

use crate::info::{Info, Kind};
use crate::sys;

#[derive(Clone)]
pub struct Type {
    pub kind: Ty,
    /// Passed by pointer (`char*`, `GtkWidget*`, `GdkRGBA*`).
    pub pointer: bool,
    /// The `GITypeInfo`, for GI's own helpers (hash table values).
    pub info: Info,
}

#[derive(Clone)]
pub enum Ty {
    Void,
    Bool,
    Int(Int),
    Float,
    Double,
    GType,
    Unichar,
    /// UTF-8 text; `filename` paths are bytes on Unix, UTF-8 on Windows.
    Utf8,
    Filename,
    Array(Rc<Array>),
    /// A GObject class or interface, or another instance type such as
    /// `GParamSpec`, `GskRenderNode` or `GdkEvent`.
    Object(Info),
    /// A C struct: boxed, plain, or a special one (GValue, GVariant...).
    Struct(Info),
    Union(Info),
    Enum {
        info: Info,
        flags: bool,
        storage: Int,
    },
    Callback(Info),
    List {
        single: bool,
        item: Rc<Type>,
    },
    Hash {
        key: Rc<Type>,
        value: Rc<Type>,
    },
    Error,
    /// A type that couldn't be found: its namespace isn't loaded.
    Unresolved(String),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Int {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
}

impl Int {
    pub fn size(self) -> usize {
        match self {
            Int::I8 | Int::U8 => 1,
            Int::I16 | Int::U16 => 2,
            Int::I32 | Int::U32 => 4,
            Int::I64 | Int::U64 => 8,
        }
    }

    fn from_tag(tag: sys::GITypeTag) -> Option<Int> {
        Some(match tag {
            sys::TAG_INT8 => Int::I8,
            sys::TAG_UINT8 => Int::U8,
            sys::TAG_INT16 => Int::I16,
            sys::TAG_UINT16 => Int::U16,
            sys::TAG_INT32 => Int::I32,
            sys::TAG_UINT32 => Int::U32,
            sys::TAG_INT64 => Int::I64,
            sys::TAG_UINT64 => Int::U64,
            _ => return None,
        })
    }
}

pub struct Array {
    pub kind: sys::GIArrayType,
    pub item: Type,
    /// The argument holding the length, for C arrays.
    pub length_arg: Option<usize>,
    pub fixed_size: Option<usize>,
    pub zero_terminated: bool,
}

impl Type {
    pub fn new(info: Info) -> Type {
        // SAFETY: a valid type info.
        let (tag, pointer) = unsafe {
            (
                sys::gi_type_info_get_tag(info.ptr()),
                sys::gi_type_info_is_pointer(info.ptr()) != 0,
            )
        };
        let kind = match tag {
            sys::TAG_VOID => Ty::Void,
            sys::TAG_BOOLEAN => Ty::Bool,
            sys::TAG_FLOAT => Ty::Float,
            sys::TAG_DOUBLE => Ty::Double,
            sys::TAG_GTYPE => Ty::GType,
            sys::TAG_UNICHAR => Ty::Unichar,
            sys::TAG_UTF8 => Ty::Utf8,
            sys::TAG_FILENAME => Ty::Filename,
            sys::TAG_ERROR => Ty::Error,
            sys::TAG_ARRAY => Ty::Array(Rc::new(Array::new(&info))),
            sys::TAG_GLIST | sys::TAG_GSLIST => Ty::List {
                single: tag == sys::TAG_GSLIST,
                item: Rc::new(param(&info, 0)),
            },
            sys::TAG_GHASH => Ty::Hash {
                key: Rc::new(param(&info, 0)),
                value: Rc::new(param(&info, 1)),
            },
            sys::TAG_INTERFACE => interface(&info),
            other => match Int::from_tag(other) {
                Some(int) => Ty::Int(int),
                None => Ty::Unresolved(format!("type tag {other}")),
            },
        };
        Type {
            kind,
            pointer,
            info,
        }
    }

    /// How many bytes one value takes in an array or a struct.
    pub fn size(&self) -> usize {
        if self.pointer {
            return size_of::<*mut u8>();
        }
        match &self.kind {
            Ty::Void => 0,
            Ty::Bool => size_of::<glib::ffi::gboolean>(),
            Ty::Int(i) => i.size(),
            Ty::Float => 4,
            Ty::Double => 8,
            Ty::GType => size_of::<glib::ffi::GType>(),
            Ty::Unichar => 4,
            Ty::Enum { storage, .. } => storage.size(),
            // SAFETY: valid infos.
            Ty::Struct(info) => unsafe { sys::gi_struct_info_get_size(info.ptr()) },
            Ty::Union(info) => unsafe { sys::gi_union_info_get_size(info.ptr()) },
            _ => size_of::<*mut u8>(),
        }
    }

    pub fn is_void(&self) -> bool {
        matches!(self.kind, Ty::Void) && !self.pointer
    }

    /// What it is, for messages: `Gtk.Widget`, `utf8`, `array of utf8`...
    pub fn describe(&self) -> String {
        match &self.kind {
            Ty::Void if self.pointer => "pointer".into(),
            Ty::Void => "nothing".into(),
            Ty::Bool => "boolean".into(),
            Ty::Int(_) | Ty::Float | Ty::Double => "number".into(),
            Ty::GType => "GType".into(),
            Ty::Unichar => "character".into(),
            Ty::Utf8 | Ty::Filename => "string".into(),
            Ty::Array(a) => format!("array of {}", a.item.describe()),
            Ty::Object(i) | Ty::Struct(i) | Ty::Union(i) | Ty::Callback(i) => i.full_name(),
            Ty::Enum { info, .. } => info.full_name(),
            Ty::List { item, .. } => format!("list of {}", item.describe()),
            Ty::Hash { key, value } => {
                format!("table of {} to {}", key.describe(), value.describe())
            }
            Ty::Error => "GLib.Error".into(),
            Ty::Unresolved(name) => format!("{name} (not loaded)"),
        }
    }
}

fn param(info: &Info, n: u32) -> Type {
    // SAFETY: containers have their parameter types.
    match unsafe { Info::from_full(sys::gi_type_info_get_param_type(info.ptr(), n)) } {
        Some(p) => Type::new(p),
        None => Type {
            kind: Ty::Void,
            pointer: true,
            info: info.clone(),
        },
    }
}

impl Array {
    fn new(info: &Info) -> Array {
        let mut length = 0;
        let mut fixed = 0;
        // SAFETY: a valid array type info.
        unsafe {
            Array {
                kind: sys::gi_type_info_get_array_type(info.ptr()),
                item: param(info, 0),
                length_arg: (sys::gi_type_info_get_array_length_index(info.ptr(), &mut length)
                    != 0)
                    .then_some(length as usize),
                fixed_size: (sys::gi_type_info_get_array_fixed_size(info.ptr(), &mut fixed) != 0)
                    .then_some(fixed),
                zero_terminated: sys::gi_type_info_is_zero_terminated(info.ptr()) != 0,
            }
        }
    }
}

fn interface(info: &Info) -> Ty {
    // SAFETY: interface types have an interface info.
    let Some(target) = (unsafe { Info::from_full(sys::gi_type_info_get_interface(info.ptr())) })
    else {
        return Ty::Unresolved("an interface".into());
    };
    match target.kind() {
        Kind::Object | Kind::Interface => Ty::Object(target),
        Kind::Struct => Ty::Struct(target),
        Kind::Union => Ty::Union(target),
        Kind::Enum | Kind::Flags => {
            // SAFETY: an enum info.
            let tag = unsafe { sys::gi_enum_info_get_storage_type(target.ptr()) };
            Ty::Enum {
                flags: target.kind() == Kind::Flags,
                storage: Int::from_tag(tag).unwrap_or(Int::I32),
                info: target,
            }
        }
        Kind::Callback => Ty::Callback(target),
        _ => Ty::Unresolved(target.full_name()),
    }
}

/// GTypes glib-rs has no constants for.
pub fn value_type() -> glib::Type {
    // SAFETY: g_value_get_type only registers the type.
    unsafe { glib::translate::FromGlib::from_glib(glib::gobject_ffi::g_value_get_type()) }
}

pub fn strv_type() -> glib::Type {
    <glib::StrV as glib::types::StaticType>::static_type()
}

pub fn gtype_type() -> glib::Type {
    <glib::Type as glib::types::StaticType>::static_type()
}
