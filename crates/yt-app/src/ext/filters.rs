use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

pub type Filter = Rc<dyn Fn(Value) -> Value>;

/// Data pipelines: each named filter is a chain of functions that receive a
/// value and return it, changed or not. The app runs its data through these
/// before using it, e.g. `feed.page` before showing a feed or `video.card`
/// before filling in a card.
#[derive(Default)]
pub struct Filters {
    chains: RefCell<HashMap<String, Vec<(i32, Filter)>>>,
}

impl Filters {
    /// Adds a filter. Lower `order` runs first; equal orders run in the
    /// order they were added.
    pub fn add(&self, name: &str, order: i32, filter: impl Fn(Value) -> Value + 'static) {
        let mut chains = self.chains.borrow_mut();
        let chain = chains.entry(name.to_owned()).or_default();
        let at = chain
            .iter()
            .position(|(o, _)| *o > order)
            .unwrap_or(chain.len());
        chain.insert(at, (order, Rc::new(filter)));
    }

    pub fn is_empty(&self, name: &str) -> bool {
        self.chains.borrow().get(name).is_none_or(Vec::is_empty)
    }

    pub fn apply(&self, name: &str, value: Value) -> Value {
        let chain: Vec<Filter> = match self.chains.borrow().get(name) {
            Some(chain) => chain.iter().map(|(_, f)| f.clone()).collect(),
            None => return value,
        };
        chain.iter().fold(value, |v, f| f(v))
    }

    /// Runs typed data through a filter, as the `key` field of `context`
    /// (an object), and reads it back. Without filters it costs nothing;
    /// if a filter returns something invalid, `data` stays as it was.
    pub fn apply_to<T: Serialize + DeserializeOwned>(
        &self,
        name: &str,
        mut context: Value,
        key: &str,
        data: &mut T,
    ) -> Result<(), String> {
        if self.is_empty(name) {
            return Ok(());
        }
        context[key] = serde_json::to_value(&*data).map_err(|e| e.to_string())?;
        let mut value = self.apply(name, context);
        let mut out = value.get_mut(key).map(Value::take).unwrap_or_default();
        *data = serde_json::from_value(out.clone()).or_else(|first| {
            // A list a plugin emptied comes back from Lua as `{}`.
            empty_objects_to_arrays(&mut out);
            serde_json::from_value(out)
                .map_err(|_| format!("a plugin's {name} filter returned an invalid {key}: {first}"))
        })?;
        Ok(())
    }
}

fn empty_objects_to_arrays(value: &mut Value) {
    match value {
        Value::Object(map) if map.is_empty() => *value = Value::Array(Vec::new()),
        Value::Object(map) => map.values_mut().for_each(empty_objects_to_arrays),
        Value::Array(items) => items.iter_mut().for_each(empty_objects_to_arrays),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn runs_in_order() {
        let filters = Filters::default();
        filters.add("n", 10, |v| json!(v.as_i64().unwrap() * 10));
        filters.add("n", 0, |v| json!(v.as_i64().unwrap() + 1));
        assert_eq!(filters.apply("n", json!(1)), json!(20));
        assert_eq!(filters.apply("other", json!(1)), json!(1));
    }

    #[test]
    fn typed_data_survives_lua_empty_lists() {
        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct Page {
            items: Vec<u32>,
            next: Option<String>,
        }
        let filters = Filters::default();
        let mut page = Page {
            items: vec![1, 2],
            next: None,
        };
        assert_eq!(filters.apply_to("p", json!({}), "page", &mut page), Ok(()));
        assert_eq!(page.items, [1, 2]);
        // What a Lua filter that empties the list gives back.
        filters.add("p", 0, |mut v| {
            v["page"]["items"] = json!({});
            v
        });
        assert_eq!(
            filters.apply_to("p", json!({ "query": "q" }), "page", &mut page),
            Ok(())
        );
        assert_eq!(
            page,
            Page {
                items: vec![],
                next: None
            }
        );
        filters.add("p", 1, |mut v| {
            v["page"] = json!("nonsense");
            v
        });
        page.items.push(3);
        assert!(filters.apply_to("p", json!({}), "page", &mut page).is_err());
        assert_eq!(page.items, [3], "left as it was");
    }
}
