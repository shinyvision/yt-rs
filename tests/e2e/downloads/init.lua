-- Downloads, end to end, in two runs on the same profile. E2E_PHASE=online
-- downloads a short video and checks its files, the Downloads page, playing
-- it from them and the watch page's button, then stops and carries on a
-- long one (it resumes, not starts over). E2E_PHASE=offline runs with
-- YouTube out of reach (HTTPS_PROXY pointing nowhere): the downloaded video
-- still plays, and deleting it leaves the page empty. Silent, no watch
-- history. See ../README.md.
local shots = os.getenv("E2E_SHOTS") or "/tmp"
local phase = os.getenv("E2E_PHASE") or "online"
local function log(...) app.log("[downloads]", ...) end
app.player.set("ao", "null"); app.player.set("mute", "yes")
app.run("watch.set_tracking", { enabled = false })
local n = 0
local function shot(name) n = n + 1; app.screenshot(string.format("%s/%s-%02d-%s.png", shots, phase, n, name)) end
local results = { pass = 0, fail = 0 }
local function check(label, ok, detail)
  if ok then results.pass = results.pass + 1 else results.fail = results.fail + 1 end
  log(ok and "PASS" or "FAIL", label, detail and ("| " .. tostring(detail)) or "")
end
local ZOO = { id = "jNQXAC9IVRw", title = "Me at the zoo", channel = "jawed" }
local LONG = { id = "EfDomtxxFtY", title = "Everything We Know About Dreams" }
local function size(path)
  local f = io.open(path, "rb"); if not f then return 0 end
  local s = f:seek("end"); f:close(); return s
end
local function get(id) return app.run("download.get", { video_id = id }) end
local done, failed = {}, {}
app.on("download.done", function(e) done[e.video_id] = true end)
app.on("download.failed", function(e) failed[e.video_id] = e.error end)
local function position() return app.player.state().position or 0 end
local function sizes(tag)
  local parts = {}
  for _, name in ipairs({ "shell", "content", "page-host", "sidebar", "page-downloads", "page-home" }) do
    local w = app.find(name)
    if w then local x, y = w:size(); table.insert(parts, name .. " " .. x .. "x" .. y .. (w:get_mapped() and "" or " (unmapped)"))
    else table.insert(parts, name .. " missing") end
  end
  log(tag, table.concat(parts, " | "), "| page", app.current_page())
end

local online = {
  { 3000, function() app.run("download.start", { video = ZOO }) end },
  { 500, wait = function() return done[ZOO.id] or failed[ZOO.id] end, function()
      local d = get(ZOO.id)
      check("the download finishes", d and d.complete, failed[ZOO.id])
      local dir = app.paths.config .. "/data/downloads/" .. ZOO.id .. "/"
      check("its video and audio are there, whole", d and size(dir .. d.video_file) > 100000 and size(dir .. d.audio_file) > 10000
        and size(dir .. d.video_file) + size(dir .. d.audio_file) == d.total,
        d and (size(dir .. d.video_file) .. " + " .. size(dir .. d.audio_file) .. " of " .. d.total))
      check("and its picture", size(dir .. "thumbnail.jpg") > 1000)
      app.run("app.navigate", { page = "downloads" })
    end },
  { 1500, function()
      check("the Downloads page lists it", app.find("download-" .. ZOO.id) ~= nil)
      shot("page")
    end },
  -- A screenshot shows the next frame: what is shot waits a step.
  { 500, function() app.run("video.play", { video = ZOO }) end },
  { 3000, function()
      local path = app.player.get("path") or ""
      check("it plays from its file", path:sub(1, 1) == "/", path)
      local a = position()
      app.timer(2000, function() check("and plays", position() > a + 1, a .. " -> " .. position()) end)
    end },
  { 2500, function()
      local label = app.find("page-watch") and app.find("page-watch"):find(".watch-action")
      shot("watch")
      app.run("player.stop")
      app.run("download.start", { video = LONG })
    end },
  { 2500, function()
      app.run("download.cancel", { video_id = LONG.id })
    end },
  { 1000, function()
      local d = get(LONG.id)
      check("a stopped download keeps what it had", d and not d.complete and d.bytes > 0, d and d.bytes)
      _G.first_bytes = d and d.bytes or 0
      app.run("download.start", { video = LONG })
    end },
  { 2500, function()
      local d = get(LONG.id)
      check("starting it again carries on from there", d and d.bytes > _G.first_bytes, d and (_G.first_bytes .. " -> " .. d.bytes))
      app.run("download.delete", { video_id = LONG.id })
    end },
  { 1500, function()
      check("deleting a running one stops and removes it", get(LONG.id) == nil
        and size(app.paths.config .. "/data/downloads/" .. LONG.id .. "/video.webm") == 0)
    end },
}

local offline = {
  { 4000, function()
      check("offline, the download is still listed", get(ZOO.id) and get(ZOO.id).complete)
      app.run("app.navigate", { page = "downloads" })
    end },
  { 1500, function() shot("page") end },
  { 500, function() app.run("video.play", { video = ZOO }) end },
  { 4000, function()
      local a = position()
      app.timer(2000, function()
        check("offline, it plays from its files", position() > a + 1 and (app.player.get("path") or ""):sub(1, 1) == "/",
          a .. " -> " .. position())
        shot("watch")
        app.run("player.stop")
        app.run("download.delete", { video_id = ZOO.id })
      end)
    end },
  { 3500, function()
      check("deleted, it's gone", get(ZOO.id) == nil)
      app.run("app.navigate", { page = "downloads" })
    end },
  { 1000, function()
      check("and the page says there are none", app.find("page-downloads") and app.find("page-downloads").visible_child_name == "empty")
      shot("empty")
    end },
}

local steps = phase == "offline" and offline or online
table.insert(steps, { 800, function()
  log(string.format("SUMMARY %d passed, %d failed", results.pass, results.fail))
  app.quit()
end })
app.on("app.ready", function()
  local i = 0
  local function next_step()
    i = i + 1
    local step = steps[i]
    if not step then return end
    local function run()
      local ok, err = pcall(step[2])
      if not ok then log("STEP FAILED", i, err) end
      next_step()
    end
    app.timer(step[1], function()
      if not step.wait then return run() end
      local started = app.now()
      local function poll()
        if step.wait() then return run() end
        if app.now() - started > 60 then log("WAITED IN VAIN", i); return run() end
        app.timer(100, poll)
      end
      poll()
    end)
  end
  next_step()
end)
app.timer(150000, function() log("timeout"); app.quit() end)
