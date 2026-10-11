use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

pub type SlotBuild = Rc<dyn Fn() -> gtk::Widget>;

struct Item {
    id: u64,
    order: i32,
    build: SlotBuild,
}

/// Named places in the layout where plugins add widgets: `topbar.end`,
/// `sidebar.bottom`, `player.controls.end`, ... Built-in components create
/// the containers with [`Slots::create`]; contributions are functions that
/// build a widget, since a slot can appear more than once.
#[derive(Default)]
pub struct Slots {
    items: RefCell<HashMap<String, Vec<Item>>>,
    mounted: RefCell<HashMap<String, Vec<glib::WeakRef<gtk::Box>>>>,
    next_id: Cell<u64>,
}

impl Slots {
    /// Adds a widget to a slot; lower `order` comes first. Slots already on
    /// screen update immediately.
    pub fn add(&self, slot: &str, order: i32, build: impl Fn() -> gtk::Widget + 'static) -> u64 {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        {
            let mut items = self.items.borrow_mut();
            let list = items.entry(slot.to_owned()).or_default();
            let at = list
                .iter()
                .position(|i| i.order > order)
                .unwrap_or(list.len());
            list.insert(
                at,
                Item {
                    id,
                    order,
                    build: Rc::new(build),
                },
            );
        }
        self.refresh(slot);
        id
    }

    pub fn remove(&self, id: u64) {
        let slots: Vec<String> = {
            let mut items = self.items.borrow_mut();
            items
                .iter_mut()
                .filter_map(|(slot, list)| {
                    let before = list.len();
                    list.retain(|i| i.id != id);
                    (list.len() != before).then(|| slot.clone())
                })
                .collect()
        };
        for slot in slots {
            self.refresh(&slot);
        }
    }

    /// Makes the container for a slot and fills it.
    pub fn create(&self, slot: &str, orientation: gtk::Orientation, spacing: i32) -> gtk::Box {
        let container = gtk::Box::new(orientation, spacing);
        container.set_widget_name(&format!("slot:{slot}"));
        container.add_css_class("slot");
        self.mounted
            .borrow_mut()
            .entry(slot.to_owned())
            .or_default()
            .push(container.downgrade());
        self.fill(slot, &container);
        container
    }

    /// Every slot name that has a container on screen or contributions.
    pub fn names(&self) -> Vec<String> {
        let mut names: BTreeSet<String> = self.items.borrow().keys().cloned().collect();
        names.extend(self.mounted.borrow().keys().cloned());
        names.into_iter().collect()
    }

    fn refresh(&self, slot: &str) {
        let containers: Vec<gtk::Box> = {
            let mut mounted = self.mounted.borrow_mut();
            let Some(list) = mounted.get_mut(slot) else {
                return;
            };
            list.retain(|w| w.upgrade().is_some());
            list.iter().filter_map(|w| w.upgrade()).collect()
        };
        for container in containers {
            self.fill(slot, &container);
        }
    }

    fn fill(&self, slot: &str, container: &gtk::Box) {
        while let Some(child) = container.first_child() {
            container.remove(&child);
        }
        let builds: Vec<SlotBuild> = self
            .items
            .borrow()
            .get(slot)
            .map(|l| l.iter().map(|i| i.build.clone()).collect())
            .unwrap_or_default();
        container.set_visible(!builds.is_empty());
        for build in builds {
            container.append(&build());
        }
    }
}
