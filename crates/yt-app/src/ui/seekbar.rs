//! The seek bar: played, buffered and remaining time, split into the video's
//! chapters, scrubbed by clicking or dragging. While dragging it seeks to keyframes, which mpv does almost
//! instantly, and seeks exactly when released.

use std::cell::{Cell, RefCell};

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{glib, graphene, gsk};

type SeekCallback = Box<dyn Fn(f64, bool)>;
type HoverCallback = Box<dyn Fn(Option<(f64, f64)>)>;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct SeekBar {
        pub position: Cell<f64>,
        pub duration: Cell<f64>,
        pub buffered: Cell<f64>,
        pub hover: Cell<Option<f64>>,
        /// Where chapters start, in seconds.
        pub chapters: RefCell<Vec<f64>>,
        pub drag_start: Cell<Option<f64>>,
        pub on_seek: RefCell<Vec<SeekCallback>>,
        pub on_hover: RefCell<Vec<HoverCallback>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SeekBar {
        const NAME: &'static str = "YtSeekBar";
        type Type = super::SeekBar;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("seekbar");
        }
    }

    impl ObjectImpl for SeekBar {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.add_css_class("seekbar");
            obj.set_hexpand(true);
            obj.set_cursor_from_name(Some("pointer"));
            obj.connect_input();
        }
    }

    impl WidgetImpl for SeekBar {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            match orientation {
                gtk::Orientation::Horizontal => (60, 400, -1, -1),
                _ => (18, 18, -1, -1),
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let obj = self.obj();
            let (width, height) = (obj.width() as f32, obj.height() as f32);
            let duration = self.duration.get();
            let fraction = |t: f64| {
                if duration > 0.0 {
                    (t / duration).clamp(0.0, 1.0) as f32
                } else {
                    0.0
                }
            };
            let active = self.hover.get().is_some() || self.drag_start.get().is_some();
            let bar = if active { 6.0 } else { 4.0 };
            let y = (height - bar) / 2.0;
            let accent = obj.color();
            let white = |alpha: f32| gtk::gdk::RGBA::new(1.0, 1.0, 1.0, alpha);

            let rounded = |x: f32, w: f32| {
                gsk::RoundedRect::from_rect(graphene::Rect::new(x, y, w.max(0.0), bar), bar / 2.0)
            };
            // Chapters are segments with a small gap between them.
            let mut edges: Vec<f32> = self
                .chapters
                .borrow()
                .iter()
                .map(|t| fraction(*t) * width)
                .filter(|x| *x > 2.0 && *x < width - 2.0)
                .collect();
            edges.insert(0, 0.0);
            edges.push(width);
            let segments: Vec<(f32, f32)> = edges
                .windows(2)
                .enumerate()
                .map(|(i, e)| {
                    let start = if i == 0 { e[0] } else { e[0] + 1.0 };
                    let end = if i == edges.len() - 2 {
                        e[1]
                    } else {
                        e[1] - 1.0
                    };
                    (start, end)
                })
                .collect();
            let fill = |to: f32, color: &gtk::gdk::RGBA| {
                for (start, end) in &segments {
                    let end = end.min(to);
                    if end > *start {
                        snapshot
                            .append_color(color, &graphene::Rect::new(*start, y, end - start, bar));
                    }
                }
            };
            snapshot.push_rounded_clip(&rounded(0.0, width));
            fill(width, &white(0.25));
            fill(
                fraction(self.position.get() + self.buffered.get()) * width,
                &white(0.45),
            );
            if let Some(hover) = self.hover.get() {
                fill(hover as f32, &white(0.55));
            }
            let played = fraction(self.position.get()) * width;
            fill(played, &accent);
            snapshot.pop();

            if active {
                let r = 7.0;
                let knob = graphene::Rect::new(played - r, height / 2.0 - r, 2.0 * r, 2.0 * r);
                snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(knob, r));
                snapshot.append_color(&accent, &knob);
                snapshot.pop();
            }
        }
    }
}

glib::wrapper! {
    pub struct SeekBar(ObjectSubclass<imp::SeekBar>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for SeekBar {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl SeekBar {
    /// Shows the playback state. The position is left alone while the user
    /// drags, so the knob doesn't jump back.
    pub fn set_state(&self, position: f64, duration: f64, buffered: f64) {
        let imp = self.imp();
        if imp.drag_start.get().is_none() {
            imp.position.set(position);
        }
        imp.duration.set(duration);
        imp.buffered.set(buffered);
        self.queue_draw();
    }

    /// Splits the bar into chapters starting at these times (seconds).
    pub fn set_chapters(&self, starts: Vec<f64>) {
        *self.imp().chapters.borrow_mut() = starts;
        self.queue_draw();
    }

    /// `f(position, exact)` runs when the user seeks.
    pub fn connect_seek(&self, f: impl Fn(f64, bool) + 'static) {
        self.imp().on_seek.borrow_mut().push(Box::new(f));
    }

    /// `f(Some((x, time)))` runs while the pointer is over the bar, and
    /// `f(None)` when it leaves.
    pub fn connect_hover(&self, f: impl Fn(Option<(f64, f64)>) + 'static) {
        self.imp().on_hover.borrow_mut().push(Box::new(f));
    }

    fn time_at(&self, x: f64) -> f64 {
        let width = self.width().max(1) as f64;
        (x / width).clamp(0.0, 1.0) * self.imp().duration.get()
    }

    fn seek_to(&self, x: f64, exact: bool) {
        let imp = self.imp();
        if imp.duration.get() <= 0.0 {
            return;
        }
        let time = self.time_at(x);
        imp.position.set(time);
        self.queue_draw();
        for f in imp.on_seek.borrow().iter() {
            f(time, exact);
        }
    }

    fn hovered(&self, x: Option<f64>) {
        let imp = self.imp();
        imp.hover.set(x);
        self.queue_draw();
        let info = x
            .filter(|_| imp.duration.get() > 0.0)
            .map(|x| (x, self.time_at(x)));
        for f in imp.on_hover.borrow().iter() {
            f(info);
        }
    }

    fn connect_input(&self) {
        let drag = gtk::GestureDrag::new();
        drag.connect_drag_begin(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            move |_, x, _| {
                bar.imp().drag_start.set(Some(x));
                bar.seek_to(x, false);
            }
        ));
        drag.connect_drag_update(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            move |_, dx, _| {
                if let Some(start) = bar.imp().drag_start.get() {
                    bar.seek_to(start + dx, false);
                    bar.hovered(Some(start + dx));
                }
            }
        ));
        drag.connect_drag_end(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            move |_, dx, _| {
                if let Some(start) = bar.imp().drag_start.take() {
                    bar.seek_to(start + dx, true);
                }
            }
        ));
        self.add_controller(drag);

        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            move |_, x, _| bar.hovered(Some(x))
        ));
        motion.connect_leave(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            move |_| {
                if bar.imp().drag_start.get().is_none() {
                    bar.hovered(None);
                }
            }
        ));
        self.add_controller(motion);
    }
}
