-- Live chat, end to end, on a live stream (E2E_VIDEO, played by its ID
-- alone, so the chat learns it is live from the player): it shows and
-- fills, switches to Live chat, hides and shows again, asks for nothing
-- off screen and starts afresh back on it, and plugins' chat.messages
-- filter changes what shows. Reads only; silent; no watch history. See
-- ../README.md.
local shots = os.getenv("E2E_SHOTS") or "/tmp"
local video = os.getenv("E2E_VIDEO") or "jfKfPfyJRdk"
local function log(...) app.log("[chat]", ...) end
app.player.set("ao", "null"); app.player.set("mute", "yes")
app.run("watch.set_tracking", { enabled = false })
local n = 0
local function shot(name) n = n + 1; app.screenshot(string.format("%s/%02d-%s.png", shots, n, name)) end
local results = { pass = 0, fail = 0 }
local function check(label, ok, detail)
  if ok then results.pass = results.pass + 1 else results.fail = results.fail + 1 end
  log(ok and "PASS" or "FAIL", label, detail and ("| " .. tostring(detail)) or "")
end
local function state() return app.run("chat.state") end
local function describe(s)
  if not s then return "no chat" end
  return string.format("%s %s messages %d running %s hidden %s", s.status, s.view, s.messages, tostring(s.running), tostring(s.hidden))
end
-- Messages as they show; the filter marks every one while `mark` is on.
local shown, marked, mark = 0, 0, false
app.on("chat.message", function(e)
  shown = shown + 1
  local text = e.message.text.runs[1] and e.message.text.runs[1].text or ""
  if text == "[filtered]" then marked = marked + 1 end
end)
app.filter("chat.messages", function(p)
  if mark then
    for _, m in ipairs(p.messages) do m.text = { runs = { { text = "[filtered]" } } } end
  end
  return p
end)

local steps = {
  { 2500, function() app.run("video.play", { video = { id = video } }) end },
  { 12000, function()
      local s = state()
      check("a live stream has its chat beside it", s and s.live and s.status == "messages" and s.messages > 0, describe(s))
      check("Top chat to begin with, following the newest", s and s.view == "top" and s.following, describe(s))
      _G.before = shown
      shot("chat")
    end },
  -- Top chat leaves much out: a quiet stretch can last a while.
  { 1000, wait = function() return shown > _G.before end, function()
      check("new messages come in", shown > _G.before, (shown - _G.before) .. " more")
      app.run("chat.set_view", { view = "live" })
    end },
  { 6000, function()
      local s = state()
      check("Live chat: every message", s and s.view == "live" and s.status == "messages" and s.messages > 0, describe(s))
      app.run("chat.hide")
    end },
  { 800, function()
      local s = state()
      check("hidden, it asks for nothing", s and s.hidden and not s.running, describe(s))
      shot("hidden")
    end },
  -- A screenshot shows the next frame: what is shot waits a step.
  { 500, function() app.run("chat.show") end },
  { 6000, function()
      local s = state()
      check("shown again, it carries on", s and not s.hidden and s.running and s.messages > 0, describe(s))
      app.run("watch.toggle_theater")
    end },
  { 3000, function()
      local s = state()
      check("in theater mode, beside the details", s and s.running, describe(s))
      shot("theater")
    end },
  { 500, function()
      app.run("watch.toggle_theater")
      app.run("app.navigate", { page = "home" })
    end },
  { 3000, function()
      check("off screen (the mini player plays), it asks for nothing", state() == nil)
      _G.off = shown
    end },
  { 6000, function()
      check("and nothing shows meanwhile", shown == _G.off, shown - _G.off)
      mark = true
      app.run("player.expand")
    end },
  { 9000, function()
      local s = state()
      check("back on the watch page, it starts afresh", s and s.running and s.messages > 0, describe(s))
      check("plugins' filter changes the messages", marked > 0 and marked == shown - _G.off, marked .. " of " .. (shown - _G.off))
      shot("filtered")
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
    local function run()
      local ok, err = pcall(step[2])
      if not ok then log("STEP FAILED", i, err) end
      next_step()
    end
    app.timer(step[1], function()
      if not step.wait then return run() end
      local started = app.now()
      local function poll()
        if step.wait() or app.now() - started > 25 then return run() end
        app.timer(200, poll)
      end
      poll()
    end)
  end
  next_step()
end)
app.timer(120000, function() log("timeout"); app.quit() end)
