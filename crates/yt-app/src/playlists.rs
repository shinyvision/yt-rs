//! Playlists and what YouTube offers to do with a video: saving it to
//! Watch later or a playlist, taking it out again, making and deleting
//! playlists, and the menu's own actions ("Not interested", "Don't
//! recommend channel", "Remove from Liked videos").
//!
//! Like the social actions, the commands return at once and report how it
//! went with an event, so every page showing the playlist or the video can
//! follow: `playlist.changed` `{ playlist_id, video_id, added }`,
//! `playlist.created` `{ playlist_id, title }`, `playlist.deleted`
//! `{ playlist_id }`, `video.hidden` `{ video_id }` (YouTube was told it
//! isn't wanted), or `<event>.failed` with an `error`.

use std::rc::Rc;

use serde_json::{Value, json};
use yt_core::actions::{ActionKind, VideoAction};
use yt_core::playlist::{LIKED, Privacy, WATCH_LATER};

use crate::app::{App, app};
use crate::commands::str_arg;
use crate::social::needs_account;
use crate::social::{app_youtube, outcome, run};

#[yt_export::export(skip = "registers the built-in commands")]
pub fn register(this: &Rc<App>) {
    let c = &this.ext.commands;
    c.register("playlist.open", |args| {
        let playlist_id = str_arg(&args, "playlist_id")?;
        app().navigate("playlist", json!({ "playlist_id": playlist_id }), true);
        Ok(Value::Null)
    });
    c.register(
        "video.save",
        needs_account("Sign in to add this video to a playlist", move |args| {
            crate::ui::save_dialog::present(&video_id(&args)?);
            Ok(Value::Null)
        }),
    );
    c.register(
        "video.save_to_watch_later",
        needs_account("Sign in to add this video to a playlist", move |args| {
            let video_id = video_id(&args)?;
            edit(WATCH_LATER, &video_id, true, None, true);
            Ok(Value::Null)
        }),
    );
    c.register(
        "playlist.add",
        needs_account("Sign in to add this video to a playlist", move |args| {
            let playlist_id = str_arg(&args, "playlist_id")?.to_owned();
            let video_id = video_id(&args)?;
            let title = args.get("title").and_then(Value::as_str).map(str::to_owned);
            edit(
                &playlist_id,
                &video_id,
                true,
                title,
                args.get("undo").and_then(Value::as_bool).unwrap_or(true),
            );
            Ok(Value::Null)
        }),
    );
    c.register(
        "playlist.remove",
        needs_account("Sign in to change your playlists", move |args| {
            let playlist_id = str_arg(&args, "playlist_id")?.to_owned();
            let video_id = video_id(&args)?;
            let title = args.get("title").and_then(Value::as_str).map(str::to_owned);
            edit(
                &playlist_id,
                &video_id,
                false,
                title,
                args.get("undo").and_then(Value::as_bool).unwrap_or(true),
            );
            Ok(Value::Null)
        }),
    );
    c.register(
        "playlist.create",
        needs_account("Sign in to make playlists", move |args| {
            let title = str_arg(&args, "title")?.trim().to_owned();
            if title.is_empty() {
                return Err("a playlist needs a title".into());
            }
            let privacy: Privacy = args
                .get("privacy")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| "privacy is private, unlisted or public")?
                .unwrap_or(Privacy::Private);
            let video_id = args
                .get("video_id")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let name = title.clone();
            run(
                async move {
                    app_youtube(move |y| async move {
                        y.create_playlist(&name, privacy, video_id.as_deref()).await
                    })
                    .await
                },
                move |result| {
                    if result.is_ok() {
                        app().toast(&format!("Created \u{201c}{title}\u{201d}"));
                    }
                    outcome(
                        "playlist.created",
                        json!({ "title": title }),
                        result.map(|id| json!({ "playlist_id": id })),
                    );
                },
            );
            Ok(Value::Null)
        }),
    );
    c.register("playlist.delete", needs_account("Sign in to change your playlists", move |args| {
        let playlist_id = str_arg(&args, "playlist_id")?.to_owned();
        if [WATCH_LATER, LIKED].contains(&playlist_id.as_str()) {
            return Err("this list can't be deleted".into());
        }
        let id = playlist_id.clone();
        run(
            async move { app_youtube(move |y| async move { y.delete_playlist(&id).await }).await },
            move |result| {
                if result.is_ok() {
                    app().toast("Playlist deleted");
                }
                outcome(
                    "playlist.deleted",
                    json!({ "playlist_id": playlist_id }),
                    result.map(|()| json!({})),
                );
            },
        );
        Ok(Value::Null)
    }));
    // One of the video's own menu actions (its `actions`).
    c.register(
        "video.action",
        needs_account("Sign in to change what YouTube recommends", move |args| {
            let video_id = video_id(&args)?;
            let action: VideoAction = args
                .get("action")
                .cloned()
                .ok_or("no action")
                .and_then(|a| serde_json::from_value(a).map_err(|_| "not a video action"))?;
            act(&video_id, action);
            Ok(Value::Null)
        }),
    );
}

/// `{ video_id }`, or `{ video = { id } }` as the video menu passes it.
fn video_id(args: &Value) -> Result<String, String> {
    args.get("video_id")
        .or_else(|| args.pointer("/video/id"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "no video_id".into())
}

/// What a playlist is called in a message.
fn name(playlist_id: &str, title: Option<&str>) -> String {
    match (playlist_id, title) {
        (WATCH_LATER, _) => "Watch later".into(),
        (LIKED, _) => "Liked videos".into(),
        (_, Some(title)) => title.into(),
        _ => "the playlist".into(),
    }
}

/// Adds the video to the playlist or takes it out, then says so, with a
/// way to undo it if `undo`.
fn edit(playlist_id: &str, video_id: &str, add: bool, title: Option<String>, undo: bool) {
    let (playlist, video) = (playlist_id.to_owned(), video_id.to_owned());
    let (p, v) = (playlist.clone(), video.clone());
    run(
        async move {
            app_youtube(move |y| async move {
                if add {
                    y.add_to_playlist(&p, &v).await
                } else {
                    y.remove_from_playlist(&p, &v).await
                }
            })
            .await
        },
        move |result| {
            if result.is_ok() {
                said(&playlist, &video, add, title.as_deref(), undo);
            }
            outcome(
                "playlist.changed",
                json!({ "playlist_id": playlist, "video_id": video, "added": add }),
                result.map(|()| json!({})),
            );
        },
    );
}

/// "Saved to Watch later", with "Undo".
fn said(playlist_id: &str, video_id: &str, added: bool, title: Option<&str>, undo: bool) {
    let message = format!(
        "{} {}",
        if added { "Saved to" } else { "Removed from" },
        name(playlist_id, title)
    );
    let mut toast = json!({ "message": message });
    if undo {
        toast["action"] = json!({
            "label": "Undo",
            "command": if added { "playlist.remove" } else { "playlist.add" },
            "args": { "playlist_id": playlist_id, "video_id": video_id, "title": title, "undo": false },
        });
    }
    app().toast_with(&toast);
}

/// Runs one of YouTube's menu actions for the video.
fn act(video_id: &str, action: VideoAction) {
    let video = video_id.to_owned();
    match action.kind {
        ActionKind::EditPlaylist {
            playlist_id,
            actions,
            params,
        } => {
            let added = actions
                .pointer("/0/action")
                .and_then(Value::as_str)
                .is_some_and(|a| a == "ACTION_ADD_VIDEO");
            let p = playlist_id.clone();
            run(
                async move {
                    app_youtube(move |y| async move {
                        y.edit_playlist(&p, actions, params.as_deref()).await
                    })
                    .await
                },
                move |result| {
                    if result.is_ok() {
                        // Taking it out by its place in the list can be
                        // undone by adding it back (at the end).
                        said(&playlist_id, &video, added, None, true);
                    }
                    outcome(
                        "playlist.changed",
                        json!({ "playlist_id": playlist_id, "video_id": video, "added": added }),
                        result.map(|()| json!({})),
                    );
                },
            );
        }
        ActionKind::SaveToPlaylist => {
            crate::ui::save_dialog::present(video_id);
        }
        ActionKind::Feedback { token, .. } => {
            let label = action.label;
            run(
                async move { app_youtube(move |y| async move { y.feedback(&token).await }).await },
                move |result| {
                    if result.is_ok() {
                        let label = label.to_lowercase();
                        app().toast(if label.contains("history") {
                            "Removed from watch history"
                        } else if label.contains("channel") {
                            "You won't see this channel in your recommendations"
                        } else {
                            "Video removed. You'll see fewer like it."
                        });
                    }
                    outcome(
                        "video.hidden",
                        json!({ "video_id": video }),
                        result.map(|()| json!({})),
                    );
                },
            );
        }
        ActionKind::Unlike => {
            let args = json!({ "video_id": video_id, "rating": "none" });
            if let Err(e) = app().ext.commands.run("video.rate", args) {
                app().toast(&e);
                return;
            }
            app().ext.events.emit(
                "playlist.changed",
                &json!({ "playlist_id": LIKED, "video_id": video_id, "added": false }),
            );
        }
    }
}
