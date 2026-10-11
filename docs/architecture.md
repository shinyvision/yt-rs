# Architecture

```
crates/
  yt-core/   YouTube logic, no UI: sign-in, InnerTube, feeds, watch pages,
             comments, channels, watch time, streams
  yt-app/    the GTK app (binary `yt-rs`): registries, UI, player, plugins
examples/plugins/   example Lua plugins (not loaded by default)
docs/               this file and the plugin API
```

`yt-core` is plain async Rust with no GTK dependency, so everything that talks
to YouTube is testable without a display. `yt-app` puts a UI on top.

## yt-core

| Module | |
|---|---|
| `auth` | The YouTube TV app's OAuth device flow (`OAuth`), and `Session`, which hands out valid access tokens: it refreshes them when they expire or YouTube rejects one, and saves the token file (mode 0600). |
| `innertube` | Calls YouTube's internal API as the TV app (`TVHTML5`). YouTube only accepts the TV sign-in's tokens from TV clients. `call` retries once with a fresh token on 401. `player` uses an older TV app version (`TV_DOWNGRADED`), since the current one needs a PO token for player requests since October 2026, and sends the TV player script's `signatureTimestamp` (fetched from youtube.com/tv and cached on disk for 12 hours). |
| `feed` | `Video`, `Shelf` and `FeedPage`, and the parser. It walks the whole response and picks out `tileRenderer` and `lockupViewModel` items wherever they are nested, so it survives YouTube reshuffling its wrapper renderers. It tells a shelf's own continuation ("more of this row") from the page's ("more shelves"). |
| `watch` | A video's details from the TV `next` response (`WatchPage`): title, stats, rating and the user's vote, rich description, chapters (from the markers list the player bar shows), owner and subscription state, the comments entry point (count and sort orders) and related videos. |
| `comments` | Comments in the TV client's `commentRenderer` format: pages, threads (replies), and the protobuf parameters for posting and replying (`proto.rs` builds them). A comment's vote tokens only come with its thread, so voting loads the thread first and caches its tokens. |
| `channel` | Channel pages (`ChannelPage`: header, banner, about, subscription, tabs, the Home tab's shelves), the subscriptions list, and `ChannelRef`, how a channel is referred to everywhere (ID, name, handle, avatar). A channel's videos in another order (`VideoOrder`: Popular, Oldest) come from the anonymous web client's chips, which the TV client has no equal of (it ignores any order in its params): `InnerTube::channel_videos`. Their pages' tokens are marked as the web client's (`feed::WEB_TOKEN`), so `continuation` loads more with the right client and callers never need to know. |
| `text` | `RichText`: descriptions and comments as runs with links (URLs with YouTube's redirects removed, moments in a video, channels, hashtags), found in plain text the way youtube.com shows them. |
| `search` | Search (`SearchPage`). Results come from the TV client, signed in, so they are the user's own: watch progress, "Previously watched", and an "Unwatched" chip that knows their history. The anonymous web client is asked the same thing at the same time, for what the TV client leaves out: description snippets with the query in bold, channel avatars and badges, the channel the query names, spelling corrections and the chips. Both take the same `params` (a protobuf built from `SearchFilters`) and chip tokens; `merge` puts the two together by video ID, and shows whichever came back if one fails (signed out, the web results are all there is). Each page carries the web client's details for videos it didn't list, for the TV client's later pages. Suggestions come from YouTube's completion endpoint. |
| `account` | The signed-in user's name, handle and photo. |
| `tracking` | The watch-time protocol as a pure state machine (`WatchSession`): it is told what the player does and when, and returns the report URLs to send. It is tested without a network. |
| `captions` | Caption tracks: YouTube's json3 format converted to SRT for mpv (YouTube's VTT for automatic captions repeats every line). |
| `streams` | Stream lookup from the TV app's own `player` request, signed in (`InnerTube::player_streams`), so YouTube treats it as the user watching and asks no "are you a bot" questions; age-restricted videos play too. The response's formats are picked here: the best video up to 1080p (VP9, then H.264, then AV1, which many GPUs can't decode in hardware), the best audio in the video's original language (dubbed videos list one audio stream per language), and a video stream per height and frame rate to switch to. Their URLs' challenges go to `solver`. The response also has the caption tracks (json3) and the storyboard spec (the grids of frames the seek bar previews). With a pre-roll ad, YouTube refuses the streams until the ad could have been skipped; the set says when (`available_at`), and `Resolver::until_served` checks every 400 ms from then, since YouTube lets go a little early. (Asking the way an ad-free playback does, `adPlaybackContext.pyv`, gets streams of a limited tier, `ctier=L`, which YouTube refuses past their first minutes.) Lookups are cached until the URLs expire, shared between concurrent callers, and prioritised: clicks go ahead of hover prefetches. `streams::chunked` reads a stream in bounded 10 MB ranges: googlevideo throttles an open-ended request (`Range: bytes=N-`) to about twice the stream's bitrate, and a very low-bitrate one (a 16 MB audio track for 10 hours) sends nothing for minutes, while a bounded range comes at full speed. It asks again from where it was after a lost connection, and for a refused URL gets a new one from its `refresh` (the player's: the video looked up again, the same format). `streams::live` reads live streams, which come as numbered two-second segments: it fetches them in order and drops each segment's repeated file header, so they make one continuous file. |
| `solver` | The stream URLs' challenges: the `n` parameter, and for some videos a scrambled signature, which only YouTube's player script turns into what its servers accept. This runs yt-dlp's challenge solver ([yt-dlp/ejs](https://github.com/yt-dlp/ejs), public domain, bundled in `solver/`) in QuickJS (`rquickjs`) on its own thread. Finding the functions in a new player version takes about three seconds, so the result (the "preprocessed player") is kept on disk per version and its functions stay loaded; a video's challenges then take milliseconds. The player script's version and `signatureTimestamp` come together (`InnerTube::player_script`): the streams a `player` request returns are solved by the script whose timestamp it sent. |
| `paths` | XDG locations, overridable with `YT_RS_CONFIG_DIR` and `YT_RS_CACHE_DIR`. |

Everything social goes through the TV sign-in too: YouTube accepts the TV
app's tokens for comments (reading, posting, replying, voting, deleting your
own), ratings and subscriptions, but only from TV clients. The TV API has no
way to edit a comment, and doesn't mark comments the creator hearted.

### Watch time

1. When a video opens, a signed-in `player` request returns `playbackTracking`:
   base URLs for a *playback* report and *watchtime* reports, and the report
   schedule. It also returns the saved resume position
   (`playerConfig.playbackStartConfig.startSeconds`).
2. The playback report, sent when playback starts, creates the history entry.
3. Watchtime reports are sent 10, 20 and 30 seconds into the session, then
   every 40, plus on pause, on resume and when the video ends or is left (with
   `final=1`). Each carries the media ranges played since the last report
   (`st`/`et`) and the position (`cmt`). A seek closes the current range and
   starts a new one.
4. All reports of one playback share a client playback nonce (`cpn`) and are
   sent with the bearer token.

This matches what SmartTube, a maintained TV-client app, does. It was checked
against a real account: the history entry moves to the top, and the resume
position follows the reports.

## yt-app

### Threads

GTK owns the main thread; all UI, the registries and Lua run there. Network
and disk work run on a small tokio runtime. `runtime::background(fut)` runs
a future on tokio and resolves on the main loop. The challenge solver has a
thread of its own (QuickJS isn't thread-safe). mpv wakes the main loop from
its own threads, and the main loop then drains mpv's events; mpv reads the
streams through `player/protocol.rs` on its own threads.

### Startup

```
services (HTTP, session, InnerTube, stream resolver, images, mpv, watch tracker)
→ built-in theme, commands, keys, components, pages     (the registries)
→ plugins                                               (may replace any of it)
→ the window, assembled from the registries
→ app.ready, first page
```

Plugins run before the window exists, which is what lets them replace any
built-in piece.

### The registries (`ext/`)

Everything the app consists of is registered by name, and the built-in code
goes through these registries exactly like plugins do:

| Registry | Holds |
|---|---|
| `events` | notifications (`player.position`, `feed.loaded`, ...) |
| `filters` | data pipelines (`feed.page`, `video.card`, `video.menu`, `watch.page`, `comments.page`, `comment`, `comment.menu`, `channel.page`) |
| `commands` | named actions; every button and key runs one |
| `keymap` | keys → commands, per page or global |
| `components` | UI factories: `build(props)` and optional `update(widget, props)` for reuse in lists; plus wrappers that run on every widget a component makes |
| `slots` | named places in layouts where widgets can be added |
| `pages` | screens (each built by a component) and sidebar entries |
| `theme` | CSS, later stylesheets taking precedence |

Data crosses the registries as JSON values, which map directly onto Lua
tables. Each registry clones its handler list before calling out, so handlers
can register or remove handlers while being called.

### UI (`ui/`)

Every piece is a component: `shell` (layout), `sidebar`,
`sidebar_subscriptions`, `topbar`, `feed` (a feed page: chips plus grid),
`chip_bar`, `video_card`, `watch`, `player`, `player_controls`, `comments`,
`comment`, `comment_composer`, `subscribe_button`, `channel`, `signin`. Pages
that are kept (the feeds, the watch page, the channel page) keep their state
and scroll position, and are refilled through `update` when navigated to
again.

A widget that follows events for as long as it exists (a card, a comment)
subscribes with `ui::follow`, which unsubscribes when the widget is
destroyed, so handlers don't pile up as lists scroll and pages change.

- **Grid:** a `GtkGridView`, which only creates cards for what is on screen and
  reuses them while scrolling. More videos load when the end comes near, or
  when the content doesn't fill the view yet.
- **Thumbnails** (`images.rs`) are fetched on tokio, cached on disk, decoded
  off the main thread *at display size* (the JPEG decoder downscales while
  decoding), and kept in a small in-memory cache. A reused card never shows a
  late image meant for the video it showed before.
- **Hover prefetch:** hovering a card for 250 ms looks its streams up, so a
  click plays almost at once.
- **Watch page:** one scrolled grid, laid out as youtube.com's default
  view (player, details and comments, "Up next" beside them), theater mode
  (the player across the width) or one column on narrow windows. Switching
  only moves grid cells, so the player is never taken out of the page,
  which would restart its video output. The player is 16:9 but leaves room
  for the title below it (`AspectBin`'s height cap). The page shows what the
  card knew at once, while `playback.rs` loads the details
  (`social::load_watch`) alongside the streams.
- **Navigation:** pages are remembered with their arguments, for Back and
  Forward (the top bar's arrows, Alt+←/→, the mouse's side buttons). The
  watch page showing a video is what starts it (`playback.rs` follows
  `page.changed`), so going back to a video plays that video again.
- **Cursor** (`cursor.rs`): an emission hook on `realize` gives every button
  the pointing hand, plugins' buttons included.
- **Player features** (`player/mod.rs`, `playback.rs`): changing quality
  opens the other stream with the same audio at the same moment; captions
  are downloaded, converted and handed to mpv from memory (`memory://`), so
  mpv never waits on the network; stable volume is an mpv audio filter.
  `playback.rs` keeps "Up next" (from the watch page), the autoplay
  countdown and the sleep timer.
- **Player UI:** `controls.rs` (YouTube's control bar), `player_settings.rs`
  (the gear's panel, built from data through the `player.settings` filter),
  `bezel.rs` (the growing, fading icon), `preview.rs` (a storyboard frame),
  `menu.rs` and `dialog.rs` (YouTube-style menus and confirmations).
- **Search** (`search.rs`, `ui/search*.rs`): `search.open` shows the
  `search` page and remembers the query (`search_history.json`, on this
  computer). Results go through the `search.page` filter, then become rows
  `{ component, props }` (through `search.rows`) in a `GtkListView`, which
  only builds the rows on screen and reuses a row's widget for the next row
  of the same component. The thumbnail takes a share of a result's width
  (`split.rs`), between youtube.com's 240 and 500 px, and narrow windows
  stack it like a card. Picking a chip replaces the current history entry
  instead of adding one, so Back leaves the search and Forward comes back to
  that chip. The search box (`search_box.rs`) asks for suggestions once
  typing pauses for 110 ms, and its list is a non-modal popover, so the
  keyboard stays in the field.
- **Settings** (`settings.rs`): what youtube.com remembers (volume, captions,
  autoplay, theater mode, stable volume) is saved in `settings.json`.
- **Social actions** (`social.rs`): the commands for rating, commenting,
  voting, deleting and subscribing return at once and report the outcome as
  events (`comment.posted`, `channel.subscription`, ... or `*.failed`). Every
  widget showing that comment or channel follows the event.
- **Channels and avatars** (`channels.rs`): the TV feed gives a video's
  channel name but not its ID or avatar. The channel directory collects
  channels from the subscriptions list, watch pages, channel pages and
  comments, and looks up channels it doesn't know through one of their
  videos (two at a time, only for cards on screen). It is saved to the cache
  directory, so each channel is looked up once.
- **Rich text** (`richtext.rs`): descriptions and comments are labels with
  Pango links; each link runs the `link.open` command.
- **Icons** (`icons.rs`): the app ships its own symbolic icons (thumbs, reply,
  pin, ...), since icon themes differ between systems.
- **Memory:** thumbnails are decoded at display size and the in-memory image
  cache has a byte budget. On glibc, freed memory is handed back to the
  system a few seconds after a page change or when playback stops
  (`runtime::release_memory_soon`).
- **Player:** mpv draws into a `GtkGLArea` through libmpv's render API
  (`player/render.rs`). Hardware decoding uses VA-API, with frames shared with
  OpenGL through the Wayland display. GTK uses its OpenGL renderer, so video
  frames need no conversion.
- **Refused streams:** when the player loads a video but can't open its
  audio, or the video fails to open (YouTube answering 403, say, after a
  player script change), `playback.rs` looks the video up again
  (`Resolver::resolve_fresh`: fresh URLs, and a newer player script if
  YouTube has one) and reloads at the same position, once. While a pre-roll
  ad holds the streams back, the player counts it down (`player.waiting`).
- **Reading the streams:** mpv opens every stream as a `ytstream://` URL
  through libmpv's stream callbacks (`player/protocol.rs`), which read it
  with `yt_core::streams::chunked` (seekable, sized from the URL's `clen`)
  or, live, with `yt_core::streams::live`. A stream that can't be read
  ends like one at its end, so the reader notes why it gave up and the
  player reports an error instead of the video ending (no "Up next").
- **Downloads:** `yt_core::downloads` keeps each download in its own
  directory (the video and audio streams as YouTube sends them, and a
  picture) and lists them in `downloads.json`; `fetch` saves a stream with
  the chunked reader, resuming a partial file. `downloads.rs` runs each on a
  thread of its own and plays a downloaded video from its files, which mpv
  opens directly (no muxing needed), online or not.
- **Media controls:** `media_controls.rs` puts the player on MPRIS (D-Bus) on
  Linux, as `org.mpris.MediaPlayer2.yt_rs` (another app ID, a test say, gets
  an `.instance<pid>` name, so it never takes the desktop's player). Its
  methods run the app's commands; its properties are the player's state and
  the list playing, announced as they change.
- **Live streams:** read segment by segment (`yt_core::streams::live`). Every segment's answer says which is the newest
  and where it starts in the stream's time, so the player knows the live
  edge and the window YouTube keeps (`maxDvrDurationSec`, up to 12 hours)
  and shows positions in the stream's own time (`rebase-start-time=no`).
  Seeking opens the stream again at the segment holding the target,
  counted back from the newest; dragging the seek bar seeks on release.
  The controls show "● LIVE", red at the edge and grey behind it (a click
  goes back), and how far behind live the position is.
- **Unplayable videos:** the lookup says why (`Error::Unplayable`): one
  that needs a sign-in offers "Sign in" (or, signed in, "Open on YouTube"
  to confirm the account's age), a members-only one "Join", and a live
  stream or premiere that hasn't started is looked up again every 30 s and
  plays once it starts.
- **Scrub bar** (`seekbar.rs`): a custom widget drawn with GTK's scene graph.
  While dragging it seeks to keyframes, which is nearly instant, and seeks
  exactly on release.

### Plugins (`plugins/`)

LuaJIT, through mlua. `api.rs` builds the `app` and `ui` tables. Lua functions
are wrapped into the same Rust closures the built-in code registers, with
errors caught, logged and shown briefly. `object.rs` wraps *any* GObject:
properties and signals go through GObject's type system, so every GTK (or
WebKit, VTE, ...) type works without hand-written bindings. LuaJIT's FFI plus
`ui.from_ffi` covers C libraries with no bindings at all. `prelude.lua` adds
Lua-side helpers (`ui.build`, `app.bind`).

## Testing

- `cargo test` covers the feed and search parsers (fixtures with YouTube's
  real nesting, personal data replaced), the watch-time protocol, tokens,
  picking and unlocking streams (formats, signatures, pre-roll waits, live
  segments, storyboards), the challenge solver loading, and the registries.
  `cargo run -p yt-core --example streams -- VIDEO_ID...` looks videos up
  with the saved sign-in and checks their streams download.
- End-to-end checks run the real app with a test plugin in a throwaway
  `YT_RS_CONFIG_DIR`. The plugin drives it through commands (play, seek,
  navigate), listens to events (`watch.reported`, `player.loaded`) and takes
  screenshots with `app.screenshot`. Test plugins set
  `app.player.set("ao", "null")` and `watch.set_tracking(false)`, so they're
  silent and leave your history alone. To test commenting, rating and
  subscribing without doing them on the account, a test plugin replaces
  those commands with ones that emit the success events. The plugins live in
  `tests/e2e/`; its README says how to run them.
