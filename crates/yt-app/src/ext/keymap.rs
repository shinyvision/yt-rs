use std::cell::RefCell;
use std::collections::BTreeMap;

use gtk::gdk;
use serde_json::Value;

/// Bindings that apply on every page.
pub const GLOBAL: &str = "global";

#[derive(Clone, Debug)]
pub struct Binding {
    pub command: String,
    pub args: Value,
}

/// Key bindings: a key in a scope (a page name, or [`GLOBAL`]) runs a
/// command. Keys use GTK's accelerator syntax (`<Control>f`, `space`,
/// `Left`) or the shorter `ctrl+f`.
#[derive(Default)]
pub struct Keymap {
    bindings: RefCell<BTreeMap<(String, String), Binding>>,
}

impl Keymap {
    pub fn set(&self, key: &str, scope: &str, command: &str, args: Value) -> Result<(), String> {
        let accel = canonical(key)?;
        self.bindings.borrow_mut().insert(
            (scope.to_owned(), accel),
            Binding {
                command: command.to_owned(),
                args,
            },
        );
        Ok(())
    }

    pub fn unset(&self, key: &str, scope: &str) -> Result<(), String> {
        let accel = canonical(key)?;
        self.bindings
            .borrow_mut()
            .remove(&(scope.to_owned(), accel));
        Ok(())
    }

    pub fn clear(&self) {
        self.bindings.borrow_mut().clear();
    }

    /// The binding for a key press on `page`: the page's own bindings win
    /// over global ones.
    ///
    /// Shifted symbols arrive as the symbol itself (Shift+. is `greater`),
    /// so a binding for `greater` matches with or without Shift.
    pub fn lookup(
        &self,
        key: gdk::Key,
        modifiers: gdk::ModifierType,
        page: &str,
    ) -> Option<Binding> {
        let mods = modifiers & gtk::accelerator_get_default_mod_mask();
        let mut candidates = vec![gtk::accelerator_name(key.to_lower(), mods).to_string()];
        if mods.contains(gdk::ModifierType::SHIFT_MASK) {
            candidates.push(
                gtk::accelerator_name(key.to_lower(), mods - gdk::ModifierType::SHIFT_MASK)
                    .to_string(),
            );
        }
        let bindings = self.bindings.borrow();
        [page, GLOBAL]
            .iter()
            .flat_map(|scope| {
                candidates
                    .iter()
                    .map(move |accel| (scope.to_string(), accel.clone()))
            })
            .find_map(|key| bindings.get(&key))
            .cloned()
    }

    /// Every binding as (scope, key, binding).
    pub fn list(&self) -> Vec<(String, String, Binding)> {
        self.bindings
            .borrow()
            .iter()
            .map(|((s, k), b)| (s.clone(), k.clone(), b.clone()))
            .collect()
    }
}

/// Normalizes a key description so equal keys compare equal.
fn canonical(key: &str) -> Result<String, String> {
    let gtk_syntax = if key.contains('<') || !key.contains('+') || key == "+" {
        key.to_owned()
    } else {
        let (mods, last) = key.rsplit_once('+').expect("contains +");
        let mut out = String::new();
        for m in mods.split('+') {
            out.push_str(match m.to_lowercase().as_str() {
                "ctrl" | "control" => "<Control>",
                "shift" => "<Shift>",
                "alt" => "<Alt>",
                "super" | "meta" | "mod4" => "<Super>",
                other => return Err(format!("unknown modifier {other:?} in {key:?}")),
            });
        }
        out + last
    };
    let (keyval, mods) =
        gtk::accelerator_parse(&gtk_syntax).ok_or_else(|| format!("invalid key {key:?}"))?;
    Ok(gtk::accelerator_name(keyval.to_lower(), mods).to_string())
}
