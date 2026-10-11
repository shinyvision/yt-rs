//! A layer over another widget (the video) for widgets placed anywhere on
//! it: each child sits where its own `halign`, `valign` and margins put it,
//! as an overlay's children do, and the rest of the layer lets the pointer
//! through to what is under it. A plain box laid over the video would take
//! every click and every move of the pointer.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Layer;

    #[glib::object_subclass]
    impl ObjectSubclass for Layer {
        const NAME: &'static str = "YtLayer";
        type Type = super::Layer;
        type ParentType = gtk::Widget;

        fn class_init(class: &mut Self::Class) {
            class.set_css_name("layer");
        }
    }

    impl ObjectImpl for Layer {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for Layer {
        // It takes no room of its own: the widget it covers decides.
        fn measure(&self, _orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            (0, 0, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            let mut child = self.obj().first_child();
            while let Some(widget) = child {
                if widget.should_layout() {
                    let (_, natural) = widget.preferred_size();
                    let (w, x) = place(widget.halign(), natural.width(), width);
                    let (_, natural_height, _, _) = widget.measure(gtk::Orientation::Vertical, w);
                    let (h, y) = place(widget.valign(), natural_height, height);
                    widget.size_allocate(&gtk::Allocation::new(x, y, w, h), -1);
                }
                child = widget.next_sibling();
            }
        }

        // Only its children take the pointer.
        fn contains(&self, _x: f64, _y: f64) -> bool {
            false
        }
    }

    /// A child's size and offset along one side, for its alignment.
    fn place(align: gtk::Align, natural: i32, available: i32) -> (i32, i32) {
        let size = natural.min(available).max(0);
        match align {
            gtk::Align::Fill => (available, 0),
            gtk::Align::Start => (size, 0),
            gtk::Align::End => (size, available - size),
            _ => (size, (available - size) / 2),
        }
    }
}

glib::wrapper! {
    pub struct Layer(ObjectSubclass<imp::Layer>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Layer {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl Layer {
    pub fn add(&self, child: &impl IsA<gtk::Widget>) {
        child.set_parent(self);
        self.queue_resize();
    }

    pub fn clear(&self) {
        while let Some(child) = self.first_child() {
            child.unparent();
        }
    }
}
