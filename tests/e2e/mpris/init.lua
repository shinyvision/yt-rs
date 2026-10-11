-- For the media controls test (mpris.sh, which runs the app on a private
-- D-Bus session and drives it from outside): plays a video and logs the
-- player's state every second. Silent, no watch history.
local function log(...) app.log("[mpris]", ...) end
app.api.mpv.set_property("ao", "null"); app.api.mpv.set_property("mute", "yes")
app.run("watch.set_tracking", { enabled = false })
app.on("app.ready", function()
  app.timer(3000, function() app.run("video.play", { video = { id = "dQw4w9WgXcQ" }, start = 30 }) end)
  app.timer(1000, function()
    local s = app.api.player.state()
    log(string.format("state: %s paused %s at %.1f volume %.0f speed %.2f looping %s", s.video_id or "-",
      tostring(s.paused), s.position or 0, s.volume or 0, s.speed or 0, tostring(s.looping)))
    return true
  end, true)
  app.timer(tonumber(os.getenv("E2E_SECONDS") or "60") * 1000, function() app.run("player.stop"); app.quit() end)
end)
