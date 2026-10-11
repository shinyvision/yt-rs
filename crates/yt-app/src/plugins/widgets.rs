//! Conveniences on every widget, beyond GTK's own methods (which plugins
//! reach through `gi`): added to `Gtk.Widget`'s class table, so they're
//! methods of every widget and plugins can replace or add their own the
//! same way.
//!
//! ```lua
//! box:append(child)          -- any container: boxes, lists, overlays, groups
//! box:remove(child)
//! for _, c in ipairs(w:children()) do ... end
//! local grid = page:find("video-grid")   -- by name, or ".class"
//! local width, height = w:size()
//! for _, c in ipairs(w:controllers()) do ... end
//! w:on_tick(function(seconds) ... return true end)
//! ```

use gtk::glib;
use gtk::prelude::*;
use mlua::{Function, Lua, UserDataRef};
use yt_gi::Object;

use super::report;

/// The widget a Lua value holds.
pub fn widget(value: &Object) -> mlua::Result<gtk::Widget> {
    value
        .0
        .clone()
        .downcast::<gtk::Widget>()
        .map_err(|o| mlua::Error::runtime(format!("a {} is not a widget", o.type_().name())))
}

pub fn wrap(widget: impl IsA<gtk::Widget>) -> Object {
    Object(widget.upcast::<gtk::Widget>().upcast())
}

pub fn children(widget: &gtk::Widget) -> Vec<gtk::Widget> {
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

pub fn install(lua: &Lua) -> mlua::Result<()> {
    let class = yt_gi::class(lua, gtk::Widget::static_type())?;
    // Adds a child to any container: as GTK's `append` where there is
    // one, an overlay's overlay, a preferences group's row, or the child.
    class.set(
        "append",
        lua.create_function(
            |_, (this, child): (UserDataRef<Object>, UserDataRef<Object>)| {
                let (parent, child) = (widget(&this)?, widget(&child)?);
                if let Some(b) = parent.downcast_ref::<gtk::Box>() {
                    b.append(&child);
                } else if let Some(l) = parent.downcast_ref::<gtk::ListBox>() {
                    l.append(&child);
                } else if let Some(f) = parent.downcast_ref::<gtk::FlowBox>() {
                    f.append(&child);
                } else if let Some(o) = parent.downcast_ref::<gtk::Overlay>() {
                    o.add_overlay(&child);
                } else if let Some(g) = parent.downcast_ref::<adw::PreferencesGroup>() {
                    adw::prelude::PreferencesGroupExt::add(g, &child);
                } else if let (Some(p), Some(g)) = (
                    parent.downcast_ref::<adw::PreferencesPage>(),
                    child.downcast_ref::<adw::PreferencesGroup>(),
                ) {
                    adw::prelude::PreferencesPageExt::add(p, g);
                } else if parent.find_property("child").is_some() {
                    parent.set_property("child", &child);
                } else {
                    return Err(mlua::Error::runtime(format!(
                        "can't add children to a {}",
                        parent.type_().name()
                    )));
                }
                Ok(())
            },
        )?,
    )?;
    // Removes a child from any container; nothing if it isn't one.
    class.set(
        "remove",
        lua.create_function(
            |_, (this, child): (UserDataRef<Object>, UserDataRef<Object>)| {
                let (parent, child) = (widget(&this)?, widget(&child)?);
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
            },
        )?,
    )?;
    class.set(
        "children",
        lua.create_function(|_, this: UserDataRef<Object>| {
            Ok(children(&widget(&this)?)
                .into_iter()
                .map(wrap)
                .collect::<Vec<_>>())
        })?,
    )?;
    class.set(
        "find",
        lua.create_function(|_, (this, query): (UserDataRef<Object>, String)| {
            Ok(find(&widget(&this)?, &query).map(wrap))
        })?,
    )?;
    class.set(
        "size",
        lua.create_function(|_, this: UserDataRef<Object>| {
            let w = widget(&this)?;
            Ok((w.width(), w.height()))
        })?,
    )?;
    // The widget's event controllers (gestures, key and scroll handlers).
    class.set(
        "controllers",
        lua.create_function(|_, this: UserDataRef<Object>| {
            let model = widget(&this)?.observe_controllers();
            Ok((0..model.n_items())
                .filter_map(|i| model.item(i))
                .map(Object)
                .collect::<Vec<_>>())
        })?,
    )?;
    // Calls `f(widget, frame_time_in_seconds)` every frame until it
    // returns false. For animations and games.
    class.set(
        "on_tick",
        lua.create_function(|_, (this, f): (UserDataRef<Object>, Function)| {
            widget(&this)?.add_tick_callback(move |_, clock| {
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
        })?,
    )?;
    Ok(())
}
