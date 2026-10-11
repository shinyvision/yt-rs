//! A container that is always a fixed shape: given a width, it asks for
//! `width × ratio` height. Thumbnails use it so cards keep their 16:9 shape
//! even before the image has loaded; the watch page uses it for the player,
//! with a maximum height so the player never gets taller than the view.

use std::cell::Cell;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

mod imp {
    use super::*;

    pub struct AspectBin {
        /// Height divided by width.
        pub ratio: Cell<f64>,
        /// At most this tall, if above 0.
        pub max_height: Cell<i32>,
        /// Ignore the ratio and be exactly `max_height` tall.
        pub fill: Cell<bool>,
        /// The width asked for when there is room.
        pub natural_width: Cell<i32>,
    }

    impl Default for AspectBin {
        fn default() -> Self {
            Self {
                ratio: Cell::new(9.0 / 16.0),
                max_height: Cell::new(0),
                fill: Cell::new(false),
                natural_width: Cell::new(320),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AspectBin {
        const NAME: &'static str = "YtAspectBin";
        type Type = super::AspectBin;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for AspectBin {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for AspectBin {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            match orientation {
                gtk::Orientation::Horizontal => (0, self.natural_width.get(), -1, -1),
                _ => {
                    let width = if for_size >= 0 {
                        for_size
                    } else {
                        self.natural_width.get()
                    };
                    let mut height = (width as f64 * self.ratio.get()).round() as i32;
                    let max = self.max_height.get();
                    if max > 0 {
                        height = if self.fill.get() {
                            max
                        } else {
                            height.min(max)
                        };
                    }
                    (height, height, -1, -1)
                }
            }
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(child) = self.obj().first_child() {
                child.allocate(width, height, baseline, None);
            }
        }
    }
}

glib::wrapper! {
    pub struct AspectBin(ObjectSubclass<imp::AspectBin>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl AspectBin {
    /// `ratio` is height divided by width, e.g. 9/16.
    pub fn new(ratio: f64, child: &impl IsA<gtk::Widget>) -> Self {
        let bin: Self = glib::Object::new();
        bin.imp().ratio.set(ratio);
        child.set_parent(&bin);
        bin
    }

    /// The width asked for when there is room (320 by default).
    pub fn set_natural_width(&self, width: i32) {
        self.imp().natural_width.set(width);
        self.queue_resize();
    }

    /// Caps the height (0 for no cap).
    pub fn set_max_height(&self, height: i32) {
        if self.imp().max_height.replace(height) != height {
            self.queue_resize();
        }
    }

    /// Fills the maximum height whatever the width, e.g. in fullscreen.
    pub fn set_fill(&self, fill: bool) {
        if self.imp().fill.replace(fill) != fill {
            self.queue_resize();
        }
    }
}
