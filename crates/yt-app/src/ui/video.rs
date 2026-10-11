//! The video picture. mpv draws each frame into a texture of an OpenGL
//! context that lives as long as the widget, and the widget shows the
//! latest one. A `GtkGLArea` would do the drawing itself, but its context
//! goes whenever it leaves the window, taking mpv's renderer with it, and
//! with that renderer mpv turns the video off. This widget can move between
//! the watch page, the theater row and the mini player as it likes.
//!
//! Frames are drawn at the widget's size in device pixels (mpv scales the
//! video better than a texture filter would), into a few framebuffers taken
//! in turn: GTK may still be showing one while mpv draws the next.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gtk::glib;
use gtk::glib::subclass::prelude::*;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, graphene};

use crate::player::gl::{Framebuffer, Gl, GlLoader};
use crate::player::mpv::Mpv;
use crate::player::render::RenderContext;

type ErrorHandler = Box<dyn Fn(&str)>;

/// Framebuffers in turn: one shown, one GTK may still be done with, one to
/// draw into.
const FRAMES: usize = 3;

struct Frame {
    buffer: Framebuffer,
    /// GTK is done with its texture.
    free: Arc<AtomicBool>,
}

/// The OpenGL side: the context, mpv's renderer in it, the framebuffers.
struct Output {
    context: gdk::GLContext,
    gl: Gl,
    frames: Vec<Frame>,
    /// The latest frame, and its size in device pixels.
    shown: Option<(gdk::Texture, (i32, i32))>,
    renderer: RenderContext,
}

impl Output {
    fn new(widget: &VideoView, mpv: &Mpv) -> Result<Self, String> {
        let context = widget
            .display()
            .create_gl_context()
            .and_then(|context| context.realize().map(|()| context))
            .map_err(|e| e.message().to_owned())?;
        context.make_current();
        let gl = Gl::load(&GlLoader::new()).ok_or("OpenGL lacks framebuffers")?;
        // SAFETY: the context is current, and the display is GTK's.
        let mut renderer = unsafe { RenderContext::new(mpv, wayland_display(widget))? };
        // mpv calls this from its render thread; draw on the main loop.
        let target = glib::SendWeakRef::from(widget.downgrade());
        let pending = Arc::new(AtomicBool::new(false));
        renderer.set_update_callback(move || {
            if !pending.swap(true, Ordering::AcqRel) {
                let (target, pending) = (target.clone(), pending.clone());
                glib::MainContext::default().invoke(move || {
                    pending.store(false, Ordering::Release);
                    if let Some(widget) = target.upgrade() {
                        widget.imp().new_frame.set(true);
                        widget.queue_draw();
                    }
                });
            }
        });
        Ok(Self {
            context,
            gl,
            frames: Vec::with_capacity(FRAMES),
            shown: None,
            renderer,
        })
    }

    /// Draws the current video frame at `width`×`height` device pixels.
    fn render(&mut self, (width, height): (i32, i32)) -> Result<gdk::Texture, String> {
        self.context.make_current();
        self.renderer.update();
        let index = match self
            .frames
            .iter()
            .position(|f| f.free.load(Ordering::Acquire))
        {
            Some(index) => index,
            None if self.frames.len() < FRAMES => {
                self.frames.push(Frame {
                    buffer: Framebuffer::new(&self.gl, width, height)?,
                    free: Arc::new(AtomicBool::new(true)),
                });
                self.frames.len() - 1
            }
            None => return Err("every frame is still on screen".into()),
        };
        let frame = &mut self.frames[index];
        frame.buffer.resize(&self.gl, width, height)?;
        self.renderer.render(frame.buffer.fbo, width, height);
        let sync = frame.buffer.fence(&self.gl);
        frame.free.store(false, Ordering::Release);
        let free = frame.free.clone();
        // SAFETY: the texture stays as it is until GTK releases it: only
        // free frames are drawn into.
        Ok(unsafe {
            gdk::GLTextureBuilder::new()
                .set_context(Some(&self.context))
                .set_id(frame.buffer.texture)
                .set_width(width)
                .set_height(height)
                .set_format(gdk::MemoryFormat::R8g8b8a8Premultiplied)
                .set_sync(Some(sync))
                .build_with_release_func(move || free.store(true, Ordering::Release))
        })
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        self.context.make_current();
        self.shown = None;
        for frame in self.frames.drain(..) {
            frame.buffer.delete(&self.gl);
        }
        // `renderer` is dropped next, with the context still current.
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct VideoView {
        pub(super) output: RefCell<Option<Output>>,
        /// mpv has a new frame for us.
        pub(super) new_frame: Cell<bool>,
        pub(super) on_error: RefCell<Option<ErrorHandler>>,
        pub(super) failed: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VideoView {
        const NAME: &'static str = "YtVideoView";
        type Type = super::VideoView;
        type ParentType = gtk::Widget;

        fn class_init(class: &mut Self::Class) {
            class.set_css_name("video");
        }
    }

    impl ObjectImpl for VideoView {
        fn dispose(&self) {
            self.output.borrow_mut().take();
            gdk::GLContext::clear_current();
        }
    }

    impl WidgetImpl for VideoView {
        fn realize(&self) {
            self.parent_realize();
            self.obj().start();
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            self.obj().draw(snapshot);
        }
    }
}

glib::wrapper! {
    pub struct VideoView(ObjectSubclass<imp::VideoView>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl VideoView {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// The frame on screen, as drawn (mpv's picture at the widget's size).
    pub fn frame(&self) -> Option<gdk::Texture> {
        self.imp()
            .output
            .borrow()
            .as_ref()
            .and_then(|output| output.shown.as_ref())
            .map(|(texture, _)| texture.clone())
    }

    /// Calls `f` with what went wrong if the video can't be shown.
    pub fn connect_error(&self, f: impl Fn(&str) + 'static) {
        *self.imp().on_error.borrow_mut() = Some(Box::new(f));
    }

    /// Frees mpv's renderer and the OpenGL context: before mpv is destroyed,
    /// which aborts otherwise. The video stays black after this.
    pub fn stop(&self) {
        let imp = self.imp();
        imp.output.borrow_mut().take();
        gdk::GLContext::clear_current();
        imp.failed.set(true);
    }

    /// Sets up OpenGL and mpv's renderer now, rather than when the widget
    /// first shows: mpv needs it before it opens a video.
    pub fn prepare(&self) {
        self.start();
    }

    /// Sets up OpenGL and mpv's renderer, once.
    fn start(&self) {
        let imp = self.imp();
        if imp.output.borrow().is_some() || imp.failed.get() {
            return;
        }
        match Output::new(self, crate::app::app().player.mpv()) {
            Ok(output) => *imp.output.borrow_mut() = Some(output),
            Err(e) => {
                imp.failed.set(true);
                tracing::error!("video: {e}");
                if let Some(f) = imp.on_error.borrow().as_ref() {
                    f(&format!("OpenGL is unavailable: {e}"));
                }
            }
        }
    }

    fn draw(&self, snapshot: &gtk::Snapshot) {
        let (width, height) = (self.width() as f32, self.height() as f32);
        let bounds = graphene::Rect::new(0.0, 0.0, width, height);
        snapshot.append_color(&gdk::RGBA::BLACK, &bounds);
        let scale = self
            .native()
            .and_then(|native| native.surface())
            .map_or(f64::from(self.scale_factor()), |surface| surface.scale());
        let size = (
            (f64::from(width) * scale).ceil() as i32,
            (f64::from(height) * scale).ceil() as i32,
        );
        if size.0 <= 0 || size.1 <= 0 {
            return;
        }
        let imp = self.imp();
        let mut output = imp.output.borrow_mut();
        let Some(output) = output.as_mut() else {
            return;
        };
        let new_frame = imp.new_frame.replace(false);
        let resized = output.shown.as_ref().map(|(_, size)| *size) != Some(size);
        if new_frame || resized {
            match output.render(size) {
                Ok(texture) => output.shown = Some((texture, size)),
                Err(e) => tracing::warn!("video: {e}"),
            }
        }
        if let Some((texture, _)) = &output.shown {
            snapshot.append_texture(texture, &bounds);
        }
    }
}

impl Default for VideoView {
    fn default() -> Self {
        Self::new()
    }
}

/// GTK's Wayland display, which lets mpv hand hardware-decoded frames
/// straight to OpenGL. Null on other platforms.
#[cfg(target_os = "linux")]
fn wayland_display(widget: &impl IsA<gtk::Widget>) -> *mut c_void {
    use gtk::glib::translate::ToGlibPtr;
    let display = widget.display();
    match display.downcast_ref::<gdk_wayland::WaylandDisplay>() {
        // SAFETY: a valid GdkWaylandDisplay.
        Some(wayland) => unsafe {
            gdk_wayland::ffi::gdk_wayland_display_get_wl_display(wayland.to_glib_none().0).cast()
        },
        None => std::ptr::null_mut(),
    }
}

#[cfg(not(target_os = "linux"))]
fn wayland_display(_widget: &impl IsA<gtk::Widget>) -> *mut c_void {
    std::ptr::null_mut()
}
