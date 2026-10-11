//! C structs and unions as Lua values: `Gdk.RGBA`, `Graphene.Rect`,
//! `Pango.FontDescription`, `GLib.Variant`, `GLib.Error`...
//!
//! ```lua
//! local color = Gdk.RGBA { red = 1, alpha = 0.5 }   -- fields
//! color:parse("#3584e4")                             -- methods
//! print(color.blue)
//! ```

use std::ffi::c_void;

use glib::translate::{FromGlib, IntoGlib};
use mlua::{Lua, MetaMethod, UserData, UserDataMethods};

use crate::convert;
use crate::info::{Info, Kind};
use crate::members;
use crate::sys;
use crate::types::{Ty, Type};

pub struct Record {
    pub ptr: *mut c_void,
    /// A struct or union info.
    pub info: Info,
    owner: Owner,
}

/// Who frees the memory.
pub enum Owner {
    /// A boxed copy of ours, freed with `g_boxed_free`.
    Boxed(glib::Type),
    /// Memory we allocated, freed with `g_free`.
    Malloc,
    /// A reference to a `GVariant`.
    Variant,
    /// Freed with the struct's own free function.
    Free(unsafe extern "C" fn(*mut c_void)),
    /// Someone else's; `keep` keeps that someone alive.
    Borrowed(#[allow(dead_code)] Keep),
}

pub enum Keep {
    Nothing,
    Lua(mlua::Value),
}

impl Drop for Record {
    fn drop(&mut self) {
        // SAFETY: frees what this record owns, as it was allocated.
        unsafe {
            match &self.owner {
                Owner::Boxed(t) => glib::gobject_ffi::g_boxed_free(t.into_glib(), self.ptr),
                Owner::Malloc => glib::ffi::g_free(self.ptr),
                Owner::Variant => glib::ffi::g_variant_unref(self.ptr.cast()),
                Owner::Free(f) => f(self.ptr),
                Owner::Borrowed(_) => {}
            }
        }
    }
}

pub fn size(info: &Info) -> usize {
    // SAFETY: struct and union infos.
    unsafe {
        match info.kind() {
            Kind::Union => sys::gi_union_info_get_size(info.ptr()),
            _ => sys::gi_struct_info_get_size(info.ptr()),
        }
    }
}

/// Whether a registered type is boxed (has copy and free functions).
pub fn is_boxed(gtype: glib::Type) -> bool {
    gtype.is_valid() && gtype != glib::Type::UNIT && gtype.is_a(glib::Type::BOXED)
}

impl Record {
    /// Takes over memory allocated by `g_malloc`.
    pub fn owned(ptr: *mut c_void, info: Info) -> Record {
        Record {
            ptr,
            info,
            owner: Owner::Malloc,
        }
    }

    pub fn with_owner(ptr: *mut c_void, info: Info, owner: Owner) -> Record {
        Record { ptr, info, owner }
    }

    /// A zeroed new struct, for `Gdk.RGBA()`.
    pub fn new_zeroed(info: Info) -> mlua::Result<Record> {
        let size = size(&info);
        if size == 0 {
            return Err(mlua::Error::runtime(format!(
                "a {} can't be made directly: its contents are private (look for a constructor function)",
                info.full_name()
            )));
        }
        // SAFETY: allocates zeroed memory of the struct's size.
        let ptr = unsafe { glib::ffi::g_malloc0(size) };
        Ok(Record::owned(ptr, info))
    }

    /// A record for a pointer GI handed out: copied when it's boxed and
    /// not ours, taken over when it is.
    ///
    /// # Safety
    /// `ptr` points to a struct of `info`'s type; with `full`, one the
    /// caller owns.
    pub unsafe fn from_gi(ptr: *mut c_void, info: Info, full: bool, keep: Keep) -> Record {
        let gtype = info.gtype();
        if gtype == glib::Type::VARIANT {
            if !full {
                // SAFETY: a GVariant; sinks a floating one or adds a reference.
                unsafe { glib::ffi::g_variant_ref_sink(ptr.cast()) };
            }
            return Record::with_owner(ptr, info, Owner::Variant);
        }
        if is_boxed(gtype) {
            let ptr = if full {
                ptr
            } else {
                // SAFETY: a boxed value of this type.
                unsafe { glib::gobject_ffi::g_boxed_copy(gtype.into_glib(), ptr) }
            };
            return Record::with_owner(ptr, info, Owner::Boxed(gtype));
        }
        if full {
            // SAFETY: free function names come with the struct's info.
            let free = unsafe {
                crate::info::static_str(match info.kind() {
                    Kind::Union => sys::gi_union_info_get_free_function_name(info.ptr()),
                    _ => sys::gi_struct_info_get_free_function_name(info.ptr()),
                })
            };
            let owner = match info.symbol(free).filter(|_| !free.is_empty()) {
                // SAFETY: the struct's free function takes a pointer to it.
                Some(f) => Owner::Free(unsafe {
                    std::mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut c_void)>(f)
                }),
                None => Owner::Malloc,
            };
            return Record::with_owner(ptr, info, owner);
        }
        Record::with_owner(ptr, info, Owner::Borrowed(keep))
    }

    /// A copy of a struct embedded somewhere (in an array, by value).
    pub fn copy_of(ptr: *const c_void, info: Info) -> Record {
        let size = size(&info);
        // SAFETY: copies `size` bytes of a struct of that size.
        let copy = unsafe {
            let copy = glib::ffi::g_malloc0(size.max(1));
            std::ptr::copy_nonoverlapping(ptr as *const u8, copy as *mut u8, size);
            copy
        };
        Record::owned(copy, info)
    }

    fn is_error(&self) -> bool {
        self.info.gtype() == glib::Type::from_name("GError").unwrap_or(glib::Type::INVALID)
    }
}

/// A field of a struct, union or instance type.
pub struct Field {
    pub name: &'static str,
    pub offset: usize,
    pub ty: Type,
    pub readable: bool,
    pub writable: bool,
}

impl Field {
    pub fn new(info: &Info) -> Field {
        // SAFETY: a field info.
        unsafe {
            let flags = sys::gi_field_info_get_flags(info.ptr());
            Field {
                name: info.name(),
                offset: sys::gi_field_info_get_offset(info.ptr()),
                ty: Type::new(
                    Info::from_full(sys::gi_field_info_get_type_info(info.ptr()))
                        .expect("field type"),
                ),
                readable: flags & sys::FIELD_IS_READABLE != 0,
                writable: flags & sys::FIELD_IS_WRITABLE != 0,
            }
        }
    }

    /// Reads the field of the struct at `base`; `owner` is the Lua value
    /// holding that memory, kept alive by embedded structs read from it.
    pub fn get(&self, lua: &Lua, base: *mut u8, owner: &mlua::Value) -> mlua::Result<mlua::Value> {
        if !self.readable {
            return Err(mlua::Error::runtime(format!(
                "field {} can't be read",
                self.name
            )));
        }
        // SAFETY: the offset is within the struct GI describes.
        let at = unsafe { base.add(self.offset) };
        if !self.ty.pointer
            && let Ty::Struct(info) | Ty::Union(info) = &self.ty.kind
        {
            let record = Record::with_owner(
                at.cast(),
                info.clone(),
                Owner::Borrowed(Keep::Lua(owner.clone())),
            );
            return Ok(mlua::Value::UserData(lua.create_userdata(record)?));
        }
        // SAFETY: reads a value of the field's type.
        let arg = unsafe { convert::load(at, &self.ty) };
        convert::to_lua(lua, &arg, &self.ty, sys::TRANSFER_NOTHING, None)
    }

    pub fn set(&self, lua: &Lua, base: *mut u8, value: &mlua::Value) -> mlua::Result<()> {
        if !self.writable {
            return Err(mlua::Error::runtime(format!(
                "field {} can't be written",
                self.name
            )));
        }
        let simple = matches!(
            self.ty.kind,
            Ty::Bool
                | Ty::Int(_)
                | Ty::Float
                | Ty::Double
                | Ty::GType
                | Ty::Unichar
                | Ty::Enum { .. }
        ) && !self.ty.pointer;
        let embedded = !self.ty.pointer && matches!(self.ty.kind, Ty::Struct(_) | Ty::Union(_));
        if !simple && !embedded {
            return Err(mlua::Error::runtime(format!(
                "field {} holds a {}, which can't be set from Lua",
                self.name,
                self.ty.describe()
            )));
        }
        let mut scratch = convert::Scratch::default();
        let arg = convert::from_lua(
            lua,
            value,
            &self.ty,
            sys::TRANSFER_NOTHING,
            false,
            &mut scratch,
        )
        .map_err(|e| mlua::Error::runtime(format!("field {}: {e}", self.name)))?;
        // SAFETY: writes a value of the field's type within the struct.
        unsafe { convert::store(base.add(self.offset), &self.ty, &arg) };
        Ok(())
    }
}

fn find_field(info: &Info, name: &str) -> Option<Field> {
    let fields: Box<dyn Iterator<Item = Info>> = match info.kind() {
        Kind::Union => Box::new(crate::info::children(
            info,
            sys::gi_union_info_get_n_fields,
            sys::gi_union_info_get_field,
        )),
        Kind::Object => Box::new(crate::info::children(
            info,
            sys::gi_object_info_get_n_fields,
            sys::gi_object_info_get_field,
        )),
        _ => Box::new(crate::info::children(
            info,
            sys::gi_struct_info_get_n_fields,
            sys::gi_struct_info_get_field,
        )),
    };
    fields
        .into_iter()
        .find(|f| f.name() == name)
        .map(|f| Field::new(&f))
}

/// The field `name` of a struct, union or instance type, cached.
pub fn field(info: &Info, name: &str) -> Option<std::rc::Rc<Field>> {
    use std::cell::RefCell;
    use std::collections::HashMap;
    type Cache = HashMap<(String, String), Option<std::rc::Rc<Field>>>;
    thread_local! {
        static FIELDS: RefCell<Cache> = RefCell::new(HashMap::new());
    }
    let key = (info.full_name(), name.to_owned());
    if let Some(found) = FIELDS.with(|f| f.borrow().get(&key).cloned()) {
        return found;
    }
    let found = find_field(info, name).map(std::rc::Rc::new);
    FIELDS.with(|f| f.borrow_mut().insert(key, found.clone()));
    found
}

/// Fills a new struct's fields from a Lua table.
pub fn fill(lua: &Lua, record: &Record, fields: &mlua::Table) -> mlua::Result<()> {
    for pair in fields.pairs::<String, mlua::Value>() {
        let (name, value) = pair?;
        let field = field(&record.info, &name).ok_or_else(|| {
            mlua::Error::runtime(format!("{} has no field {name:?}", record.info.full_name()))
        })?;
        field.set(lua, record.ptr.cast(), &value)?;
    }
    Ok(())
}

impl UserData for Record {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("ptr", |_, this, ()| Ok(mlua::LightUserData(this.ptr)));
        methods.add_method("type_name", |_, this, ()| Ok(this.info.full_name()));
        methods.add_meta_function(
            MetaMethod::Index,
            |lua, (ud, key): (mlua::AnyUserData, mlua::Value)| {
                let Some(key) = key
                    .as_string()
                    .and_then(|s| s.to_str().ok())
                    .map(|s| s.to_string())
                else {
                    return Ok(mlua::Value::Nil);
                };
                let (info, ptr, error) = {
                    let this = ud.borrow::<Record>()?;
                    (this.info.clone(), this.ptr, this.is_error())
                };
                if let Some(f) = members::record(lua, &info, &key)? {
                    return Ok(mlua::Value::Function(f));
                }
                if error && key == "domain" {
                    // SAFETY: a GError.
                    let quark = unsafe { (*(ptr as *mut glib::ffi::GError)).domain };
                    // SAFETY: a quark GLib made.
                    let domain = unsafe { glib::Quark::from_glib(quark) };
                    return Ok(mlua::Value::String(lua.create_string(domain.as_str())?));
                }
                match field(&info, &key) {
                    Some(f) => f.get(lua, ptr.cast(), &mlua::Value::UserData(ud.clone())),
                    None => Ok(mlua::Value::Nil),
                }
            },
        );
        methods.add_meta_function(
            MetaMethod::NewIndex,
            |lua, (ud, key, value): (mlua::AnyUserData, String, mlua::Value)| {
                let (info, ptr) = {
                    let this = ud.borrow::<Record>()?;
                    (this.info.clone(), this.ptr)
                };
                let f = field(&info, &key).ok_or_else(|| {
                    mlua::Error::runtime(format!("{} has no field {key:?}", info.full_name()))
                })?;
                f.set(lua, ptr.cast(), &value)
            },
        );
        methods.add_meta_method(MetaMethod::Eq, |_, this, other: mlua::AnyUserData| {
            Ok(other.borrow::<Record>().is_ok_and(|o| o.ptr == this.ptr))
        });
        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            if this.is_error() {
                // SAFETY: a GError.
                let message = unsafe {
                    std::ffi::CStr::from_ptr((*(this.ptr as *mut glib::ffi::GError)).message)
                };
                return Ok(message.to_string_lossy().into_owned());
            }
            Ok(this.info.full_name())
        });
    }
}
