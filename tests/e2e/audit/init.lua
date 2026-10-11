-- Visual audit: walks through every page and state and takes a screenshot
-- of each, for a person to review. Run it at several window sizes (the
-- monitor's mode) and with the network down (E2E_OFFLINE=1 and a dead
-- proxy) for the error states. Silent, no watch history, no account
-- changes, and the only search is "hello". See ../README.md.
local shots = os.getenv("E2E_SHOTS") or "/tmp"
local prefix = os.getenv("E2E_PREFIX") or ""
local function log(...) app.log("[audit]", ...) end

app.player.set("ao", "null")
app.player.set("mute", "yes")
app.run("watch.set_tracking", { enabled = false })

local function find_all(root, class, out)
  out = out or {}
  if root:has_class(class) then table.insert(out, root) end
  for _, c in ipairs(root:children()) do find_all(c, class, out) end
  return out
end
local function first_visible(class)
  for _, w in ipairs(find_all(app.window(), class)) do
    if w:mapped() then return w end
  end
end
local function click(class)
  local w = first_visible(class)
  if w then w:emit("clicked") else log("NOT FOUND", class) end
end
local n = 0
local function shot(name)
  n = n + 1
  local file = string.format("%s/%s%02d-%s.png", shots, prefix, n, name)
  app.screenshot(file)
  log("shot", file)
end
-- Pointer motion over the player brings its controls up.
local moves = 0
local function show_controls()
  moves = moves + 1
  local player = app.find("player")
  if not player then return end
  local function walk(w)
    for _, c in ipairs(w:controllers()) do
      if c:is_a("GtkEventControllerMotion") then c:emit("motion", 200.0 + moves, 150.0 + moves) end
    end
    for _, ch in ipairs(w:children()) do walk(ch) end
  end
  walk(player)
end
local function scroll(name, value)
  local w = app.find(name)
  if not w then return log("NOT FOUND", name) end
  local adj = w.vadjustment or w:parent().vadjustment
  adj.value = value
end
-- Shows a component's widget in a slot of the page on screen, for things
-- the test window can't show as popovers (menus, the settings panel).
local inline
local function show_inline(widget)
  for _, name in ipairs({ "slot:watch.below_player", "slot:feed.header", "slot:search.header" }) do
    local slot = app.find(name)
    if slot and slot:parent():mapped() then
      slot:append(widget); slot:show(); inline = { slot, widget }
      return
    end
  end
  log("NO SLOT for inline")
end
local function hide_inline()
  if inline then inline[1]:remove(inline[2]); inline[1]:hide(); inline = nil end
end

local steps = {
  { 7000, function() shot("home") end },
  { 100, function() scroll("video-grid", 900) end },
  { 1500, function() shot("home-scrolled") end },
  { 100, function() scroll("video-grid", 0); app.run("sidebar.toggle") end },
  { 1200, function() shot("home-mini-sidebar") end },
  { 100, function() app.run("sidebar.toggle"); app.run("app.toast", { message = "This is what a toast looks like" }) end },
  { 600, function() shot("toast") end },
  { 100, function() app.run("app.navigate", { page = "subscriptions" }) end },
  { 5000, function() shot("subscriptions") end },
  { 100, function() app.run("app.navigate", { page = "history" }) end },
  { 5000, function() shot("history") end },
  { 100, function() app.run("video.play", { video = { id = "dQw4w9WgXcQ" }, start = 30 }) end },
  { 10000, function() show_controls() end },
  { 400, function() shot("watch-playing-controls") end },
  { 100, function() app.run("player.pause") end },
  { 800, function() show_controls() end },
  { 400, function() shot("watch-paused") end },
  { 100, function() click("description-more") end },
  { 1000, function() shot("watch-description") end },
  { 100, function() scroll("watch-info", 1400) end },
  { 2500, function() shot("watch-comments") end },
  { 100, function() click("replies-toggle") end },
  { 2500, function() shot("watch-replies") end },
  { 100, function() scroll("watch-info", 0); app.run("watch.toggle_theater") end },
  { 1500, function() show_controls() end },
  { 400, function() shot("watch-theater") end },
  { 100, function() app.run("watch.toggle_theater") end },
  { 500, function() show_inline(app.build("player_settings", app.run("player.settings"))) end },
  { 800, function() shot("player-settings-inline") end },
  { 100, function()
      hide_inline()
      show_inline(app.build("menu", { items = {
        { label = "Play", icon = "yt-play-symbolic", command = "video.play" },
        { label = "Go to channel", icon = "yt-person-symbolic", command = "channel.open" },
        { label = "Copy link", icon = "yt-link-symbolic", command = "video.copy_link" },
      } }))
    end },
  { 800, function() shot("menu-inline") end },
  { 100, function() hide_inline(); app.run("player.play") end },
  { 100, function() app.run("channel.open", { channel_id = "UCuAXFkgsw1L7xaCfnd5JJOw" }) end },
  { 6000, function() shot("channel-home") end },
  { 100, function()
      for _, t in ipairs(find_all(app.window(), "channel-tab")) do
        if t:mapped() and t.label == "Videos" then t:emit("clicked") end
      end
    end },
  { 4000, function() shot("channel-videos") end },
  { 100, function() app.run("search.open", { query = "hello" }) end },
  { 6000, function() shot("search") end },
  { 100, function() click("search-filters-button") end },
  { 1200, function() shot("search-filters") end },
  { 100, function()
      local dialog = first_visible("search-filters-dialog")
      for _, b in ipairs(find_all(app.window(), "icon-button")) do
        if b:mapped() and b.tooltip_text == "Close" then b:emit("clicked") end
      end
    end },
  { 800, function() app.run("app.back") end },
  { 3000, function() shot("back-to-channel") end },
  { 100, function() app.run("app.navigate", { page = "home" }) end },
  { 3000, function() shot("home-while-playing") end },
  { 100, function() app.run("player.stop"); app.quit() end },
}

app.on("app.ready", function()
  local i = 0
  local function next_step()
    i = i + 1
    local step = steps[i]
    if not step then return end
    app.timer(step[1], function()
      local ok, err = pcall(step[2])
      if not ok then log("STEP FAILED", i, err) end
      next_step()
    end)
  end
  next_step()
end)
app.timer(240000, function() log("timeout"); app.quit() end)
