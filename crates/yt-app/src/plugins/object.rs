//! `LuaObject`: any GObject (widgets, gesture controllers, adjustments,
//! objects from other libraries) as a Lua value.
//!
//! Properties and signals are reached through GObject's type system, so
//! every widget type works without hand-written bindings:
//!
//! ```lua
//! local button = ui.new("GtkButton", { label = "Hi" })
//! button.label = "Hello"                       -- any property
//! button:on("clicked", function(self) end)     -- any signal
//! button:emit("clicked")                       -- emit one
//! ```

use gtk::glib;
use gtk::prelude::*;
use mlua::{Function, Lua, MetaMethod, UserData, UserDataMethods, UserDataRef};

use super::convert::{glib_to_lua, lua_to_glib};
use super::report;

#[derive(Clone)]
pub struct LuaObject(pub glib::Object);

impl LuaObject {
    pub fn widget(&self) -> mlua::Result<gtk::Widget> {
        self.0
            .clone()
            .downcast::<gtk::Widget>()
            .map_err(|o| mlua::Error::runtime(format!("a {} is not a widget", o.type_().name())))
    }

    fn set(&self, name: &str, value: &mlua::Value) -> mlua::Result<()> {
        let name = &name.replace('_', "-");
        let pspec = self.0.find_property(name).ok_or_else(|| {
            mlua::Error::runtime(format!(
                "{} has no property {name:?}",
                self.0.type_().name()
            ))
        })?;
        let converted = lua_to_glib(value, pspec.value_type())
            .map_err(|e| mlua::Error::runtime(format!("property {name:?}: {e}")))?;
        if !pspec.flags().contains(glib::ParamFlags::WRITABLE) {
            return Err(mlua::Error::runtime(format!(
                "property {name:?} is read-only"
            )));
        }
        self.0.set_property_from_value(name, &converted);
        Ok(())
    }

    fn get(&self, lua: &Lua, name: &str) -> mlua::Result<mlua::Value> {
        let name = &name.replace('_', "-");
        if self.0.find_property(name).is_none() {
            return Ok(mlua::Value::Nil);
        }
        glib_to_lua(lua, &self.0.property_value(name))
    }

    /// Connects a Lua function to a signal. The function gets the object
    /// and the signal's arguments; its return value is used if the signal
    /// expects one (e.g. `true` to stop an event).
    fn connect(&self, lua: &Lua, signal: &str, f: Function) -> mlua::Result<u64> {
        let (id, _) = glib::subclass::signal::SignalId::parse_name(signal, self.0.type_(), true)
            .ok_or_else(|| {
                mlua::Error::runtime(format!(
                    "{} has no signal {signal:?}",
                    self.0.type_().name()
                ))
            })?;
        let return_type = id.query().return_type().type_();
        let lua = lua.clone();
        let handler = self.0.connect_closure(
            signal,
            false,
            glib::RustClosure::new_local(move |args| {
                let lua_args: mlua::Result<Vec<mlua::Value>> =
                    args.iter().map(|a| glib_to_lua(&lua, a)).collect();
                let result =
                    lua_args.and_then(|a| f.call::<mlua::Value>(mlua::MultiValue::from_iter(a)));
                let result = match result {
                    Ok(v) => v,
                    Err(e) => {
                        report("signal handler", &e);
                        mlua::Value::Nil
                    }
                };
                if return_type == glib::Type::UNIT {
                    None
                } else {
                    match lua_to_glib(&result, return_type) {
                        Ok(v) => Some(v),
                        Err(_) => Some(glib::Value::from_type(return_type)),
                    }
                }
            }),
        );
        // SAFETY: only reads the id of a handler that was just connected.
        Ok(unsafe { handler.as_raw() })
    }

    /// Emits a signal with Lua arguments, returning its result.
    fn emit(&self, lua: &Lua, signal: &str, args: &[mlua::Value]) -> mlua::Result<mlua::Value> {
        let (id, _) = glib::subclass::signal::SignalId::parse_name(signal, self.0.type_(), true)
            .ok_or_else(|| {
                mlua::Error::runtime(format!(
                    "{} has no signal {signal:?}",
                    self.0.type_().name()
                ))
            })?;
        let query = id.query();
        let types = query.param_types();
        if args.len() != types.len() {
            return Err(mlua::Error::runtime(format!(
                "signal {signal:?} takes {} arguments, not {}",
                types.len(),
                args.len()
            )));
        }
        let values = args
            .iter()
            .zip(types)
            .map(|(a, t)| lua_to_glib(a, t.type_()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| mlua::Error::runtime(format!("signal {signal:?}: {e}")))?;
        match self.0.emit_by_name_with_values(signal, &values) {
            Some(result) => glib_to_lua(lua, &result),
            None => Ok(mlua::Value::Nil),
        }
    }

    fn disconnect(&self, id: u64) {
        // SAFETY: g_signal_handler_disconnect ignores ids that aren't
        // connected to this instance (with a warning).
        unsafe {
            glib::gobject_ffi::g_signal_handler_disconnect(self.0.as_ptr() as *mut _, id as _)
        };
    }
}

fn children(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut out = Vec::new();
    let mut child = widget.first_child();
    while let Some(w) = child {
        child = w.next_sibling();
        out.push(w);
    }
    out
}

/// Finds a descendant by widget name (`"name"`) or CSS class (`".class"`).
pub fn find(root: &gtk::Widget, query: &str) -> Option<gtk::Widget> {
    let matches = |w: &gtk::Widget| match query.strip_prefix('.') {
        Some(class) => w.has_css_class(class),
        None => w.widget_name() == query,
    };
    if matches(root) {
        return Some(root.clone());
    }
    children(root).iter().find_map(|c| find(c, query))
}

impl UserData for LuaObject {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("set", |_, this, (name, value): (String, mlua::Value)| {
            this.set(&name, &value)
        });
        methods.add_method("get", |lua, this, name: String| this.get(lua, &name));
        methods.add_method("on", |lua, this, (signal, f): (String, Function)| {
            this.connect(lua, &signal, f)
        });
        methods.add_method(
            "emit",
            |lua, this, (signal, args): (String, mlua::Variadic<mlua::Value>)| {
                this.emit(lua, &signal, &args)
            },
        );
        methods.add_method("off", |_, this, id: u64| {
            this.disconnect(id);
            Ok(())
        });
        methods.add_method("type_name", |_, this, ()| {
            Ok(this.0.type_().name().to_owned())
        });
        methods.add_method("is_a", |_, this, name: String| {
            Ok(glib::Type::from_name(&name).is_some_and(|t| this.0.type_().is_a(t)))
        });
        methods.add_method("properties", |_, this, ()| {
            Ok(this
                .0
                .list_properties()
                .iter()
                .map(|p| p.name().to_owned())
                .collect::<Vec<_>>())
        });
        // The C pointer, for LuaJIT's FFI: `ffi.cast("GtkWidget*", w:ptr())`.
        methods.add_method("ptr", |_, this, ()| {
            Ok(mlua::LightUserData(this.0.as_ptr().cast()))
        });

        // Widget tree.
        methods.add_method("append", |_, this, child: UserDataRef<LuaObject>| {
            let (parent, child) = (this.widget()?, child.widget()?);
            if let Some(b) = parent.downcast_ref::<gtk::Box>() {
                b.append(&child);
            } else if let Some(l) = parent.downcast_ref::<gtk::ListBox>() {
                l.append(&child);
            } else if let Some(f) = parent.downcast_ref::<gtk::FlowBox>() {
                f.append(&child);
            } else if let Some(o) = parent.downcast_ref::<gtk::Overlay>() {
                o.add_overlay(&child);
            } else if parent.find_property("child").is_some() {
                parent.set_property("child", &child);
            } else {
                return Err(mlua::Error::runtime(format!(
                    "can't add children to a {}",
                    parent.type_().name()
                )));
            }
            Ok(())
        });
        methods.add_method("prepend", |_, this, child: UserDataRef<LuaObject>| {
            let (parent, child) = (this.widget()?, child.widget()?);
            match parent.downcast_ref::<gtk::Box>() {
                Some(b) => b.prepend(&child),
                None => return Err(mlua::Error::runtime("prepend needs a GtkBox")),
            }
            Ok(())
        });
        methods.add_method("remove", |_, this, child: UserDataRef<LuaObject>| {
            let (parent, child) = (this.widget()?, child.widget()?);
            if child.parent().as_ref() != Some(&parent) {
                return Ok(());
            }
            if let Some(b) = parent.downcast_ref::<gtk::Box>() {
                b.remove(&child);
            } else if let Some(o) = parent.downcast_ref::<gtk::Overlay>() {
                o.remove_overlay(&child);
            } else if parent.find_property("child").is_some() {
                parent.set_property("child", None::<gtk::Widget>);
            } else {
                child.unparent();
            }
            Ok(())
        });
        methods.add_method(
            "set_child",
            |_, this, child: Option<UserDataRef<LuaObject>>| {
                let parent = this.widget()?;
                let child = child.map(|c| c.widget()).transpose()?;
                parent.set_property("child", child);
                Ok(())
            },
        );
        methods.add_method("children", |_, this, ()| {
            Ok(children(&this.widget()?)
                .into_iter()
                .map(|w| LuaObject(w.upcast()))
                .collect::<Vec<_>>())
        });
        methods.add_method("parent", |_, this, ()| {
            Ok(this.widget()?.parent().map(|w| LuaObject(w.upcast())))
        });
        methods.add_method("find", |_, this, query: String| {
            Ok(find(&this.widget()?, &query).map(|w| LuaObject(w.upcast())))
        });
        methods.add_method("add_class", |_, this, class: String| {
            this.widget()?.add_css_class(&class);
            Ok(())
        });
        methods.add_method("remove_class", |_, this, class: String| {
            this.widget()?.remove_css_class(&class);
            Ok(())
        });
        methods.add_method("has_class", |_, this, class: String| {
            Ok(this.widget()?.has_css_class(&class))
        });
        methods.add_method("show", |_, this, ()| {
            this.widget()?.set_visible(true);
            Ok(())
        });
        methods.add_method("hide", |_, this, ()| {
            this.widget()?.set_visible(false);
            Ok(())
        });
        methods.add_method("queue_draw", |_, this, ()| {
            this.widget()?.queue_draw();
            Ok(())
        });
        methods.add_method("grab_focus", |_, this, ()| Ok(this.widget()?.grab_focus()));
        // The widget's event controllers (gestures, key and scroll handlers).
        methods.add_method("controllers", |_, this, ()| {
            let model = this.widget()?.observe_controllers();
            Ok((0..model.n_items())
                .filter_map(|i| model.item(i))
                .map(LuaObject)
                .collect::<Vec<_>>())
        });
        // Whether the widget is on screen (shown, and all its parents too).
        methods.add_method("mapped", |_, this, ()| Ok(this.widget()?.is_mapped()));
        methods.add_method("size", |_, this, ()| {
            let w = this.widget()?;
            Ok((w.width(), w.height()))
        });
        methods.add_method(
            "add_controller",
            |_, this, controller: UserDataRef<LuaObject>| {
                let controller = controller
                    .0
                    .clone()
                    .downcast::<gtk::EventController>()
                    .map_err(|_| {
                        mlua::Error::runtime("add_controller needs an event controller")
                    })?;
                this.widget()?.add_controller(controller);
                Ok(())
            },
        );
        // Calls `f(widget, frame_time_in_seconds)` every frame until it
        // returns false. For animations and games.
        methods.add_method("on_tick", |_, this, f: Function| {
            this.widget()?.add_tick_callback(move |_, clock| {
                let seconds = clock.frame_time() as f64 / 1_000_000.0;
                match f.call::<mlua::Value>(seconds) {
                    Ok(mlua::Value::Boolean(false)) => glib::ControlFlow::Break,
                    Ok(_) => glib::ControlFlow::Continue,
                    Err(e) => {
                        report("tick callback", &e);
                        glib::ControlFlow::Break
                    }
                }
            });
            Ok(())
        });

        methods.add_meta_method(MetaMethod::Index, |lua, this, key: String| {
            this.get(lua, &key)
        });
        methods.add_meta_method(
            MetaMethod::NewIndex,
            |_, this, (key, value): (String, mlua::Value)| this.set(&key, &value),
        );
        methods.add_meta_method(MetaMethod::Eq, |_, this, other: UserDataRef<LuaObject>| {
            Ok(this.0 == other.0)
        });
        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            let name = this
                .0
                .downcast_ref::<gtk::Widget>()
                .map(|w| w.widget_name().to_string())
                .unwrap_or_default();
            Ok(format!("{}({name})", this.0.type_().name()))
        });
    }
}

/// Wraps a raw GObject pointer (e.g. from LuaJIT's FFI), checking that it
/// really is a GObject.
pub fn from_ptr(ptr: *mut std::ffi::c_void) -> mlua::Result<LuaObject> {
    if ptr.is_null() {
        return Err(mlua::Error::runtime("null pointer"));
    }
    let instance = ptr as *mut glib::gobject_ffi::GTypeInstance;
    // SAFETY: checks the instance's type before treating it as a GObject.
    let is_object = unsafe {
        glib::gobject_ffi::g_type_check_instance_is_a(instance, glib::gobject_ffi::G_TYPE_OBJECT)
    } != 0;
    if !is_object {
        return Err(mlua::Error::runtime("the pointer is not a GObject"));
    }
    let raw = ptr as *mut glib::gobject_ffi::GObject;
    // SAFETY: a valid GObject. New widgets from C constructors carry a
    // floating reference; sink it and take it over, otherwise add one.
    let object: glib::Object = unsafe {
        if glib::gobject_ffi::g_object_is_floating(raw) != glib::ffi::GFALSE {
            glib::gobject_ffi::g_object_ref_sink(raw);
            glib::translate::from_glib_full(raw)
        } else {
            glib::translate::from_glib_none(raw)
        }
    };
    Ok(LuaObject(object))
}
