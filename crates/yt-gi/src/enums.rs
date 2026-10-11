//! Enums and flags. An enum value is its nick as a string (`"vertical"`,
//! `"word-char"`), as GObject properties already were; flags are numbers,
//! so they combine with `bit.bor`. Both are accepted as numbers, names in
//! any case, or (flags) `"a|b"` and `{ "a", "b" }`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::info::{Info, children};
use crate::sys;

pub struct Values {
    /// Lower-case names with `-`, to values.
    pub by_name: HashMap<String, i64>,
    /// Values to their nick (the first name, for aliases).
    pub by_value: HashMap<i64, Rc<str>>,
    /// In declaration order: (`UPPER_NAME`, nick, value).
    pub list: Vec<(String, Rc<str>, i64)>,
}

thread_local! {
    static TABLES: RefCell<HashMap<String, Rc<Values>>> = RefCell::new(HashMap::new());
}

/// Names as written: `WORD_CHAR`, `word-char` and `word_char` are one.
pub fn normalize(name: &str) -> String {
    name.trim().to_ascii_lowercase().replace('_', "-")
}

pub fn values(info: &Info) -> Rc<Values> {
    let key = info.full_name();
    if let Some(v) = TABLES.with(|t| t.borrow().get(&key).cloned()) {
        return v;
    }
    let mut values = Values {
        by_name: HashMap::new(),
        by_value: HashMap::new(),
        list: Vec::new(),
    };
    for value in children(
        info,
        sys::gi_enum_info_get_n_values,
        sys::gi_enum_info_get_value,
    ) {
        // SAFETY: a value info.
        let v = unsafe { sys::gi_value_info_get_value(value.ptr()) };
        let nick: Rc<str> = normalize(value.name()).into();
        values.by_name.insert(nick.to_string(), v);
        values.by_value.entry(v).or_insert_with(|| nick.clone());
        values
            .list
            .push((value.name().to_ascii_uppercase(), nick, v));
    }
    let values = Rc::new(values);
    TABLES.with(|t| t.borrow_mut().insert(key, values.clone()));
    values
}

/// A Lua value as an enum or flags value of `info`.
pub fn from_lua(info: &Info, flags: bool, value: &mlua::Value) -> Result<i64, String> {
    let values = values(info);
    let by_name = |name: &str| -> Result<i64, String> {
        values
            .by_name
            .get(&normalize(name))
            .copied()
            .ok_or_else(|| {
                let known: Vec<&str> = values.list.iter().map(|(_, n, _)| &**n).collect();
                format!(
                    "{} has no value {name:?} (it has {})",
                    info.full_name(),
                    known.join(", ")
                )
            })
    };
    match value {
        mlua::Value::Integer(i) => Ok(*i),
        mlua::Value::Number(n) => Ok(*n as i64),
        mlua::Value::String(s) => {
            let s = s.to_str().map_err(|e| e.to_string())?;
            if flags {
                s.split('|')
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .try_fold(0, |bits, part| Ok(bits | by_name(part)?))
            } else {
                by_name(&s)
            }
        }
        mlua::Value::Table(t) if flags => {
            let mut bits = 0;
            for v in t.sequence_values::<mlua::Value>() {
                bits |= from_lua(info, flags, &v.map_err(|e| e.to_string())?)?;
            }
            Ok(bits)
        }
        mlua::Value::Nil if flags => Ok(0),
        other => Err(format!(
            "expected a {} value, got {}",
            info.full_name(),
            other.type_name()
        )),
    }
}

/// An enum value as Lua: its nick, or the number if it has none. Flags
/// stay numbers.
pub fn to_lua(lua: &mlua::Lua, info: &Info, flags: bool, value: i64) -> mlua::Result<mlua::Value> {
    if flags {
        return Ok(mlua::Value::Integer(value));
    }
    match values(info).by_value.get(&value) {
        Some(nick) => Ok(mlua::Value::String(lua.create_string(&**nick)?)),
        None => Ok(mlua::Value::Integer(value)),
    }
}
