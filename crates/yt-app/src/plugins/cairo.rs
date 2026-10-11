//! Cairo drawing for Lua, used by `ui.drawing_area`: custom widgets, charts
//! and games.

use gtk::cairo;
use mlua::{UserData, UserDataMethods};

pub struct LuaCairo(pub cairo::Context);

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
            let rgba = gtk::gdk::RGBA::parse(&color)
                .map_err(|_| mlua::Error::runtime(format!("invalid color {color:?}")))?;
            c.0.set_source_rgba(
                rgba.red() as f64,
                rgba.green() as f64,
                rgba.blue() as f64,
                rgba.alpha() as f64,
            );
            Ok(())
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
