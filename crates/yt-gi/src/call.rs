//! Calling functions, methods, constructors and virtual functions.
//!
//! A `Callable` reads a function's signature once: which arguments Lua
//! passes (lengths, user data and destroy notifiers are filled in), which
//! come back (the return value, then out arguments, in order), and how
//! each converts.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::rc::Rc;

use mlua::{Function, Lua, MultiValue};

use crate::closure::{self, Closure};
use crate::convert::{self, Scratch, Temp};
use crate::info::{Info, Kind};
use crate::sys::{self, GIArgument};
use crate::types::{Ty, Type};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    /// An argument Lua passes or gets back.
    Normal,
    /// The length of another argument (an array).
    Length,
    /// A callback's user data.
    UserData,
    /// A callback's destroy notifier.
    Destroy,
    /// Hidden by the function's annotations.
    Skip,
}

pub struct Arg {
    pub name: &'static str,
    pub dir: sys::GIDirection,
    pub transfer: sys::GITransfer,
    pub ty: Type,
    pub nullable: bool,
    pub caller_allocates: bool,
    pub scope: sys::GIScopeType,
    /// For callbacks: the user data and destroy notifier arguments.
    pub closure: Option<usize>,
    pub destroy: Option<usize>,
    role: Role,
    /// Positions among GI's in and out argument arrays.
    in_slot: Option<usize>,
    out_slot: Option<usize>,
}

pub struct Callable {
    pub info: Info,
    /// `Gtk.Widget.measure`, for messages.
    pub name: String,
    pub is_method: bool,
    pub throws: bool,
    pub args: Vec<Arg>,
    pub ret: Type,
    pub ret_transfer: sys::GITransfer,
    skip_return: bool,
    n_in: usize,
    n_out: usize,
    /// The arguments Lua passes, in order.
    inputs: Vec<usize>,
}

impl Arg {
    /// Whether Lua passes or gets this argument.
    pub fn is_visible(&self) -> bool {
        self.role == Role::Normal
    }
}

impl Callable {
    pub fn new(info: Info) -> Callable {
        let p = info.ptr();
        let name = match info.container() {
            Some(c) => format!("{}.{}", c.full_name(), info.name()),
            None => info.full_name(),
        };
        // SAFETY: a callable info; each getter returns a new reference.
        unsafe {
            let is_method = sys::gi_callable_info_is_method(p) != 0;
            let throws = sys::gi_callable_info_can_throw_gerror(p) != 0;
            let ret = Type::new(
                Info::from_full(sys::gi_callable_info_get_return_type(p)).expect("return type"),
            );
            let mut args: Vec<Arg> = crate::info::children(
                &info,
                sys::gi_callable_info_get_n_args,
                sys::gi_callable_info_get_arg,
            )
            .map(|a| {
                let ap = a.ptr();
                let mut index = 0;
                Arg {
                    name: a.name(),
                    dir: sys::gi_arg_info_get_direction(ap),
                    transfer: sys::gi_arg_info_get_ownership_transfer(ap),
                    ty: Type::new(
                        Info::from_full(sys::gi_arg_info_get_type_info(ap)).expect("argument type"),
                    ),
                    nullable: sys::gi_arg_info_may_be_null(ap) != 0
                        || sys::gi_arg_info_is_optional(ap) != 0,
                    caller_allocates: sys::gi_arg_info_is_caller_allocates(ap) != 0,
                    scope: sys::gi_arg_info_get_scope(ap),
                    closure: (sys::gi_arg_info_get_closure_index(ap, &mut index) != 0)
                        .then_some(index as usize),
                    destroy: (sys::gi_arg_info_get_destroy_index(ap, &mut index) != 0)
                        .then_some(index as usize),
                    role: if sys::gi_arg_info_is_skip(ap) != 0 {
                        Role::Skip
                    } else {
                        Role::Normal
                    },
                    in_slot: None,
                    out_slot: None,
                }
            })
            .collect();
            // Arguments Lua never sees: array lengths, callbacks' user data
            // and destroy notifiers (and the user data arguments that
            // point back at their callback).
            let mut roles = Vec::new();
            for (i, arg) in args.iter().enumerate() {
                if let Ty::Array(a) = &arg.ty.kind
                    && let Some(l) = a.length_arg
                {
                    roles.push((l, Role::Length));
                }
                match &arg.ty.kind {
                    Ty::Callback(_) => {
                        roles.extend(arg.closure.map(|c| (c, Role::UserData)));
                        roles.extend(arg.destroy.map(|d| (d, Role::Destroy)));
                    }
                    Ty::Void
                        if arg.ty.pointer
                            && arg
                                .closure
                                .and_then(|c| args.get(c))
                                .is_some_and(|c| matches!(c.ty.kind, Ty::Callback(_))) =>
                    {
                        roles.push((i, Role::UserData));
                    }
                    _ => {}
                }
            }
            if let Ty::Array(a) = &ret.kind
                && let Some(l) = a.length_arg
            {
                roles.push((l, Role::Length));
            }
            for (i, role) in roles {
                if let Some(arg) = args.get_mut(i) {
                    arg.role = role;
                }
            }
            let mut n_in = usize::from(is_method);
            let mut n_out = 0;
            let mut inputs = Vec::new();
            for (i, arg) in args.iter_mut().enumerate() {
                if arg.dir != sys::DIRECTION_OUT {
                    arg.in_slot = Some(n_in);
                    n_in += 1;
                }
                if arg.dir != sys::DIRECTION_IN {
                    arg.out_slot = Some(n_out);
                    n_out += 1;
                }
                if arg.dir != sys::DIRECTION_OUT && arg.role == Role::Normal {
                    inputs.push(i);
                }
            }
            Callable {
                name,
                is_method,
                throws,
                ret_transfer: sys::gi_callable_info_get_caller_owns(p),
                skip_return: sys::gi_callable_info_skip_return(p) != 0,
                ret,
                args,
                n_in,
                n_out,
                inputs,
                info,
            }
        }
    }

    fn error(&self, what: impl std::fmt::Display) -> mlua::Error {
        mlua::Error::runtime(format!("{}: {what}", self.name))
    }

    /// Calls `function` (the C function, or a virtual function's
    /// implementation) with Lua arguments; `self` first for methods.
    pub fn invoke(
        &self,
        lua: &Lua,
        function: *mut c_void,
        args: MultiValue,
    ) -> mlua::Result<MultiValue> {
        let mut lua_args = args.into_iter();
        let mut scratch = Scratch::default();
        let mut ins = vec![GIArgument::default(); self.n_in];
        let mut outs = vec![GIArgument::default(); self.n_out];
        // What out arguments point to.
        let mut storage = vec![GIArgument::default(); self.n_out];

        if self.is_method {
            let this = lua_args.next().unwrap_or(mlua::Value::Nil);
            ins[0].v_pointer = self.instance(&this)?;
        }
        let given: Vec<mlua::Value> = lua_args.collect();
        if given.len() > self.inputs.len() {
            return Err(self.error(format!(
                "takes {} argument{}, got {}",
                self.inputs.len(),
                if self.inputs.len() == 1 { "" } else { "s" },
                given.len()
            )));
        }
        let mut lengths: HashMap<usize, usize> = HashMap::new();
        for (n, &i) in self.inputs.iter().enumerate() {
            let arg = &self.args[i];
            let value = given.get(n).cloned().unwrap_or(mlua::Value::Nil);
            let converted = match &arg.ty.kind {
                Ty::Callback(info) => {
                    self.callback(lua, &value, arg, info, &mut ins, &mut scratch)?
                }
                Ty::Array(array) => {
                    let (a, len) = convert::array_from_lua(
                        lua,
                        &value,
                        array,
                        arg.transfer,
                        arg.nullable,
                        &mut scratch,
                    )
                    .map_err(|e| self.error(format!("argument {} ({}): {e}", n + 1, arg.name)))?;
                    if let Some(l) = array.length_arg {
                        lengths.insert(l, len);
                    }
                    a
                }
                _ => convert::from_lua(
                    lua,
                    &value,
                    &arg.ty,
                    arg.transfer,
                    arg.nullable,
                    &mut scratch,
                )
                .map_err(|e| self.error(format!("argument {} ({}): {e}", n + 1, arg.name)))?,
            };
            match arg.dir {
                sys::DIRECTION_INOUT => {
                    let slot = arg.out_slot.expect("inout slot");
                    storage[slot] = converted;
                }
                _ => ins[arg.in_slot.expect("in slot")] = converted,
            }
        }
        // Lengths of the arrays passed in.
        for (&l, &len) in &lengths {
            let arg = &self.args[l];
            let mut v = GIArgument::default();
            set_len(&mut v, &arg.ty, len);
            match arg.dir {
                sys::DIRECTION_INOUT => storage[arg.out_slot.expect("slot")] = v,
                sys::DIRECTION_IN => ins[arg.in_slot.expect("slot")] = v,
                _ => {}
            }
        }
        // Out arguments point to storage, or to memory we allocate for
        // the callee to fill.
        let mut allocated: Vec<Option<*mut c_void>> = vec![None; self.n_out];
        for arg in &self.args {
            let Some(slot) = arg.out_slot else { continue };
            if arg.caller_allocates && arg.dir == sys::DIRECTION_OUT {
                let size = match &arg.ty.kind {
                    Ty::Array(a) => {
                        let n = a
                            .fixed_size
                            .or_else(|| a.length_arg.and_then(|l| self.in_length(l, &ins)))
                            .unwrap_or(0);
                        n * a.item.size().max(1)
                    }
                    _ => arg.ty.size(),
                };
                // SAFETY: zeroed memory for the callee to fill.
                let p = unsafe { glib::ffi::g_malloc0(size.max(1)) };
                allocated[slot] = Some(p);
                outs[slot].v_pointer = p;
            } else {
                outs[slot].v_pointer = (&mut storage[slot] as *mut GIArgument).cast();
            }
            if arg.dir == sys::DIRECTION_INOUT {
                // SAFETY: just set as a pointer.
                ins[arg.in_slot.expect("slot")].v_pointer = unsafe { outs[slot].v_pointer };
            }
        }

        let mut ret = GIArgument::default();
        let mut error = std::ptr::null_mut();
        // SAFETY: arguments as GI describes the function.
        let ok = unsafe {
            sys::gi_callable_info_invoke(
                self.info.ptr(),
                function,
                ins.as_ptr(),
                ins.len(),
                outs.as_mut_ptr(),
                outs.len(),
                &mut ret,
                &mut error,
            )
        };
        if ok == 0 {
            for p in allocated.into_iter().flatten() {
                // SAFETY: memory we allocated above.
                unsafe { glib::ffi::g_free(p) };
            }
            if error.is_null() {
                return Err(self.error("the call failed"));
            }
            return Err(Thrown::raise(lua, error));
        }
        drop(scratch);

        let mut results = Vec::new();
        if !self.ret.is_void() && !self.skip_return {
            let length = match &self.ret.kind {
                Ty::Array(a) => a
                    .length_arg
                    .and_then(|l| self.out_length(l, &storage, &ins)),
                _ => None,
            };
            results.push(convert::to_lua(
                lua,
                &ret,
                &self.ret,
                self.ret_transfer,
                length,
            )?);
        }
        for arg in &self.args {
            let Some(slot) = arg.out_slot else { continue };
            if arg.role != Role::Normal {
                continue;
            }
            let value = if let Some(p) = allocated[slot] {
                match &arg.ty.kind {
                    Ty::Struct(info) | Ty::Union(info) => convert::caller_allocated(lua, p, info)?,
                    Ty::Array(a) => {
                        let length = a
                            .length_arg
                            .and_then(|l| self.out_length(l, &storage, &ins));
                        let mut v = GIArgument::default();
                        v.v_pointer = p;
                        // The memory is ours: free it with the array.
                        convert::to_lua(lua, &v, &arg.ty, sys::TRANSFER_CONTAINER, length)?
                    }
                    _ => {
                        // SAFETY: memory we allocated.
                        unsafe { glib::ffi::g_free(p) };
                        mlua::Value::Nil
                    }
                }
            } else {
                let length = match &arg.ty.kind {
                    Ty::Array(a) => a
                        .length_arg
                        .and_then(|l| self.out_length(l, &storage, &ins)),
                    _ => None,
                };
                convert::to_lua(lua, &storage[slot], &arg.ty, arg.transfer, length)?
            };
            results.push(value);
        }
        Ok(MultiValue::from_iter(results))
    }

    fn instance(&self, this: &mlua::Value) -> mlua::Result<*mut c_void> {
        if let Some((p, _)) = crate::object::instance_ptr(this) {
            return Ok(p);
        }
        if let mlua::Value::UserData(ud) = this
            && let Ok(r) = ud.borrow::<crate::record::Record>()
        {
            return Ok(r.ptr);
        }
        if let Some(p) = crate::cairo::ptr(this) {
            return Ok(p);
        }
        let class = self
            .info
            .container()
            .map(|c| c.full_name())
            .unwrap_or_default();
        Err(self.error(format!(
            "is a method: call it as obj:{}(...) on a {class} (got {})",
            self.info.name(),
            this.type_name()
        )))
    }

    fn in_length(&self, l: usize, ins: &[GIArgument]) -> Option<usize> {
        let arg = self.args.get(l)?;
        let slot = arg.in_slot?;
        Some(convert::get_int(&ins[slot], convert::int_kind(&arg.ty)).max(0) as usize)
    }

    fn out_length(&self, l: usize, storage: &[GIArgument], ins: &[GIArgument]) -> Option<usize> {
        let arg = self.args.get(l)?;
        match arg.out_slot {
            Some(slot) => {
                Some(convert::get_int(&storage[slot], convert::int_kind(&arg.ty)).max(0) as usize)
            }
            None => self.in_length(l, ins),
        }
    }

    /// A Lua function as a callback argument, with its user data and
    /// destroy notifier filled in.
    fn callback(
        &self,
        lua: &Lua,
        value: &mlua::Value,
        arg: &Arg,
        info: &Info,
        ins: &mut [GIArgument],
        scratch: &mut Scratch,
    ) -> mlua::Result<GIArgument> {
        let mut out = GIArgument::default();
        let f = match value {
            mlua::Value::Nil => return Ok(out),
            mlua::Value::Function(f) => f.clone(),
            mlua::Value::LightUserData(p) => {
                // A C function pointer, passed through.
                out.v_pointer = p.0;
                return Ok(out);
            }
            other => {
                return Err(self.error(format!(
                    "argument {}: expected a function, got {}",
                    arg.name,
                    other.type_name()
                )));
            }
        };
        let scope = match arg.scope {
            sys::SCOPE_INVALID => sys::SCOPE_FOREVER,
            s => s,
        };
        let closure = Closure::new(lua, callable_for(info), f, scope)?;
        out.v_pointer = closure.native();
        let data = closure.as_user_data();
        if let Some(c) = arg.closure
            && let Some(slot) = self.args.get(c).and_then(|a| a.in_slot)
        {
            ins[slot].v_pointer = data;
        }
        if let Some(d) = arg.destroy
            && let Some(slot) = self.args.get(d).and_then(|a| a.in_slot)
        {
            ins[slot].v_pointer = closure::destroy_notify as *mut c_void;
        }
        match scope {
            sys::SCOPE_CALL => scratch.push(Temp::Closure(closure)),
            // Freed after its one call, by its destroy notifier, or never.
            _ => {
                let _ = Box::into_raw(closure);
            }
        }
        Ok(out)
    }
}

fn set_len(arg: &mut GIArgument, ty: &Type, len: usize) {
    use crate::types::Int;
    match convert::int_kind(ty) {
        Int::I8 | Int::U8 => arg.v_uint8 = len as u8,
        Int::I16 | Int::U16 => arg.v_uint16 = len as u16,
        Int::I32 | Int::U32 => arg.v_uint32 = len as u32,
        Int::I64 | Int::U64 => arg.v_uint64 = len as u64,
    }
}

/// A GError raised as a Lua error value: the `GLib.Error` record itself,
/// so `pcall` gets `err.message`, `err.code` and `err.domain`. Rust
/// functions can only raise Rust errors, so the record travels inside one
/// and a small Lua wrapper rethrows it (see `function`).
pub struct Thrown;

thread_local! {
    static PENDING: RefCell<Option<mlua::Value>> = const { RefCell::new(None) };
}

impl Thrown {
    fn raise(lua: &Lua, error: *mut glib::ffi::GError) -> mlua::Error {
        // SAFETY: the error is ours.
        let message = unsafe { std::ffi::CStr::from_ptr((*error).message) }
            .to_string_lossy()
            .into_owned();
        match convert::error_to_lua(lua, error, true) {
            Ok(value) => {
                PENDING.with(|p| *p.borrow_mut() = Some(value));
                mlua::Error::runtime(GERROR_MARK.to_owned() + &message)
            }
            Err(e) => e,
        }
    }

    /// The error record of the last failed call (for the Lua wrapper).
    pub fn take() -> Option<mlua::Value> {
        PENDING.with(|p| p.borrow_mut().take())
    }
}

/// Starts the message of a Rust error carrying a GError.
pub const GERROR_MARK: &str = "\u{1}gerror\u{1}";

thread_local! {
    static CALLABLES: RefCell<HashMap<String, Rc<Callable>>> = RefCell::new(HashMap::new());
}

/// The plan of a callback type, cached.
pub fn callable_for(info: &Info) -> Rc<Callable> {
    let key = format!("callback:{}", info.full_name());
    if let Some(c) = CALLABLES.with(|c| c.borrow().get(&key).cloned()) {
        return c;
    }
    let c = Rc::new(Callable::new(info.clone()));
    CALLABLES.with(|m| m.borrow_mut().insert(key, c.clone()));
    c
}

/// A Lua function calling a GI function (or method, or constructor),
/// made once per function.
pub fn function(lua: &Lua, info: &Info) -> mlua::Result<Function> {
    debug_assert_eq!(info.kind(), Kind::Function);
    // SAFETY: a function info.
    let symbol = crate::info::static_str(unsafe { sys::gi_function_info_get_symbol(info.ptr()) });
    if let Some(f) = crate::state(lua)
        .functions
        .borrow()
        .get(symbol)
        .map(|k| lua.registry_value::<Function>(k))
    {
        return f;
    }
    let address = info.symbol(symbol).ok_or_else(|| {
        mlua::Error::runtime(format!(
            "{} isn't in its library ({symbol})",
            info.full_name()
        ))
    })?;
    let callable = Rc::new(Callable::new(info.clone()));
    let f = wrap(lua, callable, move |_| Ok(address))?;
    let key = lua.create_registry_value(f.clone())?;
    crate::state(lua)
        .functions
        .borrow_mut()
        .insert(symbol.to_owned(), key);
    Ok(f)
}

/// A Lua function calling `callable` at the address `address(args)` picks.
pub fn wrap(
    lua: &Lua,
    callable: Rc<Callable>,
    address: impl Fn(&MultiValue) -> mlua::Result<*mut c_void> + 'static,
) -> mlua::Result<Function> {
    let throws = callable.throws;
    let raw = lua.create_function(move |lua, args: MultiValue| {
        let at = address(&args)?;
        callable.invoke(lua, at, args)
    })?;
    if !throws {
        return Ok(raw);
    }
    rethrowing(lua, raw)
}

/// Wraps a function so GErrors it raises reach Lua as error records.
fn rethrowing(lua: &Lua, raw: Function) -> mlua::Result<Function> {
    let existing = crate::state(lua)
        .rethrow
        .borrow()
        .as_ref()
        .map(|k| lua.registry_value::<Function>(k));
    let maker: Function = match existing {
        Some(f) => f?,
        None => {
            let take = lua.create_function(|_, ()| Ok(Thrown::take()))?;
            let f: Function = lua
                .load(
                    r#"
local take, mark = ...
local pcall, error, select, find = pcall, error, select, string.find
local function settle(ok, ...)
  if ok then return ... end
  local err = ...
  if type(err) ~= "string" and type(err) ~= "userdata" then error(err, 0) end
  local thrown = take()
  if thrown ~= nil and find(tostring(err), mark, 1, true) then error(thrown, 0) end
  error(err, 0)
end
return function(raw)
  return function(...) return settle(pcall(raw, ...)) end
end
"#,
                )
                .set_name("@gi-errors")
                .call((take, GERROR_MARK))?;
            let key = lua.create_registry_value(f.clone())?;
            *crate::state(lua).rethrow.borrow_mut() = Some(key);
            f
        }
    };
    maker.call(raw)
}
