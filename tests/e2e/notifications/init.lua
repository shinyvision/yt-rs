-- Runs beside the notifications plugin that comes with the app; install it
-- as notifications-test.
-- All account and network operations use Lua fixtures.
local _, DIR = ...
local Gtk = gi.require("Gtk", "4.0")
local Gdk = gi.require("Gdk", "4.0")
local original_image = ui.image
ui.image = function(url, width)
  if url and url:find("fixture://", 1, true) == 1 then return Gtk.Picture {} end
  return original_image(url, width)
end
local fixture = require("notifications-test.fixture")
local requests, loaded, unseen, read, opened = fixture.requests, nil, nil, nil, nil
app.api.mpv.set_property("ao", "null")
app.api.mpv.set_property("mute", "yes")
app.run("watch.set_tracking", { enabled = false })
-- Exercise the real link router; stop at playback so no video/network starts.
app.command("video.play", function(args)
  opened = { kind = "video", video_id = args.video.id, start = args.start }
end)
app.command("app.open_url", function(args) opened = args.url end)
app.on("notifications.loaded", function(e) loaded = e end)
app.on("notifications.unseen", function(e) unseen = e.count end)
app.on("notifications.read", function(e) read = e.id end)
app.filter("notifications.page", function(page)
  for _, item in ipairs(page.items) do item.filtered = true end
  return page
end)
local passed, failed = 0, 0
local function check(label, condition)
  if condition then passed = passed + 1 else failed = failed + 1 end
  app.log("[notifications]", condition and "PASS" or "FAIL", label)
end
local function shot(name) app.screenshot((os.getenv("E2E_SHOTS") or "/tmp") .. "/" .. name .. ".png") end
local function pop() return app.find("notifications-popover") end
local steps = {
  { 800, function()
    check("bell beside account", app.find("notifications-button").visible)
    local icons = Gtk.IconTheme.get_for_display(Gdk.Display.get_default())
    check("YouTube icons available", icons:has_icon("notifications-bell-symbolic") and
      icons:has_icon("notifications-settings-symbolic") and icons:has_icon("notifications-more-symbolic"))
    check("signed-out startup sends no notification requests", #requests == 0)
    check("sign-in notice badge", app.find(".notification-badge").label == "!")
    app.run("notifications.open")
  end },
  { 500, function()
    check("sign-in notice inside bell", app.find("notifications-notice") ~= nil)
    local notice = app.find("notifications-notice-area")
    local parent = notice:get_parent()
    local ok, x, y = notice:translate_coordinates(parent, 0, 0)
    check("sign-in prompt centered in the full panel", notice:get_mapped() and ok and
      math.abs(x + notice:get_width() / 2 - parent:get_width() / 2) <= 0.5 and
      math.abs(y + notice:get_height() / 2 - parent:get_height() / 2) <= 0.5)
    local labels = app.find("notifications-notice"):children()
    check("sign-in prompt text centered", labels[1].xalign == 0.5 and labels[2].xalign == 0.5 and labels[2].justify == "center")
    shot("00-sign-in-prompt")
    app.find("notifications-sign-in"):emit("clicked")
    check("sign-in stays in the app", opened == nil and app.find("notifications-login") ~= nil)
    shot("01-sign-in")
    app.find("notifications-login"):close()
  end },
  { 500, function()
    check("cancelling sign-in cleans up and returns to prompt", fixture.closed > 0 and app.find("notifications-sign-in") ~= nil)
    app.find("notifications-sign-in"):emit("clicked")
    fixture.complete()
  end },
  { 600, function()
    check("sign-in automatically opens inbox", loaded and #loaded.items == 2)
    app.run("notifications.refresh")
    check("centered loading view", app.find("notifications-loading"):get_mapped())
    check("YouTube loading message", app.find("notifications-loading-message").label == "Fetching notifications…")
    local spinner = app.find("notifications-spinner")
    check("visible centered spinner", spinner:get_mapped() and spinner:get_width() == 32 and spinner.halign == "center")
    shot("loading-notifications")
  end },
  { 500, function()
    check("real menu rows protocol", loaded and #loaded.items == 2)
    check("plugin filter applies", loaded and loaded.items[1].filtered)
    check("loaded content hides loading status", not app.find("notifications-loading"):get_mapped())
    local label = app.find("notification-upload"):find(".notification-message")
    check("six-line message limit", label.lines == 6)
    local panel = pop():get_child()
    check("YouTube panel dimensions", panel:get_width() == 480 and panel:get_height() == 642)
    check("opening clears unseen badge", unseen == 0 and not app.find(".notification-badge").visible)
    check("unread dot", app.find("notification-upload"):find(".notification-dot").opacity == 1)
    check("read row has no dot", app.find("notification-reply"):find(".notification-dot").opacity == 0)
    shot("02-notifications")
  end },
  { 100, function() app.run("notifications.more") end },
  { 500, function()
    check("continuation appends and deduplicates", loaded and #loaded.items == 3 and not loaded.first)
    check("last page removes Show more", app.find("notifications-more") == nil)
    opened = nil
    app.find("notification-upload"):find(".notification-open"):emit("clicked")
    check("upload opens native video", type(opened) == "table" and opened.video_id == "abcdefghijk")
  end },
  { 500, function()
    check("click recorded", read == "upload")
    app.run("notifications.open")
  end },
  { 500, function()
    opened = nil
    app.find("notification-reply"):find(".notification-open"):emit("clicked")
    check("comment notification opens native video", type(opened) == "table" and opened.video_id == "abcdefghijk")
  end },
  { 500, function()
    for _, url in ipairs({
      "https://www.youtube.com/watch?v=abcdefghijk&lc=comment&t=90",
      "https://youtu.be/abcdefghijk?t=90",
      "https://www.youtube.com/shorts/abcdefghijk?t=90",
      "https://www.youtube.com/live/abcdefghijk?t=90",
    }) do
      opened = nil
      app.run("notifications.activate", { item = { destination = { url = url } } })
      check("video URL opens native player with timestamp", type(opened) == "table" and
        opened.video_id == "abcdefghijk" and opened.start == 90)
    end
    opened = nil
    app.run("notifications.activate", { item = { destination = { url = "https://www.youtube.com/post/example" } } })
    check("unsupported destination retains browser fallback", opened == "https://www.youtube.com/post/example")
  end },
  { 500, function() app.run("notifications.refresh") end },
  { 500, function()
    check("badge caps at 9+", app.find(".notification-badge").label == "9+")
    fixture.mode = "empty"; app.run("notifications.open")
  end },
  { 500, function()
    check("empty inbox", loaded and #loaded.items == 0)
    shot("03-empty")
  end },
  { 100, function() fixture.mode = "error"; app.run("notifications.refresh") end },
  { 500, function()
    check("network error offers retry", app.find("notifications-retry") ~= nil)
    shot("04-error")
    fixture.mode = "slow"; app.run("notifications.refresh")
    app.emit("auth.signed_out")
  end },
  { 1500, function()
    check("sign-out clears inbox despite a late response", app.find("notifications-button").visible and not fixture.connected and app.find("notification-upload") == nil)
    app.run("notifications.sign_in"); fixture.complete()
  end },
  { 500, function()
    check("sign-in restores the inbox", fixture.connected and loaded and #loaded.items == 2)
    app.run("notifications.open")
  end },
  { 500, function()
    local actions = app.find("notification-upload"):children()
    actions[#actions]:popup()
  end },
  { 300, function()
    shot("05-actions")
    app.run("notifications.action", { action = loaded.items[1].menu[1].action })
  end },
  { 600, function()
    check("context action re-reads inbox", requests[#requests] == "menu" and requests[#requests - 1] == "interact")
    fixture.mode = "geometry"; app.run("notifications.refresh")
  end },
  { 500, function()
    local row = app.find("notification-upload")
    local message = row:find(".notification-message")
    local thumb = row:find(".notification-thumbnail")
    local avatar = row:find(".notification-avatar")
    local scroll = app.find(".notifications-scroll")
    check("scrollbar preserves message width", scroll:get_vscrollbar():get_width() == 8 and message:get_width() == 234)
    check("reference message wraps into five 20px lines", message:get_height() == 100)
    check("reference row height", row:get_height() == 158)
    check("reference image sizes", avatar:get_width() == 48 and avatar:get_height() == 48 and
      thumb:get_width() == 86 and thumb:get_height() == 48)
    scroll.vadjustment.value = scroll.vadjustment.upper - scroll.vadjustment.page_size
  end },
  { 500, function()
    check("scrolling to bottom fetches continuation", requests[#requests] == "more" and loaded and #loaded.items == 9)
    fixture.mode = "expired"; app.run("notifications.refresh")
  end },
  { 500, function()
    check("expired sign-in clears rows and offers sign-in", app.find("notifications-sign-in") ~= nil and app.find("notification-upload") == nil)
    app.run("notifications.sign_in"); fixture.complete()
  end },
  { 600, function()
    app.run("notifications.settings")
    check("gear offers account settings", app.find("notifications-settings") ~= nil and app.find("notifications-account-action").label == "Sign out of notifications")
    app.find("notifications-account-action"):emit("clicked")
  end },
  { 500, function()
    app.run("notifications.open")
    check("account settings disconnect the bell", not fixture.connected and app.find("notifications-sign-in") ~= nil)
    fixture.locked = true
    app.run("notifications.sign_out")
  end },
  { 100, function()
    check("failed credential removal offers retry", app.find("notifications-retry") ~= nil)
    fixture.locked = false
    fixture.restore_checkpoint, fixture.forget_checkpoint = fixture.restores, fixture.forgets
    app.find("notifications-retry"):emit("clicked")
  end },
  { 100, function()
    check("sign-out retry removes the credential without reconnecting", fixture.restores == fixture.restore_checkpoint and
      fixture.forgets == fixture.forget_checkpoint + 1 and app.find("notifications-retry") == nil and not fixture.connected)
    app.log("[notifications]", string.format("SUMMARY %d passed, %d failed", passed, failed))
    app.quit()
  end },
}
local function run(index)
  local step = steps[index]
  if not step then return end
  app.timer(step[1], function()
    local ok, error = pcall(step[2])
    if not ok then check("step " .. index .. ": " .. tostring(error), false) end
    run(index + 1)
  end)
end
app.on("app.ready", function() run(1) end)
