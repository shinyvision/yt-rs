use std::cell::{Cell, RefCell};

use gtk::gdk;

/// The app's CSS. The built-in theme loads first; CSS added later (e.g. by
/// plugins) takes precedence over it.
#[derive(Default)]
pub struct Theme {
    providers: RefCell<Vec<(u64, gtk::CssProvider)>>,
    next_id: Cell<u64>,
}

impl Theme {
    /// Adds a stylesheet and returns an id to remove it with.
    pub fn add_css(&self, css: &str) -> u64 {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let provider = gtk::CssProvider::new();
        // Mistakes in a stylesheet (a plugin's, say) are said, not skipped
        // in silence.
        provider.connect_parsing_error(|_, section, error| {
            tracing::warn!("CSS {}: {error}", section.to_str());
        });
        provider.load_from_string(css);
        if let Some(display) = gdk::Display::default() {
            // Each stylesheet one step above the previous one.
            let priority = gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + id as u32;
            gtk::style_context_add_provider_for_display(&display, &provider, priority);
        }
        self.providers.borrow_mut().push((id, provider));
        id
    }

    pub fn remove(&self, id: u64) {
        let mut providers = self.providers.borrow_mut();
        if let Some(at) = providers.iter().position(|(i, _)| *i == id) {
            let (_, provider) = providers.remove(at);
            if let Some(display) = gdk::Display::default() {
                gtk::style_context_remove_provider_for_display(&display, &provider);
            }
        }
    }
}
