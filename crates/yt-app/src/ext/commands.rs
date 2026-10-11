use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use serde_json::Value;

pub type CommandResult = Result<Value, String>;
pub type Command = Rc<dyn Fn(Value) -> CommandResult>;

/// Named actions, e.g. `player.seek` or `app.navigate`. Buttons, menus and
/// key bindings all run commands, so replacing a command changes every way
/// of triggering it.
#[derive(Default)]
pub struct Commands {
    map: RefCell<BTreeMap<String, Command>>,
}

impl Commands {
    /// Registers a command, replacing any command with the same name.
    /// Returns the replaced command, so a replacement can call it.
    pub fn register(
        &self,
        name: &str,
        command: impl Fn(Value) -> CommandResult + 'static,
    ) -> Option<Command> {
        self.map
            .borrow_mut()
            .insert(name.to_owned(), Rc::new(command))
    }

    pub fn get(&self, name: &str) -> Option<Command> {
        self.map.borrow().get(name).cloned()
    }

    pub fn run(&self, name: &str, args: Value) -> CommandResult {
        let command = self
            .get(name)
            .ok_or_else(|| format!("unknown command {name:?}"))?;
        command(args)
    }

    pub fn names(&self) -> Vec<String> {
        self.map.borrow().keys().cloned().collect()
    }
}
