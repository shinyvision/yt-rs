//! The system's media controls: media keys, headset buttons, and the
//! desktop's player widget, which shows what plays (title, channel,
//! picture, position) and controls it (play, pause, next, previous, seek,
//! volume, speed, loop, shuffle). On Linux that is MPRIS over D-Bus; macOS
//! (`MPNowPlayingInfoCenter`) and Windows (System Media Transport Controls)
//! have their own, not done yet.
//!
//! Everything goes through the app's commands, so the controls do what the
//! player's buttons do.

#[yt_export::export(skip = "sets the app up")]
pub fn init() {
    #[cfg(target_os = "linux")]
    mpris::start();
}

#[cfg(target_os = "linux")]
mod mpris {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use gtk::gio;
    use gtk::glib::{self, Variant, variant::ToVariant};
    use gtk::prelude::*;
    use serde_json::{Value, json};

    use crate::app::app;
    use crate::player::PlayerEvent;

    const PATH: &str = "/org/mpris/MediaPlayer2";
    const ROOT: &str = "org.mpris.MediaPlayer2";
    const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
    /// The rates YouTube offers.
    const MIN_RATE: f64 = 0.25;
    const MAX_RATE: f64 = 2.0;

    const XML: &str = r#"<node>
  <interface name="org.mpris.MediaPlayer2">
    <method name="Raise"/>
    <method name="Quit"/>
    <property name="CanQuit" type="b" access="read"/>
    <property name="CanRaise" type="b" access="read"/>
    <property name="HasTrackList" type="b" access="read"/>
    <property name="Identity" type="s" access="read"/>
    <property name="DesktopEntry" type="s" access="read"/>
    <property name="SupportedUriSchemes" type="as" access="read"/>
    <property name="SupportedMimeTypes" type="as" access="read"/>
  </interface>
  <interface name="org.mpris.MediaPlayer2.Player">
    <method name="Next"/>
    <method name="Previous"/>
    <method name="Pause"/>
    <method name="PlayPause"/>
    <method name="Stop"/>
    <method name="Play"/>
    <method name="Seek"><arg name="Offset" type="x" direction="in"/></method>
    <method name="SetPosition">
      <arg name="TrackId" type="o" direction="in"/>
      <arg name="Position" type="x" direction="in"/>
    </method>
    <method name="OpenUri"><arg name="Uri" type="s" direction="in"/></method>
    <signal name="Seeked"><arg name="Position" type="x"/></signal>
    <property name="PlaybackStatus" type="s" access="read"/>
    <property name="LoopStatus" type="s" access="readwrite"/>
    <property name="Rate" type="d" access="readwrite"/>
    <property name="Shuffle" type="b" access="readwrite"/>
    <property name="Metadata" type="a{sv}" access="read"/>
    <property name="Volume" type="d" access="readwrite"/>
    <property name="Position" type="x" access="read"/>
    <property name="MinimumRate" type="d" access="read"/>
    <property name="MaximumRate" type="d" access="read"/>
    <property name="CanGoNext" type="b" access="read"/>
    <property name="CanGoPrevious" type="b" access="read"/>
    <property name="CanPlay" type="b" access="read"/>
    <property name="CanPause" type="b" access="read"/>
    <property name="CanSeek" type="b" access="read"/>
    <property name="CanControl" type="b" access="read"/>
  </interface>
</node>"#;

    /// What plays, as the watch page knows it.
    #[derive(Default, Clone)]
    struct Track {
        video_id: String,
        title: String,
        channel: Option<String>,
    }

    thread_local! {
        static CONNECTION: RefCell<Option<gio::DBusConnection>> = const { RefCell::new(None) };
        static TRACK: RefCell<Option<Track>> = const { RefCell::new(None) };
    }

    pub fn start() {
        // The usual name for the app; another app ID (a test, a second
        // profile) gets its own, so it doesn't take the desktop's player
        // from the one in use.
        let id = app().gtk.application_id().unwrap_or_default();
        let name = if id == crate::app::APP_ID {
            "org.mpris.MediaPlayer2.yt_rs".to_owned()
        } else {
            format!(
                "org.mpris.MediaPlayer2.yt_rs.instance{}",
                std::process::id()
            )
        };
        gio::bus_own_name(
            gio::BusType::Session,
            &name,
            gio::BusNameOwnerFlags::DO_NOT_QUEUE,
            |connection, _| register(connection),
            |_, _| {},
            |_, name| tracing::info!("media controls: {name} is taken; not showing there"),
        );
        follow();
    }

    fn register(connection: gio::DBusConnection) {
        let node = match gio::DBusNodeInfo::for_xml(XML) {
            Ok(node) => node,
            Err(e) => return tracing::warn!("media controls: {e}"),
        };
        for interface in [ROOT, PLAYER] {
            let Some(info) = node.lookup_interface(interface) else {
                continue;
            };
            let result = connection
                .register_object(PATH, &info)
                .method_call(|_, _, _, interface, method, params, invocation| {
                    let reply = call(interface.unwrap_or_default(), method, &params);
                    match reply {
                        Ok(()) => invocation.return_value(None),
                        Err(e) => invocation.return_dbus_error("org.mpris.MediaPlayer2.Error", &e),
                    }
                })
                .property(|_, _, _, interface, property| get(interface, property))
                .set_property(|_, _, _, _, property, value| set(property, &value))
                .build();
            if let Err(e) = result {
                tracing::warn!("media controls: {e}");
            }
        }
        CONNECTION.with(|c| *c.borrow_mut() = Some(connection));
    }

    /// Announces property changes as the player, the list and the video
    /// change.
    fn follow() {
        let app = app();
        app.player.connect(|event, state| match event {
            PlayerEvent::Paused(_) | PlayerEvent::Ended | PlayerEvent::Stopped => {
                changed(&["PlaybackStatus", "CanPlay", "CanPause", "CanSeek"]);
            }
            PlayerEvent::Loaded | PlayerEvent::Duration(_) => {
                changed(&[
                    "PlaybackStatus",
                    "Metadata",
                    "CanPlay",
                    "CanPause",
                    "CanSeek",
                    "CanGoNext",
                ]);
            }
            PlayerEvent::Volume { .. } => changed(&["Volume"]),
            PlayerEvent::Speed(_) => changed(&["Rate"]),
            PlayerEvent::Settings => changed(&["LoopStatus"]),
            PlayerEvent::Seeked { position } => seeked(*position),
            _ => {
                let _ = state;
            }
        });
        app.ext.events.on("video.opened", |payload| {
            let video = payload.get("video");
            let text = |key: &str| {
                video
                    .and_then(|v| v.get(key))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            };
            TRACK.with(|t| {
                *t.borrow_mut() = Some(Track {
                    video_id: text("id").unwrap_or_default(),
                    title: text("title").unwrap_or_default(),
                    channel: text("channel"),
                })
            });
            changed(&["Metadata", "CanGoNext", "CanGoPrevious"]);
        });
        app.ext.events.on("watch.loaded", |payload| {
            let page = payload.get("page");
            let text = |pointer: &str| {
                page.and_then(|p| p.pointer(pointer))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            };
            TRACK.with(|t| {
                if let Some(track) = t.borrow_mut().as_mut()
                    && payload.get("video_id").and_then(Value::as_str)
                        == Some(track.video_id.as_str())
                {
                    if let Some(title) = text("/title") {
                        track.title = title;
                    }
                    track.channel = text("/owner/name").or(track.channel.take());
                }
            });
            changed(&["Metadata", "CanGoNext"]);
        });
        app.ext.events.on("list.changed", |_| {
            changed(&["Shuffle", "LoopStatus", "CanGoNext", "CanGoPrevious"]);
        });
    }

    fn call(interface: &str, method: &str, params: &Variant) -> Result<(), String> {
        let run = |command: &str, args: Value| app().ext.commands.run(command, args).map(drop);
        match (interface, method) {
            (ROOT, "Raise") => {
                if let Some(window) = app().window() {
                    window.present();
                }
                Ok(())
            }
            (ROOT, "Quit") => {
                app().gtk.quit();
                Ok(())
            }
            (PLAYER, "Next") => run("player.next", Value::Null),
            (PLAYER, "Previous") => {
                if crate::lists::current().is_some_and(|l| l.position > 0) {
                    run("list.previous", Value::Null)
                } else {
                    run("player.seek", json!({ "position": 0 }))
                }
            }
            (PLAYER, "Pause") => run("player.pause", Value::Null),
            (PLAYER, "Play") => run("player.play", Value::Null),
            (PLAYER, "PlayPause") => run("player.toggle_pause", Value::Null),
            (PLAYER, "Stop") => run("player.stop", Value::Null),
            (PLAYER, "Seek") => {
                let (offset,) = params.get::<(i64,)>().ok_or("Seek takes an offset")?;
                run("player.seek_by", json!({ "seconds": offset as f64 / 1e6 }))
            }
            (PLAYER, "SetPosition") => {
                let (track, position) = params
                    .get::<(glib::variant::ObjectPath, i64)>()
                    .ok_or("SetPosition takes a track and a position")?;
                // Only for the track playing, as the spec says.
                if track.as_str() != track_id().as_str() || position < 0 {
                    return Ok(());
                }
                run("player.seek", json!({ "position": position as f64 / 1e6 }))
            }
            (PLAYER, "OpenUri") => {
                let (uri,) = params.get::<(String,)>().ok_or("OpenUri takes a URI")?;
                match yt_core::text::parse_url(&uri) {
                    Some(yt_core::text::Link::Video { video_id, start }) => run(
                        "video.play",
                        json!({ "video": { "id": video_id }, "start": start }),
                    ),
                    Some(yt_core::text::Link::Playlist { playlist_id }) => {
                        run("playlist.play", json!({ "playlist_id": playlist_id }))
                    }
                    _ => Err("not a YouTube video or playlist".into()),
                }
            }
            _ => Err(format!("{interface}.{method} isn't supported")),
        }
    }

    fn get(interface: &str, property: &str) -> Variant {
        let app = app();
        let state = app.player.state();
        let list = crate::lists::current();
        let loaded = state.video_id.is_some();
        match (interface, property) {
            (ROOT, "CanQuit") | (ROOT, "CanRaise") => true.to_variant(),
            (ROOT, "HasTrackList") => false.to_variant(),
            (ROOT, "Identity") => "yt-rs".to_variant(),
            (ROOT, "DesktopEntry") => app.gtk.application_id().unwrap_or_default().to_variant(),
            (ROOT, "SupportedUriSchemes") => vec!["https"].to_variant(),
            (ROOT, "SupportedMimeTypes") => Vec::<String>::new().to_variant(),
            (_, "PlaybackStatus") => match (loaded, state.paused || state.ended) {
                (false, _) => "Stopped",
                (true, true) => "Paused",
                (true, false) => "Playing",
            }
            .to_variant(),
            (_, "LoopStatus") => if state.looping {
                "Track"
            } else if list.as_ref().is_some_and(|l| l.looping) {
                "Playlist"
            } else {
                "None"
            }
            .to_variant(),
            (_, "Rate") => state.speed.to_variant(),
            (_, "Shuffle") => list.as_ref().is_some_and(|l| l.shuffled).to_variant(),
            (_, "Metadata") => metadata(&state).to_variant(),
            (_, "Volume") => (state.volume / 100.0).clamp(0.0, 1.0).to_variant(),
            (_, "Position") => ((state.position * 1e6) as i64).to_variant(),
            (_, "MinimumRate") => MIN_RATE.to_variant(),
            (_, "MaximumRate") => MAX_RATE.to_variant(),
            (_, "CanGoNext") => crate::playback::up_next().is_some().to_variant(),
            (_, "CanGoPrevious") => loaded.to_variant(),
            (_, "CanPlay") | (_, "CanPause") => loaded.to_variant(),
            (_, "CanSeek") => {
                (loaded && (state.duration > 0.0 || state.live.is_some())).to_variant()
            }
            (_, "CanControl") => true.to_variant(),
            _ => Variant::from_none(glib::VariantTy::VARIANT),
        }
    }

    fn set(property: &str, value: &Variant) -> bool {
        let run = |command: &str, args: Value| app().ext.commands.run(command, args).is_ok();
        match property {
            "Volume" => value.get::<f64>().is_some_and(|v| {
                run(
                    "player.set_volume",
                    json!({ "volume": v.clamp(0.0, 1.0) * 100.0 }),
                )
            }),
            "Rate" => value.get::<f64>().filter(|r| *r > 0.0).is_some_and(|r| {
                run(
                    "player.set_speed",
                    json!({ "speed": r.clamp(MIN_RATE, MAX_RATE) }),
                )
            }),
            "Shuffle" => value
                .get::<bool>()
                .is_some_and(|s| run("list.set_shuffled", json!({ "shuffled": s }))),
            "LoopStatus" => match value.get::<String>().as_deref() {
                Some("Track") => run("player.set_looping", json!({ "looping": true })),
                Some("Playlist") => {
                    run("player.set_looping", json!({ "looping": false }))
                        && run("list.set_looping", json!({ "looping": true }))
                }
                Some("None") => {
                    run("player.set_looping", json!({ "looping": false }))
                        && run("list.set_looping", json!({ "looping": false }))
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// The track's ID: its video ID, in letters D-Bus allows in a path.
    fn track_id() -> glib::variant::ObjectPath {
        let id = TRACK.with(|t| t.borrow().as_ref().map(|t| t.video_id.clone()));
        let path = match id.filter(|id| !id.is_empty()) {
            Some(id) => format!(
                "/dev/ytrs/video/v{}",
                id.bytes().map(|b| format!("{b:02x}")).collect::<String>()
            ),
            None => "/org/mpris/MediaPlayer2/TrackList/NoTrack".to_owned(),
        };
        glib::variant::ObjectPath::try_from(path).expect("letters, digits and slashes")
    }

    fn metadata(state: &crate::player::PlayerState) -> HashMap<String, Variant> {
        let mut map = HashMap::new();
        let Some(track) = TRACK
            .with(|t| t.borrow().clone())
            .filter(|_| state.video_id.is_some())
        else {
            map.insert("mpris:trackid".into(), track_id().to_variant());
            return map;
        };
        map.insert("mpris:trackid".into(), track_id().to_variant());
        if state.duration > 0.0 && state.live.is_none() {
            map.insert(
                "mpris:length".into(),
                ((state.duration * 1e6) as i64).to_variant(),
            );
        }
        map.insert(
            "mpris:artUrl".into(),
            format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", track.video_id).to_variant(),
        );
        map.insert("xesam:title".into(), track.title.to_variant());
        if let Some(channel) = &track.channel {
            map.insert("xesam:artist".into(), vec![channel.as_str()].to_variant());
        }
        map.insert(
            "xesam:url".into(),
            format!("https://www.youtube.com/watch?v={}", track.video_id).to_variant(),
        );
        map
    }

    /// `PropertiesChanged` for these Player properties.
    fn changed(properties: &[&str]) {
        CONNECTION.with(|c| {
            let Some(connection) = c.borrow().clone() else {
                return;
            };
            let values: HashMap<String, Variant> = properties
                .iter()
                .map(|p| ((*p).to_owned(), get(PLAYER, p)))
                .collect();
            let params = (PLAYER, values, Vec::<String>::new()).to_variant();
            if let Err(e) = connection.emit_signal(
                None,
                PATH,
                "org.freedesktop.DBus.Properties",
                "PropertiesChanged",
                Some(&params),
            ) {
                tracing::debug!("media controls: {e}");
            }
        });
    }

    fn seeked(position: f64) {
        CONNECTION.with(|c| {
            if let Some(connection) = c.borrow().as_ref() {
                let params = ((position * 1e6) as i64,).to_variant();
                let _ = connection.emit_signal(None, PATH, PLAYER, "Seeked", Some(&params));
            }
        });
    }
}
