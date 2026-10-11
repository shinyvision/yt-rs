//! The few OpenGL functions the video needs, looked up at run time in the
//! GL that GTK uses (so there is no GL library to link, and no GL crate),
//! and the framebuffers mpv draws video frames into.

use std::ffi::{CStr, c_char, c_int, c_void};

/// Looks up OpenGL functions. GTK uses EGL on Wayland (and on X11 in recent
/// versions), so `eglGetProcAddress` comes first; `dlsym` on the GL library
/// is the fallback.
pub struct GlLoader {
    egl_get_proc_address: Option<unsafe extern "C" fn(*const c_char) -> *mut c_void>,
    _egl: Option<libloading::Library>,
    gl: Option<libloading::Library>,
}

impl GlLoader {
    pub fn new() -> Self {
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

    pub fn get(&self, name: &CStr) -> *mut c_void {
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

pub type GLsync = *const c_void;

const TEXTURE_2D: u32 = 0x0DE1;
const TEXTURE_MIN_FILTER: u32 = 0x2801;
const TEXTURE_MAG_FILTER: u32 = 0x2800;
const TEXTURE_WRAP_S: u32 = 0x2802;
const TEXTURE_WRAP_T: u32 = 0x2803;
const LINEAR: c_int = 0x2601;
const CLAMP_TO_EDGE: c_int = 0x812F;
const RGBA: u32 = 0x1908;
pub const RGBA8: c_int = 0x8058;
const UNSIGNED_BYTE: u32 = 0x1401;
const FRAMEBUFFER: u32 = 0x8D40;
const COLOR_ATTACHMENT0: u32 = 0x8CE0;
const FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;
const SYNC_GPU_COMMANDS_COMPLETE: u32 = 0x9117;

macro_rules! functions {
    ($($field:ident = $name:literal: fn($($arg:ty),*) $(-> $ret:ty)?;)*) => {
        /// OpenGL functions, for the context that was current when loaded
        /// (or one sharing with it).
        pub struct Gl {
            $($field: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
        }

        impl Gl {
            /// `None` when the GL lacks one of them.
            pub fn load(loader: &GlLoader) -> Option<Self> {
                Some(Self {
                    $($field: {
                        let ptr = loader.get($name);
                        if ptr.is_null() {
                            return None;
                        }
                        // SAFETY: GL's own function of this name and type.
                        unsafe {
                            std::mem::transmute::<*mut c_void, unsafe extern "C" fn($($arg),*) $(-> $ret)?>(ptr)
                        }
                    },)*
                })
            }
        }
    };
}

functions! {
    gen_textures = c"glGenTextures": fn(c_int, *mut u32);
    delete_textures = c"glDeleteTextures": fn(c_int, *const u32);
    bind_texture = c"glBindTexture": fn(u32, u32);
    tex_parameteri = c"glTexParameteri": fn(u32, u32, c_int);
    tex_image_2d = c"glTexImage2D": fn(u32, c_int, c_int, c_int, c_int, c_int, u32, u32, *const c_void);
    gen_framebuffers = c"glGenFramebuffers": fn(c_int, *mut u32);
    delete_framebuffers = c"glDeleteFramebuffers": fn(c_int, *const u32);
    bind_framebuffer = c"glBindFramebuffer": fn(u32, u32);
    framebuffer_texture_2d = c"glFramebufferTexture2D": fn(u32, u32, u32, u32, c_int);
    check_framebuffer_status = c"glCheckFramebufferStatus": fn(u32) -> u32;
    fence_sync = c"glFenceSync": fn(u32, u32) -> GLsync;
    delete_sync = c"glDeleteSync": fn(GLsync);
    flush = c"glFlush": fn();
}

/// A texture to draw into, `width`×`height` pixels of RGBA. Use it only
/// while its context is current, and [`Framebuffer::delete`] it there.
pub struct Framebuffer {
    pub fbo: u32,
    pub texture: u32,
    pub width: i32,
    pub height: i32,
    /// Signals when the last drawing into it is done.
    sync: GLsync,
}

impl Framebuffer {
    pub fn new(gl: &Gl, width: i32, height: i32) -> Result<Self, String> {
        let (mut texture, mut fbo) = (0, 0);
        unsafe {
            (gl.gen_textures)(1, &mut texture);
            (gl.gen_framebuffers)(1, &mut fbo);
        }
        let mut buffer = Self {
            fbo,
            texture,
            width: 0,
            height: 0,
            sync: std::ptr::null(),
        };
        unsafe {
            (gl.bind_texture)(TEXTURE_2D, texture);
            for (name, value) in [
                (TEXTURE_MIN_FILTER, LINEAR),
                (TEXTURE_MAG_FILTER, LINEAR),
                (TEXTURE_WRAP_S, CLAMP_TO_EDGE),
                (TEXTURE_WRAP_T, CLAMP_TO_EDGE),
            ] {
                (gl.tex_parameteri)(TEXTURE_2D, name, value);
            }
        }
        match buffer.resize(gl, width, height) {
            Ok(()) => Ok(buffer),
            Err(e) => {
                buffer.delete(gl);
                Err(e)
            }
        }
    }

    /// Makes it `width`×`height`; what it held is gone if that's a change.
    pub fn resize(&mut self, gl: &Gl, width: i32, height: i32) -> Result<(), String> {
        if (self.width, self.height) == (width, height) {
            return Ok(());
        }
        let status = unsafe {
            (gl.bind_texture)(TEXTURE_2D, self.texture);
            (gl.tex_image_2d)(
                TEXTURE_2D,
                0,
                RGBA8,
                width,
                height,
                0,
                RGBA,
                UNSIGNED_BYTE,
                std::ptr::null(),
            );
            (gl.bind_texture)(TEXTURE_2D, 0);
            (gl.bind_framebuffer)(FRAMEBUFFER, self.fbo);
            (gl.framebuffer_texture_2d)(
                FRAMEBUFFER,
                COLOR_ATTACHMENT0,
                TEXTURE_2D,
                self.texture,
                0,
            );
            let status = (gl.check_framebuffer_status)(FRAMEBUFFER);
            (gl.bind_framebuffer)(FRAMEBUFFER, 0);
            status
        };
        if status != FRAMEBUFFER_COMPLETE {
            self.width = 0;
            return Err(format!("OpenGL framebuffer incomplete ({status:#x})"));
        }
        (self.width, self.height) = (width, height);
        Ok(())
    }

    /// After drawing into it: what signals that the drawing is done, valid
    /// until the next call.
    pub fn fence(&mut self, gl: &Gl) -> GLsync {
        unsafe {
            if !self.sync.is_null() {
                (gl.delete_sync)(self.sync);
            }
            self.sync = (gl.fence_sync)(SYNC_GPU_COMMANDS_COMPLETE, 0);
            (gl.flush)();
        }
        self.sync
    }

    pub fn delete(self, gl: &Gl) {
        unsafe {
            if !self.sync.is_null() {
                (gl.delete_sync)(self.sync);
            }
            (gl.delete_framebuffers)(1, &self.fbo);
            (gl.delete_textures)(1, &self.texture);
        }
    }
}
