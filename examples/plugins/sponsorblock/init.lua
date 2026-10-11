-- SponsorBlock for yt-rs: skips sponsor segments and the other parts of a
-- video the SponsorBlock community marked (https://sponsor.ajay.app), as
-- the browser extension does.
--
--  * Privacy: it asks for the segments of every video whose ID hashes to the
--    same first four characters (SHA-256), so the server can't tell which
--    one is playing.
--  * Every category, each set to skip automatically, show a skip button,
--    only show on the seek bar, or nothing (Settings, "SponsorBlock").
--  * After a skip, a notice with "Unskip" and votes for the segment; inside
--    a segment set to a button, "Skip"; "Jump to highlight" for a video's
--    highlight; segments to mute are muted; a label for videos that are a
--    sponsor (or exclusive access) as a whole.
--  * The segments on the seek bar, in SponsorBlock's colors.
--  * Skips are counted for the people who marked them (Settings can turn
--    that off).
--  * Marking new segments (";" where one starts and again where it ends)
--    and submitting them ("'").
--
-- Install: copy or link this directory into the plugins directory (see
-- Settings, "About").

local NAME = "sponsorblock"
local _, DIR = ...
local store = app.storage(NAME)

local Gtk = gi.require("Gtk", "4.0")
local Gdk = gi.require("Gdk", "4.0")
-- The extension's own player icons, found by name as the app's are (see
-- icons/NOTICE).
Gtk.IconTheme.get_for_display(Gdk.Display.get_default()):add_search_path(DIR .. "/icons")

-- id, as people say it, color, what it does unless set otherwise.
local CATEGORIES = {
  { id = "sponsor", name = "Sponsor", color = "#00d400", default = "auto" },
  { id = "selfpromo", name = "Unpaid/Self promotion", color = "#ffff00", default = "button" },
  { id = "exclusive_access", name = "Exclusive access", color = "#008a5c", default = "show" },
  { id = "interaction", name = "Interaction reminder", color = "#cc00ff", default = "button" },
  { id = "poi_highlight", name = "Highlight", color = "#ff1684", default = "button" },
  { id = "intro", name = "Intermission/Intro animation", color = "#00ffff", default = "show" },
  { id = "outro", name = "Endcards/Credits", color = "#0202ed", default = "show" },
  { id = "preview", name = "Preview/Recap", color = "#008fd6", default = "show" },
  { id = "filler", name = "Tangents/Jokes", color = "#7300ff", default = "show" },
  { id = "music_offtopic", name = "Music: non-music section", color = "#ff9900", default = "show" },
}
local BY_ID = {}
for _, c in ipairs(CATEGORIES) do BY_ID[c.id] = c end
-- The choices, in the order the settings offer them.
local ACTIONS = { "auto", "button", "show", "off" }
local ACTION_NAMES = { "Skip automatically", "Show a skip button", "Show in the seek bar", "Off" }

-- Settings, kept between runs.
store.server = store.server or "https://sponsor.ajay.app"
store.actions = store.actions or {}
for _, c in ipairs(CATEGORIES) do store.actions[c.id] = store.actions[c.id] or c.default end
if store.count_skips == nil then store.count_skips = true end
if store.notice == nil then store.notice = true end
store.min_duration = store.min_duration or 0
-- Your anonymous ID for votes and submissions (never anything about you).
store.user_id = store.user_id or app.hash.sha256(tostring(os.time()) .. app.now() .. tostring({}) .. math.random())
store:save()

local function log(...) app.log("[sponsorblock]", ...) end
local function action(category) return store.actions[category] or "off" end
local function fmt(t)
  t = math.max(0, math.floor(t + 0.5))
  return string.format("%d:%02d", math.floor(t / 60), t % 60)
end
local function url_encode(text)
  return (text:gsub("[^%w%-_%.~]", function(c) return string.format("%%%02X", c:byte()) end))
end

-- The video playing and what is known about it.
local video_id, segments = nil, {}
local skipped, unskipped = {}, {}   -- by UUID
local muted_by_us = false
local exact_timer = nil
local last_skipped = nil            -- for "Unskip" and votes
local marks, mark_start = {}, nil   -- segments being marked for submitting
local lookup = "idle"               -- idle, looking, done or failed

-- Widgets, made in their slots.
local overlay = {}   -- notice, skip, highlight
local label_box = nil
local controls = {}  -- the player's buttons
local refresh_controls -- updates them; set with them below

---------------------------------------------------------------------------
-- The server
---------------------------------------------------------------------------

local function post(path, query, body, done)
  local parts = {}
  for k, v in pairs(query or {}) do table.insert(parts, k .. "=" .. url_encode(tostring(v))) end
  local url = store.server .. path .. (#parts > 0 and ("?" .. table.concat(parts, "&")) or "")
  app.http({ url = url, method = "POST", body = body }, function(response, err)
    local ok = response and response.status >= 200 and response.status < 300
    if not ok then log("POST", path, "failed:", err or (response.status .. " " .. (response.body or ""))) end
    if done then done(ok, response, err) end
  end)
end

local function count_skip(segment)
  if store.count_skips then post("/api/viewedVideoSponsorTime", { UUID = segment.UUID }) end
end

local function vote(segment, up)
  post("/api/voteOnSponsorTime", { UUID = segment.UUID, userID = store.user_id, type = up and 1 or 0 }, nil,
    function(ok, response)
      if ok then app.toast(up and "Thanks for voting" or "Thanks: segments with fewer votes are shown less")
      else app.toast("SponsorBlock didn't take the vote" .. (response and (": " .. response.status) or "")) end
    end)
end

---------------------------------------------------------------------------
-- Showing the segments
---------------------------------------------------------------------------

local function show_ranges()
  local ranges = {}
  for _, s in ipairs(segments) do
    if s.actionType ~= "full" then
      table.insert(ranges, { start = s.segment[1], ["end"] = s.segment[2], color = BY_ID[s.category].color,
        label = BY_ID[s.category].name .. (s.actionType == "mute" and " (muted)" or "") })
    end
  end
  app.run("seekbar.set_ranges", { owner = NAME, ranges = app.array(ranges) })
end

local function show_marks()
  local ranges = {}
  for _, m in ipairs(marks) do
    table.insert(ranges, { start = m.start, ["end"] = m["end"], color = "#ffffff99", label = "To submit" })
  end
  if mark_start then table.insert(ranges, { start = mark_start, ["end"] = mark_start, color = "#ffffff", label = "Marked start" }) end
  app.run("seekbar.set_ranges", { owner = NAME .. ".marks", ranges = app.array(ranges) })
end

-- The label for a video that is a sponsor (or exclusive access) as a whole.
local function show_label()
  if not label_box then return end
  for _, child in ipairs(label_box:children()) do label_box:remove(child) end
  for _, s in ipairs(segments) do
    if s.actionType == "full" and action(s.category) ~= "off" then
      local c = BY_ID[s.category]
      local dot = ui.drawing_area(function(cr, w, h) cr:set_color(c.color); cr:arc(w / 2, h / 2, 5, 0, 2 * math.pi); cr:fill() end)
      dot.width_request, dot.height_request = 12, 12
      label_box:append(ui.box { orientation = "horizontal", spacing = 6, class = "sb-label",
        tooltip_text = "The whole video is labelled " .. c.name:lower() .. " on SponsorBlock",
        children = { dot, ui.label(c.name) } })
    end
  end
  label_box.visible = #label_box:children() > 0
end

local function hide(name)
  if overlay[name] then overlay[name].visible = false end
end

local notice_timer
local function show_notice(segment)
  local notice = overlay.notice
  if not notice or not store.notice then return end
  notice:find("sb-notice-text").label = "Skipped " .. BY_ID[segment.category].name:lower()
  notice.visible = true
  if notice_timer then notice_timer:cancel() end
  notice_timer = app.timer(5000, function() notice_timer = nil; hide("notice") end)
end

---------------------------------------------------------------------------
-- Skipping
---------------------------------------------------------------------------

local function skip(segment, automatic)
  skipped[segment.UUID] = true
  last_skipped = segment
  local to = segment.segment[2]
  local duration = app.api.player.state().duration or 0
  if duration > 0 then to = math.min(to, duration - 0.05) end
  app.run("player.seek", { position = to })
  log(automatic and "skipped" or "skipped (button)", segment.category, fmt(segment.segment[1]), "->", fmt(to))
  hide("skip")
  show_notice(segment)
  count_skip(segment)
end

local function unskip()
  local s = last_skipped
  if not s then return end
  unskipped[s.UUID] = true
  hide("notice")
  app.run("player.seek", { position = s.segment[1] })
end

local function current(position, wanted)
  for _, s in ipairs(segments) do
    if s.actionType == wanted and position >= s.segment[1] and position < s.segment[2] then return s end
  end
end

-- Looks at where playback is: skips, mutes, and shows the buttons.
local function check(position)
  if not video_id then return end
  local speed = app.api.player.state().speed or 1
  -- Rewound before a segment: it can be skipped again.
  for _, s in ipairs(segments) do
    if position < s.segment[1] - 0.5 then skipped[s.UUID] = nil end
    if position < s.segment[1] - 0.5 or position >= s.segment[2] then unskipped[s.UUID] = nil end
  end
  local inside = current(position, "skip")
  if inside and not unskipped[inside.UUID] and not skipped[inside.UUID]
      and position < inside.segment[2] - 0.2 then
    local a = action(inside.category)
    if a == "auto" then return skip(inside, true) end
  end
  -- The button, while inside a segment set to one.
  local button = inside and action(inside.category) == "button" and not unskipped[inside.UUID] and inside or nil
  if overlay.skip then
    overlay.skip.visible = button ~= nil
    if button then overlay.skip:find("sb-skip-text").label = "Skip " .. BY_ID[button.category].name:lower() end
  end
  -- Muting.
  local mute = current(position, "mute")
  if mute and action(mute.category) == "auto" and not muted_by_us then
    if app.api.mpv.get_string("mute") ~= "yes" then
      muted_by_us = true
      app.api.mpv.set_property("mute", "yes")
    end
  elseif not (mute and action(mute.category) == "auto") and muted_by_us then
    muted_by_us = false
    app.api.mpv.set_property("mute", "no")
  end
  -- The highlight, until playback gets there.
  local highlight
  for _, s in ipairs(segments) do
    if s.actionType == "poi" and action(s.category) ~= "off" and action(s.category) ~= "show" and position < s.segment[1] - 1 then
      highlight = s
    end
  end
  if overlay.highlight then overlay.highlight.visible = highlight ~= nil end
  -- To the moment, rather than the next position report (every 0.25 s).
  if exact_timer then exact_timer:cancel(); exact_timer = nil end
  for _, s in ipairs(segments) do
    local wait = (s.segment[1] - position) / speed
    if (s.actionType == "skip" or s.actionType == "mute") and wait > 0 and wait < 0.6 and action(s.category) == "auto" then
      exact_timer = app.timer(math.floor(wait * 1000), function()
        exact_timer = nil
        check(app.api.player.state().position or 0)
      end)
      break
    end
  end
end

---------------------------------------------------------------------------
-- Looking the segments up
---------------------------------------------------------------------------

local function clear()
  segments, skipped, unskipped, last_skipped = {}, {}, {}, nil
  marks, mark_start = {}, nil
  if muted_by_us then muted_by_us = false; app.api.mpv.set_property("mute", "no") end
  for name in pairs(overlay) do hide(name) end
  show_label()
  refresh_controls()
end

local function look_up(id)
  local categories, wanted = {}, {}
  for _, c in ipairs(CATEGORIES) do
    if action(c.id) ~= "off" then table.insert(categories, c.id); wanted[c.id] = true end
  end
  if #categories == 0 then lookup = "done"; return end
  lookup = "looking"
  local prefix = app.hash.sha256(id):sub(1, 4)
  local url = store.server .. "/api/skipSegments/" .. prefix
    .. "?categories=" .. url_encode(app.json.encode(categories))
    .. "&actionTypes=" .. url_encode(app.json.encode({ "skip", "mute", "full", "poi" }))
  app.http({ url = url }, function(response, err)
    if id ~= video_id then return end
    if not response or (response.status ~= 200 and response.status ~= 404) then
      lookup = "failed"
      return log("lookup failed:", err or response.status)
    end
    lookup = "done"
    local found = {}
    for _, entry in ipairs(response.status == 200 and (app.json.decode(response.body) or {}) or {}) do
      if entry.videoID == id then
        for _, s in ipairs(entry.segments or {}) do
          local long_enough = s.actionType == "full" or s.actionType == "poi"
            or (s.segment[2] - s.segment[1]) >= (store.min_duration or 0)
          if wanted[s.category] and BY_ID[s.category] and long_enough then table.insert(found, s) end
        end
      end
    end
    table.sort(found, function(a, b) return a.segment[1] < b.segment[1] end)
    segments = found
    log(#segments, "segments for", id)
    show_ranges()
    show_label()
    app.emit("sponsorblock.segments", { video_id = id, segments = app.array(segments) })
    check(app.api.player.state().position or 0)
  end)
end

app.on("player.loading", function(e)
  clear()
  video_id = e.video_id
  lookup = "idle"
end)
app.on("player.loaded", function()
  local state = app.api.player.state()
  if state.live or not video_id then return end
  look_up(video_id)
end)
app.on("player.position", function(e) check(e.position) end)
app.on("player.seeked", function(e) check(e.position) end)
app.on("player.stopped", function() video_id = nil; clear() end)
-- The mini player has no room for the buttons.
app.on("player.docked", function(e)
  if e.slot == "mini" then for name in pairs(overlay) do hide(name) end end
end)

---------------------------------------------------------------------------
-- Widgets
---------------------------------------------------------------------------

-- A button with only an icon.
local function icon_button(icon, tooltip, on_click)
  local b = ui.new("GtkButton", { icon_name = icon, tooltip_text = tooltip })
  b:add_css_class("sb-notice-button")
  b:on("clicked", on_click)
  return b
end

local function pill(icon, text_name, text, on_click, class)
  local b = ui.build { "GtkButton", class = "sb-button " .. (class or ""),
    child = ui.box { orientation = "horizontal", spacing = 8, children = {
      icon and ui.new("GtkImage", { icon_name = icon }) or nil,
      { "GtkLabel", name = text_name, label = text },
    } } }
  b:on("clicked", on_click)
  return b
end

-- The notice, at the bottom left over the video (the player's overlay slot
-- places each widget by its own alignment and lets clicks through the rest).
app.slot("player.overlay", function()
  local notice = ui.box { orientation = "horizontal", spacing = 4, name = "sb-notice", class = "sb-notice",
    halign = "start", valign = "end", margin_start = 12, margin_bottom = 76, visible = false,
    children = {
      { "GtkLabel", name = "sb-notice-text", label = "", margin_start = 8, margin_end = 8 },
      ui.button("Unskip", unskip, { class = "sb-notice-button", name = "sb-unskip" }),
      icon_button("yt-like-symbolic", "This segment is right", function() if last_skipped then vote(last_skipped, true) end end),
      icon_button("yt-dislike-symbolic", "This segment is wrong", function() if last_skipped then vote(last_skipped, false) end end),
      icon_button("yt-close-symbolic", "Close", function() hide("notice") end),
    } }
  overlay.notice = notice
  return notice
end)

-- "Skip ..." and "Jump to highlight", at the bottom right.
app.slot("player.overlay", function()
  local skip_button = pill("yt-next-symbolic", "sb-skip-text", "Skip", function()
    local s = current(app.api.player.state().position or 0, "skip")
    if s then skip(s, false) end
  end, "sb-skip")
  skip_button.name = "sb-skip"
  local highlight = pill("yt-forward-symbolic", "sb-highlight-text", "Jump to highlight", function()
    app.run("sponsorblock.jump_highlight")
  end, "sb-highlight")
  highlight.name = "sb-highlight"
  local corner = ui.box { orientation = "vertical", spacing = 8, halign = "end", valign = "end",
    margin_end = 12, margin_bottom = 76, children = { highlight, skip_button } }
  skip_button.visible, highlight.visible = false, false
  overlay.skip, overlay.highlight = skip_button, highlight
  return corner
end)

app.slot("watch.actions", function()
  label_box = ui.box { orientation = "horizontal", spacing = 8, name = "sb-labels", visible = false }
  show_label()
  return label_box
end, -10)

-- The player's buttons, as the extension puts them in YouTube's: the
-- segments of the video ("i"), and marking new ones: "Segment starts now"
-- (then "Segment ends now"), with Cancel while marking, and Clear and
-- Submit once there are segments marked.
function refresh_controls()
  if not controls.start then return end
  local creating = mark_start ~= nil
  controls.start.icon_name = creating and "sponsorblock-stop" or "sponsorblock-start"
  controls.start.tooltip_text = creating and "Segment ends now" or "Segment starts now"
  controls.cancel.visible = creating
  controls.submit.visible = #marks > 0
  controls.clear.visible = #marks > 1 or (#marks > 0 and not creating)
end

local function small_button(icon, tooltip, on_click)
  local b = Gtk.Button { icon_name = icon, tooltip_text = tooltip, valign = "center" }
  b:add_css_class("flat")
  b:add_css_class("sb-info-button")
  b:on("clicked", on_click)
  return b
end

-- A segment in the info popover: its category, when, votes, and a jump to it.
local function segment_row(s, popover)
  local c = BY_ID[s.category] or { name = s.category, color = "#888888" }
  local when = s.actionType == "full" and "The whole video"
    or s.actionType == "poi" and ("At " .. fmt(s.segment[1]))
    or (fmt(s.segment[1]) .. " to " .. fmt(s.segment[2]))
  local up, down
  local function voted(is_up)
    vote(s, is_up)
    up.sensitive, down.sensitive = false, false
  end
  up = small_button("yt-like-symbolic", "This segment is right", function() voted(true) end)
  down = small_button("yt-dislike-symbolic", "This segment is wrong", function() voted(false) end)
  local row = ui.box { orientation = "horizontal", spacing = 10, class = "sb-info-row", children = {
    Gtk.Label { use_markup = true, valign = "center",
      label = string.format("<span foreground='%s'>\u{25CF}</span>", c.color) },
    ui.box { orientation = "vertical", hexpand = true, valign = "center", children = {
      { "GtkLabel", label = c.name, xalign = 0, class = "sb-info-category", ellipsize = "end" },
      { "GtkLabel", label = when, xalign = 0, class = "sb-info-time" },
    } },
    up, down,
  } }
  if s.actionType ~= "full" then
    row:append(small_button("yt-play-symbolic", "Jump to it", function()
      app.run("player.seek", { position = s.segment[1] })
      popover:popdown()
    end))
  else
    -- Where the jump would be, so the votes line up with the others'.
    row:append(Gtk.Box { width_request = 32 })
  end
  return row
end

-- What the info popover shows, made each time it opens.
local function fill_info(popover)
  local status
  if not video_id then status = "Nothing is playing"
  elseif lookup == "looking" or lookup == "idle" then status = "Looking for segments\u{2026}"
  elseif lookup == "failed" then status = "SponsorBlock couldn't be reached"
  elseif #segments == 0 then status = "No segments for this video"
  elseif #segments == 1 then status = "This video has 1 segment"
  else status = "This video has " .. #segments .. " segments" end
  local list = ui.box { orientation = "vertical", spacing = 2, name = "sb-info-list" }
  for _, s in ipairs(segments) do list:append(segment_row(s, popover)) end
  local scroller = Gtk.ScrolledWindow { child = list, hscrollbar_policy = "never",
    propagate_natural_height = true, max_content_height = 360 }
  scroller.visible = #segments > 0
  local content = ui.box { orientation = "vertical", spacing = 10, class = "sb-info", name = "sb-info", children = {
    ui.box { orientation = "horizontal", spacing = 10, children = {
      Gtk.Image { icon_name = "sponsorblock-info", pixel_size = 24 },
      { "GtkLabel", label = "SponsorBlock", xalign = 0, hexpand = true, class = "sb-info-title" },
    } },
    { "GtkLabel", label = status, xalign = 0, wrap = true, class = "sb-info-status", name = "sb-info-status" },
    scroller,
  } }
  if #marks > 0 or mark_start then
    local marked = #marks == 1 and "1 segment marked" or (#marks .. " segments marked")
    if mark_start then marked = marked .. ", one being marked from " .. fmt(mark_start) end
    local submit = ui.button("Submit", function() popover:popdown(); app.run("sponsorblock.submit") end,
      { class = "sb-info-action" })
    submit.visible = #marks > 0
    content:append(ui.box { orientation = "horizontal", spacing = 10, children = {
      { "GtkLabel", label = marked, xalign = 0, hexpand = true, wrap = true, class = "sb-info-status" },
      submit,
    } })
  end
  local settings = ui.button("Settings", function()
    popover:popdown()
    app.navigate("settings")
  end, { class = "sb-info-action", halign = "start" })
  content:append(settings)
  popover.child = content
end

app.slot("player.controls.end", function()
  local function control(name, icon, tooltip, command)
    local b = Gtk.Button { icon_name = icon, tooltip_text = tooltip, name = "sb-control-" .. name }
    b:add_css_class("control")
    b:add_css_class("sb-control")
    b:on("clicked", function()
      local ok, err = pcall(app.run, command)
      if not ok then app.toast(tostring(err)) end
    end)
    controls[name] = b
    return b
  end
  local popover = Gtk.Popover { has_arrow = false }
  popover:add_css_class("sb-info-popover")
  popover:on("show", function() fill_info(popover) end)
  local info = Gtk.MenuButton { icon_name = "sponsorblock-info", tooltip_text = "SponsorBlock", name = "sb-control-info",
    popover = popover }
  info:add_css_class("control")
  info:add_css_class("sb-control")
  controls.info = info
  local box = ui.box { orientation = "horizontal", spacing = 2, children = {
    info,
    control("submit", "sponsorblock-submit", "Submit segments", "sponsorblock.submit"),
    control("clear", "sponsorblock-clear", "Clear segments", "sponsorblock.clear_marks"),
    control("cancel", "sponsorblock-cancel", "Cancel marking the segment", "sponsorblock.cancel_mark"),
    control("start", "sponsorblock-start", "Segment starts now", "sponsorblock.mark"),
  } }
  refresh_controls()
  return box
end)

---------------------------------------------------------------------------
-- Commands and keys
---------------------------------------------------------------------------

app.command("sponsorblock.skip", function()
  local s = current(app.api.player.state().position or 0, "skip")
  if not s then return nil end
  skip(s, false)
end)
app.command("sponsorblock.unskip", function() unskip() end)
app.command("sponsorblock.vote", function(args)
  local target = last_skipped
  if args and args.uuid then
    for _, s in ipairs(segments) do if s.UUID == args.uuid then target = s end end
  end
  if not target then error("no segment to vote on") end
  vote(target, not (args and args.up == false))
end)
app.command("sponsorblock.jump_highlight", function()
  for _, s in ipairs(segments) do
    if s.actionType == "poi" then
      app.run("player.seek", { position = s.segment[1] })
      hide("highlight")
      return
    end
  end
  error("this video has no highlight")
end)
app.command("sponsorblock.segments", function() return app.array(segments) end)

-- ";" where a segment starts, and again where it ends.
app.command("sponsorblock.mark", function()
  local state = app.api.player.state()
  if not state.video_id then error("no video is playing") end
  local t = state.position or 0
  if not mark_start then
    mark_start = t
    app.toast("Segment starts at " .. fmt(t) .. ". Press ; again where it ends.")
  else
    local a, b = math.min(mark_start, t), math.max(mark_start, t)
    mark_start = nil
    if b - a < 0.5 then
      app.toast("That segment is too short")
    else
      table.insert(marks, { start = a, ["end"] = b, category = "sponsor" })
      app.toast(string.format("Segment %s \u{2013} %s marked. Press ' to submit.", fmt(a), fmt(b)))
    end
  end
  show_marks()
  refresh_controls()
end)

app.command("sponsorblock.cancel_mark", function()
  mark_start = nil
  show_marks()
  refresh_controls()
end)

-- Forgets the segments marked, after asking.
app.command("sponsorblock.clear_marks", function(args)
  if #marks == 0 and not mark_start then return end
  if not (args and args.confirmed) then
    app.run("app.confirm", { title = "Clear the segments marked?", body = "They haven't been submitted.",
      confirm = "Clear", command = "sponsorblock.clear_marks", args = { confirmed = true } })
    return
  end
  marks, mark_start = {}, nil
  show_marks()
  refresh_controls()
end)

local function submit(dialog_rows)
  local state = app.api.player.state()
  local list = {}
  for i, m in ipairs(marks) do
    local row = dialog_rows and dialog_rows[i]
    local category = row and CATEGORIES[(row.selected or 0) + 1].id or m.category
    table.insert(list, { segment = { m.start, m["end"] }, category = category, actionType = "skip" })
  end
  if #list == 0 then error("no segments marked: press ; where one starts and again where it ends") end
  local body = { videoID = state.video_id, userID = store.user_id, userAgent = "yt-rs sponsorblock plugin",
    videoDuration = state.duration, segments = list }
  post("/api/skipSegments", nil, body, function(ok, response)
    if ok then
      marks = {}
      show_marks()
      refresh_controls()
      app.toast("Submitted. Thank you for helping everyone skip these!")
      app.run("app.close_dialog")
      look_up(state.video_id)
    else
      app.toast("SponsorBlock didn't take them: " .. (response and (response.body or response.status) or "no answer"))
    end
  end)
end

-- "'": the segments marked, each with its category, and Submit.
app.command("sponsorblock.submit", function()
  if #marks == 0 then error("no segments marked: press ; where one starts and again where it ends") end
  local rows = {}
  local list = ui.box { orientation = "vertical", spacing = 8 }
  local names = {}
  for _, c in ipairs(CATEGORIES) do if c.id ~= "poi_highlight" and c.id ~= "exclusive_access" then table.insert(names, c.name) end end
  for i, m in ipairs(marks) do
    local choice = ui.new("GtkDropDown", { model = ui.new("GtkStringList", { strings = names }) })
    rows[i] = choice
    local remove = ui.new("GtkButton", { icon_name = "yt-delete-symbolic", tooltip_text = "Don't submit this one" })
    remove:add_css_class("flat")
    remove:on("clicked", function()
      table.remove(marks, i)
      show_marks()
      refresh_controls()
      app.run("app.close_dialog")
      if #marks > 0 then app.run("sponsorblock.submit") end
    end)
    list:append(ui.box { orientation = "horizontal", spacing = 12, children = {
      { "GtkLabel", label = fmt(m.start) .. " \u{2013} " .. fmt(m["end"]), hexpand = true, xalign = 0 },
      choice, remove } })
  end
  local submit_button = ui.button("Submit", function() submit(rows) end, { class = "yt-dialog-button confirm", name = "sb-submit" })
  local cancel = ui.button("Cancel", function() app.run("app.close_dialog") end, { class = "yt-dialog-button" })
  local content = ui.box { orientation = "vertical", spacing = 16, class = "yt-dialog-content", children = {
    { "GtkLabel", label = "Submit segments", class = "yt-dialog-title", xalign = 0 },
    { "GtkLabel", label = "What each part is. Everyone using SponsorBlock will skip it, so mark only what fits the category.",
      wrap = true, xalign = 0, class = "dim", max_width_chars = 46 },
    list,
    ui.box { orientation = "horizontal", spacing = 8, halign = "end", children = { cancel, submit_button } },
  } }
  app.dialog(content, { title = "Submit segments" })
end)

app.keymap.set("semicolon", "sponsorblock.mark", nil, "player")
app.keymap.set("apostrophe", "sponsorblock.submit", nil, "player")

---------------------------------------------------------------------------
-- Settings
---------------------------------------------------------------------------

app.slot("settings.sections", function()
  local group = ui.new("AdwPreferencesGroup", { title = "SponsorBlock", name = "sb-settings",
    description = "Skips sponsors and other parts the SponsorBlock community marked. Lookups don't say which video plays." })
  for _, c in ipairs(CATEGORIES) do
    local selected = 0
    for i, a in ipairs(ACTIONS) do if store.actions[c.id] == a then selected = i - 1 end end
    local row = ui.new("AdwComboRow", { title = c.name, model = ui.new("GtkStringList", { strings = ACTION_NAMES }),
      name = "sb-" .. c.id })
    -- After the model, which resets it.
    row.selected = selected
    local swatch = ui.drawing_area(function(cr, w, h) cr:set_color(c.color); cr:arc(w / 2, h / 2, 6, 0, 2 * math.pi); cr:fill() end)
    swatch.width_request, swatch.height_request = 16, 16
    swatch.valign = "center"
    row:add_prefix(swatch)
    row:on("notify::selected", function(self)
      store.actions[c.id] = ACTIONS[self.selected + 1]
      store:save()
      if video_id then local id = video_id; clear(); video_id = id; look_up(id) end
    end)
    group:append(row)
  end
  local function switch(title, subtitle, key)
    local row = ui.new("AdwSwitchRow", { title = title, subtitle = subtitle, active = store[key] })
    row:on("notify::active", function(self) store[key] = self.active; store:save() end)
    group:append(row)
  end
  switch("Count my skips", "Lets the people who marked a segment know it helped", "count_skips")
  switch("Notice after skipping", "With \u{201c}Unskip\u{201d} and votes for the segment", "notice")
  local server = ui.new("AdwEntryRow", { title = "Server", text = store.server, show_apply_button = true })
  server:on("apply", function(self)
    local url = self.text:gsub("/+$", "")
    if url:match("^https?://") then store.server = url; store:save() else app.toast("The server is a web address") end
  end)
  group:append(server)
  return group
end)

-- How its widgets look: dark, rounded, over the video as YouTube's own are.
app.css([[
  .sb-notice { background: rgba(28, 28, 28, 0.92); border-radius: 10px; padding: 4px; color: white; }
  .sb-notice label { font-size: 13px; }
  .sb-notice-button { min-height: 28px; min-width: 28px; padding: 0 10px; border-radius: 8px;
    background: rgba(255, 255, 255, 0.1); color: white; box-shadow: none; }
  .sb-notice-button:hover { background: rgba(255, 255, 255, 0.2); }
  .sb-button { background: rgba(28, 28, 28, 0.88); color: white; border-radius: 18px; padding: 6px 14px;
    font-size: 13px; font-weight: 500; box-shadow: none; }
  .sb-button:hover { background: rgba(70, 70, 70, 0.95); }
  .sb-label { padding: 4px 12px; border-radius: 16px; background: rgba(255, 255, 255, 0.1); font-size: 13px; }
  popover.sb-info-popover > contents { background: rgba(28, 28, 28, 0.96); border-radius: 12px; padding: 14px 16px;
    box-shadow: 0 4px 32px rgba(0, 0, 0, 0.5); color: #eee; }
  .sb-info { min-width: 360px; }
  .sb-info-title { font-size: 15px; font-weight: 600; }
  .sb-info-status { font-size: 13px; color: #aaa; }
  .sb-info-row { padding: 4px 0; }
  .sb-info-category { font-size: 14px; }
  .sb-info-time { font-size: 12px; color: #aaa; font-feature-settings: "tnum"; }
  .sb-info-button { min-width: 32px; min-height: 32px; padding: 0; border-radius: 999px; color: #eee; }
  .sb-info-button:hover { background: rgba(255, 255, 255, 0.12); }
  .sb-info-action { background: rgba(255, 255, 255, 0.1); color: white; border-radius: 16px; padding: 0 14px;
    min-height: 32px; font-size: 13px; font-weight: 500; box-shadow: none; }
  .sb-info-action:hover { background: rgba(255, 255, 255, 0.2); }
]])

log("ready; segments from", store.server)
