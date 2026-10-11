//! The pointing hand over everything clickable, as on youtube.com. Every
//! button, and so every chip, tab, menu button and sidebar entry, plugins'
//! buttons included, gets it when it is realized, unless it was given a
//! cursor of its own. Links in text get it from GTK already.

use gtk::glib::{self, gobject_ffi, translate::*};
use gtk::prelude::*;

pub fn install() {
    // SAFETY: adds a C emission hook to GtkWidget::realize, which exists for
    // the whole process; the hook only looks at the widget being realized.
    unsafe {
        let realize = gobject_ffi::g_signal_lookup(
            c"realize".as_ptr(),
            gtk::Widget::static_type().into_glib(),
        );
        gobject_ffi::g_signal_add_emission_hook(
            realize,
            0,
            Some(on_realize),
            std::ptr::null_mut(),
            None,
        );
    }
}

unsafe extern "C" fn on_realize(
    _hint: *mut gobject_ffi::GSignalInvocationHint,
    n_values: u32,
    values: *const gobject_ffi::GValue,
    _data: glib::ffi::gpointer,
) -> glib::ffi::gboolean {
    if n_values > 0 {
        // SAFETY: a signal's first value is the instance emitting it, here
        // a GtkWidget, which outlives the emission.
        let widget: Borrowed<gtk::Widget> = unsafe {
            from_glib_borrow(gobject_ffi::g_value_get_object(values) as *mut gtk::ffi::GtkWidget)
        };
        if widget.is::<gtk::Button>() && widget.cursor().is_none() {
            widget.set_cursor_from_name(Some("pointer"));
        }
    }
    // Keep the hook.
    glib::ffi::GTRUE
}
