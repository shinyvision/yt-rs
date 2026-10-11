-- Plays one video (E2E_VIDEO, or the first home feed video whose title
-- contains E2E_MATCH) and logs the player's state every second, for
-- looking into a video that won't play. Silent, no watch history.
local function log(...) app.log("[probe]", ...) end
app.api.mpv.set_property("ao", "null")
app.api.mpv.set_property("mute", "yes")
app.run("watch.set_tracking", { enabled = false })

local id, match = os.getenv("E2E_VIDEO"), os.getenv("E2E_MATCH")
local started = false
local function start(video)
  if started then return end
  started = true
  log("playing", video.id, video.title or "")
  app.run("video.play", { video = video, start = tonumber(os.getenv("E2E_START") or "0") })
  local n = 0
  local function tick()
    n = n + 1
    local s = app.api.player.state()
    log(string.format("%2d s: position %.1f paused %s ended %s quality %s duration %.0f live %s", n, s.position or -1,
      tostring(s.paused), tostring(s.ended), s.quality or "-", s.duration or 0, tostring(s.live ~= nil)))
    -- E2E_SHOT_AT=N: a screenshot at N s, with the controls up.
    if n == tonumber(os.getenv("E2E_SHOT_AT") or "-1") then
      local function walk(w)
        for _, c in ipairs(w:controllers()) do
          if c:is_a("GtkEventControllerMotion") then c:emit("motion", 300.0, 200.0) end
        end
        for _, ch in ipairs(w:children()) do walk(ch) end
      end
      app.run("player.expand")
      app.timer(1500, function()
        if app.find("player") then walk(app.find("player")) end
        app.timer(300, function() app.screenshot((os.getenv("E2E_SHOTS") or "/tmp") .. "/probe.png") end)
      end)
    end
    -- E2E_SEEKS="600,3000,...": seeks there one after another from 3 s on,
    -- logging how long each takes to play on.
    if n == 3 and os.getenv("E2E_SEEKS") then
      local targets = {}
      for t in os.getenv("E2E_SEEKS"):gmatch("[^,]+") do table.insert(targets, tonumber(t)) end
      local function seek(i)
        local target = targets[i]
        if not target then return app.timer(1000, tick) end
        local t0 = app.now()
        app.run("player.seek", { position = target })
        local function poll()
          local s = app.api.player.state()
          if s.position >= target + 0.2 and not s.paused then
            log(string.format("seek to %d s: playing on after %.0f ms", target, (app.now() - t0) * 1000))
            return seek(i + 1)
          end
          if app.now() - t0 > 20 then log("seek to", target, "TIMED OUT"); return seek(i + 1) end
          app.timer(20, poll)
        end
        poll()
      end
      return seek(1)
    end
    if n < tonumber(os.getenv("E2E_SECONDS") or "20") then app.timer(1000, tick) else app.run("player.stop"); app.quit() end
  end
  app.timer(1000, tick)
end
for _, name in ipairs({ "player.error", "player.waiting", "player.upcoming" }) do
  app.on(name, function(p) log(name, app.json.encode(p)) end)
end
app.on("feed.loaded", function(e)
  if id or not match then return end
  for _, shelf in ipairs(e.page.shelves or {}) do
    for _, v in ipairs(shelf.videos or {}) do
      if (v.title or ""):find(match, 1, true) then return start(v) end
    end
  end
  log("no home feed video matches", match)
end)
app.on("app.ready", function()
  if id then start({ id = id }) end
end)
app.timer(120000, function() log("timeout"); app.quit() end)
