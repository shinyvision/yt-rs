use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use serde_json::Value;

pub type Handler = Rc<dyn Fn(&Value)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HandlerId(u64);

impl HandlerId {
    /// The id as a number, e.g. to hand to Lua.
    pub fn raw(self) -> u64 {
        self.0
    }

    pub fn from_raw(id: u64) -> Self {
        Self(id)
    }
}

/// Named notifications, e.g. `player.position` or `feed.loaded`. Anyone can
/// emit any event, including new ones.
#[derive(Default)]
pub struct Events {
    handlers: RefCell<HashMap<String, Vec<(HandlerId, Handler)>>>,
    next_id: Cell<u64>,
}

impl Events {
    pub fn on(&self, name: &str, handler: impl Fn(&Value) + 'static) -> HandlerId {
        let id = HandlerId(self.next_id.get());
        self.next_id.set(id.0 + 1);
        self.handlers
            .borrow_mut()
            .entry(name.to_owned())
            .or_default()
            .push((id, Rc::new(handler)));
        id
    }

    pub fn off(&self, id: HandlerId) {
        for handlers in self.handlers.borrow_mut().values_mut() {
            handlers.retain(|(h, _)| *h != id);
        }
    }

    /// How many handlers each event has, for finding handlers that outlive
    /// their widgets.
    pub fn counts(&self) -> std::collections::BTreeMap<String, usize> {
        self.handlers
            .borrow()
            .iter()
            .filter(|(_, h)| !h.is_empty())
            .map(|(name, h)| (name.clone(), h.len()))
            .collect()
    }

    pub fn has_handlers(&self, name: &str) -> bool {
        self.handlers
            .borrow()
            .get(name)
            .is_some_and(|h| !h.is_empty())
    }

    pub fn emit(&self, name: &str, payload: &Value) {
        // Copy the list first, so handlers can add or remove handlers.
        let handlers: Vec<Handler> = match self.handlers.borrow().get(name) {
            Some(list) => list.iter().map(|(_, h)| h.clone()).collect(),
            None => return,
        };
        for handler in handlers {
            handler(payload);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handlers_can_unsubscribe_while_being_called() {
        let events = Rc::new(Events::default());
        let calls = Rc::new(Cell::new(0));
        let id = Rc::new(Cell::new(None));
        let (e, c, i) = (events.clone(), calls.clone(), id.clone());
        id.set(Some(events.on("x", move |_| {
            c.set(c.get() + 1);
            e.off(i.get().unwrap());
        })));
        events.emit("x", &Value::Null);
        events.emit("x", &Value::Null);
        assert_eq!(calls.get(), 1);
    }
}
