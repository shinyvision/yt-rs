-- End-to-end check of playback. Each check plays something, waits, and
-- passes if the position moves on with no player errors:
--   - a video with a pre-roll ad, then resuming deep into a long video
--     (YouTube once served limited streams that refused that)
--   - seeking near the end, switching quality, captions
--   - an age-restricted video
--   - the first videos of the home feed, each from the start and at 75 %
--   - a live stream
-- Silent, and no watch history. Logs "[playback] PASS/FAIL ..." per check
-- and a summary. See ../README.md.
local shots = os.getenv("E2E_SHOTS") or "/tmp"
local feed_count = tonumber(os.getenv("E2E_FEED_VIDEOS") or "6")
local function log(...) app.log("[playback]", ...) end

app.player.set("ao", "null")
app.player.set("mute", "yes")
app.run("watch.set_tracking", { enabled = false })

local errors = {}
for _, name in ipairs({ "player.error", "player.audio_failed", "player.fallback" }) do
  app.on(name, function(e) table.insert(errors, name .. ": " .. tostring(e.message or e.video_id or "")) end)
end
app.on("player.waiting", function(e) log("  waiting", e.seconds, "s for", e.video_id) end)

local home = {}
app.on("feed.loaded", function(e)
  if e.browse_id ~= "FEwhat_to_watch" then return end
  for _, shelf in ipairs(e.page.shelves) do
    for _, v in ipairs(shelf.videos) do
      if #home < feed_count and not v.live and not v.short then table.insert(home, v) end
    end
  end
end)

local results = { pass = 0, fail = 0 }
local function shot(name) app.screenshot(shots .. "/" .. name .. ".png") end

-- Waits `settle` ms, then checks the position moves on over 3 s.
local function check(label, settle, expect, done)
  errors = {}
  app.timer(settle, function()
    local a = app.player.state()
    app.timer(3000, function()
      local b = app.player.state()
      local moved = (b.position or 0) - (a.position or 0)
      local ok = moved > 1.5 and not b.paused and #errors == 0 and (not expect or expect(b))
      if ok then results.pass = results.pass + 1 else results.fail = results.fail + 1 end
      log(ok and "PASS" or "FAIL", label, string.format("| at %.0f s, moved %.1f s | %s %s", b.position or -1, moved,
        b.quality or "-", #errors > 0 and ("| " .. table.concat(errors, "; ")) or ""))
      done()
    end)
  end)
end

local function play(id, start)
  app.run("video.play", { video = { id = id }, start = start })
end

local steps = {}
local function step(f) table.insert(steps, f) end

step(function(next) play("dQw4w9WgXcQ", 0); check("video with a pre-roll ad", 9000, nil, next) end)
step(function(next)
  play("EfDomtxxFtY", 5000)
  check("resuming a long video at 1:23:20, after a pre-roll video", 7000, function(s) return s.position > 4990 end, next)
end)
step(function(next)
  app.run("player.seek", { position = 6500 })
  check("seeking to 96 %", 4000, function(s) return s.position > 6490 end, next)
end)
step(function(next)
  local q = app.run("player.qualities")
  app.run("player.set_quality", { index = math.min(2, #q - 1) })
  check("switching quality there", 4000, function(s) return s.position > 6490 end, next)
end)
step(function(next)
  app.run("player.set_quality", { index = nil })
  app.run("player.set_captions", { lang = "en" })
  check("captions", 4000, function(s) return s.captions == "en" end, function() shot("captions"); next() end)
end)
step(function(next)
  app.run("player.set_captions", { lang = nil })
  play("HtVdAasjOgU", 60)
  check("an age-restricted video", 8000, nil, next)
end)
step(function(next)
  log("home feed videos:", #home)
  local i = 0
  local function each()
    i = i + 1
    local v = home[i]
    if not v then return next() end
    play(v.id, 0)
    check("home " .. i .. ": " .. (v.title or v.id):sub(1, 50), 8000, nil, function()
      local d = app.player.state().duration or 0
      if d <= 60 then return each() end
      app.run("player.seek", { position = d * 0.75 })
      check("home " .. i .. " at 75 %", 4000, function(s) return s.position > d * 0.7 end, each)
    end)
  end
  each()
end)
step(function(next)
  play(os.getenv("E2E_LIVE") or "t1NfmaqyIoI")
  local function near_edge(s) return s.live and s.live.edge - s.position < 30 end
  check("a live stream, at the edge", 9000, near_edge, function() shot("live"); next() end)
end)
-- Seeking in a live stream: back a minute, deep into what YouTube keeps,
-- to the middle, and back to live.
local window
step(function(next)
  window = app.player.state().live
  log("  live window:", string.format("%.0f s back, edge at %.0f", window.edge - window.start, window.edge))
  app.run("player.seek_by", { seconds = -60 })
  check("live: back a minute", 6000, function(s) return s.live.edge - s.position > 50 and s.live.edge - s.position < 100 end,
    function() shot("live-behind"); next() end)
end)
step(function(next)
  local target = window.start + 120
  app.run("player.seek", { position = target })
  check("live: near the start of what YouTube keeps", 6000, function(s) return math.abs(s.position - target) < 15 end, next)
end)
step(function(next)
  app.run("player.seek_percent", { percent = 50 })
  check("live: to the middle", 6000, function(s)
    local middle = (s.live.start + s.live.edge) / 2
    return math.abs(s.position - middle) < 20
  end, next)
end)
step(function(next)
  app.run("player.go_live")
  check("live: back to live", 6000, near_edge, next)
end)

app.on("app.ready", function()
  local i = 0
  local function next_step()
    i = i + 1
    if not steps[i] then
      log(string.format("SUMMARY %d passed, %d failed", results.pass, results.fail))
      app.run("player.stop")
      app.timer(500, function() app.quit() end)
      return
    end
    local ok, err = pcall(steps[i], next_step)
    if not ok then log("FAIL step", i, err); results.fail = results.fail + 1; next_step() end
  end
  app.timer(5000, next_step)
end)
app.timer(400000, function() log("FAIL timeout"); app.quit() end)
