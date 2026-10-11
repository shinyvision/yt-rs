//! YouTube's internal API (InnerTube), called as the YouTube TV app
//! (`TVHTML5`), since that's the client the sign-in tokens belong to. Search
//! also asks the web client, anonymously, for what the TV client leaves out
//! (see [`crate::search`]).

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::account::{self, Account};
use crate::auth::{Session, TV_USER_AGENT};
use crate::channel::{self, ChannelPage, ChannelRef, Subscription};
use crate::comments::{self, Comment, CommentThread, CommentsPage};
use crate::error::{Error, Result};
use crate::feed::{self, FeedPage};
use crate::playlist::{self, Playlist, PlaylistPage, Privacy, SaveOption};
use crate::search::{self, SearchContinuation, SearchFilters, SearchPage, Suggestion};
use crate::watch::{self, WatchPage};

pub const CLIENT_NAME: &str = "TVHTML5";
pub const CLIENT_NAME_ID: &str = "7";
pub const CLIENT_VERSION: &str = "7.20260901.15.00";

/// A version of the TV app and the user agent it sends.
#[derive(Clone, Copy, Debug)]
pub struct TvClient {
    pub version: &'static str,
    pub user_agent: &'static str,
}

/// The current TV app, used for browsing.
pub const TV: TvClient = TvClient {
    version: CLIENT_VERSION,
    user_agent: TV_USER_AGENT,
};

/// An older TV app. Since October 2026 YouTube wants a PO token with the
/// current TV app's `player` requests; this version (with an old Cobalt user
/// agent, as SmartTube uses) still gets watch-time reporting URLs and the
/// resume position without one.
pub const TV_DOWNGRADED: TvClient = TvClient {
    version: "5.20260901",
    user_agent: "Mozilla/5.0 (DirectFB; Linux x86_64) Cobalt/4.13031-qa (unlike Gecko) Starboard/1",
};

const BASE_URL: &str = "https://www.youtube.com";

/// The web client, for anonymous requests.
const WEB_CLIENT_NAME: &str = "WEB";
const WEB_CLIENT_NAME_ID: &str = "1";
const WEB_CLIENT_VERSION: &str = "2.20261001.00.00";
const WEB_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0 Safari/537.36";
/// Search suggestions while typing, as youtube.com gets them.
const SUGGEST_URL: &str = "https://suggestqueries-clients6.youtube.com/complete/search";

/// A rating for [`InnerTube::rate_video`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rating {
    Like,
    Dislike,
    None,
}

/// The home feed's browse ID.
pub const HOME: &str = "FEwhat_to_watch";
/// The user's playlists.
pub const PLAYLISTS: &str = "FEplaylist_aggregation";

/// How long the TV player script found is trusted before checking for a
/// newer one.
const PLAYER_SCRIPT_MAX_AGE: Duration = Duration::from_secs(12 * 3600);

pub struct InnerTube {
    http: reqwest::Client,
    base: String,
    session: Arc<Session>,
    player_script: Mutex<Option<PlayerScript>>,
    /// Requests that would change the account are answered here, not sent.
    dry_run: std::sync::atomic::AtomicBool,
}

/// The endpoints that change the account (for [`InnerTube::set_dry_run`]).
const WRITES: &[&str] = &[
    "browse/edit_playlist",
    "comment/create_comment",
    "comment/create_comment_reply",
    "comment/perform_comment_action",
    "feedback",
    "like/dislike",
    "like/like",
    "like/removelike",
    "playlist/create",
    "playlist/delete",
    "subscription/subscribe",
    "subscription/unsubscribe",
];

/// The TV app's player script: its version (the ID in its URL), where it
/// is, and its `signatureTimestamp`. `player` requests send the timestamp,
/// and YouTube answers with stream URLs whose challenges that version of
/// the script solves (see [`crate::solver`]), and with watch-time
/// reporting URLs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerScript {
    pub id: String,
    pub url: String,
    pub sts: u64,
    fetched: u64,
}

impl PlayerScript {
    fn is_fresh(&self) -> bool {
        unix_now().saturating_sub(self.fetched) < PLAYER_SCRIPT_MAX_AGE.as_secs()
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl InnerTube {
    pub fn new(http: reqwest::Client, session: Arc<Session>) -> Self {
        Self::with_base(http, session, BASE_URL)
    }

    /// The HTTP client it sends requests with.
    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// Uses another host, for tests.
    pub fn with_base(http: reqwest::Client, session: Arc<Session>, base: &str) -> Self {
        let player_script = std::fs::read(player_script_file())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        Self {
            http,
            base: base.to_owned(),
            session,
            player_script: Mutex::new(player_script),
            dry_run: Default::default(),
        }
    }

    pub fn session(&self) -> &Arc<Session> {
        &self.session
    }

    /// Calls an endpoint such as `browse` or `player` as the signed-in user.
    /// The client context is added to `body`. If YouTube rejects the access
    /// token before it was due to expire, it is replaced and the call retried
    /// once.
    pub async fn call(&self, endpoint: &str, body: Value) -> Result<Value> {
        self.call_as(TV, endpoint, body).await
    }

    /// Like [`InnerTube::call`], as a specific version of the TV app.
    pub async fn call_as(&self, client: TvClient, endpoint: &str, body: Value) -> Result<Value> {
        if self.dry_run.load(std::sync::atomic::Ordering::Relaxed) && WRITES.contains(&endpoint) {
            return dry_run_answer(endpoint, &body);
        }
        let body = with_tv_context(client, body);
        let token = self.session.access_token().await?;
        match self.send(client, endpoint, &body, Some(&token)).await {
            Err(Error::Unauthorized) => {
                let token = self.session.refresh_rejected(&token).await?;
                self.send(client, endpoint, &body, Some(&token)).await
            }
            result => result,
        }
    }

    async fn send(
        &self,
        client: TvClient,
        endpoint: &str,
        body: &Value,
        token: Option<&str>,
    ) -> Result<Value> {
        let mut request = self.http.post(format!(
            "{}/youtubei/v1/{endpoint}?prettyPrint=false",
            self.base
        ));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let resp = request
            .header(reqwest::header::USER_AGENT, client.user_agent)
            .header(reqwest::header::ORIGIN, "https://www.youtube.com")
            .header(reqwest::header::REFERER, "https://www.youtube.com/tv")
            .header("X-Youtube-Client-Name", CLIENT_NAME_ID)
            .header("X-Youtube-Client-Version", client.version)
            .json(body)
            .send()
            .await?;
        read_json(resp, endpoint).await
    }

    /// Calls an endpoint as the web client, without signing in.
    pub async fn call_web(&self, endpoint: &str, mut body: Value) -> Result<Value> {
        if !body.is_object() {
            body = json!({});
        }
        body["context"] = json!({
            "client": { "clientName": WEB_CLIENT_NAME, "clientVersion": WEB_CLIENT_VERSION, "hl": "en" },
        });
        let resp = self
            .http
            .post(format!(
                "{}/youtubei/v1/{endpoint}?prettyPrint=false",
                self.base
            ))
            .header(reqwest::header::USER_AGENT, WEB_USER_AGENT)
            .header(reqwest::header::ORIGIN, "https://www.youtube.com")
            .header("X-Youtube-Client-Name", WEB_CLIENT_NAME_ID)
            .header("X-Youtube-Client-Version", WEB_CLIENT_VERSION)
            .json(&body)
            .send()
            .await?;
        read_json(resp, endpoint).await
    }

    /// Searches: the TV client's results for the user, with the web client's
    /// details (see [`crate::search`]). `chip` is a chip's token from an
    /// earlier page of the same query.
    pub async fn search(
        &self,
        query: &str,
        filters: &SearchFilters,
        chip: Option<&str>,
    ) -> Result<SearchPage> {
        let body = match chip {
            Some(token) => json!({ "continuation": token }),
            None => match filters.params() {
                Some(params) => json!({ "query": query, "params": params }),
                None => json!({ "query": query }),
            },
        };
        // Signed out, the web client's results are all there is.
        let tv = async {
            match self.session.is_signed_in().await {
                true => Some(self.call("search", body.clone()).await),
                false => None,
            }
        };
        let (tv, web) = tokio::join!(tv, self.call_web("search", body.clone()));
        merge_search(tv, Some(web))
    }

    /// The next page of a search.
    pub async fn search_more(&self, next: &SearchContinuation) -> Result<SearchPage> {
        let tv = async {
            match &next.tv {
                Some(token) => Some(self.call("search", json!({ "continuation": token })).await),
                None => None,
            }
        };
        let web = async {
            match &next.web {
                Some(token) => Some(
                    self.call_web("search", json!({ "continuation": token }))
                        .await,
                ),
                None => None,
            }
        };
        let (tv, web) = tokio::join!(tv, web);
        if tv.is_none() && web.is_none() {
            return Ok(SearchPage::default());
        }
        merge_search(tv, web)
    }

    /// Suggestions for a partly typed query.
    pub async fn suggestions(&self, query: &str) -> Result<Vec<Suggestion>> {
        let url = reqwest::Url::parse_with_params(
            SUGGEST_URL,
            [
                ("client", "youtube"),
                ("ds", "yt"),
                ("hl", "en"),
                ("oe", "utf-8"),
                ("q", query),
            ],
        )
        .expect("the suggestion URL is valid");
        let body = self
            .http
            .get(url)
            .header(reqwest::header::USER_AGENT, WEB_USER_AGENT)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        Ok(search::parse_suggestions(&body))
    }
}

/// Reads an InnerTube reply: JSON, or the error YouTube gave.
async fn read_json(resp: reqwest::Response, endpoint: &str) -> Result<Value> {
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(Error::Unauthorized);
    }
    let bytes = resp.bytes().await?;
    if !status.is_success() {
        let message = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|v| v.pointer("/error/message")?.as_str().map(str::to_owned));
        return Err(Error::Api {
            endpoint: endpoint.to_owned(),
            status: status.as_u16(),
            message,
        });
    }
    Ok(serde_json::from_slice(&bytes)?)
}

/// What a write answers in a dry run: success, and for a new playlist an ID
/// that is clearly not one. Comments can't be faked convincingly, so they
/// fail.
fn dry_run_answer(endpoint: &str, body: &Value) -> Result<Value> {
    tracing::info!(target: "dry_run", "not sent: {endpoint} {body}");
    match endpoint {
        "playlist/create" => Ok(json!({ "playlistId": "PLdry-run" })),
        e if e.starts_with("comment/create") => Err(Error::Api {
            endpoint: endpoint.into(),
            status: 0,
            message: Some("not posted: dry run".into()),
        }),
        _ => Ok(json!({ "status": "STATUS_SUCCEEDED" })),
    }
}

/// Puts one page of results together from the two clients' replies (`None`
/// for a client that wasn't asked); an error only if nothing came back.
fn merge_search(tv: Option<Result<Value>>, web: Option<Result<Value>>) -> Result<SearchPage> {
    match (tv, web) {
        (Some(Err(e)), None | Some(Err(_))) | (None, Some(Err(e))) => Err(e),
        (tv, web) => {
            let tv = tv.and_then(|r| {
                r.map_err(|e| tracing::warn!("TV search failed, showing the web results: {e}"))
                    .ok()
            });
            let web = web.and_then(|r| {
                r.map_err(|e| {
                    tracing::warn!("web search failed, showing results without details: {e}")
                })
                .ok()
            });
            Ok(search::merge(
                tv.map(|r| search::parse_tv(&r)),
                web.map(|r| search::parse_web(&r)),
            ))
        }
    }
}

impl InnerTube {
    /// With `on`, requests that would change the account (rating,
    /// subscribing, playlists, comments, feedback) aren't sent: they
    /// succeed as far as the caller can tell, and are logged. For trying
    /// things, and for tests.
    pub fn set_dry_run(&self, on: bool) {
        self.dry_run.store(on, std::sync::atomic::Ordering::Relaxed);
    }

    /// Loads a feed page by browse ID, e.g. [`HOME`].
    pub async fn browse(&self, browse_id: &str) -> Result<FeedPage> {
        Ok(feed::parse(
            &self
                .call("browse", json!({ "browseId": browse_id }))
                .await?,
        ))
    }

    /// Loads more of a feed: more shelves, or more of one shelf.
    pub async fn continuation(&self, token: &str) -> Result<FeedPage> {
        if let Some(token) = token.strip_prefix(feed::WEB_TOKEN) {
            let resp = self
                .call_web("browse", json!({ "continuation": token }))
                .await?;
            return Ok(feed::parse_web_grid(&resp));
        }
        Ok(feed::parse(
            &self
                .call("browse", json!({ "continuation": token }))
                .await?,
        ))
    }

    /// Loads a browse page with `params`, e.g. a channel tab.
    pub async fn browse_with(&self, browse_id: &str, params: &str) -> Result<FeedPage> {
        let body = json!({ "browseId": browse_id, "params": params });
        Ok(feed::parse(&self.call("browse", body).await?))
    }

    /// A channel's videos in an order. Latest is the TV client's Videos tab
    /// (the user's, with YouTube's menus); the TV client has no other
    /// orders, so Popular and Oldest come from youtube.com's chips.
    pub async fn channel_videos(
        &self,
        channel_id: &str,
        order: channel::VideoOrder,
    ) -> Result<FeedPage> {
        if order == channel::VideoOrder::Latest {
            return self.browse_with(channel_id, channel::VIDEOS_TAB).await;
        }
        let body = json!({ "browseId": channel_id, "params": channel::WEB_VIDEOS_TAB });
        let tab = self.call_web("browse", body).await?;
        let token = channel::order_token(&tab, order).ok_or_else(|| Error::Api {
            endpoint: "browse".into(),
            status: 200,
            message: Some(format!(
                "this channel's videos can't be sorted by {}",
                order.label().to_lowercase()
            )),
        })?;
        let resp = self
            .call_web("browse", json!({ "continuation": token }))
            .await?;
        Ok(feed::parse_web_grid(&resp))
    }

    /// A video's watch page: details, channel, description, comments entry.
    pub async fn watch(&self, video_id: &str) -> Result<WatchPage> {
        let resp = self.call("next", json!({ "videoId": video_id })).await?;
        Ok(watch::parse(&resp, video_id))
    }

    /// Likes, dislikes or removes the rating of a video.
    pub async fn rate_video(&self, video_id: &str, rating: Rating) -> Result<()> {
        let endpoint = match rating {
            Rating::Like => "like/like",
            Rating::Dislike => "like/dislike",
            Rating::None => "like/removelike",
        };
        self.call(endpoint, json!({ "target": { "videoId": video_id } }))
            .await
            .map(drop)
    }

    /// A page of comments, from a sort order's token or a page's `next`.
    pub async fn comments(&self, token: &str, video_id: &str) -> Result<CommentsPage> {
        let resp = self.call("next", json!({ "continuation": token })).await?;
        Ok(comments::parse_page(&resp, video_id))
    }

    /// A comment's thread (replies and vote tokens), from its `thread` token
    /// or a replies page's `next`.
    pub async fn comment_thread(&self, token: &str, video_id: &str) -> Result<CommentThread> {
        let resp = self.call("next", json!({ "continuation": token })).await?;
        Ok(comments::parse_thread(&resp, video_id))
    }

    /// Posts a comment on a video and returns it as YouTube shows it.
    pub async fn post_comment(&self, video_id: &str, text: &str) -> Result<Comment> {
        let body = json!({ "createCommentParams": comments::create_params(video_id), "commentText": text });
        comments::parse_created(&self.call("comment/create_comment", body).await?, video_id)
    }

    /// Replies to a top-level comment.
    pub async fn reply_to_comment(
        &self,
        video_id: &str,
        parent_id: &str,
        text: &str,
    ) -> Result<Comment> {
        let body = json!({ "createReplyParams": comments::reply_params(video_id, parent_id), "commentText": text });
        comments::parse_created(
            &self.call("comment/create_comment_reply", body).await?,
            video_id,
        )
    }

    /// Deletes one of the user's comments or replies (a reply's ID is
    /// `"<parent>.<reply>"`).
    pub async fn delete_comment(&self, video_id: &str, comment_id: &str) -> Result<()> {
        self.comment_action(&comments::delete_action(video_id, comment_id))
            .await
    }

    /// Runs a comment action token, e.g. one of [`comments::VoteActions`].
    pub async fn comment_action(&self, token: &str) -> Result<()> {
        let resp = self
            .call(
                "comment/perform_comment_action",
                json!({ "actions": [token] }),
            )
            .await?;
        comments::check_result(&resp)
    }

    /// A channel's page: header, subscribe button, tabs and featured shelves.
    pub async fn channel(&self, channel_id: &str) -> Result<ChannelPage> {
        let resp = self
            .call("browse", json!({ "browseId": channel_id }))
            .await?;
        Ok(channel::parse_page(&resp, channel_id))
    }

    /// The channel ID behind an `@handle`.
    pub async fn resolve_handle(&self, handle: &str) -> Result<String> {
        let handle = handle.trim_start_matches('@');
        let resp = self
            .call(
                "navigation/resolve_url",
                json!({ "url": format!("https://www.youtube.com/@{handle}") }),
            )
            .await?;
        resp.pointer("/endpoint/browseEndpoint/browseId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| Error::Api {
                endpoint: "resolve_url".into(),
                status: 200,
                message: Some(format!("no channel @{handle}")),
            })
    }

    /// A playlist's page: its details and first videos (`continuation`
    /// loads more, as a feed page).
    pub async fn playlist(&self, playlist_id: &str) -> Result<PlaylistPage> {
        let resp = self
            .call("browse", json!({ "browseId": format!("VL{playlist_id}") }))
            .await?;
        Ok(playlist::parse_page(&resp, playlist_id))
    }

    /// The user's playlists, Watch later and Liked videos first.
    pub async fn playlists(&self) -> Result<Vec<Playlist>> {
        let page = self.browse(PLAYLISTS).await?;
        Ok(page.shelves.into_iter().flat_map(|s| s.playlists).collect())
    }

    /// The user's playlists to save `video_id` to, saying which hold it.
    pub async fn save_options(&self, video_id: &str) -> Result<Vec<SaveOption>> {
        let resp = self
            .call(
                "playlist/get_add_to_playlist",
                json!({ "videoIds": [video_id] }),
            )
            .await?;
        Ok(playlist::parse_save_options(&resp))
    }

    /// Changes a playlist: `actions` as YouTube writes them (see
    /// [`add_to_playlist`](Self::add_to_playlist)), and the `params` a
    /// video's menu may come with.
    pub async fn edit_playlist(
        &self,
        playlist_id: &str,
        actions: Value,
        params: Option<&str>,
    ) -> Result<()> {
        let mut body = json!({ "playlistId": playlist_id, "actions": actions });
        if let Some(params) = params {
            body["params"] = params.into();
        }
        let resp = self.call("browse/edit_playlist", body).await?;
        match resp.get("status").and_then(Value::as_str) {
            Some("STATUS_SUCCEEDED") | None => Ok(()),
            Some(status) => Err(Error::Api {
                endpoint: "edit_playlist".into(),
                status: 200,
                message: Some(format!("YouTube answered {status}")),
            }),
        }
    }

    pub async fn add_to_playlist(&self, playlist_id: &str, video_id: &str) -> Result<()> {
        let actions = json!([{ "action": "ACTION_ADD_VIDEO", "addedVideoId": video_id }]);
        self.edit_playlist(playlist_id, actions, None).await
    }

    pub async fn remove_from_playlist(&self, playlist_id: &str, video_id: &str) -> Result<()> {
        let actions =
            json!([{ "action": "ACTION_REMOVE_VIDEO_BY_VIDEO_ID", "removedVideoId": video_id }]);
        self.edit_playlist(playlist_id, actions, None).await
    }

    /// Makes a playlist (with `video_id` in it, if given); its ID.
    pub async fn create_playlist(
        &self,
        title: &str,
        privacy: Privacy,
        video_id: Option<&str>,
    ) -> Result<String> {
        let body = json!({
            "title": title,
            "privacyStatus": privacy.as_api(),
            "videoIds": video_id.into_iter().collect::<Vec<_>>(),
        });
        let resp = self.call("playlist/create", body).await?;
        resp.get("playlistId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| Error::Api {
                endpoint: "playlist/create".into(),
                status: 200,
                message: Some("YouTube didn't say which playlist it made".into()),
            })
    }

    pub async fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        self.call("playlist/delete", json!({ "playlistId": playlist_id }))
            .await
            .map(drop)
    }

    /// What YouTube offers to do with a video whose menu comes on request
    /// (its `menu`).
    pub async fn video_actions(&self, menu: &str) -> Result<Vec<crate::actions::VideoAction>> {
        let resp = self
            .call(
                "get_panel",
                json!({ "panelId": "PAcontext_menu", "params": menu }),
            )
            .await?;
        Ok(crate::actions::from_panel(&resp))
    }

    /// Tells YouTube something about a video ("Not interested", "Don't
    /// recommend channel"), with a token from its menu.
    pub async fn feedback(&self, token: &str) -> Result<()> {
        self.call("feedback", json!({ "feedbackTokens": [token] }))
            .await
            .map(drop)
    }

    pub async fn subscribe(&self, subscription: &Subscription) -> Result<()> {
        let body = json!({ "channelIds": [subscription.channel_id], "params": subscription.subscribe_params.as_deref().unwrap_or("EgIIAhgA") });
        self.call("subscription/subscribe", body).await.map(drop)
    }

    pub async fn unsubscribe(&self, subscription: &Subscription) -> Result<()> {
        let body = json!({ "channelIds": [subscription.channel_id], "params": subscription.unsubscribe_params.as_deref().unwrap_or("CgIIAhgA") });
        self.call("subscription/unsubscribe", body).await.map(drop)
    }

    /// Every channel the user subscribes to, with avatars.
    pub async fn subscriptions(&self) -> Result<Vec<ChannelRef>> {
        let (mut channels, mut next) = channel::parse_subscriptions(
            &self
                .call("browse", json!({ "browseId": "FEchannels" }))
                .await?,
        );
        // A safety limit; a page holds about 20 channels.
        for _ in 0..50 {
            let Some(token) = next.take() else { break };
            let (more, after) = channel::parse_subscriptions(
                &self
                    .call("browse", json!({ "continuation": token }))
                    .await?,
            );
            if more.is_empty() {
                break;
            }
            channels.extend(more);
            next = after;
        }
        Ok(channels)
    }

    /// The signed-in account.
    pub async fn account(&self) -> Result<Option<Account>> {
        Ok(account::parse(
            &self.call("account/accounts_list", json!({})).await?,
        ))
    }

    /// Requests the player response for a video, signed in, for watch-time
    /// reporting (see [`crate::tracking`]) under the playback ID `cpn`.
    pub async fn player(&self, video_id: &str, cpn: &str) -> Result<Value> {
        let resp = self
            .player_with(video_id, cpn, &self.player_script(false).await?)
            .await?;
        if resp.get("playbackTracking").is_some() {
            return Ok(resp);
        }
        // An outdated signatureTimestamp silently drops the tracking URLs;
        // check for a newer TV player once.
        let fresh = self.player_script(true).await?;
        self.player_with(video_id, cpn, &fresh).await
    }

    /// The player response for a video's streams, and the player script
    /// that solves their challenges. Signed out, it asks anonymously, which
    /// YouTube answers for some videos.
    ///
    /// It asks the way the TV app does. Asking the way an ad-free playback
    /// does (`adPlaybackContext.pyv`) gets streams of a limited tier
    /// (`ctier=L`), which YouTube refuses past their first minutes; with a
    /// pre-roll ad, the streams are held back instead, see
    /// [`crate::streams::preroll`].
    pub async fn player_streams(&self, video_id: &str) -> Result<(Value, PlayerScript)> {
        let cpn = crate::tracking::new_cpn();
        let script = self.player_script(false).await?;
        let resp = self.player_with(video_id, &cpn, &script).await?;
        // YouTube asks to "reload the page" when the script is out of date.
        let outdated = resp
            .pointer("/playabilityStatus/status")
            .and_then(Value::as_str)
            == Some("UNPLAYABLE")
            && resp.get("streamingData").is_none();
        if !outdated {
            return Ok((resp, script));
        }
        let fresh = self.player_script(true).await?;
        if fresh == script {
            return Ok((resp, script));
        }
        Ok((self.player_with(video_id, &cpn, &fresh).await?, fresh))
    }

    async fn player_with(&self, video_id: &str, cpn: &str, script: &PlayerScript) -> Result<Value> {
        let body = json!({
            "videoId": video_id,
            "cpn": cpn,
            "contentCheckOk": true,
            "racyCheckOk": true,
            "playbackContext": {
                "contentPlaybackContext": {
                    "html5Preference": "HTML5_PREF_WANTS",
                    "lactMilliseconds": "-1",
                    "referer": format!("https://www.youtube.com/tv#/watch?v={video_id}"),
                    "signatureTimestamp": script.sts,
                }
            },
        });
        if self.session.is_signed_in().await {
            self.call_as(TV_DOWNGRADED, "player", body).await
        } else {
            self.send(
                TV_DOWNGRADED,
                "player",
                &with_tv_context(TV_DOWNGRADED, body),
                None,
            )
            .await
        }
    }

    /// Downloads a player script (2.5 MB), for the challenge solver.
    pub async fn player_script_code(&self, script: &PlayerScript) -> Result<String> {
        let resp = self
            .http
            .get(&script.url)
            .header(reqwest::header::USER_AGENT, TV_DOWNGRADED.user_agent)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.text().await?)
    }

    /// Sends a watch-time report (a URL from [`crate::tracking`]).
    pub async fn report(&self, url: &str) -> Result<()> {
        let token = self.session.access_token().await?;
        let resp = self
            .http
            .get(url)
            .bearer_auth(token)
            .header(reqwest::header::USER_AGENT, TV_DOWNGRADED.user_agent)
            .header(reqwest::header::REFERER, "https://www.youtube.com/tv")
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(Error::Api {
                endpoint: "stats".into(),
                status: resp.status().as_u16(),
                message: None,
            });
        }
        Ok(())
    }

    /// The TV app's current player script, from the cache unless it is old
    /// or `refresh` is set. Found by loading youtube.com/tv as the TV app
    /// that makes `player` requests, then the script it links to: YouTube
    /// serves different players to different TV apps.
    pub async fn player_script(&self, refresh: bool) -> Result<PlayerScript> {
        let mut cached = self.player_script.lock().await;
        if let Some(script) = &*cached
            && script.is_fresh()
            && !refresh
        {
            return Ok(script.clone());
        }
        let get = |url: String| async move {
            let resp = self
                .http
                .get(url)
                .header(reqwest::header::USER_AGENT, TV_DOWNGRADED.user_agent)
                .send()
                .await?
                .error_for_status()?;
            Ok::<_, Error>(resp.text().await?)
        };
        let page = get(format!("{}/tv", self.base)).await?;
        let player_url = between(&page, "\"player_url\":\"", "\"")
            .map(|u| u.replace("\\/", "/"))
            .ok_or_else(|| Error::Streams("the TV page has no player URL".into()))?;
        let player_url = if player_url.starts_with('/') {
            format!("{}{player_url}", self.base)
        } else {
            player_url
        };
        let code = get(player_url.clone()).await?;
        let sts = code
            .split_once("signatureTimestamp:")
            .and_then(|(_, rest)| {
                rest.split(|c: char| !c.is_ascii_digit())
                    .next()?
                    .parse()
                    .ok()
            })
            .ok_or_else(|| Error::Streams("the TV player has no signatureTimestamp".into()))?;
        let id = between(&player_url, "/s/player/", "/")
            .ok_or_else(|| Error::Streams(format!("unexpected player URL {player_url}")))?
            .to_owned();
        let script = PlayerScript {
            id,
            url: player_url,
            sts,
            fetched: unix_now(),
        };
        if let Ok(json) = serde_json::to_vec(&script) {
            let _ = std::fs::create_dir_all(crate::paths::cache_dir());
            let _ = std::fs::write(player_script_file(), json);
        }
        tracing::info!(player = script.id, sts, "found the TV player script");
        *cached = Some(script.clone());
        Ok(script)
    }
}

/// `body` with the context of a request from this TV app.
fn with_tv_context(client: TvClient, mut body: Value) -> Value {
    if !body.is_object() {
        body = json!({});
    }
    body["context"] = json!({
        "client": { "clientName": CLIENT_NAME, "clientVersion": client.version, "hl": "en" },
    });
    body
}

fn player_script_file() -> std::path::PathBuf {
    crate::paths::cache_dir().join("player-script.json")
}

/// The text between the first `start` and the next `end` after it.
fn between<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let from = text.find(start)? + start.len();
    let len = text[from..].find(end)?;
    Some(&text[from..from + len])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::OAuth;
    use crate::playlist::Privacy;

    /// Signed out (no token file), so a request that got through would fail.
    fn api() -> InnerTube {
        let http = reqwest::Client::new();
        let session = Arc::new(Session::load(
            OAuth::new(http.clone()),
            std::path::PathBuf::from("/nonexistent/token.json"),
        ));
        InnerTube::new(http, session)
    }

    #[tokio::test]
    async fn a_dry_run_sends_no_writes() {
        let api = api();
        api.set_dry_run(true);
        api.add_to_playlist("WL", "dQw4w9WgXcQ").await.unwrap();
        api.remove_from_playlist("WL", "dQw4w9WgXcQ").await.unwrap();
        api.rate_video("dQw4w9WgXcQ", Rating::Like).await.unwrap();
        api.feedback("token").await.unwrap();
        api.delete_playlist("PL1").await.unwrap();
        let id = api
            .create_playlist("Test", Privacy::Private, None)
            .await
            .unwrap();
        assert_eq!(id, "PLdry-run");
        assert!(api.post_comment("dQw4w9WgXcQ", "hi").await.is_err());
        // Reads still go out (and fail here, signed out and offline).
        assert!(api.browse(HOME).await.is_err());
        // Off again, writes go out too.
        api.set_dry_run(false);
        assert!(api.add_to_playlist("WL", "dQw4w9WgXcQ").await.is_err());
    }
}
