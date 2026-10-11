//! Draws mpv's video into an OpenGL framebuffer through libmpv's render API
//! (mpv/render.h and mpv/render_gl.h).

use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr::NonNull;

use super::mpv::{Mpv, ffi::mpv_handle};

#[allow(non_camel_case_types)]
mod ffi {
    use std::ffi::{c_char, c_int, c_void};

    use super::mpv_handle;

    pub enum mpv_render_context {}

    pub const PARAM_INVALID: c_int = 0;
    pub const PARAM_API_TYPE: c_int = 1;
    pub const PARAM_OPENGL_INIT_PARAMS: c_int = 2;
    pub const PARAM_OPENGL_FBO: c_int = 3;
    pub const PARAM_FLIP_Y: c_int = 4;
    pub const PARAM_WL_DISPLAY: c_int = 9;
    pub const PARAM_ADVANCED_CONTROL: c_int = 10;

    #[repr(C)]
    pub struct mpv_render_param {
        pub kind: c_int,
        pub data: *mut c_void,
    }

    #[repr(C)]
    pub struct mpv_opengl_init_params {
        pub get_proc_address:
            Option<unsafe extern "C" fn(ctx: *mut c_void, name: *const c_char) -> *mut c_void>,
        pub get_proc_address_ctx: *mut c_void,
    }

    #[repr(C)]
    pub struct mpv_opengl_fbo {
        pub fbo: c_int,
        pub w: c_int,
        pub h: c_int,
        pub internal_format: c_int,
    }

    #[link(name = "mpv")]
    unsafe extern "C" {
        pub fn mpv_render_context_create(
            res: *mut *mut mpv_render_context,
            mpv: *mut mpv_handle,
            params: *mut mpv_render_param,
        ) -> c_int;
        pub fn mpv_render_context_set_update_callback(
            ctx: *mut mpv_render_context,
            cb: Option<unsafe extern "C" fn(*mut c_void)>,
            data: *mut c_void,
        );
        pub fn mpv_render_context_update(ctx: *mut mpv_render_context) -> u64;
        pub fn mpv_render_context_render(
            ctx: *mut mpv_render_context,
            params: *mut mpv_render_param,
        ) -> c_int;
        pub fn mpv_render_context_free(ctx: *mut mpv_render_context);
    }
}

const GL_DRAW_FRAMEBUFFER_BINDING: u32 = 0x8CA6;

/// Looks up OpenGL functions for mpv. GTK uses EGL on Wayland (and on X11
/// in recent versions), so `eglGetProcAddress` comes first; `dlsym` on the
/// GL library is the fallback.
struct GlLoader {
    egl_get_proc_address: Option<unsafe extern "C" fn(*const c_char) -> *mut c_void>,
    _egl: Option<libloading::Library>,
    gl: Option<libloading::Library>,
}

impl GlLoader {
    fn new() -> Self {
        let egl = unsafe { libloading::Library::new("libEGL.so.1") }.ok();
        let egl_get_proc_address = egl.as_ref().and_then(|lib| unsafe {
            lib.get::<unsafe extern "C" fn(*const c_char) -> *mut c_void>(b"eglGetProcAddress\0")
                .ok()
                .map(|f| *f)
        });
        let gl = ["libGL.so.1", "libOpenGL.so.0"]
            .iter()
            .find_map(|name| unsafe { libloading::Library::new(*name) }.ok());
        Self {
            egl_get_proc_address,
            _egl: egl,
            gl,
        }
    }

    fn get(&self, name: &CStr) -> *mut c_void {
        if let Some(f) = self.egl_get_proc_address {
            let ptr = unsafe { f(name.as_ptr()) };
            if !ptr.is_null() {
                return ptr;
            }
        }
        self.gl
            .as_ref()
            .and_then(|lib| unsafe {
                lib.get::<*mut c_void>(name.to_bytes_with_nul())
                    .ok()
                    .map(|s| *s)
            })
            .unwrap_or(std::ptr::null_mut())
    }
}

unsafe extern "C" fn get_proc_address(ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    // SAFETY: `ctx` is the boxed loader owned by the render context.
    let loader = unsafe { &*(ctx as *const GlLoader) };
    loader.get(unsafe { CStr::from_ptr(name) })
}

type Callback = Box<dyn Fn() + Send + Sync>;

/// mpv's renderer for one OpenGL context. Create, use and drop it only while
/// that context is current.
pub struct RenderContext {
    ctx: NonNull<ffi::mpv_render_context>,
    /// Only kept alive: mpv calls into it through a raw pointer.
    _loader: Box<GlLoader>,
    update_callback: Option<Box<Callback>>,
    get_integerv: Option<unsafe extern "C" fn(u32, *mut c_int)>,
}

impl RenderContext {
    /// # Safety
    /// An OpenGL context must be current, and `wl_display` must be null or
    /// the Wayland display GTK uses.
    pub unsafe fn new(mpv: &Mpv, wl_display: *mut c_void) -> Result<Self, String> {
        let loader = Box::new(GlLoader::new());
        let mut init = ffi::mpv_opengl_init_params {
            get_proc_address: Some(get_proc_address),
            get_proc_address_ctx: (&*loader as *const GlLoader).cast_mut().cast(),
        };
        let mut advanced: c_int = 1;
        let mut params = vec![
            ffi::mpv_render_param {
                kind: ffi::PARAM_API_TYPE,
                data: c"opengl".as_ptr().cast_mut().cast(),
            },
            ffi::mpv_render_param {
                kind: ffi::PARAM_OPENGL_INIT_PARAMS,
                data: (&raw mut init).cast(),
            },
            ffi::mpv_render_param {
                kind: ffi::PARAM_ADVANCED_CONTROL,
                data: (&raw mut advanced).cast(),
            },
        ];
        if !wl_display.is_null() {
            // Lets mpv share hardware-decoded frames with GL (VA-API).
            params.push(ffi::mpv_render_param {
                kind: ffi::PARAM_WL_DISPLAY,
                data: wl_display,
            });
        }
        params.push(ffi::mpv_render_param {
            kind: ffi::PARAM_INVALID,
            data: std::ptr::null_mut(),
        });

        let mut ctx = std::ptr::null_mut();
        let code =
            unsafe { ffi::mpv_render_context_create(&mut ctx, mpv.raw(), params.as_mut_ptr()) };
        let ctx = NonNull::new(ctx)
            .filter(|_| code >= 0)
            .ok_or_else(|| format!("mpv could not create a renderer (error {code})"))?;
        let get_integerv = {
            let ptr = loader.get(c"glGetIntegerv");
            (!ptr.is_null()).then(|| unsafe {
                std::mem::transmute::<*mut c_void, unsafe extern "C" fn(u32, *mut c_int)>(ptr)
            })
        };
        Ok(Self {
            ctx,
            _loader: loader,
            update_callback: None,
            get_integerv,
        })
    }

    /// Calls `callback` (on an mpv thread) when a new frame should be drawn.
    pub fn set_update_callback(&mut self, callback: impl Fn() + Send + Sync + 'static) {
        unsafe extern "C" fn trampoline(data: *mut c_void) {
            let callback = unsafe { &*(data as *const Callback) };
            callback();
        }
        let boxed: Box<Callback> = Box::new(Box::new(callback));
        let data = (&*boxed as *const Callback).cast_mut().cast();
        unsafe {
            ffi::mpv_render_context_set_update_callback(self.ctx.as_ptr(), Some(trampoline), data)
        };
        self.update_callback = Some(boxed);
    }

    /// Must be called after the update callback fired, before rendering.
    pub fn update(&self) -> u64 {
        unsafe { ffi::mpv_render_context_update(self.ctx.as_ptr()) }
    }

    /// Draws the current frame into the framebuffer that is bound for
    /// drawing, which is `width`×`height` pixels.
    pub fn render(&self, width: i32, height: i32) {
        let mut fbo: c_int = 0;
        if let Some(get_integerv) = self.get_integerv {
            unsafe { get_integerv(GL_DRAW_FRAMEBUFFER_BINDING, &mut fbo) };
        }
        let mut target = ffi::mpv_opengl_fbo {
            fbo,
            w: width,
            h: height,
            internal_format: 0,
        };
        let mut flip: c_int = 1;
        let mut params = [
            ffi::mpv_render_param {
                kind: ffi::PARAM_OPENGL_FBO,
                data: (&raw mut target).cast(),
            },
            ffi::mpv_render_param {
                kind: ffi::PARAM_FLIP_Y,
                data: (&raw mut flip).cast(),
            },
            ffi::mpv_render_param {
                kind: ffi::PARAM_INVALID,
                data: std::ptr::null_mut(),
            },
        ];
        unsafe { ffi::mpv_render_context_render(self.ctx.as_ptr(), params.as_mut_ptr()) };
    }
}

impl Drop for RenderContext {
    fn drop(&mut self) {
        unsafe {
            ffi::mpv_render_context_set_update_callback(
                self.ctx.as_ptr(),
                None,
                std::ptr::null_mut(),
            );
            ffi::mpv_render_context_free(self.ctx.as_ptr());
        }
        // `_loader` is dropped after this, so it outlives the context, which
        // may call into it until freed.
    }
}
