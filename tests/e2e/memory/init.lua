-- Memory over time: the same round of pages (Home, a video, its channel,
-- a search, back home) again and again, with the memory in use after each.
-- Memory that keeps growing round after round is a leak. Silent; no watch
-- history; the only search is "hello". See ../README.md.
local function log(...) app.log("[memory]", ...) end
app.player.set("ao", "null"); app.player.set("mute", "yes")
app.run("watch.set_tracking", { enabled = false })
local rounds = tonumber(os.getenv("E2E_ROUNDS") or "10")
local function rss()
  local f = io.open("/proc/self/status"); if not f then return 0 end
  local text = f:read("*a"); f:close()
  return tonumber(text:match("VmRSS:%s+(%d+)")) or 0
end
-- What the app itself costs: its share of shared pages (PSS), and the
-- memory only it has written to (private dirty: heap, buffers, caches).
local function own()
  local f = io.open("/proc/self/smaps_rollup"); if not f then return 0, 0 end
  local text = f:read("*a"); f:close()
  return (tonumber(text:match("\nPss:%s+(%d+)")) or 0) / 1024, (tonumber(text:match("Private_Dirty:%s+(%d+)")) or 0) / 1024
end
-- Where the memory is: private dirty memory by what is mapped.
local function breakdown(tag)
  local f = io.open("/proc/self/smaps"); if not f then return end
  local totals, current = {}, "?"
  for line in f:lines() do
    local name = line:match("^%x+%-%x+ %S+ %S+ %S+ %S+%s*(.*)$")
    if name then
      current = name ~= "" and name:gsub("^.*/", "") or "[anon]"
    else
      local kb = line:match("^Private_Dirty:%s+(%d+)")
      if kb then totals[current] = (totals[current] or 0) + tonumber(kb) end
    end
  end
  f:close()
  local list = {}
  for k, v in pairs(totals) do table.insert(list, { k, v }) end
  table.sort(list, function(a, b) return a[2] > b[2] end)
  local parts = {}
  for i = 1, math.min(12, #list) do table.insert(parts, string.format("%s %.1f", list[i][1], list[i][2] / 1024)) end
  log(tag, table.concat(parts, " | "))
end
local round = 0
-- Pages left alive after the window let them go, and handlers that outlive
-- their widgets: leaks, whatever the memory says.
local leaked, handlers_at_2, handlers_last = 0, nil, nil
local samples = {}
local steps = {
  { 3000, function() app.run("app.navigate", { page = "home" }) end },
  -- E2E_NO_VIDEO=1: the pages alone, to tell what playing costs.
  { 4000, function()
      if os.getenv("E2E_NO_VIDEO") then return app.run("app.navigate", { page = "subscriptions" }) end
      app.run("video.play", { video = { id = "dQw4w9WgXcQ", title = "Rick Astley" } })
    end },
  { 6000, function() app.run("channel.open", { channel_id = "UCuAXFkgsw1L7xaCfnd5JJOw" }) end },
  { 4000, function() app.run("search.open", { query = "hello" }) end },
  { 4000, function() app.run("app.navigate", { page = "playlists" }) end },
  { 2000, function() app.run("app.navigate", { page = "settings" }) end },
  { 2000, function() app.run("app.navigate", { page = "downloads" }) end },
  { 3000, function() app.run("player.stop"); app.run("app.navigate", { page = "home" }) end },
}
local function run_round()
  round = round + 1
  local i = 0
  local function next_step()
    i = i + 1
    local step = steps[i]
    if not step then
      collectgarbage()
      app.timer(2000, function()
        local kb = rss()
        local pss, dirty = own()
        table.insert(samples, dirty * 1024)
        log(string.format("round %d: RSS %.1f MB, PSS %.1f MB, private dirty %.1f MB", round, kb / 1024, pss, dirty))
        -- Handlers that outlive their widgets grow round after round.
        local stats = app.run("app.stats")
        local total, parts = 0, {}
        for name, n in pairs(stats.handlers) do total = total + n; if n > 3 then table.insert(parts, name .. " " .. n) end end
        table.sort(parts)
        log(string.format("  handlers %d, pages %d, leaked pages %d (%s), images %d (%.1f MB) | %s", total, stats.pages,
          #stats.leaked_pages, table.concat(stats.leaked_pages, " "), stats.images, stats.image_bytes / 1048576, table.concat(parts, ", ")))
        leaked = math.max(leaked, #stats.leaked_pages)
        if round == 2 then handlers_at_2 = total end
        handlers_last = total
        if round == 1 or round == rounds then breakdown("round " .. round .. ":") end
        if round < rounds then run_round() else
          local first, last = samples[2] or samples[1], samples[#samples]
          local growing = handlers_at_2 and handlers_last > handlers_at_2
          log(string.format("%s leaked pages %d, handlers %s -> %s", (leaked == 0 and not growing) and "PASS" or "FAIL",
            leaked, tostring(handlers_at_2), tostring(handlers_last)))
          log(string.format("SUMMARY private dirty from round 2 to %d: %+.1f MB (%.2f MB a round)", #samples, (last - first) / 1024, (last - first) / 1024 / math.max(1, #samples - 2)))
          app.quit()
        end
      end)
      return
    end
    app.timer(step[1], function()
      -- The first round, step by step: what each page costs.
      if round == 1 then
        local _, dirty = own()
        log(string.format("  before step %d: private dirty %.1f MB, page %s", i, dirty, tostring(app.current_page())))
        if os.getenv("E2E_BREAKDOWN") then breakdown("  step " .. i .. ":") end
      end
      local ok, err = pcall(step[2])
      if not ok then log("STEP FAILED", i, err) end
      next_step()
    end)
  end
  next_step()
end
app.on("app.ready", function()
  local pss, dirty = own()
  log(string.format("start: RSS %.1f MB, PSS %.1f MB, private dirty %.1f MB", rss() / 1024, pss, dirty))
  breakdown("start:")
  run_round()
end)
app.timer(900000, function() log("timeout"); app.quit() end)
