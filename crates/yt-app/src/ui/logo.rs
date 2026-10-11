//! The logo's mark: YouTube's play button, drawn in the widget's CSS
//! `color` (so a theme can recolor it, `.logo-mark { color: ... }`) with a
//! white triangle. It scales to whatever size CSS gives it.

use std::cell::OnceCell;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene, gsk};

/// The mark's drawing, in a 159 × 110 box.
const SIZE: (f32, f32) = (159.0, 110.0);
const BUTTON: &str = "M154 17.5c-1.82-6.73-7.07-12-13.8-13.8-9.04-3.49-96.6-5.2-122 .1-6.73 \
    1.82-12 7.07-13.8 13.8-4.08 17.9-4.39 56.6.1 74.9 1.82 6.73 7.07 12 13.8 13.8 17.9 4.12 103 \
    4.7 122 0 6.73-1.82 12-7.07 13.8-13.8 4.35-19.5 4.66-55.8-.1-75z";
const TRIANGLE: &str = "M105 55 64.2 31.6v46.8z";
/// The mark's size unless CSS asks for another.
const HEIGHT: i32 = 20;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct LogoMark {
        pub paths: OnceCell<(gsk::Path, gsk::Path)>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LogoMark {
        const NAME: &'static str = "YtLogoMark";
        type Type = super::LogoMark;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("logo-mark");
        }
    }

    impl ObjectImpl for LogoMark {}

    impl WidgetImpl for LogoMark {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let size = match orientation {
                gtk::Orientation::Horizontal => (HEIGHT as f32 * SIZE.0 / SIZE.1).round() as i32,
                _ => HEIGHT,
            };
            (size, size, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let (width, height) = (widget.width() as f32, widget.height() as f32);
            let (button, triangle) = self.paths.get_or_init(|| {
                let parse = |d| gsk::Path::parse(d).expect("the logo's paths are valid");
                (parse(BUTTON), parse(TRIANGLE))
            });
            // Fit the drawing in the widget, centered.
            let scale = (width / SIZE.0).min(height / SIZE.1);
            snapshot.save();
            snapshot.translate(&graphene::Point::new(
                (width - SIZE.0 * scale) / 2.0,
                (height - SIZE.1 * scale) / 2.0,
            ));
            snapshot.scale(scale, scale);
            snapshot.append_fill(button, gsk::FillRule::Winding, &widget.color());
            snapshot.append_fill(triangle, gsk::FillRule::Winding, &gdk::RGBA::WHITE);
            snapshot.restore();
        }
    }
}

glib::wrapper! {
    pub struct LogoMark(ObjectSubclass<imp::LogoMark>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for LogoMark {
    fn default() -> Self {
        let mark: Self = glib::Object::new();
        mark.add_css_class("logo-mark");
        mark
    }
}
