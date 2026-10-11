//! Ambient mode, as on youtube.com: the video's colors glow softly around
//! the player on the watch page.
//!
//! Every couple of seconds while the video plays there, the frame on screen
//! is drawn once, small and blurred, into a texture with clear margins (so
//! the glow fades out at its edges), and the player's frame shows it behind
//! the video, larger, crossfading from the last. Between updates the glow
//! costs a small texture drawn scaled up: no blur on every frame.

use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, glib, graphene};

use super::aspect::AspectBin;
use crate::app::app;

/// How often the glow follows the picture.
const EVERY: Duration = Duration::from_secs(2);
/// The glow's texture: the picture in the middle, blurred into margins.
const PICTURE: (f32, f32) = (160.0, 90.0);
const MARGIN: f32 = 40.0;
const BLUR: f64 = 16.0;
/// The picture shows this much larger than the player behind it, so its
/// edges glow around the player before the blur fades them out.
const LARGER: f32 = 1.25;
/// How much of it shows over the page's background.
const OPACITY: f32 = 0.55;

/// Keeps `frame`'s glow following the video while it plays there.
pub fn follow(frame: &AspectBin, slot: &'static str) {
    let weak = frame.downgrade();
    glib::timeout_add_local(EVERY, move || {
        let Some(frame) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        // Paused, the picture stays: so does the glow.
        if frame.has_backdrop() && app().player.state().paused && app().settings.get().ambient {
            return glib::ControlFlow::Continue;
        }
        let glow = app()
            .settings
            .get()
            .ambient
            .then(|| frame.is_mapped().then(|| here(slot)).flatten())
            .flatten()
            .and_then(|frame_texture| glow(&frame, &frame_texture));
        let spread = LARGER * (PICTURE.0 + 2.0 * MARGIN) / PICTURE.0;
        frame.set_backdrop(glow, spread, OPACITY);
        glib::ControlFlow::Continue
    });
}

/// The frame on screen, if the video plays in `slot` (not paused there
/// since the last update: the glow would be the same).
fn here(slot: &str) -> Option<gdk::Texture> {
    if super::player_dock::current().as_deref() != Some(slot) {
        return None;
    }
    app().player.state().video_id?;
    let player = super::player_dock::player();
    super::find_widget(&player, "video")
        .and_downcast::<super::video::VideoView>()?
        .frame()
}

/// The picture (the frame without its black bars) drawn small, stretched
/// to the player's shape and blurred into clear margins.
fn glow(widget: &AspectBin, frame: &gdk::Texture) -> Option<gdk::Texture> {
    let renderer = widget.native()?.renderer()?;
    let (w, h) = (PICTURE.0 + 2.0 * MARGIN, PICTURE.1 + 2.0 * MARGIN);
    // Where the picture is in the frame: mpv bars it to the widget's shape.
    let (fw, fh) = (frame.width() as f32, frame.height() as f32);
    let aspect = app().player.video_aspect().unwrap_or(f64::from(fw / fh)) as f32;
    let (pw, ph) = if aspect < fw / fh {
        (fh * aspect, fh)
    } else {
        (fw, fw / aspect)
    };
    let (sx, sy) = (PICTURE.0 / pw, PICTURE.1 / ph);
    let target = graphene::Rect::new(MARGIN, MARGIN, PICTURE.0, PICTURE.1);
    let snapshot = gtk::Snapshot::new();
    snapshot.push_blur(BLUR);
    snapshot.push_clip(&target);
    snapshot.append_texture(
        frame,
        &graphene::Rect::new(
            MARGIN - (fw - pw) / 2.0 * sx,
            MARGIN - (fh - ph) / 2.0 * sy,
            fw * sx,
            fh * sy,
        ),
    );
    snapshot.pop();
    snapshot.pop();
    let node = snapshot.to_node()?;
    let texture = renderer.render_texture(&node, Some(&graphene::Rect::new(0.0, 0.0, w, h)));
    Some(texture)
}
