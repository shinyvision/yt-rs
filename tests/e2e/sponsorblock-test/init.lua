-- SponsorBlock end to end, with the plugin from examples/plugins/sponsorblock
-- and a stand-in server (../sponsorblock_server.py, whose address is in the
-- plugin's storage): looking segments up, the seek bar, auto skip (to the
-- moment), Unskip, the skip button, muting, the highlight, the label,
-- minimum length, votes, marking and submitting, the settings, and that the
-- plugin's widgets over the video let clicks through. Silent (no audio
-- output: muting is part of the test), no watch history.
local shots = os.getenv("E2E_SHOTS") or "/tmp"
local function log(...) app.log("[sbtest]", ...) end
app.player.set("ao", "null")
app.run("watch.set_tracking", { enabled = false })
local n = 0
local function shot(name) n = n + 1; app.screenshot(string.format("%s/%02d-%s.png", shots, n, name)) end
local results = { pass = 0, fail = 0 }
local function check(label, ok, detail)
  if ok then results.pass = results.pass + 1 else results.fail = results.fail + 1 end
  log(ok and "PASS" or "FAIL", label, detail and ("| " .. tostring(detail)) or "")
end
local function visible(name) local w = app.find(name); return w ~= nil and w:mapped() end
local function position() return app.player.state().position or 0 end

local loaded
app.on("sponsorblock.segments", function(e) loaded = e end)
-- Positions seen inside the sponsor segment while it should be skipped.
local leaked, watching = {}, true
app.on("player.position", function(e)
  if watching and e.position > 40.4 and e.position < 44.6 then table.insert(leaked, e.position) end
end)

local steps = {
  { 3000, function() app.run("video.play", { video = { id = "dQw4w9WgXcQ" }, start = 33 }) end },
  -- Each step waits `delay` ms, and then for `wait` to say yes (20 s at most).
  { 500, wait = function() return loaded ~= nil and position() > 30 end, function()
      check("the segments load", loaded and #loaded.segments == 6,
        loaded and (#loaded.segments .. " (the 2 s filler left out)") or "none")
      local ranges = 0
      for _, r in ipairs(app.run("seekbar.ranges")) do if r.label then ranges = ranges + 1 end end
      check("they show on the seek bar (not the label)", ranges == 5, ranges)
      check("the label shows", visible("sb-labels"))
      check("Jump to highlight shows before the highlight", visible("sb-highlight"))
      check("playback is before the sponsor", position() < 39.5, position())
      shot("before")
    end },
  -- Playback reaches 40: skipped to 45.
  { 100, wait = function() return position() >= 45.3 end, function()
      watching = false
      check("the sponsor is skipped", #leaked == 0, #leaked > 0 and ("played " .. leaked[1]) or "never played")
      check("the notice says so", visible("sb-notice"))
      local notice = app.find("sb-notice")
      log("notice classes: sb-notice", tostring(notice and notice:has_class("sb-notice")), "type", notice and notice:type_name())
      shot("skipped")
      app.run("sponsorblock.vote", { up = true })
      app.run("sponsorblock.unskip")
    end },
  { 2500, function()
      local p = position()
      check("Unskip goes back, and it isn't skipped again", p >= 40 and p < 44.5, p)
      app.run("player.seek", { position = 58.5 })
    end },
  { 3500, function()
      check("inside a button segment: the button", visible("sb-skip"))
      local text = app.find("sb-skip-text")
      check("it says which", text and text.label == "Skip unpaid/self promotion", text and text.label)
      shot("skip-button")
      app.run("sponsorblock.skip")
    end },
  { 1500, function()
      check("Skip skips it", position() >= 65.5, position())
      app.run("player.seek", { position = 73 })
    end },
  { 3500, function()
      check("a segment to mute is muted", app.player.get("mute") == "yes", position())
    end },
  { 5000, function()
      check("and unmuted after", app.player.get("mute") == "no", position())
      app.run("sponsorblock.jump_highlight")
    end },
  { 1500, function()
      local p = position()
      check("Jump to highlight", p >= 99.5 and p < 103, p)
      check("and the button goes", not visible("sb-highlight"))
      app.run("player.seek", { position = 4 })
    end },
  { 5500, function()
      local p = position()
      check("a segment only to show isn't skipped", p >= 8 and p < 11, p)
      app.run("player.seek", { position = 120 })
    end },
  { 1500, function() app.run("sponsorblock.mark") end },
  { 2500, function()
      app.run("sponsorblock.mark")
      local marked = 0
      for _, r in ipairs(app.run("seekbar.ranges")) do if r.label == "To submit" then marked = marked + 1 end end
      check("a marked segment shows on the seek bar", marked == 1, marked)
      app.run("sponsorblock.submit")
    end },
  { 1000, function()
      check("the submit dialog shows", visible("sb-submit"))
      shot("submit")
      app.find("sb-submit"):emit("clicked")
    end },
  { 2000, function()
      check("submitting closes the dialog", not visible("sb-submit"))
      -- The plugin's widgets lie over the video, but clicks get through.
      local player = app.find("player")
      local w, h = player:size()
      local hit = player:pick(w / 2, h / 2)
      local name = hit and hit:type_name() or "nothing"
      check("a click on the video reaches the video", name == "YtVideoView", name)
      app.run("app.navigate", { page = "settings" })
    end },
  { 1500, function()
      check("the settings have a SponsorBlock section", visible("sb-settings"))
      check("the settings show each category's choice", app.find("sb-selfpromo") and app.find("sb-selfpromo").selected == 1,
        app.find("sb-selfpromo") and app.find("sb-selfpromo").selected)
      shot("settings")
      local row = app.find("sb-sponsor")
      row.selected = 3
    end },
  { 800, function()
      local file = io.open(app.paths.config .. "/plugin-data/sponsorblock.json")
      local saved = file and app.json.decode(file:read("*a")) or {}
      if file then file:close() end
      check("a choice there is kept", saved.actions and saved.actions.sponsor == "off",
        saved.actions and saved.actions.sponsor)
      app.run("player.stop")
    end },
  { 800, function()
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
        if app.now() - started > 20 then log("WAITED IN VAIN", i); return run() end
        app.timer(50, poll)
      end
      poll()
    end)
  end
  next_step()
end)
app.timer(150000, function() log("timeout"); app.quit() end)
