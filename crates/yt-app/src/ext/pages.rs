use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A screen you can navigate to.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Page {
    pub name: String,
    pub title: String,
    /// The component that builds the page; defaults to the page's name.
    #[serde(default)]
    pub component: Option<String>,
    /// Properties for the component when navigating without arguments.
    #[serde(default)]
    pub args: Value,
    /// Keep the page around when navigating away (e.g. to keep the scroll
    /// position), instead of building it again next time.
    #[serde(default)]
    pub keep: bool,
}

/// An entry in the sidebar.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NavItem {
    pub id: String,
    pub title: String,
    /// A symbolic icon name, e.g. `go-home-symbolic`.
    #[serde(default)]
    pub icon: String,
    pub page: String,
    #[serde(default)]
    pub args: Value,
    /// Lower comes first.
    #[serde(default)]
    pub order: i32,
    /// `main`, `library` or `bottom`.
    #[serde(default = "main_section")]
    pub section: String,
    /// Only for a signed-in user: hidden signed out.
    #[serde(default)]
    pub needs_account: bool,
}

impl Page {
    pub fn component(&self) -> &str {
        self.component.as_deref().unwrap_or(&self.name)
    }
}

fn section_rank(section: &str) -> u8 {
    match section {
        "main" => 0,
        "library" => 1,
        "bottom" => 3,
        _ => 2,
    }
}

fn main_section() -> String {
    "main".into()
}

/// The screens and the sidebar entries that lead to them.
#[derive(Default)]
pub struct Pages {
    pages: RefCell<BTreeMap<String, Page>>,
    nav: RefCell<Vec<NavItem>>,
    on_nav_change: RefCell<Vec<Rc<dyn Fn()>>>,
}

impl Pages {
    pub fn register(&self, page: Page) {
        self.pages.borrow_mut().insert(page.name.clone(), page);
    }

    pub fn get(&self, name: &str) -> Option<Page> {
        self.pages.borrow().get(name).cloned()
    }

    pub fn list(&self) -> Vec<Page> {
        self.pages.borrow().values().cloned().collect()
    }

    /// Adds a sidebar entry, replacing one with the same id.
    pub fn add_nav(&self, item: NavItem) {
        {
            let mut nav = self.nav.borrow_mut();
            nav.retain(|i| i.id != item.id);
            nav.push(item);
            nav.sort_by_key(|i| (section_rank(&i.section), i.order));
        }
        self.nav_changed();
    }

    pub fn remove_nav(&self, id: &str) {
        self.nav.borrow_mut().retain(|i| i.id != id);
        self.nav_changed();
    }

    pub fn nav(&self) -> Vec<NavItem> {
        self.nav.borrow().clone()
    }

    pub fn on_nav_change(&self, f: impl Fn() + 'static) {
        self.on_nav_change.borrow_mut().push(Rc::new(f));
    }

    fn nav_changed(&self) {
        let listeners: Vec<_> = self.on_nav_change.borrow().clone();
        for f in listeners {
            f();
        }
    }
}
