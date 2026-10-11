-- Isolated feasibility probe. Does not replace the existing notification bell.
local _, DIR = ...
local Gtk = gi.require("Gtk", "4.0")
local backend, ready, busy, login_close, login_view
local status_label, rows, login_box, more_button
local client, continuation = "ANDROID", nil
local buttons = {}

local function status(message)
  if status_label then status_label.label = message end
  -- Status text is deliberately fixed/sanitized. No raw server errors.
  app.emit("android-notifications.status", { message = message })
end
local function availability(value)
  busy = value
  for _, button in ipairs(buttons) do button.sensitive = not value end
  if more_button then more_button.sensitive = not value and continuation ~= nil end
end
local function clear_rows()
  continuation = nil
  if not rows then return end
  while rows:get_first_child() do rows:remove(rows:get_first_child()) end
end
local function close_login()
  if login_close then login_close(); login_close = nil end
  if login_view then login_box:remove(login_view); login_view = nil end
end
local function fetch(force, more)
  if not backend or busy or not ready then return end
  if os.getenv("YT_RS_DRY_RUN") == "1" then status("Inbox requests are disabled in dry-run mode because they can mark notifications seen."); return end
  close_login()
  availability(true)
  if not more then clear_rows() end
  status(force and "Renewing YouTube access…" or "Fetching notifications…")
  backend.fetch(client, more and continuation or nil, force, function(page, err)
    if page then
      continuation = page.continuation
      for _, item in ipairs(page.items) do
        local body = ui.box { orientation = "vertical", spacing = 6, margin_top = 10, margin_bottom = 10,
          children = { ui.label(item.message, { xalign = 0, wrap = true, selectable = true }),
            ui.label(item.sent, { xalign = 0, class = "dim-label" }) } }
        if item.video_id then
          local button = Gtk.Button { child = body, has_frame = false }
          button:on("clicked", function() app.run("video.play", { video = { id = item.video_id } }) end)
          rows:append(button)
        else rows:append(body) end
      end
      status(client .. ": received " .. #page.items .. " notifications" .. (force and " using renewed access." or "."))
    else status(err) end
    availability(false)
  end)
end

local function sign_in()
  if not backend or busy then return end
  close_login()
  status("Sign in on accounts.google.com. The account credential will be saved in your system keyring.")
  local ok, Login = pcall(require, "android-notifications.login")
  if not ok then status("This Linux prototype needs WebKitGTK 6.0 for sign-in."); return end
  login_view, login_close = Login.new(DIR, function(handoff)
    app.idle(function()
      close_login()
      availability(true)
      status("Exchanging the Android sign-in credential…")
      backend.sign_in(handoff, function(ok, err)
        handoff = nil
        ready = ok
        availability(false)
        if ok then fetch(true) else status(err) end
      end)
    end)
  end, status)
  login_box:append(login_view)
end

app.page { name = "android-notifications", title = "Android notifications", keep = true, build = function()
  buttons = {}
  status_label = ui.label("Checking the system keyring…", { name = "android-notifications-status", xalign = 0, wrap = true, selectable = true })
  rows = ui.box { orientation = "vertical", spacing = 2, name = "android-notifications-rows" }
  login_box = ui.box { orientation = "vertical", vexpand = true }
  local function button(label, name, handler)
    local widget = ui.button(label, handler, { name = name })
    buttons[#buttons + 1] = widget
    return widget
  end
  more_button = button("More", "android-notifications-more", function() fetch(false, true) end)
  local page = ui.box { orientation = "vertical", spacing = 16, margin_top = 24, margin_bottom = 24,
    margin_start = 24, margin_end = 24, children = {
      ui.label("Android notifications · prototype", { xalign = 0, class = "title-1" }),
      ui.label("Test Google sign-in, the full inbox, and renewed access after restarting yt-rs.", { xalign = 0, wrap = true }),
      ui.box { spacing = 8, children = {
        button("Sign in", "android-notifications-sign-in", sign_in),
        button("Fetch inbox", "android-notifications-fetch", function() fetch(false) end),
        button("Renew access", "android-notifications-renew", function() fetch(true) end),
        button("Forget account", "android-notifications-forget", function()
          if not backend or busy then return end
          close_login(); clear_rows(); ready = false; availability(true)
          backend.forget(function(ok, err) status(ok and "Account removed from the keyring." or err); availability(false) end)
        end),
      } },
      ui.box { spacing = 8, children = {
        ui.label("Inbox client", { xalign = 0 }),
        button("Android", "android-notifications-android", function() client = "ANDROID"; fetch(false) end),
        button("Web with Android token", "android-notifications-web", function() client = "WEB"; fetch(false) end),
      } },
      status_label, login_box, rows, more_button,
    } }
  availability(true)
  local ok, service = pcall(function()
    local http = require("android-notifications.transport").new()
    local keyring = require("android-notifications.keyring").new(app.api.paths.config_dir())
    return require("android-notifications.backend").new(http, keyring, app.json, os.time)
  end)
  if ok then
    backend = service
    backend.restore(function(found, err)
      ready = found
      status(err or (found and "Account restored from the system keyring. Fetch the inbox to test access after restart." or "Sign in to test Android notifications."))
      availability(false)
    end)
  else status("This Linux prototype needs libsecret and libsoup 3.") end
  return ui.scrolled(page, { vexpand = true, hscrollbar_policy = "never" })
end }
app.nav { id = "android-notifications", title = "Android notifications", icon = "notifications-symbolic", page = "android-notifications", section = "library", order = 90 }
app.command("android-notifications.sign_in", sign_in)
app.command("android-notifications.fetch", function() fetch(false) end)
app.command("android-notifications.renew", function() fetch(true) end)
