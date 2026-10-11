//! The search filters dialog, as on youtube.com: Type, Duration, Upload
//! date, Features and Prioritize. Choosing an option searches again with it
//! and closes the dialog; choosing one that is on turns it off.
//!
//! The content is the `search_filters` component (`{ query, filters }`),
//! so plugins can restyle or replace it.

use adw::prelude::*;
use serde_json::{Value, json};
use yt_core::search::SearchFilters;

use crate::app::app;

/// One of the dialog's sections.
struct Section {
    title: &'static str,
    /// The filter it sets: a field of [`SearchFilters`] in JSON.
    field: &'static str,
    /// Each option's label and value.
    options: &'static [(&'static str, &'static str)],
}

const SECTIONS: &[Section] = &[
    Section {
        title: "Type",
        field: "type",
        options: &[
            ("Videos", "video"),
            ("Shorts", "short"),
            ("Channels", "channel"),
            ("Playlists", "playlist"),
        ],
    },
    Section {
        title: "Duration",
        field: "duration",
        options: &[
            ("Under 3 minutes", "short"),
            ("3–20 minutes", "medium"),
            ("Over 20 minutes", "long"),
        ],
    },
    Section {
        title: "Upload date",
        field: "upload_date",
        options: &[
            ("Today", "today"),
            ("This week", "week"),
            ("This month", "month"),
            ("This year", "year"),
        ],
    },
    Section {
        title: "Features",
        field: "features",
        options: &[
            ("Live", "live"),
            ("4K", "4k"),
            ("HD", "hd"),
            ("Subtitles/CC", "subtitles"),
            ("Creative Commons", "creative_commons"),
            ("360°", "360"),
            ("VR180", "vr180"),
            ("3D", "3d"),
            ("HDR", "hdr"),
            ("Location", "location"),
        ],
    },
    Section {
        title: "Prioritize",
        field: "sort",
        options: &[("Relevance", "relevance"), ("Popularity", "popularity")],
    },
];

/// Shows the dialog for the search on screen.
pub fn present(parent: &impl IsA<gtk::Widget>, query: &str, filters: &SearchFilters) {
    let props = json!({ "query": query, "filters": filters });
    let content = app().ext.components.build("search_filters", &props);
    let dialog = adw::Dialog::builder()
        .title("Search filters")
        .content_width(900)
        .child(&content)
        .build();
    dialog.add_css_class("yt-dialog");
    dialog.add_css_class("search-filters-dialog");
    dialog.present(Some(parent));
}

/// The `search_filters` component: the dialog's content.
pub fn build(props: &Value) -> gtk::Widget {
    let query = props
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let filters = serde_json::to_value(
        props
            .get("filters")
            .and_then(|f| serde_json::from_value::<SearchFilters>(f.clone()).ok())
            .unwrap_or_default(),
    )
    .unwrap_or_default();

    let title = gtk::Label::new(Some("Search filters"));
    title.add_css_class("yt-dialog-title");
    title.set_xalign(0.0);
    title.set_hexpand(true);
    let close = gtk::Button::from_icon_name("yt-close-symbolic");
    close.add_css_class("icon-button");
    close.set_tooltip_text(Some("Close"));
    close.connect_clicked(close_dialog);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.append(&title);
    header.append(&close);

    let columns = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .min_children_per_line(1)
        .max_children_per_line(SECTIONS.len() as u32)
        .column_spacing(16)
        .row_spacing(24)
        .homogeneous(true)
        .build();
    columns.add_css_class("search-filters-columns");
    for section in SECTIONS {
        columns.append(&build_section(section, &query, &filters));
    }

    // Taller than a small window, the sections scroll under the header.
    let scroller = gtk::ScrolledWindow::builder()
        .child(&columns)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .vexpand(true)
        .build();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 16);
    root.add_css_class("yt-dialog-content");
    root.add_css_class("search-filters");
    root.append(&header);
    root.append(&scroller);
    root.upcast()
}

fn build_section(section: &Section, query: &str, filters: &Value) -> gtk::Widget {
    let field = section.field;
    let title = gtk::Label::new(Some(section.title));
    title.add_css_class("search-filters-heading");
    title.set_xalign(0.0);
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    column.append(&title);
    column.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    for (label, value) in section.options {
        let on = is_on(filters, field, value);
        let text = gtk::Label::new(Some(label));
        text.set_xalign(0.0);
        text.set_hexpand(true);
        text.set_wrap(true);
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        content.append(&text);
        // The default order has nothing to turn off.
        if on && !(field == "sort" && *value == "relevance") {
            let remove = gtk::Image::from_icon_name("yt-close-symbolic");
            remove.set_tooltip_text(Some("Remove"));
            content.append(&remove);
        }
        let option = gtk::Button::builder()
            .child(&content)
            .css_classes(["flat", "search-filter-option"])
            .build();
        if on {
            option.add_css_class("selected");
        }
        let changed = toggled(filters, field, value);
        let query = query.to_owned();
        option.connect_clicked(move |button| {
            close_dialog(button);
            if let Err(e) = app()
                .ext
                .commands
                .run("search.open", json!({ "query": query, "filters": changed }))
            {
                app().toast(&e);
            }
        });
        column.append(&option);
    }
    column.upcast()
}

fn is_on(filters: &Value, field: &str, value: &str) -> bool {
    match &filters[field] {
        Value::Array(values) => values.iter().any(|v| v == value),
        other => other == value,
    }
}

/// The filters with this option turned on, or off if it was on.
fn toggled(filters: &Value, field: &str, value: &str) -> Value {
    let mut changed = filters.clone();
    let on = is_on(filters, field, value);
    match &mut changed[field] {
        Value::Array(values) if on => values.retain(|v| v != value),
        Value::Array(values) => values.push(json!(value)),
        slot if field == "sort" => *slot = json!(if on { "relevance" } else { value }),
        slot => *slot = if on { Value::Null } else { json!(value) },
    }
    changed
}

fn close_dialog(widget: &impl IsA<gtk::Widget>) {
    if let Some(dialog) = widget
        .ancestor(adw::Dialog::static_type())
        .and_downcast::<adw::Dialog>()
    {
        dialog.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yt_core::search::{Feature, Kind, Sort};

    fn parsed(value: Value) -> SearchFilters {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn options_turn_on_and_off() {
        let none = serde_json::to_value(SearchFilters::default()).unwrap();
        let videos = toggled(&none, "type", "video");
        assert_eq!(parsed(videos.clone()).kind, Some(Kind::Video));
        assert_eq!(
            parsed(toggled(&videos, "type", "video")),
            SearchFilters::default()
        );
        assert_eq!(
            parsed(toggled(&videos, "type", "short")).kind,
            Some(Kind::Short)
        );

        let hd = toggled(&none, "features", "hd");
        let both = toggled(&hd, "features", "4k");
        assert_eq!(parsed(both.clone()).features, [Feature::Hd, Feature::FourK]);
        assert_eq!(
            parsed(toggled(&both, "features", "hd")).features,
            [Feature::FourK]
        );

        let popular = toggled(&none, "sort", "popularity");
        assert_eq!(parsed(popular.clone()).sort, Sort::Popularity);
        assert_eq!(
            parsed(toggled(&popular, "sort", "popularity")).sort,
            Sort::Relevance
        );
        assert!(is_on(&none, "sort", "relevance"));
    }

    #[test]
    fn every_option_is_a_filter() {
        let none = serde_json::to_value(SearchFilters::default()).unwrap();
        for section in SECTIONS {
            for (label, value) in section.options {
                let filters: Result<SearchFilters, _> =
                    serde_json::from_value(toggled(&none, section.field, value));
                assert!(
                    filters.is_ok_and(|f| !f.is_empty() || *value == "relevance"),
                    "{label}"
                );
            }
        }
    }
}
