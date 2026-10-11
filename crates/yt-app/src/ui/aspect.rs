//! A container that is always a fixed shape: given a width, it asks for
//! `width × ratio` height. Thumbnails use it so cards keep their 16:9 shape
//! even before the image has loaded; the watch page uses it for the player,
//! with a maximum height so the player never gets taller than the view, and
//! a backdrop behind it (ambient mode's glow, see `ambient.rs`).

use std::cell::{Cell, RefCell};

use adw::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib, graphene};

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
        /// Never taller than at this width, if above 0.
        pub max_width: Cell<i32>,
        /// Drawn behind the child, past its edges (see `set_backdrop`), and
        /// the one fading out, with how far the change has got (0 to 1).
        pub backdrop: RefCell<Option<gdk::Texture>>,
        pub previous: RefCell<Option<gdk::Texture>>,
        pub fade: Cell<f64>,
        pub spread: Cell<f32>,
        pub opacity: Cell<f32>,
        pub animation: RefCell<Option<adw::TimedAnimation>>,
    }

    impl Default for AspectBin {
        fn default() -> Self {
            Self {
                ratio: Cell::new(9.0 / 16.0),
                max_height: Cell::new(0),
                fill: Cell::new(false),
                natural_width: Cell::new(320),
                max_width: Cell::new(0),
                backdrop: RefCell::new(None),
                previous: RefCell::new(None),
                fade: Cell::new(1.0),
                spread: Cell::new(1.0),
                opacity: Cell::new(1.0),
                animation: RefCell::new(None),
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
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let (width, height) = (self.obj().width() as f32, self.obj().height() as f32);
            if width > 0.0 && height > 0.0 {
                // The backdrop covers the bin spread out around its middle.
                let spread = self.spread.get();
                let (w, h) = (width * spread, height * spread);
                let bounds = graphene::Rect::new((width - w) / 2.0, (height - h) / 2.0, w, h);
                let fade = self.fade.get() as f32;
                let opacity = self.opacity.get();
                for (texture, alpha) in [
                    (self.previous.borrow().clone(), 1.0 - fade),
                    (self.backdrop.borrow().clone(), fade),
                ] {
                    if let Some(texture) = texture
                        && alpha > 0.0
                    {
                        snapshot.push_opacity(f64::from(alpha * opacity));
                        snapshot.append_texture(&texture, &bounds);
                        snapshot.pop();
                    }
                }
            }
            self.parent_snapshot(snapshot);
        }

        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            match orientation {
                gtk::Orientation::Horizontal => (0, self.natural_width.get(), -1, -1),
                _ => {
                    let mut width = if for_size >= 0 {
                        for_size
                    } else {
                        self.natural_width.get()
                    };
                    if self.max_width.get() > 0 {
                        width = width.min(self.max_width.get());
                    }
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

    /// An empty bin, for a child placed later with [`AspectBin::set_child`].
    pub fn empty(ratio: f64) -> Self {
        let bin: Self = glib::Object::new();
        bin.imp().ratio.set(ratio);
        bin
    }

    pub fn child(&self) -> Option<gtk::Widget> {
        self.first_child()
    }

    /// Replaces the child (the player moves between bins this way).
    pub fn set_child(&self, child: Option<&impl IsA<gtk::Widget>>) {
        while let Some(old) = self.first_child() {
            old.unparent();
        }
        if let Some(child) = child {
            child.set_parent(self);
        }
        self.queue_resize();
    }

    /// The width asked for when there is room (320 by default).
    pub fn set_natural_width(&self, width: i32) {
        self.imp().natural_width.set(width);
        self.queue_resize();
    }

    /// Asks for no more height than at `width` (0 for no cap): for a bin
    /// that is never wider than that (a thumbnail beside text), so being
    /// measured wider, as a box does while sharing out room, doesn't make
    /// it ask for more height than its width gives. Without it the box's
    /// minimum and natural heights disagree and GTK complains every frame.
    pub fn set_max_width(&self, width: i32) {
        if self.imp().max_width.replace(width) != width {
            self.queue_resize();
        }
    }

    pub fn has_backdrop(&self) -> bool {
        self.imp().backdrop.borrow().is_some()
    }

    /// Draws `texture` behind the child, `spread` times its size around its
    /// middle and at `opacity`, crossfading from the last one over a second;
    /// `None` fades it out.
    pub fn set_backdrop(&self, texture: Option<gdk::Texture>, spread: f32, opacity: f32) {
        let imp = self.imp();
        if imp.backdrop.borrow().is_none() && texture.is_none() {
            return;
        }
        imp.spread.set(spread);
        imp.opacity.set(opacity);
        let last = imp.backdrop.replace(texture);
        *imp.previous.borrow_mut() = last;
        imp.fade.set(0.0);
        let weak = self.downgrade();
        let target = adw::CallbackAnimationTarget::new(move |value| {
            if let Some(bin) = weak.upgrade() {
                bin.imp().fade.set(value);
                if value >= 1.0 {
                    bin.imp().previous.borrow_mut().take();
                }
                bin.queue_draw();
            }
        });
        let animation = adw::TimedAnimation::new(self, 0.0, 1.0, 1000, target);
        animation.play();
        *imp.animation.borrow_mut() = Some(animation);
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
