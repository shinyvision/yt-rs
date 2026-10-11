//! Search: the commands, the search history, and loading results.
//!
//! `search.open` shows the `search` page for a query, with filters or a
//! chip, and remembers the query in the search history (on this computer,
//! in `search_history.json`). While typing, `search.suggest` asks for
//! suggestions, which arrive as `search.suggestions`. Results run through
//! the `search.page` filter and are announced as `search.loaded`; the
//! `search` page (see `ui::search`) shows them.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::{Value, json};
use yt_core::search::{SearchContinuation, SearchFilters, SearchPage};

use crate::app::{App, app};
use crate::commands::str_arg;
use crate::runtime;

/// Queries remembered.
const HISTORY_SIZE: usize = 100;

pub struct SearchHistory {
    queries: RefCell<Vec<String>>,
    file: PathBuf,
}

impl SearchHistory {
    pub fn load() -> Rc<Self> {
        let file = yt_core::paths::config_dir().join("search_history.json");
        let queries = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Rc::new(Self {
            queries: RefCell::new(queries),
            file,
        })
    }

    /// Newest first.
    pub fn list(&self) -> Vec<String> {
        self.queries.borrow().clone()
    }

    /// Up to `limit` remembered queries starting with what was typed.
    pub fn matching(&self, typed: &str, limit: usize) -> Vec<String> {
        let typed = typed.trim().to_lowercase();
        self.queries
            .borrow()
            .iter()
            .filter(|q| q.to_lowercase().starts_with(&typed))
            .take(limit)
            .cloned()
            .collect()
    }

    pub fn add(&self, query: &str) {
        let mut queries = self.queries.borrow_mut();
        queries.retain(|q| !q.eq_ignore_ascii_case(query));
        queries.insert(0, query.to_owned());
        queries.truncate(HISTORY_SIZE);
        drop(queries);
        self.save();
    }

    pub fn remove(&self, query: &str) {
        self.queries
            .borrow_mut()
            .retain(|q| !q.eq_ignore_ascii_case(query));
        self.save();
    }

    pub fn clear(&self) {
        self.queries.borrow_mut().clear();
        self.save();
    }

    fn save(&self) {
        let (file, data) = (
            self.file.clone(),
            serde_json::to_vec(&*self.queries.borrow()).unwrap_or_default(),
        );
        runtime::handle().spawn_blocking(move || {
            if let Some(dir) = file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(file, data);
        });
    }
}

pub fn register(this: &Rc<App>) {
    let c = &this.ext.commands;
    // { query, filters?, chip?, chips? }: `chip` is a chip's token, `chips`
    // the query's chips (they come with its first page only).
    c.register("search.open", |args| {
        let query = str_arg(&args, "query")?.trim().to_owned();
        if query.is_empty() {
            return Err("type something to search for".into());
        }
        let filters: SearchFilters = args
            .get("filters")
            .filter(|f| !f.is_null())
            .map(|f| serde_json::from_value(f.clone()))
            .transpose()
            .map_err(|e| format!("invalid filters: {e}"))?
            .unwrap_or_default();
        let app = app();
        app.search_history.add(&query);
        app.ext.events.emit("search.history_changed", &Value::Null);
        let args = json!({
            "query": query,
            "filters": filters,
            "chip": args.get("chip").cloned().unwrap_or_default(),
            "chips": args.get("chips").cloned().unwrap_or_default(),
        });
        // Searching again for what is on screen searches again.
        if app.current_page().as_deref() == Some("search")
            && app.current_args().as_ref() == Some(&args)
        {
            app.ext.events.emit("search.refresh", &Value::Null);
        } else {
            app.navigate("search", args, true);
        }
        Ok(Value::Null)
    });
    // Loads more results on the search page (or tries again after a
    // failure).
    c.register("search.more", |_| {
        app().ext.events.emit("search.more", &Value::Null);
        Ok(Value::Null)
    });
    c.register("search.focus", |_| {
        app().ext.events.emit("search.focus", &Value::Null);
        Ok(Value::Null)
    });
    // { query }: suggestions arrive as `search.suggestions`.
    c.register("search.suggest", |args| {
        let query = str_arg(&args, "query")?.to_owned();
        runtime::spawn_local(async move {
            let youtube = app().youtube.clone();
            let typed = query.clone();
            let result =
                runtime::background(async move { youtube.suggestions(&typed).await }).await;
            match result {
                Ok(suggestions) => {
                    let value = app().ext.filters.apply(
                        "search.suggestions",
                        json!({ "query": query, "suggestions": suggestions }),
                    );
                    app().ext.events.emit("search.suggestions", &value);
                }
                Err(e) => tracing::debug!("no suggestions for {query:?}: {e}"),
            }
        });
        Ok(Value::Null)
    });
    c.register("search.history", |_| Ok(json!(app().search_history.list())));
    c.register("search.history_remove", |args| {
        app().search_history.remove(str_arg(&args, "query")?);
        app()
            .ext
            .events
            .emit("search.history_changed", &Value::Null);
        Ok(Value::Null)
    });
    c.register("search.history_clear", |_| {
        app().search_history.clear();
        app()
            .ext
            .events
            .emit("search.history_changed", &Value::Null);
        Ok(Value::Null)
    });
}

/// The first page of results for a query.
pub async fn fetch(
    query: &str,
    filters: &SearchFilters,
    chip: Option<&str>,
) -> Result<SearchPage, String> {
    let (q, f, c) = (query.to_owned(), filters.clone(), chip.map(str::to_owned));
    let youtube = app().youtube.clone();
    runtime::background(async move { youtube.search(&q, &f, c.as_deref()).await })
        .await
        .map_err(|e| e.to_string())
}

/// The page after the one `next` came with.
pub async fn fetch_more(next: &SearchContinuation) -> Result<SearchPage, String> {
    let next = next.clone();
    let youtube = app().youtube.clone();
    runtime::background(async move { youtube.search_more(&next).await })
        .await
        .map_err(|e| e.to_string())
}

/// Runs a page of results through the `search.page` filter, so plugins can
/// change it, and announces it as `search.loaded`. `first` tells the first
/// page from the ones loaded while scrolling.
pub fn publish(query: &str, first: bool, page: &mut SearchPage) {
    let app = app();
    let context = json!({ "query": query, "first": first });
    if let Err(e) = app
        .ext
        .filters
        .apply_to("search.page", context, "page", page)
    {
        app.toast(&e);
    }
    if app.ext.events.has_handlers("search.loaded") {
        app.ext.events.emit(
            "search.loaded",
            &json!({ "query": query, "first": first, "page": page }),
        );
    }
}
