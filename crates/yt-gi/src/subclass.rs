//! New GObject types defined in Lua: widgets that draw and measure
//! themselves, list models, anything that overrides virtual functions or
//! implements interfaces.
//!
//! ```lua
//! local Meter = gi.subclass(Gtk.Widget, {
//!   name = "MyPluginMeter",                 -- the GType name
//!   properties = { GObject.param_spec_double("level", nil, nil, 0, 1, 0, GObject.ParamFlags.READWRITE) },
//!   signals = { full = { params = { "gint" } } },
//!   interfaces = { Gtk.Accessible },        -- interfaces to implement
//!   vfuncs = {                              -- the parent's and interfaces'
//!     snapshot = function(self, snapshot) ... end,
//!   },
//!   init = function(self) end,              -- each new instance
//!   class_init = function(class) end,       -- the class struct, once
//! })
//! local m = Meter { level = 0.5 }
//! ```
//!
//! Instances also take any other field (`self.items = {}`), kept until the
//! instance is finalized. Chain up with `Parent.vfunc_<name>(self, ...)`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{CString, c_void};
use std::rc::Rc;

use glib::gobject_ffi as go;
use glib::translate::{FromGlib, IntoGlib};
use mlua::{Function, Lua, RegistryKey, Table};

use crate::call::Callable;
use crate::closure::Closure;
use crate::info::{Info, Kind};
use crate::namespace;
use crate::sys;

struct ClassData {
    lua: Lua,
    name: String,
    gtype: Cell<glib::Type>,
    parent: glib::Type,
    vfuncs: Vec<(String, RegistryKey)>,
    properties: Vec<*mut go::GParamSpec>,
    signals: Vec<(String, Vec<glib::Type>, glib::Type, u32)>,
    init: Option<RegistryKey>,
    class_init: Option<RegistryKey>,
    error: RefCell<Option<mlua::Error>>,
}

thread_local! {
    /// Property values of instances, by instance id and param spec.
    static VALUES: RefCell<HashMap<(u64, usize), glib::Value>> = RefCell::new(HashMap::new());
}

static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Instances finalized on other threads, whose fields and values are
/// forgotten on the main thread.
static DEAD: std::sync::Mutex<Vec<u64>> = std::sync::Mutex::new(Vec::new());

fn forget(lua: &Lua, id: u64) {
    if let Some(state) = lua.app_data_ref::<crate::State>() {
        state.fields.borrow_mut().remove(&id);
    }
    // Gone already when the process is exiting.
    let _ = VALUES.try_with(|v| v.borrow_mut().retain(|(i, _), _| *i != id));
}

/// Forgets instances finalized elsewhere since last time.
fn forget_dead(lua: &Lua) {
    let dead = std::mem::take(&mut *DEAD.lock().unwrap_or_else(|e| e.into_inner()));
    for id in dead {
        forget(lua, id);
    }
}

fn id_quark() -> glib::ffi::GQuark {
    // SAFETY: interns a static string.
    unsafe { glib::ffi::g_quark_from_static_string(c"yt-gi-instance".as_ptr()) }
}

/// The id of an instance of a Lua class (0 for other objects).
fn instance_id(object: *mut go::GObject) -> u64 {
    // SAFETY: reads qdata of a valid object.
    unsafe { go::g_object_get_qdata(object, id_quark()) as u64 }
}

fn class_quark() -> glib::ffi::GQuark {
    // SAFETY: interns a static string.
    unsafe { glib::ffi::g_quark_from_static_string(c"yt-gi-class".as_ptr()) }
}

/// The Lua class data of `gtype` itself (not an ancestor's). Kept as type
/// data, which GLib owns: instances can be finalized while the process
/// exits, after thread-local storage is gone.
fn own_class_data(gtype: glib::Type) -> Option<&'static ClassData> {
    // SAFETY: the data was set by `subclass` and lives forever.
    unsafe {
        let p = go::g_type_get_qdata(gtype.into_glib(), class_quark()) as *const ClassData;
        p.as_ref()
    }
}

pub fn subclass(lua: &Lua, parent: &mlua::Value, def: Table) -> mlua::Result<Table> {
    let parent = crate::gtype_from_lua(parent)?;
    if !parent.is_a(glib::Type::OBJECT) {
        return Err(mlua::Error::runtime(format!(
            "{} isn't a GObject class",
            parent.name()
        )));
    }
    let name: String = match def.get::<Option<String>>("name")? {
        Some(n) => n,
        None => {
            let n = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            format!("Lua{}{n}", parent.name())
        }
    };
    if glib::Type::from_name(&name).is_some() {
        return Err(mlua::Error::runtime(format!(
            "a type named {name} exists already"
        )));
    }
    let mut vfuncs = Vec::new();
    if let Some(t) = def.get::<Option<Table>>("vfuncs")? {
        for pair in t.pairs::<String, Function>() {
            let (k, f) = pair?;
            vfuncs.push((k, lua.create_registry_value(f)?));
        }
    }
    let mut properties = Vec::new();
    if let Some(t) = def.get::<Option<Table>>("properties")? {
        for v in t.sequence_values::<mlua::Value>() {
            let v = v?;
            let (p, ty) = crate::object::instance_ptr(&v).ok_or_else(|| {
                mlua::Error::runtime("properties are GParamSpecs (GObject.param_spec_*)")
            })?;
            if !ty.is_a(glib::Type::PARAM_SPEC) {
                return Err(mlua::Error::runtime(
                    "properties are GParamSpecs (GObject.param_spec_*)",
                ));
            }
            // SAFETY: a param spec; the class keeps a reference.
            unsafe { go::g_param_spec_ref_sink(p.cast()) };
            properties.push(p.cast());
        }
    }
    let mut signals = Vec::new();
    if let Some(t) = def.get::<Option<Table>>("signals")? {
        for pair in t.pairs::<String, Table>() {
            let (signal, spec) = pair?;
            let params = match spec.get::<Option<Table>>("params")? {
                Some(p) => p
                    .sequence_values::<mlua::Value>()
                    .map(|v| crate::gtype_from_lua(&v?))
                    .collect::<mlua::Result<Vec<_>>>()?,
                None => Vec::new(),
            };
            let ret = match spec.get::<mlua::Value>("ret")? {
                mlua::Value::Nil => glib::Type::UNIT,
                v => crate::gtype_from_lua(&v)?,
            };
            let flags = spec
                .get::<Option<u32>>("flags")?
                .unwrap_or(go::G_SIGNAL_RUN_LAST);
            signals.push((signal, params, ret, flags));
        }
    }
    let interfaces = match def.get::<Option<Table>>("interfaces")? {
        Some(t) => t
            .sequence_values::<mlua::Value>()
            .map(|v| crate::gtype_from_lua(&v?))
            .collect::<mlua::Result<Vec<_>>>()?,
        None => Vec::new(),
    };
    let data: &'static ClassData = Box::leak(Box::new(ClassData {
        lua: lua.clone(),
        name: name.clone(),
        gtype: Cell::new(glib::Type::INVALID),
        parent,
        vfuncs,
        properties,
        signals,
        init: def
            .get::<Option<Function>>("init")?
            .map(|f| lua.create_registry_value(f))
            .transpose()?,
        class_init: def
            .get::<Option<Function>>("class_init")?
            .map(|f| lua.create_registry_value(f))
            .transpose()?,
        error: RefCell::new(None),
    }));

    // SAFETY: registers a type deriving from `parent` with its sizes.
    let gtype = unsafe {
        let mut query = std::mem::zeroed::<go::GTypeQuery>();
        go::g_type_query(parent.into_glib(), &mut query);
        let info = go::GTypeInfo {
            class_size: query.class_size as u16,
            base_init: None,
            base_finalize: None,
            class_init: Some(class_init),
            class_finalize: None,
            class_data: (data as *const ClassData).cast(),
            instance_size: query.instance_size as u16,
            n_preallocs: 0,
            instance_init: Some(instance_init),
            value_table: std::ptr::null(),
        };
        let cname = CString::new(name.as_str()).map_err(mlua::Error::external)?;
        let flags = if def.get::<Option<bool>>("abstract")?.unwrap_or(false) {
            go::G_TYPE_FLAG_ABSTRACT
        } else {
            0
        };
        glib::Type::from_glib(go::g_type_register_static(
            parent.into_glib(),
            cname.as_ptr(),
            &info,
            flags,
        ))
    };
    if !gtype.is_valid() {
        return Err(mlua::Error::runtime(format!(
            "can't register the type {name}"
        )));
    }
    data.gtype.set(gtype);
    // SAFETY: attaches the class data (leaked: it lives forever).
    unsafe {
        go::g_type_set_qdata(
            gtype.into_glib(),
            class_quark(),
            (data as *const ClassData).cast_mut().cast(),
        )
    };
    for iface in interfaces {
        let iface_data = Box::leak(Box::new((data as *const ClassData, iface)));
        let info = go::GInterfaceInfo {
            interface_init: Some(interface_init),
            interface_finalize: None,
            interface_data: (iface_data as *mut (*const ClassData, glib::Type)).cast(),
        };
        // SAFETY: adds an interface before the class is first used.
        unsafe { go::g_type_add_interface_static(gtype.into_glib(), iface.into_glib(), &info) };
    }
    // Make the class now (it lives forever), so errors show here.
    // SAFETY: a registered class type.
    unsafe { go::g_type_class_ref(gtype.into_glib()) };
    if let Some(e) = data.error.borrow_mut().take() {
        return Err(e);
    }
    let table = namespace::class_table(lua, gtype)?;
    Ok(table)
}

/// Implements a virtual function in the class (or interface) structure at
/// `vtable`. Its place there is the structure's field of the same name
/// (GI's own vfunc offsets are often unknown).
fn implement(data: &ClassData, vtable: *mut u8, vfunc: &Info, f: Function) -> mlua::Result<()> {
    let offset = vfunc_offset(vfunc).ok_or_else(|| {
        mlua::Error::runtime(format!(
            "{}: can't find where {} goes in its class",
            data.name,
            vfunc.name()
        ))
    })?;
    let callable = Rc::new(Callable::new(vfunc.clone()));
    let closure = Closure::new(&data.lua, callable, f, sys::SCOPE_FOREVER)?;
    // SAFETY: the offset is a function pointer field of the structure.
    unsafe { *(vtable.add(offset) as *mut *mut c_void) = closure.native() };
    // The class lives forever: so does its implementation.
    Box::leak(closure);
    Ok(())
}

/// Where a virtual function's pointer is in its class or interface struct.
fn vfunc_offset(vfunc: &Info) -> Option<usize> {
    let declarer = vfunc.container()?;
    // SAFETY: object and interface infos; new references or null.
    let structure = unsafe {
        Info::from_full(match declarer.kind() {
            Kind::Object => sys::gi_object_info_get_class_struct(declarer.ptr()),
            Kind::Interface => sys::gi_interface_info_get_iface_struct(declarer.ptr()),
            _ => return None,
        })
    }?;
    crate::record::field(&structure, vfunc.name()).map(|f| f.offset)
}

unsafe extern "C" fn class_init(klass: glib::ffi::gpointer, data: glib::ffi::gpointer) {
    // SAFETY: the data registered with the type.
    let data = unsafe { &*(data as *const ClassData) };
    let object_class = klass as *mut go::GObjectClass;
    // SAFETY: a class being initialized.
    unsafe {
        (*object_class).set_property = Some(set_property);
        (*object_class).get_property = Some(get_property);
        (*object_class).constructed = Some(constructed);
        (*object_class).finalize = Some(finalize);
    }
    let gtype = data.gtype_from_class(klass);
    for (i, pspec) in data.properties.iter().enumerate() {
        // SAFETY: installs a param spec on a class being initialized.
        unsafe { go::g_object_class_install_property(object_class, i as u32 + 1, *pspec) };
    }
    for (name, params, ret, flags) in &data.signals {
        let Ok(cname) = CString::new(name.as_str()) else {
            continue;
        };
        let mut types: Vec<glib::ffi::GType> = params.iter().map(|t| t.into_glib()).collect();
        // SAFETY: a new signal with GLib's generic marshaller.
        unsafe {
            go::g_signal_newv(
                cname.as_ptr(),
                gtype.into_glib(),
                *flags,
                std::ptr::null_mut(),
                None,
                std::ptr::null_mut(),
                None,
                ret.into_glib(),
                types.len() as u32,
                types.as_mut_ptr(),
            );
        }
    }
    let lua = data.lua.clone();
    let result = (|| -> mlua::Result<()> {
        for (name, key) in &data.vfuncs {
            let f: Function = lua.registry_value(key)?;
            match namespace::find_vfunc(data.parent, name) {
                Some((vfunc, declarer))
                    if crate::repo::find_by_gtype(declarer)
                        .is_some_and(|i| i.kind() == Kind::Object) =>
                {
                    implement(data, klass.cast(), &vfunc, f)?;
                }
                // An interface's: set in interface_init, if the class
                // implements it itself.
                Some((_, iface))
                    if gtype.interfaces().contains(&iface) && !data.parent.is_a(iface) => {}
                Some((_, iface)) => {
                    return Err(mlua::Error::runtime(format!(
                        "{}: {name:?} belongs to {}; list it in `interfaces` to implement it",
                        data.name,
                        iface.name()
                    )));
                }
                None => {
                    // Maybe one of the interfaces being added declares it.
                    if !gtype.interfaces().iter().any(|i| {
                        crate::repo::find_by_gtype(*i).is_some_and(|info| {
                            let n = CString::new(name.as_str()).unwrap_or_default();
                            // SAFETY: an interface info.
                            unsafe {
                                Info::from_full(sys::gi_interface_info_find_vfunc(
                                    info.ptr(),
                                    n.as_ptr(),
                                ))
                            }
                            .is_some()
                        })
                    }) {
                        return Err(mlua::Error::runtime(format!(
                            "{}: {} has no virtual function {name:?}",
                            data.name,
                            data.parent.name()
                        )));
                    }
                }
            }
        }
        if let Some(key) = &data.class_init {
            let f: Function = lua.registry_value(key)?;
            let class_struct = crate::members::ancestors(data.parent)
                .find_map(|t| crate::repo::find_by_gtype(t).filter(|i| i.kind() == Kind::Object))
                // SAFETY: an object info.
                .and_then(|i| unsafe {
                    Info::from_full(sys::gi_object_info_get_class_struct(i.ptr()))
                });
            let class_value = match class_struct {
                Some(info) => mlua::Value::UserData(lua.create_userdata(
                    crate::record::Record::with_owner(
                        klass,
                        info,
                        crate::record::Owner::Borrowed(crate::record::Keep::Nothing),
                    ),
                )?),
                None => mlua::Value::LightUserData(mlua::LightUserData(klass)),
            };
            f.call::<()>(class_value)?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        *data.error.borrow_mut() = Some(e);
    }
}

impl ClassData {
    fn gtype_from_class(&self, klass: glib::ffi::gpointer) -> glib::Type {
        // SAFETY: a class structure starts with its GType.
        unsafe { glib::Type::from_glib((*(klass as *mut go::GTypeClass)).g_type) }
    }
}

unsafe extern "C" fn interface_init(iface: glib::ffi::gpointer, data: glib::ffi::gpointer) {
    // SAFETY: the data registered with the interface.
    let (class, iface_type) = unsafe { *(data as *mut (*const ClassData, glib::Type)) };
    // SAFETY: the class data lives forever.
    let class = unsafe { &*class };
    let Some(info) = crate::repo::find_by_gtype(iface_type) else {
        *class.error.borrow_mut() = Some(mlua::Error::runtime(format!(
            "{}: {} has no introspection data",
            class.name,
            iface_type.name()
        )));
        return;
    };
    let result = (|| -> mlua::Result<()> {
        for (name, key) in &class.vfuncs {
            let n = CString::new(name.as_str()).map_err(mlua::Error::external)?;
            // SAFETY: an interface info.
            if let Some(vfunc) = unsafe {
                Info::from_full(sys::gi_interface_info_find_vfunc(info.ptr(), n.as_ptr()))
            } {
                implement(class, iface.cast(), &vfunc, class.lua.registry_value(key)?)?;
            }
        }
        Ok(())
    })();
    if let Err(e) = result {
        *class.error.borrow_mut() = Some(e);
    }
}

unsafe extern "C" fn instance_init(instance: *mut go::GTypeInstance, _class: glib::ffi::gpointer) {
    let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // SAFETY: tags a GObject being constructed with its id.
    unsafe { go::g_object_set_qdata(instance.cast(), id_quark(), id as *mut c_void) };
}

/// The class of the nearest ancestor that isn't a Lua class: where every
/// Lua class chains up to (they share these functions).
fn native_class(gtype: glib::Type) -> *mut go::GObjectClass {
    let mut t = gtype;
    while let Some(d) = own_class_data(t) {
        t = d.parent;
    }
    // SAFETY: an initialized ancestor class.
    unsafe { go::g_type_class_peek(t.into_glib()) as *mut go::GObjectClass }
}

unsafe extern "C" fn constructed(object: *mut go::GObject) {
    // SAFETY: a constructed instance of a Lua class.
    let gtype = unsafe { crate::object::instance_type(object.cast()) };
    // SAFETY: chains up to the native parent's constructed.
    unsafe {
        if let Some(parent) = (*native_class(gtype)).constructed {
            parent(object);
        }
    }
    // Every Lua class from the first native one down runs its init.
    let mut chain = Vec::new();
    let mut t = Some(gtype);
    while let Some(ty) = t {
        match own_class_data(ty) {
            Some(d) => chain.push(d),
            None => break,
        }
        t = ty.parent();
    }
    for d in chain.into_iter().rev() {
        if let Some(key) = &d.init {
            let lua = &d.lua;
            let result = lua
                .registry_value::<Function>(key)
                .and_then(|f| f.call::<()>(crate::object::wrap(lua, object.cast(), false, false)?));
            if let Err(e) = result {
                crate::report(&format!("{} init", d.name), &e);
            }
        }
    }
}

unsafe extern "C" fn finalize(object: *mut go::GObject) {
    let id = instance_id(object);
    // SAFETY: an instance of a Lua class.
    let gtype = unsafe { crate::object::instance_type(object.cast()) };
    if id != 0 {
        match own_class_data(gtype) {
            Some(d) if crate::closure::on_main_thread() => forget(&d.lua, id),
            _ => DEAD.lock().unwrap_or_else(|e| e.into_inner()).push(id),
        }
    }
    // SAFETY: chains up to the native parent's finalize.
    unsafe {
        if let Some(f) = (*native_class(gtype)).finalize {
            f(object);
        }
    }
}

unsafe extern "C" fn set_property(
    object: *mut go::GObject,
    _id: u32,
    value: *mut go::GValue,
    pspec: *mut go::GParamSpec,
) {
    let instance = instance_id(object);
    // SAFETY: copies the value being set.
    let v: glib::Value = unsafe { glib::translate::from_glib_none(value) };
    let _ = VALUES.try_with(|m| m.borrow_mut().insert((instance, pspec as usize), v));
}

unsafe extern "C" fn get_property(
    object: *mut go::GObject,
    _id: u32,
    value: *mut go::GValue,
    pspec: *mut go::GParamSpec,
) {
    let instance = instance_id(object);
    let stored = VALUES
        .try_with(|m| m.borrow().get(&(instance, pspec as usize)).cloned())
        .ok()
        .flatten();
    // SAFETY: fills the GValue GObject passes, of the property's type.
    unsafe {
        match stored {
            Some(v) => go::g_value_copy(glib::translate::ToGlibPtr::to_glib_none(&v).0, value),
            None => go::g_param_value_set_default(pspec, value),
        }
    }
}

/// A Lua field of an instance (`self.items`), for Lua classes.
pub fn get_field(lua: &Lua, ud: &mlua::AnyUserData, key: &str) -> mlua::Result<mlua::Value> {
    let Ok(object) = ud.borrow::<crate::object::Object>() else {
        return Ok(mlua::Value::Nil);
    };
    let id = instance_id(glib::translate::ToGlibPtr::to_glib_none(&object.0).0);
    if id == 0 {
        return Ok(mlua::Value::Nil);
    }
    forget_dead(lua);
    let table = crate::state(lua)
        .fields
        .borrow()
        .get(&id)
        .map(|k| lua.registry_value::<Table>(k));
    match table {
        Some(t) => t?.raw_get(key),
        None => Ok(mlua::Value::Nil),
    }
}

pub fn set_field(
    lua: &Lua,
    _ud: &mlua::AnyUserData,
    object: &glib::Object,
    key: &str,
    value: mlua::Value,
) -> mlua::Result<()> {
    let id = instance_id(glib::translate::ToGlibPtr::to_glib_none(object).0);
    if id == 0 {
        return Err(mlua::Error::runtime(format!(
            "{} has no property {key:?}",
            glib::prelude::ObjectExt::type_(object).name()
        )));
    }
    forget_dead(lua);
    let existing = crate::state(lua)
        .fields
        .borrow()
        .get(&id)
        .map(|k| lua.registry_value::<Table>(k));
    let table = match existing {
        Some(t) => t?,
        None => {
            let t = lua.create_table()?;
            let key = lua.create_registry_value(t.clone())?;
            crate::state(lua).fields.borrow_mut().insert(id, key);
            t
        }
    };
    table.raw_set(key, value)
}
