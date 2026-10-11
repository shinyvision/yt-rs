-- Scrolling to the end of the comments loads more, on the first video
-- watched and on the next one too (the watch page stays, its comments are
-- made anew). Signed out works: comments need no account. Silent, no watch
-- history.
local function log(...) app.log("[comments]", ...) end
app.api.mpv.set_property("ao", "null")
app.api.mpv.set_property("mute", "yes")
app.run("watch.set_tracking", { enabled = false })
local results = { pass = 0, fail = 0 }
local function check(label, ok, detail)
  if ok then results.pass = results.pass + 1 else results.fail = results.fail + 1 end
  log(ok and "PASS" or "FAIL", label, detail and ("| " .. tostring(detail)) or "")
end

local pages = {}   -- comment pages loaded, by video
app.on("comments.loaded", function(e)
  pages[e.video_id] = (pages[e.video_id] or 0) + 1
end)
local function scroll_to_end()
  local scroller = app.find("watch-info")
  local adj = scroller:get_vadjustment()
  adj:set_value(adj:get_upper() - adj:get_page_size())
end

local FIRST, SECOND = "jNQXAC9IVRw", "dQw4w9WgXcQ"
local steps = {
  { 2000, function() app.run("video.play", { video = { id = FIRST } }) end },
  { 500, wait = function() return (pages[FIRST] or 0) >= 1 end, function() scroll_to_end() end },
  { 500, wait = function() return (pages[FIRST] or 0) >= 2 end, function()
      check("scrolling down loads more comments", (pages[FIRST] or 0) >= 2, pages[FIRST])
      app.run("video.play", { video = { id = SECOND } })
    end },
  { 500, wait = function() return (pages[SECOND] or 0) >= 1 end, function() scroll_to_end() end },
  { 500, wait = function() return (pages[SECOND] or 0) >= 2 end, function()
      check("and on the next video too", (pages[SECOND] or 0) >= 2, pages[SECOND])
    end },
  { 200, function()
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
        app.timer(100, poll)
      end
      poll()
    end)
  end
  next_step()
end)
app.timer(120000, function() log("timeout"); app.quit() end)
