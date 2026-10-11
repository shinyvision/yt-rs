//! Watch details, comments, channels, subscriptions and ratings: the loading
//! functions the UI uses, and the commands for every action.
//!
//! Actions are network calls, so their commands return right away and report
//! the outcome as events: `comment.posted`, `comment.deleted`,
//! `comment.voted`, `channel.subscription`, `video.rated`, or
//! `*.failed` with an `error`. Every widget showing the same comment or
//! channel updates from the event, and plugins see it too.

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::rc::Rc;

use gtk::prelude::*;

use serde_json::{Value, json};
use yt_core::account::Account;
use yt_core::channel::{ChannelPage, ChannelRef, Subscription};
use yt_core::comments::{Comment, CommentThread, CommentsPage, Vote, VoteActions};
use yt_core::innertube::Rating;
use yt_core::text::{Link, RichText};
use yt_core::watch::WatchPage;

use crate::app::{App, app};
use crate::commands::str_arg;
use crate::runtime;

thread_local! {
    /// Vote tokens of comments whose thread was loaded, by comment ID.
    static VOTES: RefCell<HashMap<String, VoteActions>> = RefCell::default();
    static ACCOUNT: RefCell<Option<Account>> = const { RefCell::new(None) };
    /// Whether a user is signed in (whether or not their account has
    /// loaded yet).
    static SIGNED_IN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The @handle of each watched video's channel, by video ID.
    static OWNERS: RefCell<HashMap<String, String>> = RefCell::default();
}

/// Whether a user is signed in: what needs an account shows, or a way to
/// sign in instead.
pub fn signed_in() -> bool {
    SIGNED_IN.with(std::cell::Cell::get)
}

/// A command that needs an account. Signed out, it offers to sign in
/// instead (`why`, with "Sign in", as youtube.com asks) and answers
/// `{ signed_in = false }`.
pub fn needs_account(
    why: &'static str,
    command: impl Fn(Value) -> Result<Value, String> + 'static,
) -> impl Fn(Value) -> Result<Value, String> + 'static {
    move |args| {
        if signed_in() {
            return command(args);
        }
        offer_sign_in(why);
        Ok(json!({ "signed_in": false }))
    }
}

/// A toast saying why to sign in, with "Sign in".
pub fn offer_sign_in(why: &str) {
    app().toast_with(&json!({
        "message": why,
        "action": { "label": "Sign in", "command": "app.navigate", "args": { "page": "signin" } },
    }));
}

pub fn register(this: &Rc<App>) {
    let c = &this.ext.commands;
    c.register("account.info", |_| {
        Ok(ACCOUNT.with(|a| serde_json::to_value(&*a.borrow()).unwrap_or_default()))
    });
    c.register("app.open_url", |args| {
        let url = str_arg(&args, "url")?;
        gtk::UriLauncher::new(url).launch(
            app().window().as_ref(),
            gtk::gio::Cancellable::NONE,
            |_| {},
        );
        Ok(Value::Null)
    });
    c.register("link.open", |args| {
        let link: Link = serde_json::from_value(args.get("link").cloned().unwrap_or(args))
            .map_err(|e| format!("invalid link: {e}"))?;
        open_link(&link)
    });
    // { channel_id | handle | name | video_id }: whatever is known.
    c.register("channel.open", |args| {
        let key = ChannelKey::from_args(&args);
        if key.is_empty() {
            return Err("no channel given".into());
        }
        app().navigate("channel", key.to_args(), true);
        Ok(Value::Null)
    });
    c.register("channels.subscriptions", |_| {
        Ok(serde_json::to_value(app().channels.subscriptions()).unwrap_or_default())
    });
    c.register("channels.lookup", |args| {
        Ok(serde_json::to_value(app().channels.lookup(str_arg(&args, "key")?)).unwrap_or_default())
    });
    SIGNED_IN.with(|s| s.set(crate::runtime::block_on(this.session.is_signed_in())));
    this.ext
        .events
        .on("auth.signed_in", |_| SIGNED_IN.with(|s| s.set(true)));
    this.ext.events.on("auth.signed_out", |_| {
        SIGNED_IN.with(|s| s.set(false));
        ACCOUNT.with(|a| a.borrow_mut().take());
        VOTES.with(|v| v.borrow_mut().clear());
    });
    // The sidebar and avatars follow the user's subscriptions.
    this.ext
        .events
        .on("channel.subscription", |_| app().channels.refresh());
    // The comments section follows the event, so a plugin can sort too.
    c.register("comments.sort", |args| {
        app().ext.events.emit("comments.sort_selected", &args);
        Ok(Value::Null)
    });
    c.register(
        "channel.subscribe",
        needs_account("Sign in to subscribe to this channel", |args| {
            set_subscribed(&args, true)
        }),
    );
    c.register(
        "channel.unsubscribe",
        needs_account("Sign in to subscribe to this channel", |args| {
            set_subscribed(&args, false)
        }),
    );
    c.register(
        "video.rate",
        needs_account("Sign in to make your opinion count", move |args| {
            let video_id = str_arg(&args, "video_id")?.to_owned();
            let rating = match args.get("rating").and_then(Value::as_str).unwrap_or("like") {
                "like" => Rating::Like,
                "dislike" => Rating::Dislike,
                _ => Rating::None,
            };
            let name = args
                .get("rating")
                .and_then(Value::as_str)
                .unwrap_or("like")
                .to_owned();
            let id = video_id.clone();
            run(
                async move {
                    app_youtube(move |y| async move { y.rate_video(&id, rating).await }).await
                },
                move |result| {
                    outcome(
                        "video.rated",
                        json!({ "video_id": video_id, "rating": name }),
                        result.map(|()| json!({})),
                    )
                },
            );
            Ok(Value::Null)
        }),
    );
    c.register(
        "comment.post",
        needs_account("Sign in to comment", move |args| {
            let video_id = str_arg(&args, "video_id")?.to_owned();
            let text = comment_text(&args)?;
            let (id, typed) = (video_id.clone(), text.clone());
            run(
                async move {
                    app_youtube(move |y| async move { y.post_comment(&id, &text).await }).await
                },
                move |result| {
                    outcome(
                        "comment.posted",
                        json!({ "video_id": video_id, "parent_id": null }),
                        result.map(|c| json!({ "comment": posted(c, &typed) })),
                    )
                },
            );
            Ok(Value::Null)
        }),
    );
    c.register(
        "comment.reply",
        needs_account("Sign in to comment", move |args| {
            let video_id = str_arg(&args, "video_id")?.to_owned();
            let parent_id = str_arg(&args, "parent_id")?.to_owned();
            let text = comment_text(&args)?;
            let (id, parent, typed) = (video_id.clone(), parent_id.clone(), text.clone());
            run(
                async move {
                    app_youtube(
                        move |y| async move { y.reply_to_comment(&id, &parent, &text).await },
                    )
                    .await
                },
                move |result| {
                    outcome(
                        "comment.posted",
                        json!({ "video_id": video_id, "parent_id": parent_id }),
                        result.map(|c| json!({ "comment": posted(c, &typed) })),
                    )
                },
            );
            Ok(Value::Null)
        }),
    );
    c.register("comment.copy", |args| {
        let comment: yt_core::comments::Comment =
            serde_json::from_value(args.get("comment").cloned().unwrap_or_default())
                .map_err(|e| e.to_string())?;
        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(&comment.text.plain());
            app().toast("Comment copied");
        }
        Ok(Value::Null)
    });
    c.register(
        "comment.delete",
        needs_account("Sign in to manage your comments", move |args| {
            let video_id = str_arg(&args, "video_id")?.to_owned();
            let comment_id = str_arg(&args, "comment_id")?.to_owned();
            let (id, comment) = (video_id.clone(), comment_id.clone());
            run(
                async move {
                    app_youtube(move |y| async move { y.delete_comment(&id, &comment).await }).await
                },
                move |result| {
                    outcome(
                        "comment.deleted",
                        json!({ "video_id": video_id, "comment_id": comment_id }),
                        result.map(|()| json!({})),
                    )
                },
            );
            Ok(Value::Null)
        }),
    );
    // { video_id, comment_id, thread, from: "none"|"like"|"dislike", to: ... }
    c.register(
        "comment.vote",
        needs_account("Sign in to make your opinion count", move |args| {
            let video_id = str_arg(&args, "video_id")?.to_owned();
            let comment_id = str_arg(&args, "comment_id")?.to_owned();
            let thread = args
                .get("thread")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let vote = |key: &str| {
                serde_json::from_value::<Vote>(args.get(key).cloned().unwrap_or(json!("none")))
                    .map_err(|e| e.to_string())
            };
            let (from, to) = (vote("from")?, vote("to")?);
            let comment = comment_id.clone();
            run(
                async move {
                    let actions = match VOTES.with(|v| v.borrow().get(&comment).cloned()) {
                        Some(actions) => actions,
                        None => {
                            let thread = thread.ok_or("this comment can't be voted on")?;
                            let id = video_id.clone();
                            let loaded = app_youtube(move |y| async move {
                                y.comment_thread(&thread, &id).await
                            })
                            .await?;
                            VOTES.with(|v| {
                                v.borrow_mut()
                                    .insert(comment.clone(), loaded.actions.clone())
                            });
                            loaded.actions
                        }
                    };
                    let token = actions
                        .token(from, to)
                        .ok_or("YouTube offers no way to change this vote")?
                        .to_owned();
                    app_youtube(move |y| async move { y.comment_action(&token).await }).await
                },
                move |result| {
                    let payload = json!({ "comment_id": comment_id, "vote": to, "from": from });
                    match result {
                        Ok(()) => app().ext.events.emit("comment.voted", &payload),
                        Err(error) => failed("comment.vote", &error, payload),
                    }
                },
            );
            Ok(Value::Null)
        }),
    );
}

/// Loads the signed-in account (for the top bar and "is this my comment").
pub fn load_account() {
    runtime::spawn_local(async {
        let youtube = app().youtube.clone();
        match runtime::background(async move { youtube.account().await }).await {
            Ok(Some(account)) => {
                ACCOUNT.with(|a| *a.borrow_mut() = Some(account.clone()));
                app()
                    .ext
                    .events
                    .emit("account.loaded", &json!({ "account": account }));
            }
            Ok(None) => {}
            Err(e) => tracing::warn!("couldn't load the account: {e}"),
        }
    });
}

pub fn account() -> Option<Account> {
    ACCOUNT.with(|a| a.borrow().clone())
}

/// Whether a comment author is the signed-in user (the TV client doesn't say,
/// so this compares handles).
pub fn is_mine(author_handle: Option<&str>) -> bool {
    let mine = account().and_then(|a| a.handle);
    matches!((mine, author_handle), (Some(m), Some(h)) if m.eq_ignore_ascii_case(h))
}

/// Whether a comment on a video is by the video's own channel.
pub fn is_by_creator(video_id: &str, author_handle: Option<&str>) -> bool {
    OWNERS.with(|o| matches!((o.borrow().get(video_id), author_handle), (Some(owner), Some(author)) if owner.eq_ignore_ascii_case(author)))
}

/// A video's watch details, run through the `watch.page` filter, then
/// announced as `watch.loaded`.
pub async fn load_watch(video_id: &str) -> Result<WatchPage, String> {
    let id = video_id.to_owned();
    let page = app_youtube(move |y| async move { y.watch(&id).await }).await?;
    let page: WatchPage = filtered("watch.page", page)?;
    let directory = &app().channels;
    if let Some(owner) = &page.owner {
        directory.remember(owner);
        let handle = owner
            .handle
            .clone()
            .or_else(|| directory.lookup(owner.id.as_deref()?)?.handle);
        if let Some(handle) = handle {
            OWNERS.with(|o| {
                let mut owners = o.borrow_mut();
                if owners.len() > 200 {
                    owners.clear();
                }
                owners.insert(video_id.to_owned(), handle);
            });
        }
    }
    app().ext.events.emit(
        "watch.loaded",
        &json!({ "video_id": video_id, "page": page }),
    );
    Ok(page)
}

/// A page of comments, run through `comments.page`, announced as `comments.loaded`.
pub async fn load_comments(video_id: &str, token: &str) -> Result<CommentsPage, String> {
    let (id, token) = (video_id.to_owned(), token.to_owned());
    let page = app_youtube(move |y| async move { y.comments(&token, &id).await }).await?;
    let page: CommentsPage = filtered("comments.page", page)?;
    for comment in &page.comments {
        app().channels.remember(&comment.author);
    }
    app().ext.events.emit(
        "comments.loaded",
        &json!({ "video_id": video_id, "page": page }),
    );
    Ok(page)
}

/// A page of a comment's replies; also caches the comment's vote tokens.
pub async fn load_thread(video_id: &str, token: &str) -> Result<CommentThread, String> {
    let (id, token) = (video_id.to_owned(), token.to_owned());
    let thread = app_youtube(move |y| async move { y.comment_thread(&token, &id).await }).await?;
    if let Some(comment) = &thread.comment {
        VOTES.with(|v| {
            v.borrow_mut()
                .insert(comment.id.clone(), thread.actions.clone())
        });
    }
    Ok(thread)
}

/// How a channel to open is known: by ID, `@handle`, display name or one
/// of its videos. The TV feed only gives a video's channel name, so a name
/// the channel directory doesn't know is found through the video.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChannelKey {
    pub id: Option<String>,
    pub handle: Option<String>,
    pub name: Option<String>,
    pub video_id: Option<String>,
}

impl ChannelKey {
    pub fn from_args(args: &Value) -> Self {
        let text = |keys: &[&str]| {
            keys.iter()
                .find_map(|k| args.get(*k).and_then(Value::as_str))
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        Self {
            id: text(&["channel_id", "id"]),
            handle: text(&["handle"]),
            name: text(&["name", "channel_name"]),
            video_id: text(&["video_id"]),
        }
    }

    pub fn to_args(&self) -> Value {
        json!({ "channel_id": self.id, "handle": self.handle, "name": self.name, "video_id": self.video_id })
    }

    pub fn is_empty(&self) -> bool {
        self.id.is_none() && self.handle.is_none() && self.name.is_none() && self.video_id.is_none()
    }

    /// What the channel directory already knows about the channel.
    pub fn known(&self) -> Option<ChannelRef> {
        let directory = &app().channels;
        [&self.id, &self.handle, &self.name]
            .into_iter()
            .flatten()
            .find_map(|k| directory.lookup(k))
    }
}

/// A channel's page, run through `channel.page`, announced as
/// `channel.loaded`.
pub async fn load_channel(key: ChannelKey) -> Result<ChannelPage, String> {
    let id = match key.id.clone().or_else(|| key.known().and_then(|c| c.id)) {
        Some(id) => id,
        None => channel_id(&key).await?,
    };
    let page = app_youtube(move |y| async move { y.channel(&id).await }).await?;
    let page: ChannelPage = filtered("channel.page", page)?;
    app().channels.remember(&page.channel);
    app()
        .ext
        .events
        .emit("channel.loaded", &json!({ "channel": page }));
    Ok(page)
}

/// The ID of a channel known only by its handle or one of its videos.
async fn channel_id(key: &ChannelKey) -> Result<String, String> {
    if let Some(handle) = key.handle.clone() {
        return app_youtube(move |y| async move { y.resolve_handle(&handle).await }).await;
    }
    let video_id = key
        .video_id
        .clone()
        .ok_or("YouTube didn't say which channel this is")?;
    let page = app_youtube(move |y| async move { y.watch(&video_id).await }).await?;
    let owner = page.owner.ok_or("YouTube didn't say whose video this is")?;
    app().channels.remember(&owner);
    owner
        .id
        .ok_or_else(|| "YouTube didn't say whose video this is".into())
}

fn set_subscribed(args: &Value, subscribe: bool) -> Result<Value, String> {
    let subscription: Subscription = match args.get("subscription") {
        Some(s) => {
            serde_json::from_value(s.clone()).map_err(|e| format!("invalid subscription: {e}"))?
        }
        None => Subscription {
            channel_id: str_arg(args, "channel_id")?.to_owned(),
            ..Subscription::default()
        },
    };
    let channel_id = subscription.channel_id.clone();
    run(
        async move {
            app_youtube(move |y| async move {
                if subscribe {
                    y.subscribe(&subscription).await
                } else {
                    y.unsubscribe(&subscription).await
                }
            })
            .await
        },
        move |result| {
            outcome(
                "channel.subscription",
                json!({ "channel_id": channel_id, "subscribed": subscribe }),
                result.map(|()| json!({})),
            )
        },
    );
    Ok(Value::Null)
}

fn open_link(link: &Link) -> Result<Value, String> {
    let app = app();
    match link {
        // A link to YouTube opens here.
        Link::Url { url } => match yt_core::text::parse_url(url) {
            Some(link) => open_link(&link),
            None => app.ext.commands.run("app.open_url", json!({ "url": url })),
        },
        Link::Video { video_id, start } => {
            let playing = app.player.state().video_id.as_deref() == Some(video_id.as_str());
            match (playing, start) {
                (true, Some(start)) => app
                    .ext
                    .commands
                    .run("player.seek", json!({ "position": start })),
                _ => app.ext.commands.run(
                    "video.play",
                    json!({ "video": { "id": video_id }, "start": start }),
                ),
            }
        }
        Link::Channel { channel_id, handle } => app.ext.commands.run(
            "channel.open",
            json!({ "channel_id": channel_id, "handle": handle }),
        ),
        Link::Playlist { playlist_id } => app
            .ext
            .commands
            .run("playlist.open", json!({ "playlist_id": playlist_id })),
        Link::Search { query } => app
            .ext
            .commands
            .run("search.open", json!({ "query": query })),
    }
}

/// A comment just posted, with what YouTube left out filled in: the text as
/// typed, and the signed-in user as its author.
fn posted(mut comment: Comment, typed: &str) -> Comment {
    if comment.text.is_empty() {
        comment.text = RichText::from_plain(typed);
    }
    if comment.author.name.is_empty()
        && let Some(account) = account()
    {
        comment.author.name = account.handle.clone().unwrap_or(account.name);
        comment.author.handle = account.handle;
        comment.author.avatar = account.photo;
    }
    if comment.published.is_empty() {
        comment.published = "just now".into();
    }
    comment
}

fn comment_text(args: &Value) -> Result<String, String> {
    let text = str_arg(args, "text")?.trim().to_owned();
    if text.is_empty() {
        return Err("the comment is empty".into());
    }
    Ok(text)
}

/// Runs `make(youtube)` on the tokio runtime.
pub(crate) async fn app_youtube<T, F>(
    make: impl FnOnce(std::sync::Arc<yt_core::innertube::InnerTube>) -> F + Send + 'static,
) -> Result<T, String>
where
    T: Send + 'static,
    F: Future<Output = yt_core::Result<T>> + Send + 'static,
{
    let youtube = app().youtube.clone();
    runtime::background(async move { make(youtube).await })
        .await
        .map_err(|e| e.to_string())
}

/// Runs `work` on the main loop and hands its result to `done`.
pub(crate) fn run<T: 'static>(
    work: impl Future<Output = Result<T, String>> + 'static,
    done: impl FnOnce(Result<T, String>) + 'static,
) {
    runtime::spawn_local(async move { done(work.await) });
}

/// Emits `event` with `context` and the result's fields, or
/// `<event>.failed` with `context` and the error (and shows it).
pub(crate) fn outcome(event: &str, context: Value, result: Result<Value, String>) {
    match result {
        Ok(fields) => {
            let mut payload = context;
            if let (Some(p), Value::Object(f)) = (payload.as_object_mut(), fields) {
                p.extend(f);
            }
            app().ext.events.emit(event, &payload);
        }
        Err(error) => failed(event, &error, context),
    }
}

fn failed(event: &str, error: &str, mut payload: Value) {
    let app = app();
    tracing::warn!("{event} failed: {error}");
    app.toast(error);
    if !payload.is_object() {
        payload = json!({});
    }
    payload["error"] = json!(error);
    app.ext.events.emit(&format!("{event}.failed"), &payload);
}

/// Runs a value through a filter, so plugins can change it.
fn filtered<T: serde::Serialize + serde::de::DeserializeOwned>(
    filter: &str,
    value: T,
) -> Result<T, String> {
    let filters = &app().ext.filters;
    if filters.is_empty(filter) {
        return Ok(value);
    }
    let json = serde_json::to_value(&value).map_err(|e| e.to_string())?;
    serde_json::from_value(filters.apply(filter, json))
        .map_err(|e| format!("a plugin's {filter} filter returned invalid data: {e}"))
}
