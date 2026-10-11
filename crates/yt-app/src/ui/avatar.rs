//! Avatars: the channel's picture when known, otherwise its initials.

use gtk::prelude::*;

use super::{attach, attached};
use crate::app::app;

/// The URL an avatar currently shows (or is loading), so late images for a
/// reused widget are dropped.
struct Showing(String);

/// Shows `name`'s initials in `avatar`, then its picture once loaded.
pub fn set(avatar: &adw::Avatar, name: &str, url: Option<&str>) {
    avatar.set_text(Some(name.trim_start_matches('@')));
    let Some(url) = url else {
        avatar.set_custom_image(None::<&gtk::gdk::Paintable>);
        attach(avatar, std::rc::Rc::new(Showing(String::new())));
        return;
    };
    if attached::<Showing>(avatar).is_some_and(|s| s.0 == url) {
        return;
    }
    avatar.set_custom_image(None::<&gtk::gdk::Paintable>);
    attach(avatar, std::rc::Rc::new(Showing(url.to_owned())));
    let size = avatar.size() * avatar.scale_factor();
    let weak = avatar.downgrade();
    let wanted = url.to_owned();
    app().images.texture_then(
        url,
        size.max(48),
        Box::new(move |texture| {
            let (Some(avatar), Some(texture)) = (weak.upgrade(), texture) else {
                return;
            };
            if attached::<Showing>(&avatar).is_some_and(|s| s.0 == wanted) {
                avatar.set_custom_image(Some(&texture));
            }
        }),
    );
}

/// An avatar of `size` pixels for a channel, looked up in the channel
/// directory by name if no picture is given.
pub fn new(size: i32, name: &str, url: Option<&str>) -> adw::Avatar {
    let avatar = adw::Avatar::new(size, None, true);
    let known = url
        .map(str::to_owned)
        .or_else(|| app().channels.lookup(name).and_then(|c| c.avatar));
    set(&avatar, name, known.as_deref());
    avatar
}
