//! A wrapping label with pictures in its text: channels' emoji and badges
//! in live chat. GTK labels can't hold pictures, and text views don't size
//! well in lists, so each picture keeps a place in the label's text (a
//! character drawn as nothing, as wide as the picture) and is laid over
//! it, where the label's own layout puts that character.

use gtk::pango;
use gtk::prelude::*;

use crate::app::app;

/// What the label shows, in order.
pub enum Piece<'a> {
    Text {
        text: &'a str,
        /// CSS colors.
        color: Option<&'a str>,
        background: Option<&'a str>,
        bold: bool,
    },
    /// A picture at a URL, `size` pixels square.
    Picture {
        url: &'a str,
        size: i32,
        tooltip: Option<&'a str>,
    },
    /// A symbolic icon, styled by `class`.
    Icon {
        name: &'a str,
        size: i32,
        class: &'a str,
        tooltip: &'a str,
    },
}

impl<'a> Piece<'a> {
    pub fn text(text: &'a str) -> Self {
        Self::Text {
            text,
            color: None,
            background: None,
            bold: false,
        }
    }
}

/// U+FFFC OBJECT REPLACEMENT CHARACTER: where a picture goes.
const PLACE: char = '\u{fffc}';

pub fn build(pieces: &[Piece]) -> gtk::Widget {
    let mut text = String::new();
    let attrs = pango::AttrList::new();
    // Each picture, and the byte index of its place in the text.
    let mut places: Vec<(gtk::Widget, i32)> = Vec::new();
    for piece in pieces {
        let start = text.len() as u32;
        match piece {
            Piece::Text {
                text: t,
                color,
                background,
                bold,
            } => {
                text.push_str(t);
                let end = text.len() as u32;
                let add = |mut attr: pango::Attribute| {
                    attr.set_start_index(start);
                    attr.set_end_index(end);
                    attrs.insert(attr);
                };
                if let Some(rgba) = color.and_then(|c| gtk::gdk::RGBA::parse(c).ok()) {
                    let (r, g, b, a) = channels(&rgba);
                    add(pango::AttrColor::new_foreground(r, g, b).upcast());
                    add(pango::AttrInt::new_foreground_alpha(a).upcast());
                }
                if let Some(rgba) = background.and_then(|c| gtk::gdk::RGBA::parse(c).ok()) {
                    let (r, g, b, a) = channels(&rgba);
                    add(pango::AttrColor::new_background(r, g, b).upcast());
                    add(pango::AttrInt::new_background_alpha(a).upcast());
                }
                if *bold {
                    add(pango::AttrInt::new_weight(pango::Weight::Semibold).upcast());
                }
            }
            Piece::Picture { url, size, tooltip } => {
                let picture = gtk::Picture::new();
                picture.set_can_shrink(true);
                picture.set_content_fit(gtk::ContentFit::Contain);
                picture.set_tooltip_text(*tooltip);
                app().images.load_into(&picture, Some(url), size * 2);
                place(&mut text, &attrs, *size);
                places.push((picture.upcast(), start as i32));
            }
            Piece::Icon {
                name,
                size,
                class,
                tooltip,
            } => {
                let image = gtk::Image::from_icon_name(name);
                image.set_pixel_size(*size);
                image.add_css_class(class);
                image.set_tooltip_text(Some(tooltip));
                place(&mut text, &attrs, *size);
                places.push((image.upcast(), start as i32));
            }
        }
    }

    let label = gtk::Label::new(Some(&text));
    label.set_attributes(Some(&attrs));
    label.set_wrap(true);
    label.set_wrap_mode(pango::WrapMode::WordChar);
    label.set_xalign(0.0);
    label.set_hexpand(true);
    if places.is_empty() {
        return label.upcast();
    }
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&label));
    for (widget, _) in &places {
        overlay.add_overlay(widget);
    }
    // Each picture over its place, as the label lays it out at its width.
    overlay.connect_get_child_position(move |_, widget| {
        let index = places.iter().find(|(w, _)| w == widget)?.1;
        let layout = label.layout();
        let (dx, dy) = label.layout_offsets();
        let rect = layout.index_to_pos(index);
        let size = pango::units_to_double(rect.width()).round() as i32;
        Some(gtk::gdk::Rectangle::new(
            dx + pango::units_to_double(rect.x()).round() as i32,
            dy + pango::units_to_double(rect.y() + rect.height()).round() as i32 - size,
            size,
            size,
        ))
    });
    overlay.upcast()
}

/// A place for a picture: a character drawn as nothing, `size` pixels
/// square, sitting on the line a little below the baseline as emoji do.
fn place(text: &mut String, attrs: &pango::AttrList, size: i32) {
    let start = text.len() as u32;
    text.push(PLACE);
    let units = pango::units_from_double(f64::from(size));
    let descent = units / 5;
    let rect = pango::Rectangle::new(0, descent - units, units, units);
    let mut shape = pango::AttrShape::new(&rect, &rect).upcast();
    shape.set_start_index(start);
    shape.set_end_index(text.len() as u32);
    attrs.insert(shape);
}

fn channels(rgba: &gtk::gdk::RGBA) -> (u16, u16, u16, u16) {
    let scale = |v: f32| (v.clamp(0.0, 1.0) * 65535.0).round() as u16;
    (
        scale(rgba.red()),
        scale(rgba.green()),
        scale(rgba.blue()),
        scale(rgba.alpha()),
    )
}
