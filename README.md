# yt-rs

A fast, native YouTube client for the Linux desktop. GTK 4 and libadwaita for
the UI, mpv for playback, and Lua plugins that can change or extend every part
of it.

- **Your feed:** Home, Subscriptions and History from your own account, with
  the shelves YouTube recommends as chips, infinite scrolling, and the red
  watched bar on videos you've started.
- **Search,** as on youtube.com: suggestions while you type and your earlier
  searches, results with big thumbnails, the query in bold in descriptions,
  channel and playlist results, "People also watched" and Shorts shelves,
  spelling corrections, the chips (Shorts, Unwatched, Videos, ...) and the
  filters (type, length, upload date, features, popularity). Signed in, the
  results are your own, with watch progress; signed out, search still works.
- **Playback** in mpv: hardware-accelerated (VA-API), up to 1080p, resuming
  where you left off, with a scrub bar you can drag.
- **Watch history:** what you watch is reported to YouTube the way the
  official TV app reports it, so it lands in your history with a resume
  position and tunes your recommendations.
- **The player,** as on youtube.com: the round play/pause bezel, quality
  (144p to 4K), playback speed, captions (uploaded and automatic, remembered
  between videos), stable volume, sleep timer, loop, chapters on the seek
  bar, preview frames while hovering it, theater mode, autoplay with an "Up
  next" countdown, a right-click menu and "stats for nerds".
- **Watch page:** title, channel, likes and dislikes, the description with
  working links and timestamps, "Up next", and the comments: sort them, read
  replies, like or dislike them, write comments and replies, and delete your
  own.
- **Channels:** channel pages with banner, about, Home, Videos, Shorts and
  Live, subscribe and unsubscribe, and your subscriptions in the sidebar.
  Channel pictures appear on every video, comment and channel.
- **Plugins:** Lua (LuaJIT) plugins can replace or wrap any component, command,
  key, page or style, and add new ones. That includes widgets from C libraries,
  such as a web browser or a terminal. The app ships with no plugins;
  [examples](examples/plugins) show what's possible.

## Requirements

Arch Linux package names:

| Package      | For                                                        |
|--------------|------------------------------------------------------------|
| `gtk4`, `libadwaita` | the UI (GTK 4.18+, libadwaita 1.7+)                |
| `mpv`        | playback (libmpv)                                          |
| `luajit`     | plugins                                                    |

Building needs Rust 1.90 or newer and a C compiler (for the bundled QuickJS).

## Build and run

```sh
cargo build --release
./target/release/yt-rs
```

On first start the app asks you to sign in. It works like signing in a smart
TV: you enter a short code at google.com/device and pick your account or
channel. Your Google account then lists it as "YouTube on TV". The sign-in is
saved in `~/.config/yt-rs/token.json`, readable only by you, and lasts until
you revoke it.

## Keys

| Key | Where | Does |
|---|---|---|
| Space, K | video | play / pause |
| ←  → | video | back / forward 5 s |
| J  L | video | back / forward 10 s |
| ↑  ↓ | video | volume |
| M | video | mute |
| C | video | captions on / off |
| <  > | video | slower / faster |
| 0 – 9, Home, End | video | jump to 0 % – 90 %, the start, the end (a live stream: the live edge) |
| ,  . | video | a frame back / forward (while paused) |
| T | video | theater mode |
| Shift+N | video | next video |
| F, F11 | video, anywhere | fullscreen |
| / | anywhere | search |
| ↑  ↓, Enter, Esc | search box | pick a suggestion, search, close the list |
| Esc | anywhere | back |
| Alt+←  Alt+→, mouse back / forward buttons | anywhere | back / forward through the pages you visited |
| Ctrl+R, F5 | anywhere | refresh the feed or the results |
| Ctrl+Q | anywhere | quit |

Clicking the video pauses it; double-clicking toggles fullscreen. Hovering a
card for a moment looks its streams up in the background, so clicking it
starts playback in well under a second. Plugins can change any of these keys
(see `app.keymap`).

## Plugins

Plugins live in `~/.config/yt-rs/plugins/<name>/init.lua`, and
`~/.config/yt-rs/init.lua` runs too. See [docs/plugins.md](docs/plugins.md)
for the API, and [examples/plugins](examples/plugins) for working examples:

| Example | Shows |
|---|---|
| [shortcuts](examples/plugins/shortcuts/init.lua) | key bindings, new commands, raw mpv properties |
| [sponsorblock](examples/plugins/sponsorblock/init.lua) | skipping sponsor segments: player events, HTTP, a button in the player |
| [feed-filter](examples/plugins/feed-filter/init.lua) | hiding videos, a badge on every card, card menu items |
| [theme](examples/plugins/theme/init.lua) | restyling with CSS, changing built-in widgets |
| [browser](examples/plugins/browser/init.lua) | a WebKit browser page, through LuaJIT's FFI |
| [snake](examples/plugins/snake/init.lua) | a game: Cairo drawing, animation, key input |
| [terminal](examples/plugins/terminal/init.lua) | a VTE terminal page, through LuaJIT's FFI (needs `vte4`) |

To try one: `ln -s "$PWD/examples/plugins/snake" ~/.config/yt-rs/plugins/`.

## Settings through the environment

| Variable | Default | |
|---|---|---|
| `YT_RS_CONFIG_DIR` | `~/.config/yt-rs` | sign-in, plugins. A separate profile runs as its own instance |
| `YT_RS_CACHE_DIR` | `~/.cache/yt-rs` | thumbnails |
| `YT_RS_APP_ID` | `dev.ytrs.YtRs` | the app ID, which is also the window's class: another one lets window-manager rules tell an instance apart |
| `YT_RS_LOG` | `warn,yt_app=info,yt_core=info` | log filter (`tracing` syntax) |
| `YT_RS_MPV_LOG` | `warn` | how much of mpv's own log to pass on (`v`, `debug`, `trace`; shown with `YT_RS_LOG=mpv=debug`) |
| `YT_RS_DRY_RUN` | unset | `1`: nothing that would change the account is sent (rating, subscribing, playlists, comments, "Not interested"); the app acts as if it had |
| `GSK_RENDERER` | `gl` | GTK's renderer |

## Performance

Measured on the release build on a laptop (Intel + NVIDIA, Hyprland):

| | |
|---|---|
| window on screen | 0.5 s after launch |
| home feed on screen | 1.5–1.7 s after launch (network) |
| click to video playing, card hovered first | 0.65 s |
| click to video playing, cold | 0.3–1.1 s (a live stream: about 4 s) |
| CPU while playing 1080p VP9 | 4–5 % of one core (VA-API) |
| memory, browsing | 93 MB of its own (217 MB PSS) |
| memory, playing 1080p | 146 MB of its own |

The process's total RSS reads higher because it includes GPU driver code that
every GPU-accelerated program on the system shares.

## How it works

See [docs/architecture.md](docs/architecture.md). In short:

- **The TV client.** The app signs in and calls YouTube's internal API
  (InnerTube) as the YouTube TV app, because YouTube only accepts the TV
  sign-in's tokens from TV clients.
- **Streams.** They come from the same signed-in request the TV app makes,
  so YouTube doesn't ask to "confirm you're not a bot". The challenges in
  their URLs are solved by yt-dlp's challenge solver (bundled) in an
  embedded JavaScript engine. Live streams are read segment by segment,
  and can be watched back as far as YouTube keeps them (often 12 hours).
- **Watch time.** It is reported the way the TV app reports it.
- **The UI.** All of it, built-in parts included, is assembled from registries
  that plugins have full access to.

## Caveats

- Using an unofficial client is against YouTube's terms of service. The app
  sends a modest number of requests, like the TV app does.
- The TV app's OAuth client ID and version numbers are built in; if YouTube
  retires them, sign-in or watch-time reporting stops until they're updated.
- When YouTube changes its player script in a way the bundled challenge
  solver doesn't handle, videos stop playing until the solver is updated;
  a newer solver can be dropped in `~/.config/yt-rs/solver/` (see
  `crates/yt-core/src/solver/README.md`).
- The app plays no ads. Without Premium, a video with a pre-roll ad starts
  a few seconds late: YouTube holds its streams back until the ad could have
  been skipped, and the player counts that down. Hovering a video first
  starts the wait early.
- Comments can't be edited (the TV API has no way to), and the creator's
  heart on comments isn't shown (the TV API doesn't say).
