-- A channel's Videos in youtube.com's orders: the chips are there at once,
-- Popular and Oldest load in their order (from the web client), scrolling
-- loads more of them, and Latest comes back. Reads only; silent. See
-- ../README.md.
local shots = os.getenv("E2E_SHOTS") or "/tmp"
local channel = "UCuAXFkgsw1L7xaCfnd5JJOw" -- Rick Astley
local function log(...) app.log("[order]", ...) end
app.player.set("ao", "null"); app.player.set("mute", "yes")
app.run("watch.set_tracking", { enabled = false })
local n = 0
local function shot(name) n = n + 1; app.screenshot(string.format("%s/%02d-%s.png", shots, n, name)) end
local results = { pass = 0, fail = 0 }
local function check(label, ok, detail)
  if ok then results.pass = results.pass + 1 else results.fail = results.fail + 1 end
  log(ok and "PASS" or "FAIL", label, detail and ("| " .. tostring(detail)) or "")
end
local function find_all(root, class, out)
  out = out or {}
  if root:has_css_class(class) then table.insert(out, root) end
  for _, c in ipairs(root:children()) do find_all(c, class, out) end
  return out
end
local function click_label(class, label)
  for _, w in ipairs(find_all(app.window(), class)) do
    if w.label == label and w:get_mapped() then w.active = true; return true end
  end
  log("NOT FOUND", class, label)
end
-- Every page the channel's feed loads: its first video, and how many.
local pages = {}
app.on("feed.loaded", function(e)
  if e.browse_id ~= channel then return end
  local shelf = e.page.shelves[1]
  table.insert(pages, { first = shelf and shelf.videos[1], count = shelf and #shelf.videos or 0,
    more = shelf and shelf.continuation })
  local ids = {}
  for i = 1, math.min(3, shelf and #shelf.videos or 0) do ids[i] = shelf.videos[i].id end
  pages[#pages].ids = table.concat(ids, ",")
end)
local function last() return pages[#pages] end
-- The channel page's grid (Home's, kept out of sight, has the same name).
local function channel_grid() return app.find("page-channel") and app.find("page-channel"):find("video-grid") end
local function chips()
  local labels = {}
  for _, w in ipairs(find_all(app.window(), "chip")) do
    if w:get_mapped() then table.insert(labels, w.label .. (w.active and "*" or "")) end
  end
  return table.concat(labels, " ")
end

local steps = {
  { 3000, function() app.run("channel.open", { channel_id = channel }) end },
  { 5000, function() click_label("channel-tab", "Videos") end },
  { 300, function()
      check("the order chips are there while the videos load", chips() == "Latest* Popular Oldest", chips())
    end },
  { 4000, function()
      local p = last()
      check("Latest: the newest first", p and p.count > 0, p and p.first and p.first.title)
      _G.latest = p and p.first and p.first.id
      shot("latest")
    end },
  -- A screenshot shows the next frame: what is shot waits a step.
  { 500, function() click_label("chip", "Popular") end },
  { 100, function()
      local content = app.find("page-channel"):find("feed-content")
      check("choosing an order shows it is loading", content and content.visible_child_name == "loading",
        content and content.visible_child_name)
    end },
  { 5000, function()
      local p = last()
      check("Popular: the most watched first", p and p.first and p.first.id == "dQw4w9WgXcQ",
        p and p.first and (p.first.title .. " " .. tostring(p.first.views)))
      check("with more to load, from the web client", p and p.more and p.more:sub(1, 4) == "web:")
      check("the chip shows it", chips() == "Latest Popular* Oldest", chips())
      _G.before = #pages
      _G.popular = p and p.ids
      shot("popular")
    end },
  { 500, function()
      local grid = channel_grid()
      if grid then grid:get_parent().vadjustment.value = 100000 end
    end },
  { 5000, function()
      local p = last()
      check("scrolling loads more of them", #pages > _G.before and p.count > 0, #pages .. " pages, last " .. (p and p.count or 0))
      click_label("chip", "Oldest")
    end },
  { 5000, function()
      local p = last()
      -- The channel's first upload may be its most watched: the lists
      -- differ, and it is old.
      check("Oldest: the first uploads first", p and p.ids ~= _G.popular and p.first.id ~= _G.latest
        and (p.first.published or ""):find("years ago"), p and p.first and (p.ids .. " " .. tostring(p.first.published)))
      local grid = channel_grid()
      check("and from the top", grid and grid:get_parent().vadjustment.value == 0)
      shot("oldest")
    end },
  -- A screenshot shows the next frame: what is shot waits a step.
  { 500, function() click_label("chip", "Latest") end },
  { 4000, function()
      local p = last()
      check("Latest again", p and p.first and p.first.id == _G.latest, p and p.first and p.first.title)
      click_label("channel-tab", "Shorts")
    end },
  { 3000, function()
      check("other tabs have no order chips", not chips():find("Popular"), chips())
      click_label("channel-tab", "Videos")
    end },
  { 4000, function()
      check("back on Videos: Latest", chips() == "Latest* Popular Oldest", chips())
    end },
  { 500, function()
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
app.timer(90000, function() log("timeout"); app.quit() end)
