-- Playlists, Watch later, the queue and the video menu, end to end. Run it
-- with YT_RS_DRY_RUN=1: nothing that would change the account is sent (the
-- app acts as if it were). Silent, no watch history. See ../README.md.
local shots = os.getenv("E2E_SHOTS") or "/tmp"
local prefix = os.getenv("E2E_PREFIX") or ""
local function log(...) app.log("[playlists]", ...) end
app.player.set("ao", "null"); app.player.set("mute", "yes")
app.run("watch.set_tracking", { enabled = false })
local n = 0
local function shot(name) n = n + 1; app.screenshot(string.format("%s/%s%02d-%s.png", shots, prefix, n, name)) end
local results = { pass = 0, fail = 0 }
local function check(label, ok, detail)
  if ok then results.pass = results.pass + 1 else results.fail = results.fail + 1 end
  log(ok and "PASS" or "FAIL", label, detail or "")
end
local events = {}
for _, e in ipairs({ "playlist.changed", "playlist.changed.failed", "playlist.created", "list.changed", "playlist.loaded" }) do
  app.on(e, function(p) events[e] = p end)
end
local function find_all(root, class, out)
  out = out or {}
  if root:has_css_class(class) then table.insert(out, root) end
  for _, c in ipairs(root:children()) do find_all(c, class, out) end
  return out
end
local function mapped(class)
  local count = 0
  for _, w in ipairs(find_all(app.window(), class)) do if w:get_mapped() then count = count + 1 end end
  return count
end
local home_video
app.on("feed.loaded", function(e)
  if home_video or e.browse_id ~= "FEwhat_to_watch" then return end
  for _, shelf in ipairs(e.page.shelves) do
    for _, v in ipairs(shelf.videos) do
      if not v.short and not v.live then home_video = v; return end
    end
  end
end)

local steps = {
  -- YouTube sends some videos' menus on request: ask, then look.
  { 6000, function() app.run("video.menu", { video = home_video }) end },
  { 2500, function()
      local items = app.run("video.menu", { video = home_video })
      local labels = {}
      for _, i in ipairs(items) do table.insert(labels, i.label) end
      log("menu:", table.concat(labels, " | "))
      local has = function(l) for _, x in ipairs(labels) do if x == l then return true end end end
      check("the video menu has YouTube's items", has("Add to queue") and has("Save to Watch later") and has("Save to playlist") and has("Not interested"), table.concat(labels, ", "))
    end },
  { 100, function() app.run("app.navigate", { page = "library" }) end },
  { 5000, function()
      check("the You page shows its rows", mapped("shelf") >= 3, mapped("shelf") .. " rows")
      shot("library")
    end },
  { 100, function() app.run("app.navigate", { page = "playlists" }) end },
  { 4000, function()
      check("the Playlists page shows the playlists", mapped("playlist-card") >= 2, mapped("playlist-card") .. " cards")
      shot("playlists")
    end },
  { 100, function() app.run("playlist.open", { playlist_id = "WL" }) end },
  { 4000, function()
      local rows = mapped("playlist-row")
      check("Watch later's page lists its videos", rows >= 5 and events["playlist.loaded"] ~= nil, rows .. " rows")
      shot("watch-later")
      local page = app.find("playlist-videos"):get_parent()
      page.vadjustment.value = page.vadjustment.upper
    end },
  { 3000, function()
      local loaded = events["playlist.loaded"] and #events["playlist.loaded"].page.videos or 0
      log("rows after scrolling to the end:", mapped("playlist-row"), "of the first page's", loaded)
      app.run("playlist.play", { playlist_id = "WL" })
    end },
  { 2500, function()
      local w, h = app.find("watch-player"):size()
      local ww = app.window():size()
      log("watch page after Play all: player", w .. "x" .. h, "in a window", ww, "wide")
      check("the player fills its column at once", w > ww * 0.6, w .. " of " .. ww)
      shot("play-all")
    end },
  { 5500, function()
      local list = app.run("list.current")
      check("Play all plays the playlist", list and list.playlist_id == "WL" and list.position == 0 and app.current_page() == "watch",
        list and (list.title .. " " .. (list.position + 1) .. "/" .. #list.order) or "no list")
      check("the panel shows the list", mapped("list-panel") == 1)
      shot("playing-watch-later")
      app.run("list.next")
    end },
  { 1000, function()
      local w = app.find("watch-player"):size()
      log("1 s after next: player", w, "wide")
      shot("next-1s")
    end },
  { 2000, function()
      local w = app.find("watch-player"):size()
      log("3 s after next: player", w, "wide")
      check("the player keeps its width when the next video starts", w > 1000, w)
      shot("next-3s")
    end },
  { 3000, function()
      local list = app.run("list.current")
      local s = app.player.state()
      check("next plays the second video", list and list.position == 1 and s.video_id == list.videos[list.order[2] + 1].id, s.video_id or "-")
      app.run("list.set_shuffled", { shuffled = true })
    end },
  { 500, function()
      local list = app.run("list.current")
      local in_order = true
      for i, index in ipairs(list.order) do if index ~= i - 1 then in_order = false end end
      check("shuffle reorders the rest and keeps the video playing first", list.shuffled and not in_order and list.position == 0)
      app.run("list.set_shuffled", { shuffled = false })
      shot("shuffled")
    end },
  { 500, function()
      local list = app.run("list.current")
      check("unshuffled, back in order where it was", not list.shuffled and list.position == 1, "at " .. list.position)
      app.run("video.save_to_watch_later", { video_id = home_video.id })
    end },
  { 1500, function()
      local e = events["playlist.changed"]
      check("Save to Watch later (dry run)", e and e.playlist_id == "WL" and e.added == true and e.video_id == home_video.id)
      shot("saved-toast")
      app.run("video.save", { video_id = home_video.id })
    end },
  { 3000, function()
      local options = mapped("save-option")
      check("the save dialog lists the playlists", options >= 2, options .. " options")
      shot("save-dialog")
      local first
      for _, w in ipairs(find_all(app.window(), "save-option")) do if w:get_mapped() then first = first or w end end
      events["playlist.changed"] = nil
      if first then first.active = not first.active end
    end },
  { 1500, function()
      local e = events["playlist.changed"]
      check("ticking a playlist saves to it (dry run)", e ~= nil, e and (e.playlist_id .. " added " .. tostring(e.added)) or "nothing")
      shot("save-ticked")
      app.run("app.dismiss_toast")
      app.run("app.close_dialog")
    end },
  { 500, function()
      check("the dialog closes", mapped("save-option") == 0)
      app.run("app.back")
    end },
  { 800, function() app.run("player.mini") end },
  { 1500, function()
      app.run("queue.add", { video = home_video })
    end },
  { 1500, function()
      local list = app.run("list.current")
      check("Add to queue makes a queue of the video playing and the new one", list and list.playlist_id == nil and #list.order == 2,
        list and (#list.order .. " in " .. list.title) or "no list")
      shot("queue-mini")
    end },
  { 800, function() app.run("player.stop") end },
  { 1000, function()
      log(string.format("SUMMARY %d passed, %d failed", results.pass, results.fail))
      app.quit()
    end },
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
app.timer(180000, function() log("timeout"); app.quit() end)
