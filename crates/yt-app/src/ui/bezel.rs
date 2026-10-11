//! The bezel: the round icon in the middle of the video that confirms what
//! just happened (play, pause, seek, volume), as on youtube.com. It appears,
//! grows to twice its size and fades out in half a second.

use std::cell::{Cell, RefCell};

use adw::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene, gsk};

/// The circle's size before it grows, and the icon's.
const SIZE: f32 = 52.0;
const ICON: f32 = 36.0;
const DURATION_MS: u32 = 500;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Bezel {
        /// From 0 (just shown) to 1 (gone).
        pub progress: Cell<f64>,
        pub icon: RefCell<Option<gtk::IconPaintable>>,
        pub animation: RefCell<Option<adw::TimedAnimation>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Bezel {
        const NAME: &'static str = "YtBezel";
        type Type = super::Bezel;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("bezel");
        }
    }

    impl ObjectImpl for Bezel {
        fn constructed(&self) {
            self.parent_constructed();
            self.progress.set(1.0);
            let obj = self.obj();
            obj.set_can_target(false);
            obj.set_halign(gtk::Align::Center);
            obj.set_valign(gtk::Align::Center);
        }
    }

    impl WidgetImpl for Bezel {
        fn measure(&self, _orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            // Room for the circle at twice its size.
            let size = (SIZE * 2.0) as i32;
            (size, size, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let progress = self.progress.get() as f32;
            if progress >= 1.0 {
                return;
            }
            let Some(icon) = self.icon.borrow().clone() else {
                return;
            };
            let obj = self.obj();
            let (cx, cy) = (obj.width() as f32 / 2.0, obj.height() as f32 / 2.0);
            let scale = 1.0 + progress;
            let alpha = 1.0 - progress;
            let size = SIZE * scale;
            let circle = graphene::Rect::new(cx - size / 2.0, cy - size / 2.0, size, size);
            snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(circle, size / 2.0));
            snapshot.append_color(&gdk::RGBA::new(0.0, 0.0, 0.0, 0.5 * alpha), &circle);
            snapshot.pop();
            let icon_size = ICON * scale;
            snapshot.save();
            snapshot.translate(&graphene::Point::new(
                cx - icon_size / 2.0,
                cy - icon_size / 2.0,
            ));
            icon.snapshot_symbolic(
                snapshot,
                icon_size as f64,
                icon_size as f64,
                &[gdk::RGBA::new(1.0, 1.0, 1.0, alpha)],
            );
            snapshot.restore();
        }
    }
}

glib::wrapper! {
    pub struct Bezel(ObjectSubclass<imp::Bezel>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Bezel {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl Bezel {
    /// Shows `icon` (a symbolic icon name) and lets it grow and fade.
    pub fn flash(&self, icon: &str) {
        let imp = self.imp();
        let paintable = gtk::IconTheme::for_display(&self.display()).lookup_icon(
            icon,
            &[],
            (ICON * 2.0) as i32,
            self.scale_factor(),
            gtk::TextDirection::None,
            gtk::IconLookupFlags::empty(),
        );
        *imp.icon.borrow_mut() = Some(paintable);
        if let Some(old) = imp.animation.borrow_mut().take() {
            old.skip();
        }
        let weak = self.downgrade();
        let target = adw::CallbackAnimationTarget::new(move |value| {
            if let Some(bezel) = weak.upgrade() {
                bezel.imp().progress.set(value);
                bezel.queue_draw();
            }
        });
        let animation = adw::TimedAnimation::builder()
            .widget(self)
            .value_from(0.0)
            .value_to(1.0)
            .duration(DURATION_MS)
            .easing(adw::Easing::EaseOutCubic)
            .target(&target)
            .build();
        animation.play();
        *imp.animation.borrow_mut() = Some(animation);
    }
}
