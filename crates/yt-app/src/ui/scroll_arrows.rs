//! Buttons at the ends of a horizontal row that goes on past its edges, as
//! youtube.com has on rows of chips and the shelves of a channel's home:
//! the row fades out under a round button that scrolls it most of a width
//! that way. Each shows only while there is more that way.

use adw::prelude::*;

/// Adds the buttons to `overlay`, whose child shows `scroller`.
pub fn add(overlay: &gtk::Overlay, scroller: &gtk::ScrolledWindow) {
    let adjustment = scroller.hadjustment();
    let arrows: Vec<(gtk::Box, f64)> = [
        ("yt-chevron-left-symbolic", "start", gtk::Align::Start, -1.0),
        ("yt-chevron-right-symbolic", "end", gtk::Align::End, 1.0),
    ]
    .into_iter()
    .map(|(icon, side, align, direction)| {
        let button = gtk::Button::from_icon_name(icon);
        button.add_css_class("scroll-arrow");
        button.set_valign(gtk::Align::Center);
        button.set_tooltip_text(Some(if direction < 0.0 { "Previous" } else { "Next" }));
        let (scroller, adjustment) = (scroller.downgrade(), adjustment.clone());
        button.connect_clicked(move |_| {
            if let Some(scroller) = scroller.upgrade() {
                scroll(&scroller, &adjustment, direction);
            }
        });
        let fade = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        fade.add_css_class("scroll-fade");
        fade.add_css_class(side);
        fade.set_halign(align);
        fade.append(&button);
        fade.set_visible(false);
        overlay.add_overlay(&fade);
        (fade, direction)
    })
    .collect();
    let show = move |adjustment: &gtk::Adjustment| {
        let (value, end) = (
            adjustment.value(),
            adjustment.upper() - adjustment.page_size(),
        );
        for (fade, direction) in &arrows {
            fade.set_visible(if *direction < 0.0 {
                value > 1.0
            } else {
                value < end - 1.0
            });
        }
    };
    let also = show.clone();
    adjustment.connect_changed(move |a| also(a));
    adjustment.connect_value_changed(move |a| show(a));
}

/// Scrolls most of a width that way, easing in.
fn scroll(scroller: &gtk::ScrolledWindow, adjustment: &gtk::Adjustment, direction: f64) {
    let from = adjustment.value();
    let to = (from + direction * adjustment.page_size() * 0.8).clamp(
        adjustment.lower(),
        adjustment.upper() - adjustment.page_size(),
    );
    let target = adjustment.clone();
    let animation = adw::TimedAnimation::new(
        scroller,
        from,
        to,
        250,
        adw::CallbackAnimationTarget::new(move |v| target.set_value(v)),
    );
    animation.set_easing(adw::Easing::EaseOutCubic);
    animation.play();
}
