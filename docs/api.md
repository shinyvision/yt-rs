# The app's functions

Every function of the app's services, as plugins call them
(`app.api.<service>.<function>(...)`) and hook them
(`app.hook("<service>.<function>", fn)`); see [plugins.md](plugins.md).
Async functions take a callback last: `fn(result, error)`.

Generated from the app by `tests/e2e/api-doc`; types are the Rust ones
(`&str` and `String` are strings, `Option<T>` may be nil, `Vec<T>` is a
list, structs are tables).

## account

- `parse(resp: &Value)` → Option<Account>
  Reads the selected account from `account/accounts_list`.

## actions

- `from_panel(resp: &Value)` → Vec<VideoAction>
  The actions in a menu sent on request (`get_panel`): a list of items, each with what a tap does.
- `from_tile(tile: &Value)` → Vec<VideoAction>
  The actions in a tile's menu (`onLongPressCommand`), less the ones the app has its own way to do (play, go to the channel).
- `menu_params(tile: &Value)` → Option<String>
  For a tile whose menu comes on request (a context menu panel): the params to ask for it with.
- `menu_params_for(video_id: &str)` → String
  The `params` asking for any video's menu (`get_panel`), for the videos that come without one (the TV client's "Up next"): built as the Home tiles' own are, naming the video and the feed it is in.

## app

- `back()` → bool
- `can_go_back()` → bool
- `can_go_forward()` → bool
- `current_args()` → Option<Value>
  The arguments the current page was shown with.
- `current_page()` → Option<String>
- `dismiss_toast()`
- `forward()` → bool
- `navigate(page: &str, args: Value, remember: bool)`
  Shows a page. `remember` puts the current page on the back stack (unless it is this page with these arguments), which also ends any way forward. Going back to a page shows it with the arguments it had, so the channel page shows that channel again and the watch page plays that video again.
- `pages_alive()` → Vec<String>
  The pages still alive, by name, though the window no longer holds them: leaked (something holds them strongly).
- `pages_held()` → usize
  The pages the window holds (the one on screen, kept ones, and one fading out).
- `set_fullscreen(fullscreen: bool)`
- `toast(message: &str)`
  Shows a message in a toast.
- `toast_with(props: &Value)`
  Shows a toast with the `toast` component's properties: a message, and maybe an action (`{ label, command, args }`) and how long.

## captions

- `json3_to_srt(json: &str)` → serde_json::Result<String>
  Converts a json3 caption track to SRT, one cue per moment the text on screen changes. Events without text (YouTube's window and style setup, line breaks of their own) are left out.

## channel

- `parse_page(resp: &Value, browse_id: &str)` → ChannelPage
- `parse_subscription(button: &Value)` → Option<Subscription>
  Reads a subscribe button (`subscribeButtonRenderer`).
- `parse_subscriptions(resp: &Value)` → (Vec<ChannelRef>, Option<String>)
  Reads the list of subscribed channels (`browse FEchannels`).

## channels

- `lookup(key: &str)` → Option<ChannelRef>
  The channel known by this ID, `@handle` or display name.
- `refresh()`
  Loads the subscriptions list in the background, then tells the UI (`channels.updated`) so visible cards pick up the avatars.
- `remember(channel: &ChannelRef)`
  Adds or updates a channel, keeping details it already had.
- `save()`
- `subscriptions()` → Vec<ChannelRef>
- `want(name: &str, video_id: &str)`
  Looks up a channel known only by name through one of its videos, unless its avatar is known. Emits `channels.learned` with the channel.

## comments

- `check_result(resp: &Value)` → Result<()>
  Fails if an action's response reports a failure (they come with status 200).
- `create_params(video_id: &str)` → String
  `params` for posting a top-level comment on a video.
- `delete_action(video_id: &str, comment_id: &str)` → String
  The action token that deletes one of the user's own comments or replies.
- `parse_created(resp: &Value, video_id: &str)` → Result<Comment>
  Reads the comment a create or reply call returns, or why it failed.  The response also holds placeholder renderers (a reply's comes before the reply itself), so this takes the one with the most of a comment.
- `parse_page(resp: &Value, video_id: &str)` → CommentsPage
  Reads a page of comments (`next` with a comments token).
- `parse_thread(resp: &Value, video_id: &str)` → CommentThread
  Reads a thread (`next` with a comment's `thread` token, or the `next` token of a previous replies page).
- `reply_params(video_id: &str, parent_id: &str)` → String
  `params` for replying to a top-level comment.

## downloads

- `dir_for(video_id: &str)` → PathBuf
  Where a video's download is kept. Only plain IDs make a directory name.
- `get(video_id: &str)` → Option<Download>
- `is_running(video_id: &str)` → bool
- `list()` → Vec<Download>
- `stream_set(video_id: &str)` → Option<StreamSet>
  A downloaded video's streams: its files.

## feed

- `parse(resp: &Value)` → FeedPage
- `parse_web_grid(resp: &Value)` → FeedPage
  A page of a web client grid (a channel's videos in another order, or more of them): the videos as one untitled shelf, with a token (marked as the web client's) for more.

## images

- `in_memory()` → (usize, usize)
  The decoded images kept in memory: how many, and their bytes.

## list

- `add_to_queue(video: Video)`
  Adds a video to the queue: the queue starts with the video playing (if one is, outside a queue), and if nothing plays, the video plays.
- `clear()`
- `current()` → Option<List>
- `next()` → Result<(), String>
  Plays the list's next video, where the player is; an error at its end.
- `next_video()` → Option<Video>
  The list's next video, if there is one.
- `play_at(position: usize)` → Result<(), String>
  Plays the video at `position` in the list's order, on the watch page.
- `play_playlist(playlist_id: &str, index: Option<usize>, shuffled: bool)`
  Plays a playlist: from the video at `index` (in the playlist's order), or from the start, or shuffled.
- `playing()` → bool
  Whether a list is playing.
- `previous()` → Result<(), String>
  Plays the list's previous video (the first again, at the start), where the player is.
- `remove(position: usize)`
  Takes the video at `position` (in the order) out of the queue.
- `set_looping(looping: bool)`
- `set_shuffled(shuffled: bool)`
  Shuffles what is left of the list (the current video stays where it is), or puts it back in order.
- `started(video_id: &str)`
  Called as any video starts: leaves the list unless it is the list's.

## live_chat

- `parse(resp: &Value)` → ChatPage
- `token(channel_id: &str, video_id: &str, view: ChatView)` → String
  The token for a live stream's chat, from now on: built as youtube.com's own, which names the channel and the video.

## mpv

- `command(args: &[&str])` → Result<(), MpvError>
- `get_string(name: &str)` → Option<String>
- `initialize()` → Result<(), MpvError>
- `quote(value: &str)` → String
  Quotes a value for mpv's option lists, which split on commas. URLs from YouTube contain commas.
- `request_log_messages(level: &str)` → Result<(), MpvError>
- `set_double(name: &str, value: f64)` → Result<(), MpvError>
- `set_flag(name: &str, value: bool)` → Result<(), MpvError>
- `set_option(name: &str, value: &str)` → Result<(), MpvError>
- `set_property(name: &str, value: &str)` → Result<(), MpvError>

## oauth

- `refresh(refresh_token: &str)` → Result<Token> *(async)*
  Gets a new access token for a refresh token.
- `request_device_code()` → Result<DeviceCode> *(async)*
  Starts a sign-in.
- `revoke(refresh_token: &str)` → Result<()> *(async)*
  Revokes a refresh token and every access token issued for it.
- `wait_for_approval(code: &DeviceCode)` → Result<Token> *(async)*
  Polls until the user has approved the device code.

## paths

- `cache_dir()` → PathBuf
  Disposable data such as thumbnails (`~/.cache/yt-rs`).
- `config_dir()` → PathBuf
  Settings, the saved sign-in and plugins (`~/.config/yt-rs`).
- `data_dir()` → PathBuf
  What the user keeps, such as downloaded videos (`~/.local/share/yt-rs`). A profile of its own (`YT_RS_CONFIG_DIR`) keeps its own, in its `data` directory, unless `YT_RS_DATA_DIR` says where.
- `downloads_dir()` → PathBuf
- `init_lua()` → PathBuf
- `plugins_dir()` → PathBuf
- `token_file()` → PathBuf

## playback

- `cancel_up_next()`
  Stops the countdown to the next video.
- `current()` → Option<Video>
  The video playing (or loaded), as the watch page was opened for it.
- `expand()` → Result<(), String>
  Back to the watch page of the video playing in the mini player, without starting it over.
- `next()` → Result<(), String>
  Plays the next video, where the player is.
- `parse_duration(text: &str)` → Option<f64>
  Parses "1:02:03" or "12:34" into seconds.
- `play(video: Video, start: Option<f64>)`
  Plays a video, from `start` seconds if given, otherwise from where the user left off: opens the watch page for it, which starts it. It starts over even if it is the video already shown.
- `play_here(video: Video)`
  Plays a video where the player is: on the watch page if that is shown, otherwise in the mini player, without leaving the page.
- `set_captions(lang: Option<String>)` → Result<(), String>
  Shows captions in `lang` (loading them first), or hides them; and remembers the choice for the next videos.
- `set_sleep_timer(minutes: Option<f64>)`
  Pauses after `minutes`, or at the end of the video with `end_of_video`; `None` turns the timer off. Announced as `player.sleep_timer`.
- `sleep_at_end()`
  Pauses at the end of the video.
- `sleep_timer()` → SleepTimer
  The sleep timer: off, at the end of the video, or the minutes left.
- `stop()`
- `up_next()` → Option<Video>
  The video after the one shown: the next in the list playing (a playlist, the queue), or else YouTube's first suggestion.

## player

- `frame_step(forward: bool)`
  One frame forward or back (pausing first).
- `go_live()`
  Jumps to a live stream's edge.
- `has_captions(lang: &str)` → bool
  Whether captions in this language were loaded for the video.
- `is_live()` → bool
  Whether what plays is a live stream (known when it opens; its window comes with the first position). A recording isn't.
- `load(video_id: &str, streams: &StreamSet, start: f64)` → Result<(), MpvError>
  Plays a video from its resolved streams, starting at `start` seconds.
- `seek(position: f64, exact: bool)`
- `seek_by(seconds: f64)`
- `select_captions(lang: Option<&str>)`
  Shows loaded captions in this language, or hides captions.
- `set_looping(looping: bool)`
- `set_muted(muted: bool)`
- `set_paused(paused: bool)`
- `set_quality(index: Option<usize>)` → Result<(), String>
  Switches to one of the video's qualities (an index into [`StreamSet::qualities`]), or back to the one picked automatically, carrying on from the same moment.
- `set_speed(speed: f64)`
- `set_stable_volume(on: bool)`
  Evens out loud and quiet parts, like YouTube's "stable volume".
- `set_volume(volume: f64)`
- `show_captions(lang: &str, title: &str, srt: String)`
  Shows captions from SRT text, in place of any shown.
- `state()` → PlayerState
- `stats()` → Vec<(String, String)>
  Details for "stats for nerds", as mpv reports them.
- `stop()`
- `streams()` → Option<StreamSet>
  The streams of the loaded video.
- `timeline()` → (f64, f64)
  Seeks to `position` seconds. `exact` seeks to that exact frame; otherwise to the nearest keyframe, which is much faster and suits dragging the seek bar. Where a video can be sought to: a live stream's window, or the whole video.
- `toggle_pause()`
- `video_aspect()` → Option<f64>
  The picture's width divided by its height, once it shows.

## playlist

- `parse_more(resp: &Value)` → FeedPage
  More of a playlist's videos, from a page's `continuation`.
- `parse_page(resp: &Value, playlist_id: &str)` → PlaylistPage
  Reads a playlist's page (a `VL<id>` browse).
- `parse_save_options(resp: &Value)` → Vec<SaveOption>
  The user's playlists to save a video to (`playlist/get_add_to_playlist`).

## search

- `fetch(query: &str, filters: &SearchFilters, chip: Option<&str>)` → Result<SearchPage, String> *(async)*
  The first page of results for a query.
- `fetch_more(next: &SearchContinuation)` → Result<SearchPage, String> *(async)*
  The page after the one `next` came with.

## search_history

- `add(query: &str)`
- `clear()`
- `list()` → Vec<String>
  Newest first.
- `matching(typed: &str, limit: usize)` → Vec<String>
  Up to `limit` remembered queries starting with what was typed.
- `remove(query: &str)`

## search_results

- `merge(tv: Option<TvResults>, web: Option<WebResults>)` → SearchPage
  One page of results from what each client returned. The TV client's results come first, with the web client's details for each video; the web client's top channel leads, as on youtube.com. Without TV results, the web client's are shown as they are.
- `parse_suggestions(body: &str)` → Vec<Suggestion>
  Reads the suggestion endpoint's reply, `window.google.ac.h(["query", [["suggestion", 0, [...], {...}], ...]])`.
- `parse_tv(resp: &Value)` → TvResults
  Reads a TV search response or continuation: shelves of lockups, the first ("Search results for ...") holding the main results.
- `parse_web(resp: &Value)` → WebResults

## session

- `access_token()` → Result<String> *(async)*
  Returns an access token, refreshing it first if it has expired.
- `is_signed_in()` → bool *(async)*
- `refresh_rejected(rejected: &str)` → Result<String> *(async)*
  Replaces the access token even if it hasn't expired, for when YouTube rejected it early. `rejected` is the token that was rejected, so that concurrent callers refresh only once.
- `sign_in(token: Token)` → Result<()> *(async)*
  Stores a token from a completed sign-in.
- `sign_out()` → Result<()> *(async)*
  Revokes the sign-in and deletes the saved token.

## settings

- `flush()`
  Saves now if a save is pending, e.g. when the app quits.
- `get()` → Settings
- `set(settings: Settings)`
  Replaces the settings (saved soon after).

## social

- `account()` → Option<Account>
- `is_by_creator(video_id: &str, author_handle: Option<&str>)` → bool
  Whether a comment on a video is by the video's own channel.
- `is_mine(author_handle: Option<&str>)` → bool
  Whether a comment author is the signed-in user (the TV client doesn't say, so this compares handles).
- `load_account()`
  Loads the signed-in account (for the top bar and "is this my comment").
- `load_channel(key: ChannelKey)` → Result<ChannelPage, String> *(async)*
  A channel's page, run through `channel.page`, announced as `channel.loaded`.
- `load_comments(video_id: &str, token: &str)` → Result<CommentsPage, String> *(async)*
  A page of comments, run through `comments.page`, announced as `comments.loaded`.
- `load_thread(video_id: &str, token: &str)` → Result<CommentThread, String> *(async)*
  A page of a comment's replies; also caches the comment's vote tokens.
- `load_watch(video_id: &str)` → Result<WatchPage, String> *(async)*
  A video's watch details, run through the `watch.page` filter, then announced as `watch.loaded`.
- `offer_sign_in(why: &str)`
  A toast saying why to sign in, with "Sign in".
- `signed_in()` → bool
  Whether a user is signed in: what needs an account shows, or a way to sign in instead.

## streams

- `forget(video_id: &str)`
  Drops a video's cached streams.
- `prefetch(video_id: &str)`
  Looks a video up in the background so that playing it later is instant. Does nothing if it is cached or already being looked up.
- `preroll(resp: &Value)` → Option<Duration>
  How long the pre-roll ads would play, if the response has any: until their skip button (or their end), YouTube refuses the video's streams.
- `resolve(video_id: &str)` → Result<Arc<StreamSet>> *(async)*
  Looks up the streams for a video, ahead of any queued prefetches.
- `resolve_fresh(video_id: &str)` → Result<Arc<StreamSet>> *(async)*
  Looks a video up again, after YouTube refused its streams: with fresh URLs, and a newer player script if there is one.
- `until_served(streams: &StreamSet)` *(async)*
  Waits until YouTube serves the streams. With a pre-roll ad it holds them back for about as long as the ad takes to become skippable (see [`preroll`]); this checks every 400 ms instead of waiting all that time, since YouTube usually lets go a little earlier.
- `warm_up()`
  Gets the challenge solver ready for the current player script, so the first lookup doesn't wait for it.

## text

- `parse_timestamp(text: &str)` → Option<f64>
  "1:02:03" or "2:03" in seconds.
- `parse_url(url: &str)` → Option<Link>
  What a YouTube link points at: a video (and a moment in it), a playlist, a channel or a search. `None` for other links.

## tracker

- `finish()`
  Sends the final report for the current video, if any.
- `finish_blocking(timeout: std::time::Duration)`
  Like [`WatchTracker::finish`], but sends the report before returning, for when the app is quitting and the main loop no longer runs.
- `prepare(video_id: &str)` → Option<f64> *(async)*
  Gets ready to report on a video that is about to play, and returns where YouTube says to resume it. Reporting is skipped (with a log message) if YouTube returns no reporting URLs.
- `set_enabled(enabled: bool)`
  Turns reporting on or off, like pausing YouTube's watch history. Takes effect from the next video.

## tracking

- `new_cpn()` → String
  A new client playback nonce: 16 random characters that tie together the reports of one playback.
- `start_seconds(resp: &Value)` → Option<f64>
  The resume position the player response asks to start at, if any.
- `unplayable_reason(resp: &Value)` → Option<String>
  Why a player response has no reporting URLs, from its playability status.

## watch

- `parse(resp: &Value, video_id: &str)` → WatchPage

## youtube

- `account()` → Result<Option<Account>> *(async)*
  The signed-in account.
- `add_to_playlist(playlist_id: &str, video_id: &str)` → Result<()> *(async)*
- `browse(browse_id: &str)` → Result<FeedPage> *(async)*
  Loads a feed page by browse ID, e.g. [`HOME`].
- `browse_with(browse_id: &str, params: &str)` → Result<FeedPage> *(async)*
  Loads a browse page with `params`, e.g. a channel tab.
- `call(endpoint: &str, body: Value)` → Result<Value> *(async)*
  Calls an endpoint such as `browse` or `player` as the signed-in user. The client context is added to `body`. If YouTube rejects the access token before it was due to expire, it is replaced and the call retried once.
- `call_as(client: TvClient, endpoint: &str, body: Value)` → Result<Value> *(async)*
  Like [`InnerTube::call`], as a specific version of the TV app.
- `call_web(endpoint: &str, body: Value)` → Result<Value> *(async)*
  Calls an endpoint as the web client, without signing in.
- `channel(channel_id: &str)` → Result<ChannelPage> *(async)*
  A channel's page: header, subscribe button, tabs and featured shelves.
- `channel_videos(channel_id: &str, order: channel::VideoOrder)` → Result<FeedPage> *(async)*
  A channel's videos in an order. Latest is the TV client's Videos tab (the user's, with YouTube's menus); the TV client has no other orders, so Popular and Oldest come from youtube.com's chips.
- `comment_action(token: &str)` → Result<()> *(async)*
  Runs a comment action token, e.g. one of [`comments::VoteActions`].
- `comment_thread(token: &str, video_id: &str)` → Result<CommentThread> *(async)*
  A comment's thread (replies and vote tokens), from its `thread` token or a replies page's `next`.
- `comments(token: &str, video_id: &str)` → Result<CommentsPage> *(async)*
  A page of comments, from a sort order's token or a page's `next`.
- `continuation(token: &str)` → Result<FeedPage> *(async)*
  Loads more of a feed: more shelves, or more of one shelf.
- `create_playlist(title: &str, privacy: Privacy, video_id: Option<&str>)` → Result<String> *(async)*
  Makes a playlist (with `video_id` in it, if given); its ID.
- `delete_comment(video_id: &str, comment_id: &str)` → Result<()> *(async)*
  Deletes one of the user's comments or replies (a reply's ID is `"<parent>.<reply>"`).
- `delete_playlist(playlist_id: &str)` → Result<()> *(async)*
- `edit_playlist(playlist_id: &str, actions: Value, params: Option<&str>)` → Result<()> *(async)*
  Changes a playlist: `actions` as YouTube writes them (see [`add_to_playlist`](Self::add_to_playlist)), and the `params` a video's menu may come with.
- `feedback(token: &str)` → Result<()> *(async)*
  Tells YouTube something about a video ("Not interested", "Don't recommend channel"), with a token from its menu.
- `live_chat(token: &str)` → Result<crate::live_chat::ChatPage> *(async)*
  A page of a live stream's chat, from a token (the first built by [`live_chat::token`](crate::live_chat::token), then each page's).
- `player(video_id: &str, cpn: &str)` → Result<Value> *(async)*
  Requests the player response for a video, signed in, for watch-time reporting (see [`crate::tracking`]) under the playback ID `cpn`.
- `player_script(refresh: bool)` → Result<PlayerScript> *(async)*
  The TV app's current player script, from the cache unless it is old or `refresh` is set. Found by loading youtube.com/tv as the TV app that makes `player` requests, then the script it links to: YouTube serves different players to different TV apps.
- `player_script_code(script: &PlayerScript)` → Result<String> *(async)*
  Downloads a player script (2.5 MB), for the challenge solver.
- `player_streams(video_id: &str)` → Result<(Value, PlayerScript)> *(async)*
  The player response for a video's streams, and the player script that solves their challenges. Signed out, it asks anonymously, which YouTube answers for some videos.  It asks the way the TV app does. Asking the way an ad-free playback does (`adPlaybackContext.pyv`) gets streams of a limited tier (`ctier=L`), which YouTube refuses past their first minutes; with a pre-roll ad, the streams are held back instead, see [`crate::streams::preroll`].
- `playlist(playlist_id: &str)` → Result<PlaylistPage> *(async)*
  A playlist's page: its details and first videos (`continuation` loads more, as a feed page).
- `playlists()` → Result<Vec<Playlist>> *(async)*
  The user's playlists, Watch later and Liked videos first.
- `post_comment(video_id: &str, text: &str)` → Result<Comment> *(async)*
  Posts a comment on a video and returns it as YouTube shows it.
- `rate_video(video_id: &str, rating: Rating)` → Result<()> *(async)*
  Likes, dislikes or removes the rating of a video.
- `remove_from_playlist(playlist_id: &str, video_id: &str)` → Result<()> *(async)*
- `reply_to_comment(video_id: &str, parent_id: &str, text: &str)` → Result<Comment> *(async)*
  Replies to a top-level comment.
- `report(url: &str)` → Result<()> *(async)*
  Sends a watch-time report (a URL from [`crate::tracking`]).
- `resolve_handle(handle: &str)` → Result<String> *(async)*
  The channel ID behind an `@handle`.
- `save_options(video_id: &str)` → Result<Vec<SaveOption>> *(async)*
  The user's playlists to save `video_id` to, saying which hold it.
- `search(query: &str, filters: &SearchFilters, chip: Option<&str>)` → Result<SearchPage> *(async)*
  Searches: the TV client's results for the user, with the web client's details (see [`crate::search`]). `chip` is a chip's token from an earlier page of the same query.
- `search_more(next: &SearchContinuation)` → Result<SearchPage> *(async)*
  The next page of a search.
- `set_dry_run(on: bool)`
  With `on`, requests that would change the account (rating, subscribing, playlists, comments, feedback) aren't sent: they succeed as far as the caller can tell, and are logged. For trying things, and for tests.
- `subscribe(subscription: &Subscription)` → Result<()> *(async)*
- `subscriptions()` → Result<Vec<ChannelRef>> *(async)*
  Every channel the user subscribes to, with avatars.
- `suggestions(query: &str)` → Result<Vec<Suggestion>> *(async)*
  Suggestions for a partly typed query.
- `unsubscribe(subscription: &Subscription)` → Result<()> *(async)*
- `video_actions(menu: &str)` → Result<Vec<crate::actions::VideoAction>> *(async)*
  What YouTube offers to do with a video whose menu comes on request (its `menu`).
- `watch(video_id: &str)` → Result<WatchPage> *(async)*
  A video's watch page: details, channel, description, comments entry.
