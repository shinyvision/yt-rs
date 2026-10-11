//! A frame from a storyboard sheet (a grid of small frames of the video),
//! for the picture above the seek bar while hovering it.

use std::cell::{Cell, RefCell};

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene, gsk};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct FramePreview {
        pub sheet: RefCell<Option<gdk::Texture>>,
        /// (rows, columns) of the sheet, and the frame shown (row, column).
        pub grid: Cell<(u32, u32)>,
        pub frame: Cell<(u32, u32)>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FramePreview {
        const NAME: &'static str = "YtFramePreview";
        type Type = super::FramePreview;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("frame-preview");
        }
    }

    impl ObjectImpl for FramePreview {}

    impl WidgetImpl for FramePreview {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let Some(sheet) = self.sheet.borrow().clone() else {
                return;
            };
            let obj = self.obj();
            let (w, h) = (obj.width() as f32, obj.height() as f32);
            let ((rows, columns), (row, column)) = (self.grid.get(), self.frame.get());
            let view = graphene::Rect::new(0.0, 0.0, w, h);
            snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(view, 8.0));
            // The whole sheet, scaled so one frame fills the widget, moved so
            // the wanted frame is in view.
            snapshot.append_texture(
                &sheet,
                &graphene::Rect::new(
                    -(column as f32) * w,
                    -(row as f32) * h,
                    columns.max(1) as f32 * w,
                    rows.max(1) as f32 * h,
                ),
            );
            snapshot.pop();
        }
    }
}

glib::wrapper! {
    pub struct FramePreview(ObjectSubclass<imp::FramePreview>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for FramePreview {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl FramePreview {
    /// Shows frame (`row`, `column`) of a sheet of `rows × columns` frames.
    pub fn show(&self, sheet: &gdk::Texture, rows: u32, columns: u32, row: u32, column: u32) {
        let imp = self.imp();
        *imp.sheet.borrow_mut() = Some(sheet.clone());
        imp.grid.set((rows, columns));
        imp.frame.set((row, column));
        self.queue_draw();
    }

    pub fn clear(&self) {
        self.imp().sheet.borrow_mut().take();
        self.queue_draw();
    }
}
