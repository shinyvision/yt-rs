//! The repository of loaded typelibs.

use std::cell::Cell;
use std::ffi::{CStr, CString};

use crate::info::Info;
use crate::sys;

thread_local! {
    static REPOSITORY: Cell<*mut sys::GIRepository> = const { Cell::new(std::ptr::null_mut()) };
}

/// The process's repository, created on first use. Main thread only, like
/// everything here.
fn repository() -> *mut sys::GIRepository {
    REPOSITORY.with(|r| {
        if r.get().is_null() {
            // SAFETY: creates the repository; it lives as long as the process.
            r.set(unsafe { sys::gi_repository_new() });
        }
        r.get()
    })
}

/// Adds a directory to look for typelibs in, before the system's (for
/// typelibs shipped with the app).
pub fn add_search_path(dir: &std::path::Path) {
    if let Ok(dir) = CString::new(dir.to_string_lossy().as_bytes()) {
        // SAFETY: a valid repository and path.
        unsafe { sys::gi_repository_prepend_search_path(repository(), dir.as_ptr()) }
    }
}

/// Loads a namespace (and what it depends on): the newest version, or
/// `version`. Returns the version loaded.
pub fn require(namespace: &str, version: Option<&str>) -> Result<String, String> {
    let ns = CString::new(namespace).map_err(|e| e.to_string())?;
    let version = version
        .map(CString::new)
        .transpose()
        .map_err(|e| e.to_string())?;
    let mut error = std::ptr::null_mut();
    // SAFETY: valid strings; the error is ours to free.
    let typelib = unsafe {
        sys::gi_repository_require(
            repository(),
            ns.as_ptr(),
            version.as_ref().map_or(std::ptr::null(), |v| v.as_ptr()),
            0,
            &mut error,
        )
    };
    if typelib.is_null() {
        // SAFETY: GI set the error.
        let error: glib::Error = unsafe { glib::translate::from_glib_full(error) };
        return Err(error.message().to_owned());
    }
    // SAFETY: the namespace is loaded; the version string is GI's.
    let loaded = unsafe { sys::gi_repository_get_version(repository(), ns.as_ptr()) };
    Ok(crate::info::static_str(loaded).to_owned())
}

/// The namespaces loaded so far.
pub fn loaded() -> Vec<String> {
    let mut n = 0;
    // SAFETY: returns a NULL-terminated string array we own.
    unsafe {
        let names = sys::gi_repository_get_loaded_namespaces(repository(), &mut n);
        let out = (0..n)
            .map(|i| CStr::from_ptr(*names.add(i)).to_string_lossy().into_owned())
            .collect();
        glib::ffi::g_strfreev(names);
        out
    }
}

pub fn find_by_name(namespace: &str, name: &str) -> Option<Info> {
    let ns = CString::new(namespace).ok()?;
    let name = CString::new(name).ok()?;
    // SAFETY: valid strings; returns a new reference or null.
    unsafe {
        Info::from_full(sys::gi_repository_find_by_name(
            repository(),
            ns.as_ptr(),
            name.as_ptr(),
        ))
    }
}

/// The introspection data of a GType, from the namespaces loaded.
pub fn find_by_gtype(gtype: glib::Type) -> Option<Info> {
    use glib::translate::IntoGlib;
    if !gtype.is_valid() {
        return None;
    }
    // SAFETY: returns a new reference or null.
    unsafe {
        Info::from_full(sys::gi_repository_find_by_gtype(
            repository(),
            gtype.into_glib(),
        ))
    }
}

/// A type by its C name (`GtkButton`), registering it if it isn't yet:
/// GTK registers most types only when first used.
pub fn type_by_name(name: &str) -> Option<glib::Type> {
    if let Some(t) = glib::Type::from_name(name) {
        return Some(t);
    }
    for ns in loaded() {
        let Ok(cns) = CString::new(ns.as_str()) else {
            continue;
        };
        // SAFETY: a loaded namespace; the prefix string is GI's.
        let prefixes = crate::info::static_str(unsafe {
            sys::gi_repository_get_c_prefix(repository(), cns.as_ptr())
        });
        for prefix in prefixes.split(',') {
            if let Some(rest) = name.strip_prefix(prefix).filter(|r| !r.is_empty())
                && let Some(info) = find_by_name(&ns, rest)
                && matches!(
                    info.kind(),
                    crate::info::Kind::Object
                        | crate::info::Kind::Interface
                        | crate::info::Kind::Struct
                        | crate::info::Kind::Union
                        | crate::info::Kind::Enum
                        | crate::info::Kind::Flags
                )
            {
                let t = info.gtype();
                if t.is_valid() && t != glib::Type::UNIT {
                    return Some(t);
                }
            }
        }
    }
    None
}
