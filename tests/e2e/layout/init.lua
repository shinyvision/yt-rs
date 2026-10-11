-- Layout check: the shell (top bar, sidebar, rail, drawer) on a feed and the
-- watch page, at the monitor's size. Screenshots only. See ../README.md.
local shots = os.getenv("E2E_SHOTS") or "/tmp"
local prefix = os.getenv("E2E_PREFIX") or ""
local function log(...) app.log("[layout]", ...) end
app.api.mpv.set_property("ao", "null"); app.api.mpv.set_property("mute", "yes"); app.run("watch.set_tracking", { enabled = false })
local n = 0
local function shot(name) n = n + 1; app.screenshot(string.format("%s/%s%02d-%s.png", shots, prefix, n, name)) end
local steps = {
  { 6000, function() shot("home") end },
  { 100, function() app.run("sidebar.toggle") end },
  { 900, function()
      local drawer = app.find("sidebar-drawer")
      log("toggled: rail", tostring(app.find("sidebar-mini"):get_mapped()), "| docked", tostring(app.find("sidebar"):get_mapped()),
        "| drawer", tostring(drawer and drawer:get_mapped()))
      shot("home-toggled")
    end },
  { 100, function() app.run("sidebar.toggle") end },
  { 400, function()
      app.toast("Saved to Watch later", { action = { label = "Undo", command = "app.dismiss_toast" } })
    end },
  { 600, function() shot("toast") end },
  { 600, function() app.run("video.play", { video = { id = "dQw4w9WgXcQ" }, start = 30 }) end },
  { 8000, function() shot("watch") end },
  { 100, function() app.run("sidebar.toggle") end },
  { 900, function() shot("watch-drawer") end },
  { 100, function() app.run("sidebar.toggle"); app.run("watch.toggle_theater") end },
  { 1500, function() shot("watch-theater") end },
  { 100, function() app.run("watch.toggle_theater") end },
  { 500, function() app.run("player.mini") end },
  { 2500, function()
      local s = app.api.player.state()
      log("mini: page", app.current_page(), "| playing", tostring(not s.paused), "| position", s.position, "| docked visible", tostring(app.find("mini-player"):get_mapped()))
      shot("mini-player")
      local function walk(w)
        for _, c in ipairs(w:controllers()) do
          if c:is_a("GtkEventControllerMotion") then c:emit("enter", 100.0, 100.0) end
        end
        for _, ch in ipairs(w:children()) do walk(ch) end
      end
      walk(app.find("mini-player"))
    end },
  { 600, function() shot("mini-hovered"); app.run("player.toggle_pause") end },
  { 600, function() log("mini paused:", tostring(app.api.player.state().paused)); shot("mini-paused"); app.run("player.toggle_pause") end },
  { 100, function() app.run("app.navigate", { page = "subscriptions" }) end },
  { 2500, function() log("mini on subscriptions:", tostring(app.find("mini-player"):get_mapped())); shot("mini-on-subscriptions") end },
  { 100, function() app.run("player.expand") end },
  { 2500, function()
      local s = app.api.player.state()
      log("expanded: page", app.current_page(), "| position", s.position, "(kept, not restarted)")
      shot("expanded")
    end },
  { 100, function() app.run("player.mini") end },
  { 1500, function() app.run("player.stop") end },
  { 1500, function() log("closed: mini visible", tostring(app.find("mini-player"):get_mapped())); shot("mini-closed") end },
  { 100, function() app.quit() end },
}
app.on("app.ready", function()
  local i = 0
  local function nxt() i = i + 1; local s = steps[i]; if s then app.timer(s[1], function() local ok, e = pcall(s[2]); if not ok then log("ERR", e) end; nxt() end) end end
  nxt()
end)
