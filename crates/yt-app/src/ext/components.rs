use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use gtk::prelude::*;
use serde_json::Value;

pub type Build = Rc<dyn Fn(&Value) -> gtk::Widget>;
pub type Update = Rc<dyn Fn(&gtk::Widget, &Value)>;
pub type Wrapper = Rc<dyn Fn(&gtk::Widget, &Value, Phase)>;

/// When a wrapper runs: right after the widget was built, or after it was
/// filled with new data (cards in a feed are reused as you scroll).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Build,
    Update,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Build => "build",
            Phase::Update => "update",
        }
    }
}

/// A UI factory. `build` makes a widget from properties; `update`, if
/// present, refills an existing widget with new properties, which lets
/// lists reuse widgets instead of building new ones.
#[derive(Clone)]
pub struct Component {
    pub build: Build,
    pub update: Option<Update>,
}

impl Component {
    pub fn new(build: impl Fn(&Value) -> gtk::Widget + 'static) -> Self {
        Self {
            build: Rc::new(build),
            update: None,
        }
    }

    pub fn with_update(mut self, update: impl Fn(&gtk::Widget, &Value) + 'static) -> Self {
        self.update = Some(Rc::new(update));
        self
    }
}

/// Every piece of UI, by name: `sidebar`, `topbar`, `video_card`, `home`,
/// `watch`, ... The app builds its UI through this registry, so defining a
/// component with an existing name replaces that part of the UI, and
/// wrapping one changes it after it is built.
#[derive(Default)]
pub struct Components {
    defs: RefCell<BTreeMap<String, Component>>,
    wrappers: RefCell<HashMap<String, Vec<Wrapper>>>,
}

impl Components {
    /// Defines a component, replacing any with the same name. Returns the
    /// replaced one, so a replacement can build on it.
    pub fn define(&self, name: &str, component: Component) -> Option<Component> {
        self.defs.borrow_mut().insert(name.to_owned(), component)
    }

    pub fn get(&self, name: &str) -> Option<Component> {
        self.defs.borrow().get(name).cloned()
    }

    pub fn names(&self) -> Vec<String> {
        self.defs.borrow().keys().cloned().collect()
    }

    /// Adds a function that runs on every widget the component makes.
    pub fn wrap(&self, name: &str, wrapper: impl Fn(&gtk::Widget, &Value, Phase) + 'static) {
        self.wrappers
            .borrow_mut()
            .entry(name.to_owned())
            .or_default()
            .push(Rc::new(wrapper));
    }

    pub fn build(&self, name: &str, props: &Value) -> gtk::Widget {
        let Some(component) = self.get(name) else {
            tracing::warn!(component = name, "unknown component");
            return missing(name);
        };
        let widget = (component.build)(props);
        self.run_wrappers(name, &widget, props, Phase::Build);
        widget
    }

    /// Refills a widget made by the component. Returns false if the
    /// component can't update widgets, in which case the caller should
    /// build a new one.
    pub fn update(&self, name: &str, widget: &gtk::Widget, props: &Value) -> bool {
        let Some(update) = self.get(name).and_then(|c| c.update) else {
            return false;
        };
        update(widget, props);
        self.run_wrappers(name, widget, props, Phase::Update);
        true
    }

    fn run_wrappers(&self, name: &str, widget: &gtk::Widget, props: &Value, phase: Phase) {
        let wrappers: Vec<Wrapper> = self
            .wrappers
            .borrow()
            .get(name)
            .cloned()
            .unwrap_or_default();
        for wrapper in wrappers {
            wrapper(widget, props, phase);
        }
    }
}

/// Stands in for a component that doesn't exist, so a broken plugin can't
/// take the whole window down.
fn missing(name: &str) -> gtk::Widget {
    let label = gtk::Label::new(Some(&format!("Missing component: {name}")));
    label.add_css_class("error");
    label.upcast()
}
