//! Choices the app remembers between runs, as youtube.com does: volume,
//! captions, autoplay, theater mode and stable volume. Saved as
//! `settings.json` in the config directory, a moment after they change.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use serde::{Deserialize, Serialize};

use crate::runtime;

const SAVE_AFTER: Duration = Duration::from_millis(800);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub volume: f64,
    pub muted: bool,
    /// Play the next video when one ends.
    pub autoplay: bool,
    /// The wide player with the details and "Up next" below it.
    pub theater: bool,
    /// The captions language, while captions are on.
    pub captions: Option<String>,
    pub stable_volume: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            volume: 100.0,
            muted: false,
            autoplay: true,
            theater: false,
            captions: None,
            stable_volume: false,
        }
    }
}

pub struct SettingsStore {
    value: RefCell<Settings>,
    file: PathBuf,
    saving: Cell<Option<glib::SourceId>>,
}

impl SettingsStore {
    pub fn load() -> Rc<Self> {
        let file = yt_core::paths::config_dir().join("settings.json");
        let value = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Rc::new(Self {
            value: RefCell::new(value),
            file,
            saving: Cell::new(None),
        })
    }

    pub fn get(&self) -> Settings {
        self.value.borrow().clone()
    }

    /// Changes the settings and saves them soon after.
    pub fn update(self: &Rc<Self>, change: impl FnOnce(&mut Settings)) {
        let before = self.get();
        change(&mut self.value.borrow_mut());
        if *self.value.borrow() == before {
            return;
        }
        if let Some(pending) = self.saving.take() {
            pending.remove();
        }
        let this = Rc::downgrade(self);
        self.saving
            .set(Some(glib::timeout_add_local_once(SAVE_AFTER, move || {
                if let Some(this) = this.upgrade() {
                    this.saving.take();
                    this.save();
                }
            })));
    }

    /// Saves now if a save is pending, e.g. when the app quits.
    pub fn flush(&self) {
        if let Some(pending) = self.saving.take() {
            pending.remove();
            let data = serde_json::to_vec_pretty(&*self.value.borrow()).unwrap_or_default();
            if let Err(e) = std::fs::write(&self.file, data) {
                tracing::warn!("couldn't save the settings: {e}");
            }
        }
    }

    fn save(&self) {
        let (file, data) = (
            self.file.clone(),
            serde_json::to_vec_pretty(&*self.value.borrow()).unwrap_or_default(),
        );
        runtime::handle().spawn_blocking(move || {
            if let Some(dir) = file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(&file, data) {
                tracing::warn!("couldn't save the settings: {e}");
            }
        });
    }
}
