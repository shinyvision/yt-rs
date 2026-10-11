//! Introspection data: functions, types, arguments, fields... each a
//! reference-counted `GIBaseInfo`.

use std::ffi::{CStr, c_char, c_uint};
use std::ptr::NonNull;
use std::sync::OnceLock;

use glib::ffi::GType;

use crate::sys;

/// One reference to a `GIBaseInfo`.
pub struct Info(NonNull<sys::GIBaseInfo>);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Function,
    Callback,
    Signal,
    VFunc,
    Object,
    Interface,
    Struct,
    Union,
    Enum,
    Flags,
    Constant,
    Value,
    Field,
    Property,
    Arg,
    Type,
    Unresolved,
}

struct Kinds {
    function: GType,
    callback: GType,
    signal: GType,
    vfunc: GType,
    object: GType,
    interface: GType,
    struct_: GType,
    union: GType,
    enum_: GType,
    flags: GType,
    constant: GType,
    value: GType,
    field: GType,
    property: GType,
    arg: GType,
    type_: GType,
    unresolved: GType,
}

fn kinds() -> &'static Kinds {
    static KINDS: OnceLock<Kinds> = OnceLock::new();
    // SAFETY: the type getters only register and return GTypes.
    KINDS.get_or_init(|| unsafe {
        Kinds {
            function: sys::gi_function_info_get_type(),
            callback: sys::gi_callback_info_get_type(),
            signal: sys::gi_signal_info_get_type(),
            vfunc: sys::gi_vfunc_info_get_type(),
            object: sys::gi_object_info_get_type(),
            interface: sys::gi_interface_info_get_type(),
            struct_: sys::gi_struct_info_get_type(),
            union: sys::gi_union_info_get_type(),
            enum_: sys::gi_enum_info_get_type(),
            flags: sys::gi_flags_info_get_type(),
            constant: sys::gi_constant_info_get_type(),
            value: sys::gi_value_info_get_type(),
            field: sys::gi_field_info_get_type(),
            property: sys::gi_property_info_get_type(),
            arg: sys::gi_arg_info_get_type(),
            type_: sys::gi_type_info_get_type(),
            unresolved: sys::gi_unresolved_info_get_type(),
        }
    })
}

/// A string owned by the typelib (which stays loaded), or "".
pub(crate) fn static_str(ptr: *const c_char) -> &'static str {
    if ptr.is_null() {
        return "";
    }
    // SAFETY: GI strings point into the mapped typelib, which is never
    // unloaded, and are UTF-8.
    unsafe { CStr::from_ptr(ptr) }.to_str().unwrap_or("")
}

impl Info {
    /// Takes over a reference returned by GI (transfer full).
    ///
    /// # Safety
    /// `ptr` is null or a `GIBaseInfo` the caller owns a reference to.
    pub unsafe fn from_full(ptr: *mut sys::GIBaseInfo) -> Option<Self> {
        NonNull::new(ptr).map(Self)
    }

    /// Adds a reference to a borrowed info.
    ///
    /// # Safety
    /// `ptr` is null or a valid `GIBaseInfo`.
    pub unsafe fn from_none(ptr: *mut sys::GIBaseInfo) -> Option<Self> {
        // SAFETY: as the caller promises.
        NonNull::new(ptr).map(|p| {
            Self(unsafe { NonNull::new_unchecked(sys::gi_base_info_ref(p.as_ptr().cast())) })
        })
    }

    pub fn ptr(&self) -> *mut sys::GIBaseInfo {
        self.0.as_ptr()
    }

    pub fn kind(&self) -> Kind {
        let k = kinds();
        let p = self.ptr();
        // Flags infos are enum infos too: test them first.
        for (ty, kind) in [
            (k.function, Kind::Function),
            (k.callback, Kind::Callback),
            (k.signal, Kind::Signal),
            (k.vfunc, Kind::VFunc),
            (k.object, Kind::Object),
            (k.interface, Kind::Interface),
            (k.struct_, Kind::Struct),
            (k.union, Kind::Union),
            (k.flags, Kind::Flags),
            (k.enum_, Kind::Enum),
            (k.constant, Kind::Constant),
            (k.value, Kind::Value),
            (k.field, Kind::Field),
            (k.property, Kind::Property),
            (k.arg, Kind::Arg),
            (k.type_, Kind::Type),
            (k.unresolved, Kind::Unresolved),
        ] {
            if sys::is_a(p, ty) {
                return kind;
            }
        }
        Kind::Unresolved
    }

    pub fn name(&self) -> &'static str {
        // SAFETY: a valid info.
        static_str(unsafe { sys::gi_base_info_get_name(self.ptr()) })
    }

    pub fn namespace(&self) -> &'static str {
        // SAFETY: a valid info.
        static_str(unsafe { sys::gi_base_info_get_namespace(self.ptr()) })
    }

    /// `Namespace.Name`, for messages.
    pub fn full_name(&self) -> String {
        format!("{}.{}", self.namespace(), self.name())
    }

    /// The info this one belongs to (a method's class), borrowed.
    pub fn container(&self) -> Option<Info> {
        // SAFETY: the container is borrowed; from_none adds a reference.
        unsafe { Info::from_none(sys::gi_base_info_get_container(self.ptr())) }
    }

    /// For registered types (objects, interfaces, structs, unions, enums):
    /// the GType, or `G_TYPE_NONE` when the type isn't registered.
    pub fn gtype(&self) -> glib::Type {
        use glib::translate::FromGlib;
        // SAFETY: only called on registered type infos.
        unsafe { glib::Type::from_glib(sys::gi_registered_type_info_get_g_type(self.ptr())) }
    }

    /// The address of a C symbol of this info's library.
    pub fn symbol(&self, name: &str) -> Option<*mut std::ffi::c_void> {
        let name = std::ffi::CString::new(name).ok()?;
        let mut address = std::ptr::null_mut();
        // SAFETY: a valid info and typelib; `address` receives the symbol.
        let found = unsafe {
            sys::gi_typelib_symbol(
                sys::gi_base_info_get_typelib(self.ptr()),
                name.as_ptr(),
                &mut address,
            )
        };
        (found != glib::ffi::GFALSE && !address.is_null()).then_some(address)
    }
}

impl Clone for Info {
    fn clone(&self) -> Self {
        // SAFETY: adds a reference to a valid info.
        unsafe { sys::gi_base_info_ref(self.ptr().cast()) };
        Self(self.0)
    }
}

impl Drop for Info {
    fn drop(&mut self) {
        // SAFETY: releases the reference this value owns.
        unsafe { sys::gi_base_info_unref(self.ptr().cast()) }
    }
}

impl PartialEq for Info {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

/// Iterates `n` child infos (`get_n_x` / `get_x` pairs).
pub fn children(
    info: &Info,
    count: unsafe extern "C" fn(*mut sys::GIBaseInfo) -> c_uint,
    get: unsafe extern "C" fn(*mut sys::GIBaseInfo, c_uint) -> *mut sys::GIBaseInfo,
) -> impl Iterator<Item = Info> + '_ {
    // SAFETY: valid info; the getters return new references.
    let n = unsafe { count(info.ptr()) };
    (0..n).filter_map(move |i| unsafe { Info::from_full(get(info.ptr(), i)) })
}
