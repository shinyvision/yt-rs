//! Cairo drawing contexts for Lua: what draw functions get
//! (`Gtk.DrawingArea:set_draw_func`, `Gtk.Snapshot:append_cairo`).
//! Cairo isn't a GObject library and has no introspected functions, so
//! its drawing calls are bound here; `cr:ptr()` reaches the rest of cairo
//! through LuaJIT's FFI.

use std::ffi::c_void;

use mlua::{Lua, UserData, UserDataMethods};

use crate::info::Info;

pub struct LuaCairo(pub cairo::Context);

/// The context behind a Lua value, for passing to C.
pub fn ptr(value: &mlua::Value) -> Option<*mut c_void> {
    let mlua::Value::UserData(ud) = value else {
        return None;
    };
    ud.borrow::<LuaCairo>()
        .ok()
        .map(|c| c.0.to_raw_none().cast())
}

/// A `cairo.Context` argument from Lua.
pub fn from_lua(value: &mlua::Value, info: &Info) -> Option<*mut c_void> {
    (info.full_name() == "cairo.Context")
        .then(|| ptr(value))
        .flatten()
}

/// A `cairo.Context` from C as Lua.
pub fn to_lua(
    lua: &Lua,
    p: *mut c_void,
    info: &Info,
    full: bool,
) -> mlua::Result<Option<mlua::Value>> {
    if info.full_name() != "cairo.Context" || p.is_null() {
        return Ok(None);
    }
    // SAFETY: a cairo context; one reference is ours either way.
    let context = unsafe {
        if full {
            cairo::Context::from_raw_full(p.cast())
        } else {
            cairo::Context::from_raw_none(p.cast())
        }
    };
    Ok(Some(mlua::Value::UserData(
        lua.create_userdata(LuaCairo(context))?,
    )))
}

/// Parses a CSS color with GDK's parser, found through introspection.
fn parse_color(text: &str) -> Option<(f64, f64, f64, f64)> {
    #[repr(C)]
    struct Rgba([f32; 4]);
    let info = crate::repo::find_by_name("Gdk", "RGBA")?;
    let parse = info.symbol("gdk_rgba_parse")?;
    // SAFETY: gdk_rgba_parse(GdkRGBA*, const char*) -> gboolean.
    let parse: unsafe extern "C" fn(*mut Rgba, *const std::ffi::c_char) -> i32 =
        unsafe { std::mem::transmute(parse) };
    let text = std::ffi::CString::new(text).ok()?;
    let mut rgba = Rgba([0.0; 4]);
    // SAFETY: as above.
    (unsafe { parse(&mut rgba, text.as_ptr()) } != 0).then(|| {
        (
            rgba.0[0] as f64,
            rgba.0[1] as f64,
            rgba.0[2] as f64,
            rgba.0[3] as f64,
        )
    })
}

fn err(e: cairo::Error) -> mlua::Error {
    mlua::Error::runtime(format!("cairo: {e}"))
}

impl UserData for LuaCairo {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("set_source_rgb", |_, c, (r, g, b): (f64, f64, f64)| {
            c.0.set_source_rgb(r, g, b);
            Ok(())
        });
        methods.add_method(
            "set_source_rgba",
            |_, c, (r, g, b, a): (f64, f64, f64, f64)| {
                c.0.set_source_rgba(r, g, b, a);
                Ok(())
            },
        );
        // A CSS color: "#ff0033", "white", "rgba(0,0,0,0.5)".
        methods.add_method("set_color", |_, c, color: String| {
            let (r, g, b, a) = parse_color(&color)
                .ok_or_else(|| mlua::Error::runtime(format!("invalid color {color:?}")))?;
            c.0.set_source_rgba(r, g, b, a);
            Ok(())
        });
        // The C pointer, for the rest of cairo through LuaJIT's FFI.
        methods.add_method("ptr", |_, c, ()| {
            Ok(mlua::LightUserData(c.0.to_raw_none().cast()))
        });
        methods.add_method("set_line_width", |_, c, w: f64| {
            c.0.set_line_width(w);
            Ok(())
        });
        methods.add_method("move_to", |_, c, (x, y): (f64, f64)| {
            c.0.move_to(x, y);
            Ok(())
        });
        methods.add_method("line_to", |_, c, (x, y): (f64, f64)| {
            c.0.line_to(x, y);
            Ok(())
        });
        methods.add_method(
            "curve_to",
            |_, c, (x1, y1, x2, y2, x3, y3): (f64, f64, f64, f64, f64, f64)| {
                c.0.curve_to(x1, y1, x2, y2, x3, y3);
                Ok(())
            },
        );
        methods.add_method("rectangle", |_, c, (x, y, w, h): (f64, f64, f64, f64)| {
            c.0.rectangle(x, y, w, h);
            Ok(())
        });
        methods.add_method(
            "arc",
            |_, c, (x, y, r, a1, a2): (f64, f64, f64, f64, f64)| {
                c.0.arc(x, y, r, a1, a2);
                Ok(())
            },
        );
        methods.add_method("new_path", |_, c, ()| {
            c.0.new_path();
            Ok(())
        });
        methods.add_method("close_path", |_, c, ()| {
            c.0.close_path();
            Ok(())
        });
        methods.add_method("fill", |_, c, ()| c.0.fill().map_err(err));
        methods.add_method("fill_preserve", |_, c, ()| c.0.fill_preserve().map_err(err));
        methods.add_method("stroke", |_, c, ()| c.0.stroke().map_err(err));
        methods.add_method("stroke_preserve", |_, c, ()| {
            c.0.stroke_preserve().map_err(err)
        });
        methods.add_method("paint", |_, c, ()| c.0.paint().map_err(err));
        methods.add_method("clip", |_, c, ()| {
            c.0.clip();
            Ok(())
        });
        methods.add_method("save", |_, c, ()| c.0.save().map_err(err));
        methods.add_method("restore", |_, c, ()| c.0.restore().map_err(err));
        methods.add_method("translate", |_, c, (x, y): (f64, f64)| {
            c.0.translate(x, y);
            Ok(())
        });
        methods.add_method("scale", |_, c, (x, y): (f64, f64)| {
            c.0.scale(x, y);
            Ok(())
        });
        methods.add_method("rotate", |_, c, angle: f64| {
            c.0.rotate(angle);
            Ok(())
        });
        methods.add_method(
            "select_font_face",
            |_, c, (family, bold): (String, Option<bool>)| {
                let weight = if bold.unwrap_or(false) {
                    cairo::FontWeight::Bold
                } else {
                    cairo::FontWeight::Normal
                };
                c.0.select_font_face(&family, cairo::FontSlant::Normal, weight);
                Ok(())
            },
        );
        methods.add_method("set_font_size", |_, c, size: f64| {
            c.0.set_font_size(size);
            Ok(())
        });
        methods.add_method("show_text", |_, c, text: String| {
            c.0.show_text(&text).map_err(err)
        });
        // Returns width, height of `text` in the current font.
        methods.add_method("text_size", |_, c, text: String| {
            let e = c.0.text_extents(&text).map_err(err)?;
            Ok((e.width(), e.height()))
        });
    }
}
