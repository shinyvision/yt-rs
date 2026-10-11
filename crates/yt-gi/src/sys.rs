//! The parts of libgirepository-2.0 the bridge uses (GLib 2.80 and later).

#![allow(non_camel_case_types, dead_code)]

use std::ffi::{c_char, c_int, c_uint, c_void};

use glib::ffi::{GError, GType, gboolean};

/// Every info (functions, types, arguments...) is a `GIBaseInfo`; the
/// kinds are told apart by their GType.
pub type GIBaseInfo = c_void;
pub type GITypelib = c_void;
pub type GIRepository = c_void;

#[repr(C)]
#[derive(Clone, Copy)]
pub union GIArgument {
    pub v_boolean: gboolean,
    pub v_int8: i8,
    pub v_uint8: u8,
    pub v_int16: i16,
    pub v_uint16: u16,
    pub v_int32: i32,
    pub v_uint32: u32,
    pub v_int64: i64,
    pub v_uint64: u64,
    pub v_float: f32,
    pub v_double: f64,
    pub v_size: usize,
    pub v_string: *mut c_char,
    pub v_pointer: *mut c_void,
}

impl Default for GIArgument {
    fn default() -> Self {
        Self { v_uint64: 0 }
    }
}

pub type GITypeTag = c_int;
pub const TAG_VOID: GITypeTag = 0;
pub const TAG_BOOLEAN: GITypeTag = 1;
pub const TAG_INT8: GITypeTag = 2;
pub const TAG_UINT8: GITypeTag = 3;
pub const TAG_INT16: GITypeTag = 4;
pub const TAG_UINT16: GITypeTag = 5;
pub const TAG_INT32: GITypeTag = 6;
pub const TAG_UINT32: GITypeTag = 7;
pub const TAG_INT64: GITypeTag = 8;
pub const TAG_UINT64: GITypeTag = 9;
pub const TAG_FLOAT: GITypeTag = 10;
pub const TAG_DOUBLE: GITypeTag = 11;
pub const TAG_GTYPE: GITypeTag = 12;
pub const TAG_UTF8: GITypeTag = 13;
pub const TAG_FILENAME: GITypeTag = 14;
pub const TAG_ARRAY: GITypeTag = 15;
pub const TAG_INTERFACE: GITypeTag = 16;
pub const TAG_GLIST: GITypeTag = 17;
pub const TAG_GSLIST: GITypeTag = 18;
pub const TAG_GHASH: GITypeTag = 19;
pub const TAG_ERROR: GITypeTag = 20;
pub const TAG_UNICHAR: GITypeTag = 21;

pub type GIDirection = c_int;
pub const DIRECTION_IN: GIDirection = 0;
pub const DIRECTION_OUT: GIDirection = 1;
pub const DIRECTION_INOUT: GIDirection = 2;

pub type GITransfer = c_int;
pub const TRANSFER_NOTHING: GITransfer = 0;
pub const TRANSFER_CONTAINER: GITransfer = 1;
pub const TRANSFER_EVERYTHING: GITransfer = 2;

pub type GIScopeType = c_int;
pub const SCOPE_INVALID: GIScopeType = 0;
pub const SCOPE_CALL: GIScopeType = 1;
pub const SCOPE_ASYNC: GIScopeType = 2;
pub const SCOPE_NOTIFIED: GIScopeType = 3;
pub const SCOPE_FOREVER: GIScopeType = 4;

pub type GIArrayType = c_int;
pub const ARRAY_C: GIArrayType = 0;
pub const ARRAY_ARRAY: GIArrayType = 1;
pub const ARRAY_PTR_ARRAY: GIArrayType = 2;
pub const ARRAY_BYTE_ARRAY: GIArrayType = 3;

pub type GIFunctionInfoFlags = c_uint;
pub const FUNCTION_IS_METHOD: GIFunctionInfoFlags = 1 << 0;
pub const FUNCTION_IS_CONSTRUCTOR: GIFunctionInfoFlags = 1 << 1;

pub type GIFieldInfoFlags = c_uint;
pub const FIELD_IS_READABLE: GIFieldInfoFlags = 1 << 0;
pub const FIELD_IS_WRITABLE: GIFieldInfoFlags = 1 << 1;

/// libffi's `ffi_cif`, only ever handled through pointers. Its size differs
/// between platforms; this is larger than any of them.
#[repr(C, align(16))]
pub struct ffi_cif {
    _opaque: [u8; 128],
}

pub type ffi_closure = c_void;
pub type GIFFIClosureCallback = unsafe extern "C" fn(
    cif: *mut ffi_cif,
    ret: *mut c_void,
    args: *mut *mut c_void,
    user_data: *mut c_void,
);

unsafe extern "C" {
    // Kinds of info.
    pub fn gi_base_info_get_type() -> GType;
    pub fn gi_callable_info_get_type() -> GType;
    pub fn gi_function_info_get_type() -> GType;
    pub fn gi_callback_info_get_type() -> GType;
    pub fn gi_signal_info_get_type() -> GType;
    pub fn gi_vfunc_info_get_type() -> GType;
    pub fn gi_registered_type_info_get_type() -> GType;
    pub fn gi_object_info_get_type() -> GType;
    pub fn gi_interface_info_get_type() -> GType;
    pub fn gi_struct_info_get_type() -> GType;
    pub fn gi_union_info_get_type() -> GType;
    pub fn gi_enum_info_get_type() -> GType;
    pub fn gi_flags_info_get_type() -> GType;
    pub fn gi_constant_info_get_type() -> GType;
    pub fn gi_value_info_get_type() -> GType;
    pub fn gi_field_info_get_type() -> GType;
    pub fn gi_property_info_get_type() -> GType;
    pub fn gi_unresolved_info_get_type() -> GType;
    pub fn gi_arg_info_get_type() -> GType;
    pub fn gi_type_info_get_type() -> GType;

    // Repository.
    pub fn gi_repository_new() -> *mut GIRepository;
    pub fn gi_repository_prepend_search_path(repo: *mut GIRepository, dir: *const c_char);
    pub fn gi_repository_require(
        repo: *mut GIRepository,
        namespace: *const c_char,
        version: *const c_char,
        flags: c_uint,
        error: *mut *mut GError,
    ) -> *mut GITypelib;
    pub fn gi_repository_get_c_prefix(
        repo: *mut GIRepository,
        namespace: *const c_char,
    ) -> *const c_char;
    pub fn gi_repository_get_version(
        repo: *mut GIRepository,
        namespace: *const c_char,
    ) -> *const c_char;
    pub fn gi_repository_find_by_name(
        repo: *mut GIRepository,
        namespace: *const c_char,
        name: *const c_char,
    ) -> *mut GIBaseInfo;
    pub fn gi_repository_find_by_gtype(repo: *mut GIRepository, gtype: GType) -> *mut GIBaseInfo;
    pub fn gi_repository_get_loaded_namespaces(
        repo: *mut GIRepository,
        n: *mut usize,
    ) -> *mut *mut c_char;
    pub fn gi_typelib_symbol(
        typelib: *mut GITypelib,
        name: *const c_char,
        symbol: *mut *mut c_void,
    ) -> gboolean;

    // Base info.
    pub fn gi_base_info_ref(info: *mut c_void) -> *mut GIBaseInfo;
    pub fn gi_base_info_unref(info: *mut c_void);
    pub fn gi_base_info_get_name(info: *mut GIBaseInfo) -> *const c_char;
    pub fn gi_base_info_get_namespace(info: *mut GIBaseInfo) -> *const c_char;
    pub fn gi_base_info_get_container(info: *mut GIBaseInfo) -> *mut GIBaseInfo;
    pub fn gi_base_info_get_typelib(info: *mut GIBaseInfo) -> *mut GITypelib;
    pub fn gi_base_info_is_deprecated(info: *mut GIBaseInfo) -> gboolean;

    // Callables.
    pub fn gi_callable_info_is_method(info: *mut GIBaseInfo) -> gboolean;
    pub fn gi_callable_info_can_throw_gerror(info: *mut GIBaseInfo) -> gboolean;
    pub fn gi_callable_info_get_return_type(info: *mut GIBaseInfo) -> *mut GIBaseInfo;
    pub fn gi_callable_info_get_caller_owns(info: *mut GIBaseInfo) -> GITransfer;
    pub fn gi_callable_info_may_return_null(info: *mut GIBaseInfo) -> gboolean;
    pub fn gi_callable_info_skip_return(info: *mut GIBaseInfo) -> gboolean;
    pub fn gi_callable_info_get_n_args(info: *mut GIBaseInfo) -> c_uint;
    pub fn gi_callable_info_get_arg(info: *mut GIBaseInfo, n: c_uint) -> *mut GIBaseInfo;
    pub fn gi_callable_info_get_instance_ownership_transfer(info: *mut GIBaseInfo) -> GITransfer;
    pub fn gi_callable_info_invoke(
        info: *mut GIBaseInfo,
        function: *mut c_void,
        in_args: *const GIArgument,
        n_in: usize,
        out_args: *mut GIArgument,
        n_out: usize,
        return_value: *mut GIArgument,
        error: *mut *mut GError,
    ) -> gboolean;
    pub fn gi_callable_info_create_closure(
        info: *mut GIBaseInfo,
        cif: *mut ffi_cif,
        callback: GIFFIClosureCallback,
        user_data: *mut c_void,
    ) -> *mut ffi_closure;
    pub fn gi_callable_info_get_closure_native_address(
        info: *mut GIBaseInfo,
        closure: *mut ffi_closure,
    ) -> *mut c_void;
    pub fn gi_callable_info_destroy_closure(info: *mut GIBaseInfo, closure: *mut ffi_closure);

    // Functions.
    pub fn gi_function_info_get_symbol(info: *mut GIBaseInfo) -> *const c_char;
    pub fn gi_function_info_get_flags(info: *mut GIBaseInfo) -> GIFunctionInfoFlags;

    // Arguments.
    pub fn gi_arg_info_get_direction(info: *mut GIBaseInfo) -> GIDirection;
    pub fn gi_arg_info_is_return_value(info: *mut GIBaseInfo) -> gboolean;
    pub fn gi_arg_info_is_optional(info: *mut GIBaseInfo) -> gboolean;
    pub fn gi_arg_info_is_caller_allocates(info: *mut GIBaseInfo) -> gboolean;
    pub fn gi_arg_info_may_be_null(info: *mut GIBaseInfo) -> gboolean;
    pub fn gi_arg_info_is_skip(info: *mut GIBaseInfo) -> gboolean;
    pub fn gi_arg_info_get_ownership_transfer(info: *mut GIBaseInfo) -> GITransfer;
    pub fn gi_arg_info_get_scope(info: *mut GIBaseInfo) -> GIScopeType;
    pub fn gi_arg_info_get_closure_index(info: *mut GIBaseInfo, out: *mut c_uint) -> gboolean;
    pub fn gi_arg_info_get_destroy_index(info: *mut GIBaseInfo, out: *mut c_uint) -> gboolean;
    pub fn gi_arg_info_get_type_info(info: *mut GIBaseInfo) -> *mut GIBaseInfo;

    // Types.
    pub fn gi_type_info_is_pointer(info: *mut GIBaseInfo) -> gboolean;
    pub fn gi_type_info_get_tag(info: *mut GIBaseInfo) -> GITypeTag;
    pub fn gi_type_info_get_param_type(info: *mut GIBaseInfo, n: c_uint) -> *mut GIBaseInfo;
    pub fn gi_type_info_get_interface(info: *mut GIBaseInfo) -> *mut GIBaseInfo;
    pub fn gi_type_info_get_array_length_index(info: *mut GIBaseInfo, out: *mut c_uint)
    -> gboolean;
    pub fn gi_type_info_get_array_fixed_size(info: *mut GIBaseInfo, out: *mut usize) -> gboolean;
    pub fn gi_type_info_is_zero_terminated(info: *mut GIBaseInfo) -> gboolean;
    pub fn gi_type_info_get_array_type(info: *mut GIBaseInfo) -> GIArrayType;
    pub fn gi_type_info_argument_from_hash_pointer(
        info: *mut GIBaseInfo,
        hash_pointer: *mut c_void,
        arg: *mut GIArgument,
    );
    pub fn gi_type_info_hash_pointer_from_argument(
        info: *mut GIBaseInfo,
        arg: *mut GIArgument,
    ) -> *mut c_void;
    pub fn gi_type_tag_to_string(tag: GITypeTag) -> *const c_char;

    // Registered types.
    pub fn gi_registered_type_info_get_g_type(info: *mut GIBaseInfo) -> GType;
    pub fn gi_registered_type_info_get_type_name(info: *mut GIBaseInfo) -> *const c_char;
    pub fn gi_registered_type_info_is_boxed(info: *mut GIBaseInfo) -> gboolean;

    // Objects.
    pub fn gi_object_info_get_parent(info: *mut GIBaseInfo) -> *mut GIBaseInfo;
    pub fn gi_object_info_get_abstract(info: *mut GIBaseInfo) -> gboolean;
    pub fn gi_object_info_get_fundamental(info: *mut GIBaseInfo) -> gboolean;
    pub fn gi_object_info_get_n_interfaces(info: *mut GIBaseInfo) -> c_uint;
    pub fn gi_object_info_get_interface(info: *mut GIBaseInfo, n: c_uint) -> *mut GIBaseInfo;
    pub fn gi_object_info_get_n_methods(info: *mut GIBaseInfo) -> c_uint;
    pub fn gi_object_info_get_method(info: *mut GIBaseInfo, n: c_uint) -> *mut GIBaseInfo;
    pub fn gi_object_info_find_method(
        info: *mut GIBaseInfo,
        name: *const c_char,
    ) -> *mut GIBaseInfo;
    pub fn gi_object_info_find_vfunc(info: *mut GIBaseInfo, name: *const c_char)
    -> *mut GIBaseInfo;
    pub fn gi_object_info_get_n_constants(info: *mut GIBaseInfo) -> c_uint;
    pub fn gi_object_info_get_constant(info: *mut GIBaseInfo, n: c_uint) -> *mut GIBaseInfo;
    pub fn gi_object_info_get_n_fields(info: *mut GIBaseInfo) -> c_uint;
    pub fn gi_object_info_get_field(info: *mut GIBaseInfo, n: c_uint) -> *mut GIBaseInfo;
    pub fn gi_object_info_get_class_struct(info: *mut GIBaseInfo) -> *mut GIBaseInfo;
    pub fn gi_object_info_get_ref_function_name(info: *mut GIBaseInfo) -> *const c_char;
    pub fn gi_object_info_get_unref_function_name(info: *mut GIBaseInfo) -> *const c_char;

    // Interfaces.
    pub fn gi_interface_info_find_method(
        info: *mut GIBaseInfo,
        name: *const c_char,
    ) -> *mut GIBaseInfo;
    pub fn gi_interface_info_find_vfunc(
        info: *mut GIBaseInfo,
        name: *const c_char,
    ) -> *mut GIBaseInfo;
    pub fn gi_interface_info_get_n_constants(info: *mut GIBaseInfo) -> c_uint;
    pub fn gi_interface_info_get_constant(info: *mut GIBaseInfo, n: c_uint) -> *mut GIBaseInfo;
    pub fn gi_interface_info_get_iface_struct(info: *mut GIBaseInfo) -> *mut GIBaseInfo;

    // Structs and unions.
    pub fn gi_struct_info_get_n_fields(info: *mut GIBaseInfo) -> c_uint;
    pub fn gi_struct_info_get_field(info: *mut GIBaseInfo, n: c_uint) -> *mut GIBaseInfo;
    pub fn gi_struct_info_find_field(info: *mut GIBaseInfo, name: *const c_char)
    -> *mut GIBaseInfo;
    pub fn gi_struct_info_find_method(
        info: *mut GIBaseInfo,
        name: *const c_char,
    ) -> *mut GIBaseInfo;
    pub fn gi_struct_info_get_size(info: *mut GIBaseInfo) -> usize;
    pub fn gi_struct_info_is_gtype_struct(info: *mut GIBaseInfo) -> gboolean;
    pub fn gi_struct_info_is_foreign(info: *mut GIBaseInfo) -> gboolean;
    pub fn gi_struct_info_get_free_function_name(info: *mut GIBaseInfo) -> *const c_char;
    pub fn gi_union_info_get_n_fields(info: *mut GIBaseInfo) -> c_uint;
    pub fn gi_union_info_get_field(info: *mut GIBaseInfo, n: c_uint) -> *mut GIBaseInfo;
    pub fn gi_union_info_find_method(info: *mut GIBaseInfo, name: *const c_char)
    -> *mut GIBaseInfo;
    pub fn gi_union_info_get_size(info: *mut GIBaseInfo) -> usize;
    pub fn gi_union_info_get_free_function_name(info: *mut GIBaseInfo) -> *const c_char;

    // Fields.
    pub fn gi_field_info_get_flags(info: *mut GIBaseInfo) -> GIFieldInfoFlags;
    pub fn gi_field_info_get_offset(info: *mut GIBaseInfo) -> usize;
    pub fn gi_field_info_get_type_info(info: *mut GIBaseInfo) -> *mut GIBaseInfo;

    // Enums and flags.
    pub fn gi_enum_info_get_n_values(info: *mut GIBaseInfo) -> c_uint;
    pub fn gi_enum_info_get_value(info: *mut GIBaseInfo, n: c_uint) -> *mut GIBaseInfo;
    pub fn gi_enum_info_get_n_methods(info: *mut GIBaseInfo) -> c_uint;
    pub fn gi_enum_info_get_method(info: *mut GIBaseInfo, n: c_uint) -> *mut GIBaseInfo;
    pub fn gi_enum_info_get_storage_type(info: *mut GIBaseInfo) -> GITypeTag;
    pub fn gi_enum_info_get_error_domain(info: *mut GIBaseInfo) -> *const c_char;
    pub fn gi_value_info_get_value(info: *mut GIBaseInfo) -> i64;

    // Constants.
    pub fn gi_constant_info_get_type_info(info: *mut GIBaseInfo) -> *mut GIBaseInfo;
    pub fn gi_constant_info_get_value(info: *mut GIBaseInfo, value: *mut GIArgument) -> usize;
    pub fn gi_constant_info_free_value(info: *mut GIBaseInfo, value: *mut GIArgument);

    // Virtual functions and signals.
    pub fn gi_vfunc_info_get_offset(info: *mut GIBaseInfo) -> usize;
    pub fn gi_vfunc_info_get_address(
        info: *mut GIBaseInfo,
        implementor: GType,
        error: *mut *mut GError,
    ) -> *mut c_void;
    pub fn gi_signal_info_get_flags(info: *mut GIBaseInfo) -> c_uint;
}

/// Whether `info` is of the info kind `kind` (`gi_function_info_get_type()`
/// and so on), as the `GI_IS_*_INFO` macros check.
pub fn is_a(info: *mut GIBaseInfo, kind: GType) -> bool {
    // SAFETY: infos are GTypeInstances; the check reads only the class.
    !info.is_null()
        && unsafe { glib::gobject_ffi::g_type_check_instance_is_a(info.cast(), kind) }
            != glib::ffi::GFALSE
}
