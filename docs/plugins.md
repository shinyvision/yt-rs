# Plugins

yt-rs is built so that a plugin can do anything the app does, and
anything it doesn't. Three ways in reach everything:

- **`gi`**: every library with GObject introspection data, from Lua:
  GTK, libadwaita, GLib, Gio, Pango, GStreamer, libsoup, WebKit... Their
  functions, classes, methods, structs, enums, callbacks and async
  functions, and new GObject types of your own (widgets that draw
  themselves, list models).
- **`app.api` and `app.hook`**: every function of the app's own services
  (YouTube's API, the stream resolver, the player and mpv, playback,
  search, playlists, settings, downloads, the parsers...), to call, and to
  hook: a hook runs around a function whoever calls it, built-in code
  included, and can change its arguments and result or replace it. Nothing
  in the app is out of reach: a test fails on any service function that
  isn't exported, unless it says why.
- **The registries**: the app's UI and behaviour by name (components,
  commands, keys, events, filters, slots, pages, CSS). The built-in UI is
  made of these, so a plugin can replace or wrap any part of it.

For any other C library, LuaJIT's `ffi` is there too. Plugins are Lua 5.1
running on LuaJIT.

## Where plugins live

```
~/.config/yt-rs/init.lua               your own tweaks, run first
~/.config/yt-rs/plugins/<name>/init.lua one plugin per directory, alphabetical
```

Two plugins come with the app, [sponsorblock](../plugins/sponsorblock) and
[notifications](../plugins/notifications). They're compiled in, written to
the cache folder the first time each version starts, and run from there in
the same alphabetical order as yours. A plugin of yours with the same name
runs in its place, so to change one, copy it into your plugins folder.

Settings has a switch for every plugin. One that's switched off doesn't run
from the next start; its name goes in `disabled_plugins` in `settings.json`.

`require("name.module")` finds modules in the plugins directory
(`plugins/name/module.lua`), then in the bundled plugins', and C modules
(`.so`) there too. A plugin's main file receives its name and directory:
`local name, dir = ...`.

Plugins run **before the window is built**, so replacing a built-in component,
command or key works from the top of the file. To change the widget tree
itself, wait for the window:

```lua
app.on("app.ready", function()
  app.find("topbar"):find(".logo"):get_parent():append(ui.label("beta"))
end)
```

Plugins are trusted code with full access to your system, like Neovim
plugins. Errors in a plugin are logged with a traceback and shown briefly in
the window; they never crash the app.

## Data and widgets

Data passes between the app and plugins as plain Lua tables (JSON underneath).
JSON `null` becomes `nil`. When you build a list that may end up empty, wrap it
in `app.array{}` so it stays a list.

A **video** looks like this. Only `id` is required when you pass one to a
command.

```lua
{ id = "dQw4w9WgXcQ", title = "...", channel = "...", views = "1.2M views",
  published = "3 days ago", duration = "3:33", thumbnail = "https://...",
  progress = 0.4, live = false, short = false }
```

A **channel**, a **comment** and **rich text** (descriptions and comments,
with their links) look like this:

```lua
{ id = "UCuAXFkgsw1L7xaCfnd5JJOw", name = "Rick Astley", handle = "@RickAstleyYT",
  avatar = "https://...", subscribers = "4.55M subscribers", verified = true }

{ id = "Ugzge340...", author = channel, text = rich_text, published = "1 year ago",
  edited = false, likes = "325K", vote = "none",   -- or "like", "dislike"
  reply_count = 962, pinned = "Pinned by @RickAstleyYT", thread = "..." }

{ runs = { { text = "Watch at " }, { text = "1:23", bold = true,
    link = { kind = "video", video_id = "...", start = 83 } } } }
-- link kinds: { kind = "url", url }, { kind = "video", video_id, start? },
--             { kind = "channel", channel_id?, handle? }, { kind = "search", query }
```

Widgets, and any other GObject, are **objects**. Everything GObject
introspection describes works on them: properties, signals, and the
methods of their class, its parents and the interfaces it implements
(see `gi` below).

```lua
local button = ui.new("GtkButton", { label = "Hi", tooltip_text = "Says hi" })
button.label = "Hello"                     -- set any property ("_" or "-")
print(button.sensitive)                    -- read any property
button:set_icon_name("emblem-ok-symbolic") -- any method
local id = button:on("clicked", function(self) app.toast("hi") end)
button:off(id)
```

What every object has, besides its own methods:

| Method | |
|---|---|
| `o.prop`, `o.prop = v`, `o:get(name)`, `o:set(name, v)` | properties. Enums take their nick (`"vertical"`), flags `"a\|b"` or a list, structs a record, a table of fields, or a string they parse (`"#ff0000"` for a color) |
| `o:on(signal, fn)` → id, `o:on_after(signal, fn)`, `o:off(id)` | signals, including `notify::prop`. `fn` gets the object, then the signal's arguments (full values: a `notify` handler's `pspec:get_name()`). Return `true` to stop an event |
| `o:emit(signal, ...)` | emits a signal, e.g. `button:emit("clicked")`, and returns its result |
| `o:type_name()`, `o:is_a(type)`, `o:properties()` | introspection; `gi.describe(o)` lists its methods, properties and signals |
| `o:ptr()` | the C pointer, for LuaJIT's FFI |

and every widget, on top of GTK's own methods (`w:get_parent()`,
`w:add_css_class(c)`, `w:grab_focus()`, `w:get_mapped()`...):

| Method | |
|---|---|
| `w:append(child)`, `w:remove(child)` | children of any container: boxes, lists, overlays (as overlays), libadwaita preferences groups and pages, single-child widgets |
| `w:children()`, `w:find(query)` | the widget tree. `query` is a widget name, or `.class` |
| `w:size()` | width, height |
| `w:controllers()` | the widget's event controllers (gestures, key and scroll handlers), as objects |
| `w:on_tick(fn(seconds))` | runs every frame until `fn` returns `false` |

These are Lua functions on `Gtk.Widget`'s class table (see "Methods of
your own" below), as yours can be.

## `gi`: every introspected library

```lua
local Gtk = gi.require("Gtk", "4.0")       -- a namespace (and what it needs)
local Gio = gi.require("Gio", "2.0")
local GLib = gi.require("GLib", "2.0")
```

| | |
|---|---|
| `Gtk.Button.new_with_label("Hi")` | constructors and other functions of a class |
| `Gtk.Label { label = "Hi", wrap = true }` | a new object with properties (as `ui.new`) |
| `Gtk.IconTheme.get_for_display(display)` | static functions |
| `GLib.get_user_name()`, `GLib.MAJOR_VERSION` | functions and constants of a namespace |
| `Gtk.Orientation.VERTICAL` → `"vertical"` | enum values are their nick; flags (`Gtk.StateFlags.ACTIVE`) are numbers. Both are accepted as numbers or names in any case, flags also as `"a\|b"` or a list |
| `Gdk.RGBA()`, `Gdk.RGBA { red = 1 }`, `Gdk.RGBA("#3584e4")` | structs: zeroed, from fields, or from a string they parse. Fields read and write as `rgba.red`, methods are `rgba:to_string()` |
| `local min, nat = label:measure("horizontal", -1)` | out arguments come after the return value |
| `pcall(GLib.file_get_contents, path)` | a GError is raised as an error record: `err.message`, `err.code`, `err.domain` |
| `GLib.idle_add(GLib.PRIORITY_DEFAULT, fn)` | Lua functions are callbacks, for as long as the function says (one call, until notified, forever) |
| `file:load_contents_async(nil, function(f, res) local ok, data = f:load_contents_finish(res) end)` | async functions take their callback, the `_finish` function gets the result |
| `Gtk.Widget.show(w)` | a class's methods, called on any instance of it; this is also how to reach a method whose name a property has (`has_focus`) |

Strings are bytes: binary data (`guint8` arrays, file contents) comes and
goes as Lua strings. Lists come back as Lua sequences, hash tables as
tables, `GValue`s as the value they hold, variants as records
(`GLib.Variant.new_string("x"):get_string()`).

Callbacks typed `gpointer` (sort and filter functions' items) get plain
pointers: `gi.wrap(pointer)` makes the object of one. Lua runs on the main
thread only: a callback called from another thread is refused and
reported.

### Methods of your own

A class's table takes Lua functions, which become methods of its
instances, its subclasses' too:

```lua
function Gtk.Widget:flash() self:add_css_class("flash") end
app.find("topbar"):flash()
```

`gi.type(name)` gives the table of any type, also one without
introspection data (the app's own widgets).

### New types

```lua
local Meter = gi.subclass(Gtk.Widget, {
  name = "MyPluginMeter",                                  -- a unique GType name
  properties = { GObject.param_spec_double("level", nil, nil, 0, 1, 0, GObject.ParamFlags.READWRITE) },
  signals = { full = { params = { "gint" } } },            -- types by name or class
  interfaces = { },                                        -- e.g. Gio.ListModel
  vfuncs = {
    measure = function(self, orientation, for_size) return 24, 24 end,
    snapshot = function(self, snapshot)
      local w = self:get_width() * self.level
      snapshot:append_color(Gdk.RGBA("#3584e4"), Graphene.Rect():init(0, 0, w, self:get_height()))
    end,
  },
  init = function(self) self.history = {} end,             -- each new instance
  class_init = function(class) class:set_css_name("meter") end,
})
local meter = Meter { level = 0.5 }
```

Virtual functions of the parent and of the interfaces listed are
overridden by name; their out arguments are what the function returns (an
out value returned as nil stays as the caller set it). Chain up with the
class's own: `Gtk.Widget.vfunc_snapshot(self, snapshot)`. Instances keep
any other field (`self.history`) until they're finalized. An interface
implemented in Lua (a `Gio.ListModel` with `get_item_type`, `get_n_items`
and `get_item`) works wherever GTK takes one.

## `ui`: making widgets

| Function | |
|---|---|
| `ui.new(type, props)` | any GObject type by name: `"GtkBox"`, `"AdwClamp"`, `"GtkGestureClick"`, ... (the same as `Gtk.Box(props)`) |
| `ui.build(spec)` | a widget tree from a table (below) |
| `ui.box(spec)`, `ui.label(text, spec)`, `ui.button(label, on_click, spec)`, `ui.scrolled(child, spec)` | shorthands for `ui.build` |
| `ui.image(url, width)` | a picture loaded through the app's image cache, decoded at `width` px |
| `ui.drawing_area(fn(cr, width, height))` | a widget you draw yourself with Cairo (below) |
| `ui.from_ffi(pointer)`, `ui.wrap(pointer)` | an object from a C pointer, e.g. a widget made through LuaJIT's FFI |
| `ui.type_exists(name)` | |

```lua
ui.build { "GtkBox", orientation = "vertical", spacing = 8, class = "card",
  on = { map = function(self) end },
  children = {
    { "GtkLabel", label = "Hello", xalign = 0 },
    ui.button("Close", function() app.back() end),
  } }
```

Cairo, inside `ui.drawing_area`: `set_color(css)`, `set_source_rgb(a)`,
`set_line_width`, `move_to`, `line_to`, `curve_to`, `rectangle`, `arc`,
`new_path`, `close_path`, `fill(_preserve)`, `stroke(_preserve)`, `paint`,
`clip`, `save`, `restore`, `translate`, `scale`, `rotate`,
`select_font_face(family, bold)`, `set_font_size`, `show_text`,
`text_size(text)` → width, height.

## `app`: the registries

### Events

```lua
local id = app.on("player.loaded", function(e) ... end)
app.off(id)
app.emit("my_plugin.something", { any = "data" })   -- plugins can define events
```

| Event | Payload |
|---|---|
| `app.ready` | the window is built |
| `page.changed` | `{ page, args, back, forward }`: `back` and `forward` say whether there is a page to go back or forward to |
| `feed.loaded` | `{ browse_id, page }` (a feed page: `shelves`, `continuation`) |
| `feed.refresh`, `feed.chip_selected` | `{ browse_id, index }` |
| `feed.order_selected` | `{ browse_id, index }`: an order chip over a channel's videos (0 Latest, 1 Popular, 2 Oldest) |
| `video.opened` | `{ video }`, before playback starts |
| `player.loading` | `{ video_id }` |
| `player.loaded` | `{ video_id, live }`: `live` for a live stream |
| `player.ended` | `{ video_id }` |
| `player.position` | `{ position, duration }`, at most four times a second |
| `player.paused`, `player.buffering` | `{ paused }`, `{ buffering }` |
| `player.seeking` | `{ from, to }` |
| `player.seeked` | `{ position }` |
| `player.duration`, `player.volume` | `{ duration }`, `{ volume, muted }` |
| `player.error` | `{ message, action? }`: playback failed or YouTube won't play the video (its own words); `action = { label, command, args }` is what the player offers to do about it ("Sign in", "Join", "Open on YouTube") |
| `player.upcoming` | `{ video_id, message }`: a live stream or premiere that hasn't started ("This live event will begin in 5 minutes."); it plays once it does |
| `player.audio_failed` | `{ video_id }`: the video loaded but its separate audio stream couldn't be opened |
| `player.fallback` | `{ video_id, position }`: a stream was refused, so playback looks the video's streams up again and carries on from `position` |
| `player.waiting` | `{ video_id, seconds }`: YouTube holds the streams back that long (the length of a pre-roll ad); playback starts after |
| `player.stopped` | |
| `player.docked` | `{ slot }`: the video moved: to `watch` (beside "Up next"), `theater` (full width, also in fullscreen), `mini` (the mini player, over another page) or `null` (shown nowhere) |
| `player.speed` | `{ speed }` |
| `player.settings` | `{ quality, auto_quality, captions, looping, stable_volume }`: one of these changed |
| `player.bezel` | `{ icon?, text? }`: emit it to flash the round icon over the video (and text above it), as play, pause, seeking and volume do |
| `player.up_next` | `{ video, seconds }`: the video ended and the next one plays in `seconds`, unless `player.cancel_up_next` runs |
| `player.up_next_cancelled` | |
| `player.sleep_timer` | `{ on, end_of_video, ends_at? }` (`ends_at` in seconds since 1970) |
| `player.autoplay` | `{ enabled }` |
| `player.toggle_stats` | "stats for nerds" was toggled |
| `watch.theater` | `{ theater }` |
| `comments.sort_selected` | `{ video_id, index }` |
| `watch.reported` | `{ video_id, kind, ok, url }` for each watch-time report |
| `watch.tracking` | `{ enabled }` |
| `window.fullscreen` | `{ fullscreen }` |
| `sidebar.toggled` | `{ mini, drawer }`: the docked guide is now its rail of icons (`mini`), or the drawer is open (`drawer`) |
| `shell.layout` | `{ size, docked, mini }`: the window layout changed: `size` is `"wide"` (1313 px and up), `"medium"` or `"narrow"` (under 792 px); `docked` whether the guide is beside the pages (not on the watch page, nor in fullscreen), `mini` whether as its rail |
| `auth.signed_in`, `auth.signed_out` | |
| `account.loaded` | `{ account = { name, handle, photo } }`: the signed-in user |
| `watch.loaded` | `{ video_id, page }`: a video's details (`title`, `views`, `published`, `likes`, `vote`, `description`, `chapters = { { start, title, thumbnail? } }`, `owner`, `subscription`, `comments = { count, sorts }`, `related`) |
| `watch.failed` | `{ video_id, error }` |
| `comments.loaded` | `{ video_id, page = { comments, next } }`: each page of comments |
| `comment.posted` | `{ video_id, parent_id, comment }`: a new comment, or a reply when `parent_id` is set |
| `comment.voted` | `{ comment_id, from, vote }` |
| `comment.deleted` | `{ video_id, comment_id }` |
| `video.rated` | `{ video_id, rating }`: `"like"`, `"dislike"` or `"none"` |
| `playlist.changed` | `{ playlist_id, video_id, added }`: a video went into a playlist (Watch later is `WL`, Liked videos `LL`) or out of it |
| `playlist.created`, `playlist.deleted` | `{ playlist_id, title? }` |
| `playlist.loaded` | `{ playlist_id, page }`: a playlist's page loaded (`page.videos`, `page.continuation`, `page.owner`, ...) |
| `video.hidden` | `{ video_id }`: YouTube was told it isn't wanted ("Not interested", "Don't recommend channel"); feeds drop it |
| `video.menu_loaded` | `{ video_id }`: YouTube's own menu for a video (sent on request) arrived; `video.menu` now has it |
| `download.changed` | `{ video_id, download?, running }`: a download started, got further, finished, stopped or was deleted (`download` null). `download = { video, quality?, bytes, total, complete, video_file, audio_file? }` |
| `download.done`, `download.failed` | `{ video_id, error? }` |
| `chat.message` | `{ video_id, message }`: a message shows in the live chat (see the `chat.messages` filter) |
| `list.changed` | `{ list }`: the list playing changed (`null`: none). `list = { playlist_id?, title, owner?, videos, order, position, looping, shuffled }`; the queue has no `playlist_id`. `order` holds indices into `videos` (0-based), `position` an index into `order` |
| `channel.loaded` | `{ channel }`: a channel page (`channel`, `banner`, `description`, `about`, `subscription`, `tabs`, `home`) |
| `channel.subscription` | `{ channel_id, subscribed }` |
| `channels.updated` | `{ subscriptions }`: the user's subscriptions, with avatars |
| `channels.learned` | `{ name, channel }`: a channel the app looked up to show its avatar |
| `*.failed` | for every action above (`comment.posted.failed`, `channel.subscription.failed`, ...): the action's payload plus `error` |
| `app.screenshot` | `{ path, ok, error }` |
| `search.loaded` | `{ query, first, page }`: each page of results (`first` for a query's first page), after `search.page` |
| `search.suggestions` | `{ query, suggestions = { { text, image?, description? } } }`: suggestions for what was typed (from `search.suggest`) |
| `search.history_changed` | the search history changed |
| `search.chip_selected` | `{ index }`: a chip on the search page was picked |
| `search.focus`, `search.refresh`, `search.more` | put the cursor in the search box; search again; load more results (the commands emit these) |

### Filters

A filter receives a value and returns it, changed or not (returning nothing
keeps it). Lower `order` runs first.

```lua
app.filter("feed.page", function(data) ... return data end, order)
```

`app.apply_filter(name, value)` runs a value through the named chain and
returns its result. Plugins can define their own data pipelines this way;
an unregistered filter returns the value unchanged.

| Filter | Value |
|---|---|
| `feed.page` | `{ browse_id, page }`: every feed page before it is shown |
| `video.card` | a video, before it fills a card |
| `video.menu` | `{ video, items }`: a card's menu items, `{ label, icon?, command, args?, confirm? }`, in youtube.com's order: Add to queue, Save to Watch later, Save to playlist, Download, Copy link, Open in browser, Go to channel, then a `{ separator = true }` and YouTube's own for the video ("Not interested", "Remove from Watch later", ...; asked for when the video came without them). The command runs with `{ video }` added to its arguments |
| `watch.page` | a video's details, before they are shown (see `watch.loaded`) |
| `chat.messages` | `{ video_id, messages }`: a live chat's new messages (each page of them), before they show: drop some, or change them. A message is `{ id, author = { name, channel_id?, avatar?, owner, moderator, verified, member?, member_badge? }, text, time_usec, kind }`, `kind` one of `text`, `super_chat` (with `amount`, `header_color`, `body_color`), `sticker` (`amount`, `sticker`, `color`), `membership` (`header`, `subtext?`), `gift` (`image?`), `notice` |
| `comments.page` | `{ comments, next }`: each page of comments, before it is shown. Remove comments to hide them |
| `comment` | a comment (plus `by_creator`), before it fills a `comment` component |
| `comment.menu` | `{ comment, items }`: a comment's menu items, like `video.menu`. The command runs with `{ video_id, comment_id, comment }` added |
| `channel.page` | a channel page, before it is shown |
| `account.menu` | `{ account, items }`: the account menu's items |
| `player.menu` | `{ items }`: the video's right-click menu |
| `player.settings` | `{ entries }`: the settings panel (the gear); see `crates/yt-app/src/ui/player_settings.rs` for the entry format |
| `search.page` | `{ query, first, page }`: each page of results before anything uses it. `page = { items, chips, correction?, next? }`; an item is `{ kind = "video" \| "channel" \| "playlist" \| "shelf", ... }` (see `crates/yt-core/src/search.rs`) |
| `search.rows` | `{ query, first, rows }`: the rows the results become, `{ component, props }`. Reorder or drop rows, or add rows of your own components anywhere |
| `search.suggestions` | `{ query, suggestions }`: YouTube's suggestions for what was typed |
| `search.box_rows` | `{ typed, rows }`: the list under the search box, `{ text, history?, image?, description?, icon?, command?, args? }`. A row with a `command` runs it instead of searching |
| `playlist.menu` | `{ playlist, items }`: a playlist result's menu items |

```lua
-- Search: no Shorts in the results, and a pasted link plays the video.
app.filter("search.page", function(e)
  local kept = {}
  for _, item in ipairs(e.page.items) do
    if not (item.short or (item.kind == "shelf" and item.shorts)) then table.insert(kept, item) end
  end
  e.page.items = kept
  return e
end)
app.filter("search.box_rows", function(e)
  local id = e.typed:match("youtu%.be/([%w_-]+)") or e.typed:match("[?&]v=([%w_-]+)")
  if id then
    table.insert(e.rows, 1, { text = "Play this video", icon = "yt-play-symbolic",
      command = "video.play", args = { video = { id = id } } })
  end
  return e
end)
```

### Commands

Every button and key in the app runs a command, so replacing a command changes
every way of triggering it.

Commands that need an account (liking, subscribing, commenting, saving and
the playlists', "Not interested") offer to sign in when no one is: a toast
with "Sign in", and the answer `{ signed_in = false }`. Signed out,
everything else works as it does on youtube.com: feeds, videos, channels,
comments, search, live chat; the account's own pages show youtube.com's
prompts to sign in.

```lua
app.run("player.seek", { position = 90 })
local original = app.command("video.play", function(args)
  app.log("playing", args.video.id)
  return original(args)           -- build on the built-in command
end)
app.commands()                     -- all names
```

| Command | Arguments |
|---|---|
| `app.navigate` | `{ page, args? }` |
| `app.back`, `app.forward` | go back or forward through the pages visited (the arrows, Alt+← / Alt+→ and the mouse's side buttons run these) |
| `app.quit`, `app.plugins` (loaded plugin names) | |
| `app.toast` | `{ message, action?, seconds? }`: a toast in the bottom-left corner; `action = { label, command, args? }` adds a button ("Undo") that runs the command |
| `app.dismiss_toast` | hides the toast showing |
| `app.close_dialog` | closes the dialog showing (while one is open, keys go to it) |
| `app.shortcuts` | the keyboard shortcuts (`?`), every binding with what it does |
| `seekbar.set_ranges` | `{ owner, ranges = { { start, end, color, label? } } }`: marks parts of the video on the seek bar, in place of `owner`'s earlier ones; a range too short to see is a mark, and `label` shows in the bubble over it. Cleared when another video loads |
| `seekbar.clear_ranges` | `{ owner }` |
| `seekbar.ranges` | → everyone's marks |
| `app.screenshot` | `{ path }`, a PNG of the window after the next frame, with the popovers open over it (menus, the player's settings) |
| `app.stats` | → what the app holds, for finding leaks: `{ handlers = { [event] = n }, pages, images, image_bytes }` (each widget following an event has a handler, so counts that keep growing mean widgets that outlive their pages) |
| `sidebar.toggle`, `window.toggle_fullscreen` | |
| `window.fullscreen` | `{ fullscreen }` |
| `feed.refresh` | |
| `video.play`, `video.prefetch`, `video.copy_link`, `video.open_in_browser` | `{ video }` or `{ video = { id = ... } }` |
| `player.toggle_pause`, `player.play`, `player.pause`, `player.stop`, `player.toggle_mute` | |
| `player.open` | `{ url, audio_url?, title?, start?, video_id? }`: plays any URL mpv can open. With `video_id`, a refused stream makes the app look that video up again |
| `player.seek` | `{ position, exact? }`. `exact = false` seeks to the nearest keyframe, which is much faster |
| `player.seek_by` | `{ seconds }` |
| `player.set_volume`, `player.change_volume` | `{ volume }`, `{ delta }` (0–130) |
| `watch.set_tracking` | `{ enabled }`: pause or resume reporting watch time |
| `auth.sign_out` | revokes the sign-in |
| `video.play` | also takes `start` (seconds) |
| `player.set_speed`, `player.change_speed` | `{ speed }`, `{ delta }` (0.25 to 4) |
| `player.qualities` | → the qualities of the video, best first, e.g. `{ "1080p60", "720p" }` |
| `player.set_quality` | `{ index }` into `player.qualities`, or `{ index = nil }` for automatic. Playback carries on from the same moment |
| `player.set_captions` | `{ lang }`, or `{ lang = nil }` for off. Remembered for the next videos |
| `player.toggle_captions` | |
| `player.set_looping`, `player.toggle_loop` | `{ looping }` |
| `player.set_stable_volume` | `{ enabled }`: evens out loud and quiet parts |
| `player.set_ambient` | `{ enabled }`: ambient mode, the video's colors glowing softly around the player (on by default) |
| `player.sleep_timer` | `{ minutes }`, `{ minutes = "end" }` for the end of the video, or `{ minutes = nil }` |
| `player.frame_step` | `{ forward }`, one frame |
| `player.seek_percent`, `player.seek_end` | `{ percent }`: of the video, or of the part of a live stream YouTube keeps; `seek_end` goes back to a live stream's edge |
| `player.go_live` | back to a live stream's edge |
| `player.next`, `player.cancel_up_next` | the next video (YouTube's first suggestion) |
| `player.mini` | off the watch page with the video carrying on in the mini player, or back from it (`i`) |
| `player.expand` | back to the watch page of the video in the mini player, where it is |
| `player.set_autoplay` | `{ enabled }` |
| `player.settings` | → the settings panel's entries |
| `player.stats`, `player.toggle_stats` | → "stats for nerds" as `{ { name, value }, ... }`; show or hide them over the video |
| `player.copy_url` | `{ at_time? }` |
| `watch.toggle_theater` | the wide player |
| `settings.get` | → the remembered settings: `{ volume, muted, autoplay, theater, captions, stable_volume }` |
| `app.confirm` | `{ title, body?, confirm, cancel?, command, args? }`: asks in a dialog, then runs the command |
| `comments.sort` | `{ video_id, index }` |
| `video.rate` | `{ video_id, rating }`: `"like"`, `"dislike"` or `"none"` |
| `video.menu` | `{ video }` → the video's menu items (see the `video.menu` filter); asks YouTube for its own items if they come on request (`video.menu_loaded`) |
| `video.action` | `{ video, action }`: one of YouTube's menu actions (an item of `video.actions`) |
| `video.save_to_watch_later` | `{ video_id }`; the toast offers "Undo" |
| `video.save` | `{ video_id }`: "Save video to..." (the user's playlists, and making one) |
| `playlist.open` | `{ playlist_id }`: its page |
| `playlist.play` | `{ playlist_id, index?, shuffle? }`: plays it, from video `index` (0-based) or shuffled |
| `playlist.add`, `playlist.remove` | `{ playlist_id, video_id, title?, undo? }` |
| `playlist.create` | `{ title, privacy? = "private" \| "unlisted" \| "public", video_id? }` → reports `playlist.created` |
| `playlist.delete` | `{ playlist_id }` |
| `download.start` | `{ video, quality? }`: downloads it to watch offline (`quality` as the player names them, `"720p"`; the player's choice otherwise). One that stopped carries on |
| `download.cancel`, `download.delete` | `{ video_id }` |
| `download.list` | → every download, newest first; `download.get` `{ video_id }` → one |
| `chat.state` | → the live chat on screen: `{ video_id, view, live, hidden, running, messages, pinned, following, status }` (`status`: `loading`, `messages` or `status` for a message instead), or `nil` |
| `chat.hide`, `chat.show` | folds the live chat away ("Show chat" brings it back), remembered |
| `chat.set_view` | `{ view }`: `"top"` (YouTube's pick, without potential spam) or `"live"` (every message) |
| `queue.add` | `{ video }`: to the queue, which starts with the video playing (or plays this one, in the mini player, if none is) |
| `list.current` | → the list playing (see `list.changed`), or nothing |
| `list.next`, `list.previous`, `list.play_at` | `{ position }` for `play_at`: an index into the list's `order` |
| `list.set_looping`, `list.set_shuffled` | `{ looping }`, `{ shuffled }` |
| `list.remove` | `{ position }`: out of the queue |
| `list.clear` | stops playing the list (the video carries on) |
| `comment.post` | `{ video_id, text }` |
| `comment.reply` | `{ video_id, parent_id, text }` |
| `comment.vote` | `{ video_id, comment_id, thread?, from, to }`: votes are `"none"`, `"like"` or `"dislike"` |
| `comment.delete` | `{ video_id, comment_id }`: only your own comments |
| `comment.copy` | `{ comment }`: copies its text |
| `channel.open` | `{ channel_id }`, `{ handle }`, or just `{ name, video_id }` (the app finds the channel) |
| `channel.subscribe`, `channel.unsubscribe` | `{ subscription }` from a page, or `{ channel_id }` |
| `channels.subscriptions` | → the user's subscriptions |
| `channels.lookup` | `{ key }` (an ID, `@handle` or name) → the channel, if the app knows it |
| `account.info` | → `{ name, handle, photo }` |
| `link.open` | `{ link }`: what clicking a link in a description or comment does |
| `app.open_url` | `{ url }`: opens the system browser |
| `app.copy_text` | `{ text, toast? }`: copies text, then shows `toast` if given |
| `search.open` | `{ query, filters?, chip? }`: shows the results. `filters = { type?, duration?, upload_date?, features?, sort?, exact? }`: `type` is `video`, `short`, `channel` or `playlist`; `duration` `short` (under 3 minutes), `medium` (3–20) or `long`; `upload_date` `today`, `week`, `month` or `year`; `features` a list of `live`, `4k`, `hd`, `subtitles`, `creative_commons`, `360`, `vr180`, `3d`, `hdr` or `location`; `sort` `relevance` or `popularity`; `exact` searches without spelling correction. `chip` is a chip's token |
| `search.focus` | puts the cursor in the search box (`/` does this) |
| `search.suggest` | `{ query }`: asks for suggestions, which arrive as `search.suggestions` |
| `search.more` | loads more results on the search page |
| `search.history` | → earlier searches, newest first (kept on this computer) |
| `search.history_remove`, `search.history_clear` | `{ query }`; forgets one or all |

The actions (rating, commenting, voting, subscribing) return at once and
report how they went with the events above, so every widget showing the same
comment or channel updates, whoever started the action.

### Keys

```lua
app.keymap.set("ctrl+f", "my_plugin.find")            -- everywhere
app.keymap.set("n", "player.seek_by", { seconds = 30 }, "watch")   -- one page
app.keymap.set("n", "player.seek_by", { seconds = 30 }, "player")  -- wherever the video shows
app.keymap.unset("space", "watch")
app.keymap.clear()                                      -- no keys at all
app.keymap.list()
app.bind("ctrl+g", function() app.toast("hi") end)      -- a Lua function
```

Keys use GTK's names (`space`, `Left`, `F5`, `<Control>f`) or the short form
(`ctrl+shift+f`). Shifted symbols are named by what they type: Shift+. is
`greater`. The `fullscreen` scope applies while the video fills the screen,
before every other (Escape leaves fullscreen there; elsewhere it goes back).
The `player` scope applies on the watch page and wherever the mini
player shows (k, j, l, m, c, <, >, Shift+N are there by default; space, the
arrows, digits, f and t only on the watch page; Shift+P plays the list's previous video). A page's own bindings win
over `player` ones, and those over global ones. Keys without Ctrl or
Alt (Escape too) don't fire while you type in a text field.

### Components

Each part of the UI is a component: a function that builds a widget from
properties, and optionally one that refills an existing widget (cards are
reused while scrolling).

```lua
-- Replace a part of the UI, building on the original:
local original = app.component("video_card", {
  build = function(props) local card = original.build(props) ... return card end,
  update = function(card, props) original.update(card, props) ... end,
})

-- Or change every widget a component makes:
app.wrap("video_card", function(card, video, phase)   -- phase: "build" or "update"
end)

app.build("chip_bar", { chips = { "A", "B" }, selected = 0 })
app.components()
```

| Component | Properties |
|---|---|
| `shell` | the window layout. A replacement must put `app.page_host()` (where pages appear) somewhere in it |
| `sidebar` | `{ mini? }`: YouTube's guide; with `mini`, its rail of icons |
| `topbar` | |
| `mini_player` | the video in the bottom-right corner while another page shows; the player itself moves into it |
| `toast` | `{ message, action?, seconds? }`: a toast's content |
| `feed` | `{ browse_id, params?, untitled?, refresh_on_show?, page?, on_channel?, sortable?, order? }`: a feed page. `params` picks a channel tab; `sortable` (a channel's Videos) adds chips for Latest, Popular and Oldest, and `order` (`"latest"`, `"popular"`, `"oldest"`) picks one; `page` is a first page to show instead of loading it; `on_channel` leaves the channel out of the cards (on its own page); `layout = "list"` shows one video a row (History); `title` puts a heading over it |
| `shelves` | `{ page, browse_id, on_channel? }`: a page's shelves as rows that scroll sideways (a channel's home) |
| `chip_bar` | `{ chips, selected, browse_id?, event? }`: clicking a chip emits `event` (`feed.chip_selected` by default) with `{ browse_id, index }` |
| `video_card` | a video, plus `layout?` (`"compact"`: a row, as in "Up next"; `"shelf"`: narrower, for a shelf's row; `"list"`: a wide row, as in History) and `on_channel?` (without the channel's name and picture) |
| `watch` | `{ video }`: the watch page. Showing it for a video starts that video, so going back to a video plays it again |
| `player`, `player_controls` | |
| `signin` | |
| `channel` | `{ channel_id }`, `{ handle }` or `{ name, video_id }`: the channel page |
| `comments` | `{ video_id, count, sorts }`: the comments section |
| `comment` | a comment plus `{ video_id, reply, by_creator }` |
| `comment_composer` | `{ video_id, parent_id?, mention?, autofocus? }`: the box to write a comment or reply |
| `subscribe_button` | `{ subscription, channel_name }` |
| `sidebar_subscriptions` | the sidebar's list of subscribed channels |
| `account_button` | the user's picture in the top bar, with its menu; signed out, "Sign in" |
| `menu` | `{ items }`: a menu's list. An item is `{ label, icon?, command, args?, confirm?, selected? }`; with `confirm` (a `confirm_dialog`'s properties) it asks first. `{ separator = true }` draws a line between groups |
| `confirm_dialog` | `{ title, body?, confirm, cancel? }`: a dialog's content. A replacement needs buttons named `dialog-cancel` and `dialog-confirm` |
| `player_settings` | `{ entries }`: the settings panel's pages |
| `playlist_card` | a playlist (`{ id, title, thumbnail?, count?, details? }`), plus `layout? = "shelf"` |
| `playlist` | `{ playlist_id }`: a playlist's page |
| `library` | the "You" page: rows of history, playlists, Watch later and liked videos |
| `playlists` | the user's playlists, as a grid |
| `list_panel` | the list playing, above "Up next" on the watch page |
| `live_chat` | a live stream's chat, above "Up next": it follows the video on screen and shows only for a live stream (or one about to start) |
| `save_dialog` | `{ video_id }`: "Save video to..." |
| `settings` | the Settings page |
| `downloads` | the Downloads page |
| `channels` | "All subscriptions" (youtube.com's feed/channels): every subscription, as YouTube orders them or A to Z, each with its subscribe button; the sidebar's "Subscriptions ›" opens it |
| `search_box` | the search field in the top bar, with its list of earlier searches and suggestions |
| `search` | `{ query, filters?, chip?, chips? }`: the search page |
| `search_video`, `search_channel`, `search_playlist`, `search_shelf` | a result (an item of `search.page`); shelves are "People also watched", a row of Shorts, ... |
| `search_correction` | `{ query, original, applied }`: "Showing results for ..." or "Did you mean ..." |
| `search_footer` | `{ state = "loading" \| "error" \| "end", message? }`: the end of the results |
| `search_filters` | `{ query, filters }`: the content of the filters dialog |

```lua
-- A layout without the sidebar:
app.component("shell", { build = function()
  return ui.box { orientation = "vertical", children = { app.build("topbar"), app.page_host() } }
end })
```

### Slots

Named places in the built-in layout where plugins add widgets:

```lua
local id = app.slot("topbar.end", function() return ui.button("★") end, order)
app.unslot(id)
```

`sidebar.top`, `sidebar.bottom`, `topbar.start`, `topbar.center` (after
the search box), `topbar.end`, `feed.header`, `search.header`,
`watch.actions`, `watch.below_player`, `live_chat.header` (the live chat's
header),
`player.overlay`, `player.controls.start`, `player.controls.end`,
`settings.sections` (the Settings page: add an `AdwPreferencesGroup`),
`mini_player.info` (under the mini player's title), `playlist.panel` (under
a playlist page's details), `library.bottom` (the end of the "You" page),
`comments.header`, `channel.header.actions`.

`player.overlay` lies over the video: each widget there sits where its own
`halign`, `valign` and margins put it (`halign = "end", valign = "end",
margin_bottom = 76` is above the controls on the right), and clicks and the
pointer pass through the rest to the video. Every other slot is a box.
A button (or menu button) in `player.controls.start` or `.end` with the
`control` CSS class looks like the player's own.

[SponsorBlock](../plugins/sponsorblock) is a complete plugin using
most of this: segments on the seek bar, widgets over the video, settings,
storage, its own commands, keys and dialog, and a web API.

[Notifications](../plugins/notifications) adds YouTube's real bell
through `topbar.end`, with replaceable components, commands, events and a
`notifications.page` filter. It signs in with an embedded WebKit window,
keeps its account in the keyring with libsecret and talks to YouTube with
libsoup, all through `gi`. The plugin's README lists its extension points.

### Pages and the sidebar

```lua
app.page { name = "notes", title = "Notes", keep = true,
  build = function(args) return ui.label("...") end,
  update = function(widget, args) end }      -- optional
app.nav { id = "notes", title = "Notes", icon = "accessories-text-editor-symbolic",
  page = "notes", section = "library", order = 10,     -- sections: main, library, bottom
  needs_account = false }                             -- true: hidden signed out
app.navigate("notes", { any = "args" })
app.back()
app.forward()
app.current_page()
app.pages()
```

Built-in pages: `home`, `subscriptions`, `history` (the `feed` component with
different browse IDs), `watch`, `channel`, `search`, `signin`.

### Theme

```lua
local id = app.css [[ :root { --yt-accent: #a855f7; } ]]
app.uncss(id)
```

The built-in stylesheet is `crates/yt-app/src/ui/style.css`; colors and the
corner radius are CSS variables (`--yt-bg`, `--yt-raised`, `--yt-text`,
`--yt-text-dim`, `--yt-accent`, `--yt-radius`). The logo is YouTube's
wordmark, its play button (`.logo-mark`) and its name (`.logo`) each drawn
in their CSS `color`: the accent and the text color by default. Every
button gets the pointing-hand cursor unless it was given another one.

### The widget tree

`app.window()` and `app.find(query)` reach every widget. Built-in widgets
have names: `main-window`, `shell`, `content`, `page-host`, `topbar`,
`sidebar`, `sidebar-items`, `sidebar-subscriptions`, `nav:<id>`,
`channel:<id>` (a subscription in the sidebar), `chip-bar`, `video-grid`,
`page-watch`, `watch-info` (the watch page's scrolled window), `watch-player`,
`watch-related` ("Up next"),
`comments`, `comment:<id>`, `page-channel`,
`player`, `player-controls`, `page-signin`, `search-box`, `search-results`,
and `slot:<name>` for slots.

## `app.api` and `app.hook`: the app's services

Every function of the app's services can be called and hooked. The list,
with each function's parameters and what it does, is
[api.md](api.md); `app.functions()` gives it at run time.

```lua
-- Calls: arguments in order. Async functions take a callback last.
local state = app.api.player.state()
app.api.mpv.set_property("speed", "1.5")
app.api.youtube.browse("FEwhat_to_watch", function(page, err) ... end)
app.api.youtube.call("browse", { browseId = "FEtrending" }, function(json, err) end)

-- Hooks: around the function, whoever calls it (the app itself, too).
local id = app.hook("streams.resolve", function(args, next)
  local set = next(args)          -- the original (and inner hooks)
  set.video = set.qualities[#set.qualities]   -- the smallest quality
  return set
end)
app.unhook(id)
```

A hook gets the arguments by name (`args.video_id`), may change them
before passing them on (`next(args)`, or `next()` for them as they came),
may call `next` more than once or not at all, and returns the result,
changed or replaced. An error in a hook is the call's error. The last hook
added runs first. Hooks run on the main thread; a call made elsewhere (a
request on the network threads) waits for them.

Some services, and what hooking them does:

| Service | |
|---|---|
| `youtube` | YouTube's API (InnerTube): `call` is every request the app makes as the TV app, signed in; `browse`, `watch`, `search`, `channel`, `playlist`, `comments`, `subscribe`... the parsed ones. Hook `call` to see or change every request and response |
| `streams` | the stream resolver: `resolve` picks a video's streams. Hook it to choose other qualities or codecs |
| `player`, `mpv` | the player (`load`, `seek`, `set_quality`, `state`...) and mpv underneath (`command`, `set_property`, `get_string`) |
| `playback`, `list` | what plays and what plays next: `play`, `next`, the queue and playlists |
| `app` | pages and toasts: `navigate`, `back`, `toast` |
| `feed`, `watch`, `channel`, `search_results`, `comments`, `live_chat`... | YouTube's responses read into the app's data: hook `feed.parse` to change every feed the app reads |
| `settings`, `downloads`, `search_history`, `channels`, `tracker`, `session`, `oauth` | |

Data crosses as tables, as described above. An empty Lua table in a place
that wants a list reads as an empty list.

## `app`: helpers

| | |
|---|---|
| `app.http({ url, method?, headers?, body? }, fn(response, err))` | `response = { status, headers, body }`. A table `body` is sent as JSON |
| `app.timer(ms, fn, repeat?)` → `t:cancel()` | `fn` returning `false` stops a repeating timer |
| `app.now()` | seconds, with fractions, on a clock that only goes forward: for measuring how long something takes |
| `app.idle(fn)` | |
| `app.toast(message, options?)` | `options` as for the `app.toast` command: `{ action = { label, command, args }, seconds }` |
| `app.log(...)` | |
| `app.screenshot(path)` | |
| `app.json.encode(v)`, `app.json.decode(s)`, `app.array(t)` | |
| `app.hash.sha256(text)` | its SHA-256, in hex |
| `app.storage(name)` → `store` | what a plugin keeps between runs: a table of what was saved under `name` (letters, digits, `-`, `_`); change it and call `store:save()`. A JSON file in the config directory's `plugin-data` |
| `app.dialog(content, { title?, width? })` | shows `content` (a widget) in a dialog over the window, as the app's own; `app.run("app.close_dialog")` closes it |
| `app.quit()` | |

Where things are: `app.api.paths.config_dir()`, `cache_dir()`,
`plugins_dir()`, `downloads_dir()`.

## C libraries

A library with introspection data is reached through `gi` (see
[browser](../examples/plugins/browser/init.lua) and
[terminal](../examples/plugins/terminal/init.lua)). For any other,
LuaJIT's `ffi` calls C directly. Objects made in C become Lua objects with
`ui.from_ffi(pointer)` (or `gi.wrap`), and go back to C with `o:ptr()`:

```lua
local ffi = require("ffi")
ffi.cdef [[ void *my_widget_new(void); ]]
local lib = ffi.load("libmywidget.so")
local widget = ui.from_ffi(lib.my_widget_new())   -- now a normal widget
widget.vexpand = true
```
