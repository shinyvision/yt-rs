//! The extension system: the registries that make up the app.
//!
//! Every screen, component, command, shortcut and style in the app is
//! registered here by name, and the built-in code uses these registries the
//! same way Lua plugins do. That is what makes everything moddable: a plugin
//! can call, replace or wrap anything the app registers, and add its own.
//!
//! | Registry     | What it holds                                            |
//! |--------------|----------------------------------------------------------|
//! | `events`     | notifications such as `player.position` or `feed.loaded` |
//! | `filters`    | data pipelines, e.g. every feed page before it is shown  |
//! | `commands`   | named actions, e.g. `player.seek` or `app.navigate`      |
//! | `keymap`     | key bindings that run commands                           |
//! | `components` | UI factories, e.g. `video_card` or `sidebar`             |
//! | `slots`      | named places in the layout where widgets can be added    |
//! | `pages`      | the screens you can navigate to, and the sidebar entries |
//! | `theme`      | CSS                                                      |
//!
//! Data crosses the registries as JSON values (`serde_json::Value`), which
//! map directly onto Lua tables.

pub mod commands;
pub mod components;
pub mod events;
pub mod filters;
pub mod keymap;
pub mod pages;
pub mod slots;
pub mod theme;

pub use commands::Commands;
pub use components::Components;
pub use events::Events;
pub use filters::Filters;
pub use keymap::Keymap;
pub use pages::Pages;
pub use slots::Slots;
pub use theme::Theme;

/// All registries, owned by the app.
#[derive(Default)]
pub struct Extensions {
    pub events: Events,
    pub filters: Filters,
    pub commands: Commands,
    pub keymap: Keymap,
    pub components: Components,
    pub slots: Slots,
    pub pages: Pages,
    pub theme: Theme,
}
