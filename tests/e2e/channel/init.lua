-- Channel page check: Home's shelves, a scrolled Home, the Videos tab and
-- back to Home, with screenshots. Only reads. See ../README.md.
local shots = os.getenv("E2E_SHOTS") or "/tmp"
local prefix = os.getenv("E2E_PREFIX") or ""
local channel = os.getenv("E2E_CHANNEL") or "UCuAXFkgsw1L7xaCfnd5JJOw"
local function log(...) app.log("[channel]", ...) end
local n = 0
local function shot(name) n = n + 1; app.screenshot(string.format("%s/%s%02d-%s.png", shots, prefix, n, name)) end
local function find_all(root, class, out)
  out = out or {}
  if root:has_class(class) then table.insert(out, root) end
  for _, c in ipairs(root:children()) do find_all(c, class, out) end
  return out
end
local function tab(label)
  for _, t in ipairs(find_all(app.window(), "channel-tab")) do
    if t:mapped() and t.label == label then t:emit("clicked"); return true end
  end
  log("NO TAB", label)
end
local steps = {
  { 3000, function() app.run("channel.open", { channel_id = channel }) end },
  { 5000, function()
      local shelves = app.find("shelves")
      log("home: shelves", tostring(shelves and shelves:mapped()), "| rows", #find_all(app.window(), "shelf"))
      shot("home")
      -- Card sizes in the first row, and what in a card is widest.
      local row = find_all(app.window(), "shelf-row")[1]
      local function walk(w, depth, out)
        local width, height = w:size()
        if depth <= 6 then table.insert(out, string.rep(" ", depth) .. w:type_name() .. " " .. width .. "x" .. height) end
        for _, c in ipairs(w:children()) do walk(c, depth + 1, out) end
        return out
      end
      if row then
        local first = row:children()[1]
        for _, line in ipairs(walk(first, 0, {})) do log(line) end
      end
    end },
  { 100, function() local s = app.find("shelves"); if s then s.vadjustment.value = 700 end end },
  { 1500, function() shot("home-scrolled") end },
  { 100, function() tab("Videos") end },
  { 4000, function() log("videos: grid", tostring(app.find("video-grid") and app.find("video-grid"):mapped())); shot("videos") end },
  { 100, function() tab("Playlists") end },
  { 4000, function()
      local cards = 0
      for _, w in ipairs(find_all(app.window(), "playlist-card")) do if w:mapped() then cards = cards + 1 end end
      log("playlists tab: cards", cards)
      shot("playlists")
    end },
  { 100, function() tab("Home") end },
  { 2000, function() log("home again: shelves", tostring(app.find("shelves") and app.find("shelves"):mapped())); shot("home-again") end },
  { 100, function() app.quit() end },
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
app.timer(120000, function() log("timeout"); app.quit() end)
