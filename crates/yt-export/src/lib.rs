//! The app's services as one API that anything can call and hook.
//!
//! `#[export("service")]` (from `yt-export-macros`) on an impl block or a
//! function exports every public function in it: each is listed in
//! [`functions()`] with its parameters and documentation, callable with
//! JSON values ([`Function::call`]), and hookable. A hook runs *around*
//! the function — it gets the arguments (by name), may change them, call
//! the original any number of times (or not at all), and change or
//! replace the result — whoever calls it: the app's own code or a plugin.
//!
//! The host decides what runs hooks (an [`Interceptor`]: the Lua plugins)
//! and turns hook points on with [`set_hooked`]. A function nobody hooks
//! costs one atomic load more than before.
//!
//! What can't cross as data (widgets, closures, references into the
//! service) is opted out of with `#[export(skip = "why")]`, which keeps
//! every exception visible and explained.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::ops::Deref;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

pub use linkme;
use serde::Serialize;
use serde::de::DeserializeOwned;
pub use serde_json::Value;
pub use yt_export_macros::export;

/// An exported function.
pub struct Function {
    /// `service.function`, e.g. `youtube.browse`.
    pub name: &'static str,
    pub doc: &'static str,
    pub params: &'static [Param],
    /// The Rust return type, for documentation.
    pub returns: &'static str,
    pub call: Call,
    pub hook: &'static HookPoint,
}

pub struct Param {
    pub name: &'static str,
    /// The Rust type, for documentation.
    pub ty: &'static str,
}

pub type SendFuture = Pin<Box<dyn Future<Output = Result<Value, String>> + Send>>;
pub type LocalFuture = Pin<Box<dyn Future<Output = Result<Value, String>>>>;

/// How to call a function with arguments in order.
pub enum Call {
    Sync(fn(Vec<Value>) -> Result<Value, String>),
    /// An async function whose future may run on any thread.
    Async(fn(Vec<Value>) -> Result<SendFuture, String>),
    /// An async function of a main-thread service.
    Local(fn(Vec<Value>) -> Result<LocalFuture, String>),
}

#[linkme::distributed_slice]
pub static FUNCTIONS: [Function];

/// Every exported function, in no particular order.
pub fn functions() -> &'static [Function] {
    &FUNCTIONS
}

pub fn find(name: &str) -> Option<&'static Function> {
    FUNCTIONS.iter().find(|f| f.name == name)
}

/// Where a function checks for hooks.
pub struct HookPoint {
    pub name: &'static str,
    active: AtomicBool,
}

impl HookPoint {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            active: AtomicBool::new(false),
        }
    }

    #[inline]
    pub fn active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }
}

/// Turns a function's hook point on or off. False if there's no such
/// function.
pub fn set_hooked(name: &str, on: bool) -> bool {
    match find(name) {
        Some(f) => {
            f.hook.active.store(on, Ordering::Relaxed);
            true
        }
        None => false,
    }
}

/// A step the hooks ask of the caller.
pub enum Step {
    /// Call the original with these arguments; send back its result.
    Next(Value),
    /// The hooks are done: this is the call's result.
    Done(Result<Value, String>),
}

/// The caller's side of hooks running elsewhere (on the thread hooks run
/// on, for a call made on another thread or by async code).
pub struct Remote {
    /// To the caller.
    pub send: Box<dyn Fn(Step) + Send>,
    /// The results of `Step::Next`, from the caller.
    pub results: tokio::sync::mpsc::UnboundedReceiver<Result<Value, String>>,
}

/// What runs hooks: the host's plugins.
pub trait Interceptor: Send + Sync {
    /// Whether the calling thread is the one hooks run on.
    fn is_home(&self) -> bool;
    /// Runs the hooks of `name` right here (on the home thread); `next`
    /// calls the original.
    fn run(
        &self,
        name: &'static str,
        args: Value,
        next: &mut dyn FnMut(Value) -> Result<Value, String>,
    ) -> Result<Value, String>;
    /// Starts the hooks of `name` on the home thread, for a caller
    /// elsewhere: they ask for the original through `remote`.
    fn start(&self, name: &'static str, args: Value, remote: Remote);
}

static INTERCEPTOR: OnceLock<Box<dyn Interceptor>> = OnceLock::new();

pub fn set_interceptor(interceptor: Box<dyn Interceptor>) {
    let _ = INTERCEPTOR.set(interceptor);
}

/// Runs a hooked synchronous call: `next` is the original.
pub fn call_sync(
    hook: &HookPoint,
    args: Value,
    next: &mut dyn FnMut(Value) -> Result<Value, String>,
) -> Result<Value, String> {
    let Some(interceptor) = INTERCEPTOR.get() else {
        return next(args);
    };
    if interceptor.is_home() {
        return interceptor.run(hook.name, args, next);
    }
    // Elsewhere: wait for the hooks, running the original when asked.
    let (send, steps) = std::sync::mpsc::channel();
    let (results, results_rx) = tokio::sync::mpsc::unbounded_channel();
    interceptor.start(
        hook.name,
        args,
        Remote {
            send: Box::new(move |s| {
                let _ = send.send(s);
            }),
            results: results_rx,
        },
    );
    loop {
        match steps.recv() {
            Ok(Step::Next(args)) => {
                let _ = results.send(next(args));
            }
            Ok(Step::Done(result)) => return result,
            Err(_) => return Err(format!("the hooks of {} stopped", hook.name)),
        }
    }
}

/// Runs a hooked async call: `next` is the original.
pub async fn call_async<F, Fut>(hook: &HookPoint, args: Value, mut next: F) -> Result<Value, String>
where
    F: FnMut(Value) -> Fut,
    Fut: Future<Output = Result<Value, String>>,
{
    let Some(interceptor) = INTERCEPTOR.get() else {
        return next(args).await;
    };
    let (send, mut steps) = tokio::sync::mpsc::unbounded_channel();
    let (results, results_rx) = tokio::sync::mpsc::unbounded_channel();
    interceptor.start(
        hook.name,
        args,
        Remote {
            send: Box::new(move |s| {
                let _ = send.send(s);
            }),
            results: results_rx,
        },
    );
    while let Some(step) = steps.recv().await {
        match step {
            Step::Next(args) => {
                let _ = results.send(next(args).await);
            }
            Step::Done(result) => return result,
        }
    }
    Err(format!("the hooks of {} stopped", hook.name))
}

/// The result of a function returning `Result`, to and from hooks.
pub trait Outcome: Sized {
    fn to_hook(self) -> Result<Value, String>;
    fn from_hook(result: Result<Value, String>) -> Self;
}

impl<T: Serialize + DeserializeOwned, E: std::fmt::Display + HookError> Outcome for Result<T, E> {
    fn to_hook(self) -> Result<Value, String> {
        match self {
            Ok(v) => to_value(&v),
            Err(e) => Err(e.to_string()),
        }
    }

    fn from_hook(result: Result<Value, String>) -> Self {
        match result {
            Ok(v) => from_value(v, "the result").map_err(E::hook_error),
            Err(e) => Err(E::hook_error(e)),
        }
    }
}

/// The result of any other function. A hook can't fail it: its error
/// is reported and the function runs unhooked.
pub trait Plain: Sized {
    fn to_hook(&self) -> Result<Value, String>;
    fn from_hook(result: Result<Value, String>) -> Result<Self, String>;
}

impl<T: Serialize + DeserializeOwned> Plain for T {
    fn to_hook(&self) -> Result<Value, String> {
        to_value(self)
    }

    fn from_hook(result: Result<Value, String>) -> Result<Self, String> {
        from_value(result?, "the result")
    }
}

static REPORTER: OnceLock<fn(&str, &str)> = OnceLock::new();

/// Sets how hook errors that can't reach the caller are reported.
pub fn set_reporter(report: fn(&str, &str)) {
    let _ = REPORTER.set(report);
}

pub fn report(name: &str, error: &str) {
    match REPORTER.get() {
        Some(r) => r(name, error),
        None => eprintln!("hook of {name}: {error}"),
    }
}

/// Error types a hook's error can become.
pub trait HookError {
    fn hook_error(message: String) -> Self;
}

impl HookError for String {
    fn hook_error(message: String) -> Self {
        message
    }
}

impl HookError for serde_json::Error {
    fn hook_error(message: String) -> Self {
        serde::de::Error::custom(message)
    }
}

impl HookError for std::io::Error {
    fn hook_error(message: String) -> Self {
        std::io::Error::other(message)
    }
}

pub fn to_value<T: Serialize + ?Sized>(value: &T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

/// Reads a value as `T`. Lua has one kind of table for lists and maps, so
/// an empty list from Lua (`videos = {}`) arrives as an empty object: if
/// the value doesn't fit as it is, it's read again with empty objects as
/// empty lists.
pub fn from_value<T: DeserializeOwned>(value: Value, what: &str) -> Result<T, String> {
    match serde_json::from_value::<T>(value.clone()) {
        Ok(v) => Ok(v),
        Err(e) => {
            let mut lists = value;
            if empty_objects_to_lists(&mut lists)
                && let Ok(v) = serde_json::from_value(lists)
            {
                return Ok(v);
            }
            Err(format!("{what}: {e}"))
        }
    }
}

/// Turns every empty object into an empty list; true if there was one.
fn empty_objects_to_lists(value: &mut Value) -> bool {
    let children: Vec<&mut Value> = match value {
        Value::Object(map) if map.is_empty() => {
            *value = Value::Array(Vec::new());
            return true;
        }
        Value::Object(map) => map.values_mut().collect(),
        Value::Array(items) => items.iter_mut().collect(),
        _ => return false,
    };
    // Every child, not just up to the first changed one.
    let mut changed = false;
    for child in children {
        changed |= empty_objects_to_lists(child);
    }
    changed
}

/// The arguments of a hooked call, by name.
pub fn args(pairs: Vec<(&'static str, Result<Value, String>)>) -> Result<Value, String> {
    let mut map = serde_json::Map::new();
    for (name, value) in pairs {
        map.insert(name.to_owned(), value?);
    }
    Ok(Value::Object(map))
}

/// Takes the argument `name` out of a hook's arguments.
pub fn arg<T: DeserializeOwned>(args: &mut Value, name: &str) -> Result<T, String> {
    let value = args.get_mut(name).map(Value::take).unwrap_or(Value::Null);
    from_value(value, name)
}

/// Takes the argument at `index` of a positional call.
pub fn positional<T: DeserializeOwned>(
    args: &mut [Value],
    index: usize,
    name: &str,
) -> Result<T, String> {
    let value = args.get_mut(index).map(Value::take).unwrap_or(Value::Null);
    from_value(value, name)
}

type Provider = Box<dyn Fn() -> Option<Box<dyn Any>> + Send + Sync>;

static INSTANCES: RwLock<Option<HashMap<TypeId, Provider>>> = RwLock::new(None);

/// Says where the instance of a service type is, for calls that don't
/// come with one: `provide_arc(|| Some(app().youtube.clone()))`.
pub fn provide_arc<T: Send + Sync + 'static>(get: fn() -> Option<Arc<T>>) {
    provide::<T>(Box::new(move || get().map(|a| Box::new(a) as Box<dyn Any>)));
}

/// The same for main-thread services.
pub fn provide_rc<T: 'static>(get: fn() -> Option<Rc<T>>) {
    provide::<T>(Box::new(move || get().map(|r| Box::new(r) as Box<dyn Any>)));
}

fn provide<T: 'static>(provider: Provider) {
    let mut instances = INSTANCES.write().unwrap_or_else(|e| e.into_inner());
    instances
        .get_or_insert_with(HashMap::new)
        .insert(TypeId::of::<T>(), provider);
}

fn provided<T: 'static>() -> Result<Box<dyn Any>, String> {
    let instances = INSTANCES.read().unwrap_or_else(|e| e.into_inner());
    let provider = instances
        .as_ref()
        .and_then(|m| m.get(&TypeId::of::<T>()))
        .ok_or_else(|| format!("no {} to call", std::any::type_name::<T>()))?;
    provider().ok_or_else(|| format!("the {} isn't running", std::any::type_name::<T>()))
}

/// A service instance, shared.
pub enum Shared<T> {
    Arc(Arc<T>),
    Rc(Rc<T>),
}

impl<T> Deref for Shared<T> {
    type Target = T;
    fn deref(&self) -> &T {
        match self {
            Shared::Arc(a) => a,
            Shared::Rc(r) => r,
        }
    }
}

pub fn instance<T: 'static>() -> Result<Shared<T>, String> {
    let any = provided::<T>()?;
    let any = match any.downcast::<Arc<T>>() {
        Ok(a) => return Ok(Shared::Arc(*a)),
        Err(any) => any,
    };
    any.downcast::<Rc<T>>()
        .map(|r| Shared::Rc(*r))
        .map_err(|_| format!("the {} is of another kind", std::any::type_name::<T>()))
}

/// A thread-safe service instance, for async calls run anywhere.
pub fn instance_arc<T: 'static>() -> Result<Arc<T>, String> {
    match instance::<T>()? {
        Shared::Arc(a) => Ok(a),
        Shared::Rc(_) => Err(format!(
            "the {} is a main-thread service",
            std::any::type_name::<T>()
        )),
    }
}

pub fn instance_rc<T: 'static>() -> Result<Rc<T>, String> {
    match instance::<T>()? {
        Shared::Rc(r) => Ok(r),
        Shared::Arc(_) => Err(format!(
            "the {} isn't a main-thread service",
            std::any::type_name::<T>()
        )),
    }
}
