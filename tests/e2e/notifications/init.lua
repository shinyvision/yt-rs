-- Install beside examples/plugins/notifications, under notifications-test.
-- All requests use a subprocess fixture; no browser/account data is read.
local _, DIR = ...
local backend = require("notifications.backend")
local original_request = backend.request
local mode, signed = "missing", true
local requests, loaded, unseen, read, opened = {}, nil, nil, nil, nil
backend.request = function(_, settings, request, done)
  request.fixture_mode = mode
  table.insert(requests, request.operation)
  return original_request(DIR, settings, request, done)
end
app.hook("social.signed_in", function() return signed end)
app.api.mpv.set_property("ao", "null")
app.api.mpv.set_property("mute", "yes")
app.run("watch.set_tracking", { enabled = false })
app.command("link.open", function(args) opened = args.link end)
app.command("app.open_url", function(args) opened = args.url end)
app.on("notifications.loaded", function(e) loaded = e end)
app.on("notifications.unseen", function(e) unseen = e.count end)
app.on("notifications.read", function(e) read = e.id end)
app.filter("notifications.page", function(page)
  for _, item in ipairs(page.items) do item.message = "[filtered] " .. item.message end
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
    check("startup polls count only", #requests == 1 and requests[1] == "count")
    check("sign-in notice badge", app.find(".notification-badge").label == "!")
    app.run("notifications.open")
  end },
  { 500, function()
    check("sign-in notice inside bell", app.find("notifications-notice") ~= nil)
    app.find("notifications-sign-in"):emit("clicked")
    check("opens YouTube in default browser", opened == "https://www.youtube.com/")
    shot("01-sign-in")
  end },
  { 100, function() mode = "ok"; app.find("notifications-retry"):emit("clicked") end },
  { 500, function()
    check("real menu rows protocol", loaded and #loaded.items == 2)
    check("plugin filter applies", loaded and loaded.items[1].message:find("[filtered]", 1, true))
    check("opening clears unseen badge", unseen == 0 and not app.find(".notification-badge").visible)
    check("unread dot", app.find("notification-upload"):find(".notification-dot").opacity == 1)
    check("read row has no dot", app.find("notification-reply"):find(".notification-dot").opacity == 0)
    shot("02-notifications")
  end },
  { 100, function() app.run("notifications.more") end },
  { 500, function()
    check("continuation appends and deduplicates", loaded and #loaded.items == 3 and not loaded.first)
    check("last page removes Show more", app.find("notifications-more") == nil)
    app.find("notification-upload"):find(".notification-open"):emit("clicked")
    check("upload opens native video", type(opened) == "table" and opened.video_id == "abcdefghijk")
  end },
  { 500, function()
    check("click recorded", read == "upload")
    app.run("notifications.open")
  end },
  { 500, function()
    app.find("notification-reply"):find(".notification-open"):emit("clicked")
    check("comment preserves thread URL", type(opened) == "string" and opened:find("lc=comment", 1, true))
  end },
  { 500, function() app.run("notifications.refresh") end },
  { 500, function()
    check("badge caps at 9+", app.find(".notification-badge").label == "9+")
    mode = "empty"; app.run("notifications.open")
  end },
  { 500, function()
    check("empty inbox", loaded and #loaded.items == 0)
    shot("03-empty")
  end },
  { 100, function() mode = "error"; app.run("notifications.refresh") end },
  { 500, function()
    check("network error offers retry", app.find("notifications-retry") ~= nil)
    shot("04-error")
    mode = "slow"; app.run("notifications.refresh")
    signed = false; app.emit("auth.signed_out")
  end },
  { 1500, function()
    check("sign-out hides bell and cancels request", not app.find("notifications-button").visible and not pop().visible)
    mode = "ok"; signed = true; app.emit("auth.signed_in")
  end },
  { 500, function()
    check("sign-in resumes count", app.find("notifications-button").visible and unseen == 12)
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
