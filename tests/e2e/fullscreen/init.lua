-- Fullscreen and the keys and pages around it: Escape in fullscreen only
-- leaves fullscreen (it goes back otherwise); leaving the video's page
-- leaves fullscreen too; the top bar and the sidebar come back. Keys go
-- through the window's own key handling (its key controller), not the
-- desktop. Signed out works; silent, no watch history.
local function log(...) app.log("[fullscreen]", ...) end
app.api.mpv.set_property("ao", "null")
app.api.mpv.set_property("mute", "yes")
app.run("watch.set_tracking", { enabled = false })
local Gdk = gi.require("Gdk", "4.0")
local results = { pass = 0, fail = 0 }
local function check(label, ok, detail)
  if ok then results.pass = results.pass + 1 else results.fail = results.fail + 1 end
  log(ok and "PASS" or "FAIL", label, detail and ("| " .. tostring(detail)) or "")
end
local function fullscreen() return app.window():is_fullscreen() end
local function page() return app.api.app.current_page() end
local function shown(name) local w = app.find(name); return w ~= nil and w:get_mapped() end
local function press(keyval)
  for _, c in ipairs(app.window():controllers()) do
    if c:is_a("GtkEventControllerKey") then c:emit("key-pressed", keyval, 0, 0) end
  end
end

local steps = {
  { 2000, function() app.run("video.play", { video = { id = "jNQXAC9IVRw" } }) end },
  { 3000, function()
      check("on the watch page", page() == "watch", page())
      app.run("window.fullscreen", { fullscreen = true })
    end },
  { 300, wait = fullscreen, function()
      check("fullscreen", fullscreen())
      press(Gdk.KEY_Escape)
    end },
  { 300, wait = function() return not fullscreen() end, function()
      check("Escape leaves fullscreen", not fullscreen())
      check("and stays on the video", page() == "watch", page())
      press(Gdk.KEY_Escape)
    end },
  { 800, function()
      check("Escape out of fullscreen goes back", page() == "home", page())
      check("the top bar shows", shown("topbar"))
      check("the sidebar shows", shown("sidebar"))
      app.run("app.forward")
    end },
  { 1500, function()
      app.run("window.fullscreen", { fullscreen = true })
    end },
  { 300, wait = fullscreen, function() app.run("app.back") end },
  { 300, wait = function() return not fullscreen() end, function()
      check("going back from a fullscreen video leaves fullscreen", not fullscreen())
      check("to the page before", page() == "home", page())
    end },
  { 800, function()
      check("with the top bar", shown("topbar"))
      check("and the sidebar", shown("sidebar"))
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
        if app.now() - started > 5 then log("WAITED IN VAIN", i); return run() end
        app.timer(100, poll)
      end
      poll()
    end)
  end
  next_step()
end)
app.timer(60000, function() log("timeout"); app.quit() end)
