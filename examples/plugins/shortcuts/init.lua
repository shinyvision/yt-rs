-- Keyboard shortcuts: jump between pages, seek by percentage, change speed.
--
-- Shows: app.keymap (bind keys to commands), app.bind (bind keys to Lua
-- functions), app.command (add new commands), app.player (raw mpv access).

-- Pages, from anywhere.
app.keymap.set("ctrl+1", "app.navigate", { page = "home" })
app.keymap.set("ctrl+2", "app.navigate", { page = "subscriptions" })
app.keymap.set("ctrl+3", "app.navigate", { page = "history" })
app.keymap.set("ctrl+b", "sidebar.toggle")

-- On the watch page: 0-9 jump to 0%-90%, like on youtube.com.
for digit = 0, 9 do
  app.bind(tostring(digit), function()
    local state = app.player.state()
    if state.duration > 0 then
      app.run("player.seek", { position = state.duration * digit / 10 })
    end
  end, "watch")
end

-- Playback speed, as a new command that anything (keys, buttons, other
-- plugins) can run.
app.command("player.change_speed", function(args)
  local speed = tonumber(app.player.get("speed")) or 1
  speed = math.max(0.25, math.min(4, speed + (args.delta or 0)))
  app.player.set("speed", tostring(speed))
  app.toast("Speed " .. speed .. "x")
  return speed
end)
-- Shift+. and Shift+, (keys are named by the symbol they type: > and <).
app.keymap.set("greater", "player.change_speed", { delta = 0.25 }, "watch")
app.keymap.set("less", "player.change_speed", { delta = -0.25 }, "watch")
