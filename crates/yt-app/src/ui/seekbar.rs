//! The seek bar: played, buffered and remaining time, split into the video's
//! chapters, scrubbed by clicking or dragging. While dragging it seeks to keyframes, which mpv does almost
//! instantly, and seeks exactly when released.
//!
//! Plugins mark parts of the video on it ([`set_ranges`]: SponsorBlock's
//! segments, say), in their own colors, over the played part; a range too
//! short to see is a mark. They are cleared when another video loads.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

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
        pub ranges: RefCell<Vec<super::Range>>,
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
            // Marked parts, over the rest; a moment is a mark 3 px wide.
            for range in self.ranges.borrow().iter() {
                let start = fraction(range.start) * width;
                let end = (fraction(range.end) * width).max(start + 3.0);
                snapshot.append_color(
                    &range.color,
                    &graphene::Rect::new(start, y, end - start, bar),
                );
            }
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

/// A marked part of the video, from `start` to `end` seconds.
#[derive(Clone, Debug)]
pub struct Range {
    pub start: f64,
    pub end: f64,
    pub color: gtk::gdk::RGBA,
    /// Said while the pointer is over it ("Sponsor").
    pub label: Option<String>,
}

thread_local! {
    /// The marked parts, by who marked them.
    static RANGES: RefCell<BTreeMap<String, Vec<Range>>> = RefCell::default();
}

/// Marks parts of the video for `owner`, in place of its earlier ones, and
/// announces it (`seekbar.ranges`).
pub fn set_ranges(owner: &str, ranges: Vec<Range>) {
    RANGES.with(|r| {
        let mut all = r.borrow_mut();
        if ranges.is_empty() {
            all.remove(owner);
        } else {
            all.insert(owner.to_owned(), ranges);
        }
    });
    crate::app::app()
        .ext
        .events
        .emit("seekbar.ranges", &serde_json::json!({ "owner": owner }));
}

/// Everyone's marked parts.
pub fn ranges() -> Vec<Range> {
    RANGES.with(|r| r.borrow().values().flatten().cloned().collect())
}

/// Clears the marks when another video loads.
pub fn init() {
    crate::app::app().player.connect(|event, _| {
        if matches!(event, crate::player::PlayerEvent::Loading { .. })
            && RANGES.with(|r| !r.borrow().is_empty())
        {
            RANGES.with(|r| r.borrow_mut().clear());
            crate::app::app()
                .ext
                .events
                .emit("seekbar.ranges", &serde_json::json!({ "owner": null }));
        }
    });
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

    /// Shows these marked parts of the video.
    pub fn set_ranges(&self, ranges: Vec<Range>) {
        *self.imp().ranges.borrow_mut() = ranges;
        self.queue_draw();
    }

    /// The label of the marked part at `time`, if any.
    pub fn label_at(&self, time: f64) -> Option<String> {
        self.imp()
            .ranges
            .borrow()
            .iter()
            .find(|r| time >= r.start && time < r.end.max(r.start + 1.0))
            .and_then(|r| r.label.clone())
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
