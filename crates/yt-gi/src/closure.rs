//! Lua functions as C function pointers: callbacks (`GLib.idle_add`,
//! `Gio.File:load_contents_async`, `Gtk.DrawingArea:set_draw_func`) and the
//! virtual functions of Lua subclasses.
//!
//! How long one lives follows GI's scope annotations: until the call
//! returns (`call`), until its one invocation (`async`), until its destroy
//! notifier runs (`notified`), or forever. Lua runs on the main thread
//! only: a callback invoked from another thread is refused, and reported.

use std::ffi::c_void;
use std::rc::Rc;
use std::sync::atomic::{AtomicPtr, Ordering};

use mlua::{Function, Lua, MultiValue};

use crate::call::Callable;
use crate::convert::{self, Scratch};
use crate::sys::{self, GIArgument};
use crate::types::{Int, Ty, Type};

/// The thread Lua runs on, as GLib knows it: `std::thread::current()`
/// isn't available while the process exits, when objects may still be
/// finalized.
static MAIN_THREAD: AtomicPtr<glib::ffi::GThread> = AtomicPtr::new(std::ptr::null_mut());

pub fn set_main_thread() {
    // SAFETY: returns the calling thread's GThread (kept by GLib).
    let _ = MAIN_THREAD.compare_exchange(
        std::ptr::null_mut(),
        unsafe { glib::ffi::g_thread_self() },
        Ordering::AcqRel,
        Ordering::Acquire,
    );
}

pub(crate) fn on_main_thread() -> bool {
    let main = MAIN_THREAD.load(Ordering::Acquire);
    // SAFETY: as above.
    main.is_null() || main == unsafe { glib::ffi::g_thread_self() }
}

pub struct Closure {
    callable: Rc<Callable>,
    function: Function,
    lua: Lua,
    scope: sys::GIScopeType,
    cif: Box<sys::ffi_cif>,
    closure: *mut sys::ffi_closure,
    native: *mut c_void,
}

impl Closure {
    pub fn new(
        lua: &Lua,
        callable: Rc<Callable>,
        function: Function,
        scope: sys::GIScopeType,
    ) -> mlua::Result<Box<Closure>> {
        let mut boxed = Box::new(Closure {
            callable,
            function,
            lua: lua.clone(),
            scope,
            // SAFETY: plain bytes; GI fills it in.
            cif: Box::new(unsafe { std::mem::zeroed() }),
            closure: std::ptr::null_mut(),
            native: std::ptr::null_mut(),
        });
        let data = (&*boxed as *const Closure).cast_mut().cast();
        // SAFETY: the cif and the closure data live as long as the closure.
        unsafe {
            boxed.closure = sys::gi_callable_info_create_closure(
                boxed.callable.info.ptr(),
                &mut *boxed.cif,
                trampoline,
                data,
            );
            if boxed.closure.is_null() {
                return Err(mlua::Error::runtime(format!(
                    "can't make a {} callback",
                    boxed.callable.name
                )));
            }
            boxed.native = sys::gi_callable_info_get_closure_native_address(
                boxed.callable.info.ptr(),
                boxed.closure,
            );
        }
        Ok(boxed)
    }

    /// The C function pointer.
    pub fn native(&self) -> *mut c_void {
        self.native
    }

    /// What C passes back as user data (and to the destroy notifier).
    pub fn as_user_data(&self) -> *mut c_void {
        (self as *const Closure).cast_mut().cast()
    }

    /// Runs the Lua function with C's arguments, writing its results back.
    ///
    /// # Safety
    /// `ret` and `args` are libffi's, for this closure's signature.
    unsafe fn run(&self, ret: *mut c_void, args: *mut *mut c_void) -> mlua::Result<()> {
        let lua = &self.lua;
        let c = &self.callable;
        let offset = usize::from(c.is_method);
        // SAFETY: libffi passes one pointer per C argument.
        let arg_at = |i: usize| unsafe { *args.add(i + offset) } as *const u8;
        let mut lua_args = Vec::with_capacity(c.args.len() + offset);
        if c.is_method {
            // SAFETY: the instance is the first argument.
            let this = unsafe { *(*args as *const *mut c_void) };
            lua_args.push(crate::object::wrap(lua, this, false, false)?);
        }
        for (i, arg) in c.args.iter().enumerate() {
            if !arg.is_visible() || arg.dir == sys::DIRECTION_OUT {
                continue;
            }
            // SAFETY: in arguments are stored as their type; in-out ones
            // behind a pointer.
            let value = unsafe {
                let at = if arg.dir == sys::DIRECTION_INOUT {
                    *(arg_at(i) as *const *const u8)
                } else {
                    arg_at(i)
                };
                let v = convert::load(at, &arg.ty);
                let length = match &arg.ty.kind {
                    Ty::Array(a) => a.length_arg.map(|l| {
                        let len_arg = &c.args[l];
                        convert::get_int(
                            &convert::load(arg_at(l), &len_arg.ty),
                            convert::int_kind(&len_arg.ty),
                        )
                        .max(0) as usize
                    }),
                    _ => None,
                };
                match &arg.ty.kind {
                    // Objects passed to callbacks may be under construction:
                    // never sink them.
                    Ty::Object(_) => crate::object::wrap(
                        lua,
                        v.v_pointer,
                        arg.transfer == sys::TRANSFER_EVERYTHING,
                        false,
                    )?,
                    _ => convert::to_lua(lua, &v, &arg.ty, arg.transfer, length)?,
                }
            };
            lua_args.push(value);
        }
        let results = self
            .function
            .call::<MultiValue>(MultiValue::from_iter(lua_args))?;
        let mut results = results.into_iter();
        // What's handed back without a transfer must outlive this call:
        // the scratch is kept.
        let mut scratch = Scratch::default();
        if !c.ret.is_void() {
            let value = results.next().unwrap_or(mlua::Value::Nil);
            let a = convert::from_lua(lua, &value, &c.ret, c.ret_transfer, true, &mut scratch)
                .map_err(|e| {
                    mlua::Error::runtime(format!("{} returned a wrong value: {e}", c.name))
                })?;
            // SAFETY: libffi's return storage.
            unsafe { write_return(ret, &c.ret, &a) };
        }
        for (i, arg) in c.args.iter().enumerate() {
            if !arg.is_visible() || arg.dir == sys::DIRECTION_IN {
                continue;
            }
            let value = results.next().unwrap_or(mlua::Value::Nil);
            // SAFETY: out arguments are pointers to where to write (or
            // null when the caller doesn't want the value).
            let target = unsafe { *(arg_at(i) as *const *mut u8) };
            // Nothing returned for it: the caller's value stays (GTK sets
            // a measurement's baselines to -1 beforehand, for instance).
            if target.is_null() || value.is_nil() {
                continue;
            }
            let a = convert::from_lua(lua, &value, &arg.ty, arg.transfer, true, &mut scratch)
                .map_err(|e| {
                    mlua::Error::runtime(format!("{}: out argument {}: {e}", c.name, arg.name))
                })?;
            // SAFETY: as above.
            unsafe { convert::store(target, &arg.ty, &a) };
        }
        std::mem::forget(scratch);
        Ok(())
    }
}

impl Drop for Closure {
    fn drop(&mut self) {
        // SAFETY: frees the closure made in `new`.
        unsafe { sys::gi_callable_info_destroy_closure(self.callable.info.ptr(), self.closure) };
    }
}

/// Writes a return value as libffi expects: integers narrower than a
/// register widened to one.
unsafe fn write_return(ret: *mut c_void, ty: &Type, a: &GIArgument) {
    // SAFETY: libffi's return storage holds at least a register.
    unsafe {
        if ty.pointer {
            *(ret as *mut *mut c_void) = a.v_pointer;
            return;
        }
        match &ty.kind {
            Ty::Bool => *(ret as *mut isize) = a.v_int32 as isize,
            Ty::Int(i) | Ty::Enum { storage: i, .. } => match i {
                Int::I64 | Int::U64 => *(ret as *mut i64) = a.v_int64,
                Int::U8 | Int::U16 | Int::U32 => {
                    *(ret as *mut usize) = convert::get_int(a, *i) as usize
                }
                _ => *(ret as *mut isize) = convert::get_int(a, *i) as isize,
            },
            Ty::Unichar => *(ret as *mut usize) = a.v_uint32 as usize,
            Ty::Float => *(ret as *mut f32) = a.v_float,
            Ty::Double => *(ret as *mut f64) = a.v_double,
            Ty::GType => *(ret as *mut usize) = a.v_size,
            Ty::Void => {}
            _ => *(ret as *mut *mut c_void) = a.v_pointer,
        }
    }
}

/// Zeroes a return value, for callbacks that failed.
unsafe fn zero_return(ret: *mut c_void, ty: &Type) {
    if !ty.is_void() {
        // SAFETY: libffi's return storage holds at least 8 bytes.
        unsafe { std::ptr::write_bytes(ret as *mut u8, 0, 8) };
    }
}

unsafe extern "C" fn trampoline(
    _cif: *mut sys::ffi_cif,
    ret: *mut c_void,
    args: *mut *mut c_void,
    data: *mut c_void,
) {
    // SAFETY: `data` is the closure that registered this trampoline.
    let closure = unsafe { &*(data as *const Closure) };
    if !on_main_thread() {
        eprintln!(
            "yt-gi: {} was called from another thread; Lua runs on the main thread only",
            closure.callable.name
        );
        // SAFETY: libffi's return storage.
        unsafe { zero_return(ret, &closure.callable.ret) };
        return;
    }
    // SAFETY: libffi's arguments for this signature.
    if let Err(e) = unsafe { closure.run(ret, args) } {
        crate::report(&format!("callback {}", closure.callable.name), &e);
        // SAFETY: as above.
        unsafe { zero_return(ret, &closure.callable.ret) };
        if closure.callable.throws {
            // SAFETY: the last argument of a throwing callable is GError**.
            unsafe {
                let n = closure.callable.args.len() + usize::from(closure.callable.is_method);
                let error = *(*args.add(n) as *mut *mut *mut glib::ffi::GError);
                if !error.is_null() {
                    let message = std::ffi::CString::new(e.to_string().replace('\0', " "))
                        .unwrap_or_default();
                    *error = glib::ffi::g_error_new_literal(
                        glib::ffi::g_quark_from_static_string(c"yt-gi-lua-error".as_ptr()),
                        0,
                        message.as_ptr(),
                    );
                }
            }
        }
    }
    if closure.scope == sys::SCOPE_ASYNC {
        free_later(data);
    }
}

/// Frees a closure on the main thread, after the callback running now has
/// returned (libffi returns through the closure's own memory).
fn free_later(data: *mut c_void) {
    struct Pointer(*mut c_void);
    // SAFETY: only moved to the main thread, where the closure is freed.
    unsafe impl Send for Pointer {}
    let p = Pointer(data);
    glib::idle_add_once(move || {
        let p = p;
        // SAFETY: the closure was leaked by its caller for this.
        drop(unsafe { Box::from_raw(p.0 as *mut Closure) });
    });
}

/// The destroy notifier handed out with notified callbacks.
pub unsafe extern "C" fn destroy_notify(data: *mut c_void) {
    if !data.is_null() {
        free_later(data);
    }
}
