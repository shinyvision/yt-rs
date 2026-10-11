-- Looks for a flash of a wrong layout when the watch page switches videos:
-- plays one video, then another, and logs the sizes of the page's parts
-- at moments right after. Silent, no watch history.
local shots = os.getenv("E2E_SHOTS") or "/tmp"
local function log(...) app.log("[flash]", ...) end
app.player.set("ao", "null"); app.player.set("mute", "yes")
app.run("watch.set_tracking", { enabled = false })
local function sizes(tag)
  local parts = {}
  for _, name in ipairs({ "content", "page-host", "sidebar", "sidebar-mini", "page-watch", "watch-info", "watch-player", "watch-related" }) do
    local w = app.find(name)
    if w then
      local x, y = w:size()
      table.insert(parts, name .. " " .. x .. "x" .. y .. (w:get_mapped() and "" or " (unmapped)") .. (w.visible == false and " (hidden)" or ""))
    end
  end
  local grid = app.find("watch-player") and app.find("watch-player"):get_parent()
  if grid then local x, y = grid:size(); table.insert(parts, "grid " .. x .. "x" .. y .. " margin " .. tostring(grid.margin_start)) end
  log(tag, table.concat(parts, " | "))
end
app.on("app.ready", function()
  -- After the app's own first page.
  -- With E2E_LIST, a playlist plays and the switch is to its next video.
  local list = os.getenv("E2E_LIST")
  app.timer(4000, function()
    if list then app.run("playlist.play", { playlist_id = list })
    else app.run("video.play", { video = { id = "dQw4w9WgXcQ" }, start = 30 }) end
  end)
  app.timer(13000, function()
    sizes("before")
    if list then app.run("list.next")
    else app.run("video.play", { video = { id = "EfDomtxxFtY" }, start = 100 }) end
    sizes("0 ms")
    app.screenshot(shots .. "/flash-0.png")
    for _, ms in ipairs({ 16, 50, 100, 200, 400, 1000, 3000 }) do
      app.timer(ms, function() sizes(ms .. " ms"); if ms == 50 or ms == 200 then app.screenshot(shots .. "/flash-" .. ms .. ".png") end end)
    end
    app.timer(4000, function() app.run("player.stop"); app.quit() end)
  end)
end)
