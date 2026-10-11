-- End-to-end check of search: the search box and its suggestions, results,
-- loading more, chips, filters and the filters dialog, corrections, channel
-- and playlist results, and what plugins can change. See ../README.md.
--
-- Signed in, searches go into the account's YouTube search history, so the
-- signed-in run (E2E_SEARCH=signed_in, the default) only searches "hello".
-- E2E_SEARCH=signed_out runs the other queries without a sign-in (the
-- results then come from the web client alone).
local shots = os.getenv("E2E_SHOTS") or "/tmp"
local mode = os.getenv("E2E_SEARCH") or "signed_in"
local function log(...) app.log("[search]", ...) end

-- Silent, and no watch history.
app.player.set("ao", "null")
app.player.set("mute", "yes")
app.run("watch.set_tracking", { enabled = false })

local function count(kind, page)
  local n = 0
  for _, item in ipairs(page.items) do if item.kind == kind then n = n + 1 end end
  return n
end
app.on("search.loaded", function(e)
  local p = e.page
  local chips = {}
  for _, c in ipairs(p.chips or {}) do table.insert(chips, c.title) end
  log("loaded:", e.query, e.first and "first" or "more", "| items:", #p.items, "videos:", count("video", p),
    "channels:", count("channel", p), "playlists:", count("playlist", p), "shelves:", count("shelf", p),
    "| chips:", table.concat(chips, ","), "| correction:", p.correction and p.correction.query or "-",
    "| next:", p.next and "yes" or "no")
end)
app.on("search.suggestions", function(e)
  local texts = {}
  for _, s in ipairs(e.suggestions) do table.insert(texts, s.text) end
  log("suggestions for", e.query .. ":", table.concat(texts, " | "))
end)
app.on("search.history_changed", function() log("history:", table.concat(app.run("search.history"), " | ")) end)
app.on("page.changed", function(e) log("page:", e.page, e.args and e.args.query or "") end)

-- What a plugin can do: drop the Shorts shelves from every page, and add a
-- row of its own above the first page's results.
app.filter("search.page", function(e)
  local kept = {}
  for _, item in ipairs(e.page.items) do
    if not (item.kind == "shelf" and item.shorts and mode == "plugin") then table.insert(kept, item) end
  end
  e.page.items = kept
  return e
end)
app.component("e2e_search_note", {
  build = function(props) return ui.label("A plugin's row: " .. props.text, { css_classes = { "dim" } }) end,
})
app.filter("search.rows", function(e)
  if e.first and mode ~= "signed_out" then
    table.insert(e.rows, 1, { component = "e2e_search_note", props = { text = #e.rows .. " rows for " .. e.query } })
  end
  return e
end)

-- A row of the plugin's own in the search box: a pasted link plays the
-- video instead of searching for it.
app.filter("search.box_rows", function(e)
  local id = e.typed:match("youtu%.be/([%w_-]+)") or e.typed:match("[?&]v=([%w_-]+)")
  if id then
    table.insert(e.rows, 1, { text = "Play this video", icon = "yt-play-symbolic", command = "video.play", args = { video = { id = id } } })
  end
  return e
end)
-- An empty page, for the "No results found" state.
app.filter("search.page", function(e)
  if e.query == "helo wrold" and e.page.correction and not e.page.correction.applied then
    e.page.items = {}; e.page.correction = nil; e.page.next = nil
  end
  return e
end, 10)

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
local function shot(name) app.screenshot(shots .. "/" .. name .. ".png"); log("screenshot", name) end
local function scroll_results(value)
  local list = app.find("search-results")
  if list then list:parent().vadjustment.value = value end
end
local function entry()
  return app.find("search-box"):find(".search-entry")
end
-- Presses a key in the search field, through its key handler.
local KEYS = { Down = 65364, Up = 65362, Escape = 65307 }
local function press(key)
  for _, c in ipairs(entry():controllers()) do
    if c:is_a("GtkEventControllerKey") then return c:emit("key-pressed", KEYS[key], 0, 0) end
  end
end
-- Scrolls down until a widget with `class` is on screen, then calls `done`.
local function scroll_until(class, done, tries)
  tries = tries or 40
  if first_visible(class) or tries == 0 then return done(first_visible(class)) end
  local list = app.find("search-results")
  list:parent().vadjustment.value = list:parent().vadjustment.value + 500
  app.timer(250, function() scroll_until(class, done, tries - 1) end)
end
local function type_text(text)
  app.run("search.focus")
  entry().text = text
end
-- Shows the suggestions list in the page for a screenshot, then puts it
-- back in its popover.
local function shoot_suggestions(name)
  local list = app.find("search-box"):find(".search-suggestions")
  local popover = list
  while popover and not popover:is_a("GtkPopover") do popover = popover:parent() end
  popover:set_child(nil)
  -- The slot below the chips of the page on screen.
  local slot
  for _, name in ipairs({ "slot:search.header", "slot:feed.header" }) do
    local s = app.find(name)
    if s and s:parent():mapped() then slot = s end
  end
  slot:append(list)
  slot:show() -- empty slots are hidden
  shot(name)
  app.timer(500, function() slot:remove(list); slot:hide(); popover:set_child(list) end)
end

local signed_in = {
  { 8000, function() shot("01-home") end },
  -- Typing: earlier searches and suggestions (the popover isn't drawn on an
  -- unfocused test window, so the log shows them).
  { 200, function() type_text("hel") end },
  { 2500, function() shot("02-typing"); shoot_suggestions("02-suggestions") end },
  { 1000, function() end },
  -- Down picks the first row (the earlier search "hello"); Enter searches it.
  { 200, function() type_text("hello"); end },
  { 1500, function() log("down:", tostring(press("Down")), "field:", entry().text); entry():emit("activate") end },
  { 7000, function() shot("03-results") end },
  { 200, function() scroll_results(1150) end },
  { 3000, function() shot("04-shelf") end },
  { 200, function()
      scroll_until("shorts", function(w)
        log("shorts shelf:", w and "shown" or "NOT FOUND")
        local adj = app.find("search-results"):parent().vadjustment
        adj.value = adj.value + 520
      end)
    end },
  { 6000, function() shot("04-shorts") end },
  { 200, function() scroll_results(3000) end },
  { 4000, function() shot("04-scrolled") end },
  { 200, function() scroll_results(100000) end },
  { 6000, function() shot("05-more") end },
  { 200, function() app.emit("search.chip_selected", { index = 2 }) end }, -- "Unwatched"
  { 7000, function() shot("06-chip") end },
  { 200, function() app.back() end },
  { 3000, function() shot("07-back") end },
  { 200, function() app.forward() end },
  { 3000, function() shot("08-forward-same-chip") end },
  { 200, function()
      app.run("search.open", { query = "hello", filters = { type = "video", upload_date = "year", features = { "hd" } } })
    end },
  { 7000, function() shot("09-filtered") end },
  { 200, function() first_visible("search-filters-button"):emit("clicked") end },
  { 1500, function() shot("10-filters-dialog") end },
  { 200, function() first_visible("search-filter-option"):emit("clicked") end }, -- "Videos", on: turns it off
  { 7000, function() shot("11-filter-removed") end },
  { 200, function() app.run("search.open", { query = "hello", filters = { type = "playlist" } }) end },
  { 7000, function() shot("12-playlists") end },
  { 200, function() app.run("search.open", { query = "hello", filters = { type = "channel" } }) end },
  { 7000, function() shot("13-channels") end },
  { 200, function()
      local status = io.open("/proc/self/status")
      if status then
        local rss = status:read("a"):match("VmRSS:%s*(%d+)")
        status:close()
        log("memory:", math.floor(tonumber(rss) / 1024), "MB")
      end
      app.run("app.navigate", { page = "home" })
    end },
  { 2000, function() log("field after home:", '"' .. entry().text .. '"') end },
  { 1000, function() app.quit() end },
}

local signed_out = {
  { 4000, function() shot("21-signed-out") end },
  { 200, function() app.run("search.open", { query = "adele" }) end },
  { 7000, function() shot("22-artist") end },
  { 200, function() type_text("https://youtu.be/dQw4w9WgXcQ") end },
  { 1500, function() shoot_suggestions("22-plugin-row") end },
  { 1000, function() entry().text = "adele"; press("Escape"); press("Escape") end },
  { 200, function() app.run("search.open", { query = "helo wrold" }) end },
  { 7000, function() shot("23-correction") end },
  { 200, function()
      local link = first_visible("search-correction"):children()[2]
      log("instead link:", link and link.label)
      app.run("search.open", { query = "helo wrold", filters = { exact = true } })
    end },
  { 7000, function() shot("24-exact-emptied-by-plugin") end },
  { 200, function() app.run("search.open", { query = "zqxjvbkwpfhgtdyr qqzxv" }) end },
  { 7000, function() shot("25-nothing") end },
  -- A narrow window: the results stack, like feed cards.
  { 200, function()
      narrow = app.css("#page-host { margin-right: 980px; }")
      app.run("search.open", { query = "adele" })
    end },
  { 7000, function() shot("26-narrow") end },
  { 1000, function() app.quit() end },
}

local steps = mode == "signed_out" and signed_out or signed_in
narrow = nil
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
app.timer(180000, function() log("timeout"); app.quit() end)
