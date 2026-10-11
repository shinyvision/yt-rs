-- How fast the app starts: logs when the window is ready and when Home has
-- its first videos (the startup steps log with YT_RS_LOG=startup=debug).
local function log(...) app.log("[startup]", ...) end
local t0 = app.now()
app.on("app.ready", function() log(string.format("ready after plugins: %.0f ms", (app.now() - t0) * 1000)) end)
local shown = false
app.on("feed.loaded", function(e)
  if shown or e.browse_id ~= "FEwhat_to_watch" then return end
  shown = true
  log(string.format("Home loaded: %.0f ms after plugins loaded", (app.now() - t0) * 1000))
  app.timer(500, function() app.quit() end)
end)
app.timer(20000, function() log("timeout"); app.quit() end)
