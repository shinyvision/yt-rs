//! Descriptions and comments as labels with working links: URLs, moments in
//! the video, channels and hashtags. Every link runs the `link.open`
//! command, so plugins can change what links do.

use gtk::glib;
use gtk::prelude::*;
use serde_json::json;
use yt_core::text::RichText;

use crate::app::app;

const SCHEME: &str = "yt-rs-link:";

/// A wrapping, selectable label showing `text`.
pub fn label(text: &RichText) -> gtk::Label {
    let label = gtk::Label::new(None);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_xalign(0.0);
    label.set_selectable(true);
    label.add_css_class("rich-text");
    set(&label, text);
    label.connect_activate_link(|_, uri| {
        let Some(json) = uri.strip_prefix(SCHEME) else {
            return glib::Propagation::Proceed;
        };
        let link: serde_json::Value = serde_json::from_str(
            &glib::Uri::unescape_string(json, None::<&str>).unwrap_or_default(),
        )
        .unwrap_or_default();
        if let Err(e) = app().ext.commands.run("link.open", json!({ "link": link })) {
            app().toast(&e);
        }
        glib::Propagation::Stop
    });
    label
}

pub fn set(label: &gtk::Label, text: &RichText) {
    label.set_markup(&markup(text));
}

/// Pango markup for `text`, with links encoded as `yt-rs-link:<json>`.
pub fn markup(text: &RichText) -> String {
    let mut out = String::new();
    for run in &text.runs {
        let mut piece = glib::markup_escape_text(&run.text).to_string();
        if run.bold {
            piece = format!("<b>{piece}</b>");
        }
        if run.italic {
            piece = format!("<i>{piece}</i>");
        }
        if let Some(link) = &run.link {
            let json = serde_json::to_string(link).unwrap_or_default();
            let href = format!("{SCHEME}{}", glib::Uri::escape_string(&json, None, false));
            piece = format!(
                "<a href=\"{}\">{piece}</a>",
                glib::markup_escape_text(&href)
            );
        }
        out.push_str(&piece);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use yt_core::text::{Link, Run};

    #[test]
    fn escapes_text_and_encodes_links() {
        let text = RichText {
            runs: vec![
                Run {
                    text: "a < b & ".into(),
                    ..Run::default()
                },
                Run {
                    text: "1:23".into(),
                    bold: true,
                    link: Some(Link::Video {
                        video_id: "v".into(),
                        start: Some(83.0),
                    }),
                    ..Run::default()
                },
            ],
        };
        let m = markup(&text);
        assert!(m.starts_with("a &lt; b &amp; <a href=\"yt-rs-link:"));
        assert!(m.ends_with("\"><b>1:23</b></a>"));
        assert!(
            !m.contains("\"start\""),
            "the JSON is escaped inside the href"
        );
    }
}
