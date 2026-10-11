//! The layout of a search result, as on youtube.com: the thumbnail takes a
//! share of the row's width, between a minimum and a maximum, and the text
//! the rest. Below a width the row stacks instead, the thumbnail on top at
//! full width, like a feed card.

use std::cell::Cell;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

mod imp {
    use super::*;

    pub struct SplitBox {
        /// The share of the width the first child gets.
        pub fraction: Cell<f64>,
        pub min_start: Cell<i32>,
        pub max_start: Cell<i32>,
        pub spacing: Cell<i32>,
        /// Narrower than this, the children stack.
        pub stack_below: Cell<i32>,
    }

    impl Default for SplitBox {
        fn default() -> Self {
            Self {
                fraction: Cell::new(0.45),
                min_start: Cell::new(240),
                max_start: Cell::new(500),
                spacing: Cell::new(16),
                stack_below: Cell::new(560),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SplitBox {
        const NAME: &'static str = "YtSplitBox";
        type Type = super::SplitBox;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for SplitBox {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl SplitBox {
        fn children(&self) -> Option<(gtk::Widget, gtk::Widget)> {
            let start = self.obj().first_child()?;
            let end = start.next_sibling()?;
            Some((start, end))
        }

        /// The first child's width in a row `width` wide, or `None` if the
        /// row stacks at that width.
        pub fn start_width(&self, width: i32) -> Option<i32> {
            if width < self.stack_below.get() {
                return None;
            }
            let share = (width as f64 * self.fraction.get()).round() as i32;
            Some(share.clamp(self.min_start.get(), self.max_start.get()))
        }

        fn height(child: &gtk::Widget, width: i32) -> (i32, i32) {
            if !child.is_visible() {
                return (0, 0);
            }
            let (min, nat, _, _) = child.measure(gtk::Orientation::Vertical, width.max(0));
            (min, nat)
        }
    }

    impl WidgetImpl for SplitBox {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let Some((start, end)) = self.children() else {
                return (0, 0, -1, -1);
            };
            let spacing = self.spacing.get();
            if orientation == gtk::Orientation::Horizontal {
                let (_, end_nat, _, _) = end.measure(orientation, -1);
                let (end_min, _, _, _) = end.measure(orientation, -1);
                let (start_min, _, _, _) = start.measure(orientation, -1);
                let natural = self.max_start.get() + spacing + end_nat;
                return (start_min.max(end_min), natural, -1, -1);
            }
            let width = if for_size >= 0 {
                for_size
            } else {
                self.max_start.get() + spacing + end.measure(gtk::Orientation::Horizontal, -1).1
            };
            match self.start_width(width) {
                Some(start_width) => {
                    let (s_min, s_nat) = Self::height(&start, start_width);
                    let (e_min, e_nat) = Self::height(&end, width - start_width - spacing);
                    (s_min.max(e_min), s_nat.max(e_nat), -1, -1)
                }
                None => {
                    let (s_min, s_nat) = Self::height(&start, width);
                    let (e_min, e_nat) = Self::height(&end, width);
                    (s_min + spacing + e_min, s_nat + spacing + e_nat, -1, -1)
                }
            }
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            let Some((start, end)) = self.children() else {
                return;
            };
            let spacing = self.spacing.get();
            let place = |child: &gtk::Widget, x: i32, y: i32, w: i32, h: i32| {
                let at = gtk::gsk::Transform::new()
                    .translate(&gtk::graphene::Point::new(x as f32, y as f32));
                child.allocate(w.max(0), h.max(0), -1, Some(at));
            };
            match self.start_width(width) {
                Some(start_width) => {
                    let (_, start_height) = Self::height(&start, start_width);
                    place(&start, 0, 0, start_width, start_height.min(height));
                    let x = start_width + spacing;
                    place(&end, x, 0, width - x, height);
                }
                None => {
                    let (_, start_height) = Self::height(&start, width);
                    place(&start, 0, 0, width, start_height);
                    let y = start_height + spacing;
                    place(&end, 0, y, width, height - y);
                }
            }
        }
    }
}

glib::wrapper! {
    pub struct SplitBox(ObjectSubclass<imp::SplitBox>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl SplitBox {
    pub fn new(start: &impl IsA<gtk::Widget>, end: &impl IsA<gtk::Widget>) -> Self {
        let split: Self = glib::Object::new();
        start.set_parent(&split);
        end.set_parent(&split);
        split
    }

    /// The first child's width: `fraction` of the row, within `min..=max`.
    pub fn set_start_width(&self, fraction: f64, min: i32, max: i32) {
        let imp = self.imp();
        imp.fraction.set(fraction);
        imp.min_start.set(min);
        imp.max_start.set(max);
        self.queue_resize();
    }

    /// Stack the children when narrower than `width` (0 never stacks).
    pub fn set_stack_below(&self, width: i32) {
        self.imp().stack_below.set(width);
        self.queue_resize();
    }
}
