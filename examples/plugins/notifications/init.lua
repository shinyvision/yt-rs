-- YouTube's real inbox, using the default browser's session. All browser
-- integration belongs to this plugin; the app's TV sign-in stays separate.
local NAME, DIR = ...
local backend = require(NAME .. ".backend")
local Gtk = gi.require("Gtk", "4.0")
local settings = app.storage(NAME)
local state = { count = 0, items = {}, status = "loading", busy = false }
local button, badge, popover, rows, footer, source
local cancel, generation, last_check = nil, 0, 0
local refresh, render, update_badge

local function signed_in() return app.api.social.signed_in() end
local function open_url(url) app.run("app.open_url", { url = url }) end
local function clear(box)
  for _, child in ipairs(box:children()) do box:remove(child) end
end
local function icon_button(icon, tooltip, click)
  local widget = Gtk.Button { icon_name = icon, tooltip_text = tooltip, css_classes = { "flat", "circular" } }
  widget:on("clicked", click)
  return widget
end
local function picture(url, width, height, class)
  -- Overlay pictures don't contribute their texture's natural size to the
  -- row. The fixed frame keeps avatars and thumbnails from squeezing text.
  local frame = Gtk.Overlay { width_request = width, height_request = height,
    valign = "center", overflow = "hidden", css_classes = { class } }
  frame:add_overlay(ui.image(url, width))
  return frame
end

update_badge = function()
  if not badge then return end
  local notice = state.status ~= "ok" and state.status ~= "loading"
  badge.label = notice and "!" or (state.count > 9 and "9+" or tostring(state.count))
  badge.visible = notice or state.count > 0
  button.tooltip_text = notice and "Notifications need attention" or
    (state.count > 0 and ("Notifications (" .. state.count .. " unseen)") or "Notifications")
end

local function request(operation, action, done)
  if state.busy then return false end
  local current = generation
  state.busy = true
  render()
  cancel = backend.request(DIR, settings, { operation = operation, action = action }, function(result)
    if current ~= generation then return end
    cancel = nil
    state.busy = false
    last_check = app.now()
    if result.status ~= "ok" then
      state.status, state.message = result.status, result.message
      if result.status == "sign_in_required" or result.status == "cookies_unavailable" then
        state.items, state.continuation, state.count = {}, nil, 0
      end
      app.emit("notifications.failed", { status = result.status, message = result.message })
    else
      state.status, state.message, state.browser = "ok", nil, result.browser or state.browser
      if done then done(result) end
    end
    update_badge()
    render()
    -- An initial badge check may still be running when the user opens the
    -- bell. Finish it, then load the menu, without overlapping helpers.
    if operation == "count" and popover and popover.visible and result.status == "ok" then
      refresh(true)
    end
  end)
  return true
end

local function load_page(result, append)
  local page = app.apply_filter("notifications.page", { items = result.items or app.array{}, continuation = result.continuation })
  if not append then state.items = {} end
  local seen = {}
  for _, item in ipairs(state.items) do seen[item.id] = true end
  for _, item in ipairs(page.items or {}) do
    if not item.id or not seen[item.id] then
      table.insert(state.items, item)
      if item.id then seen[item.id] = true end
    end
  end
  state.continuation = page.continuation
  state.count = 0
  app.emit("notifications.unseen", { count = 0 })
  app.emit("notifications.loaded", { items = app.array(state.items), continuation = state.continuation, first = not append })
end

refresh = function(menu)
  if not signed_in() or state.busy then return end
  if menu then
    request("menu", nil, function(result) load_page(result, false) end)
  else
    request("count", nil, function(result)
      state.count = result.count or 0
      app.emit("notifications.unseen", { count = state.count })
    end)
  end
end

app.command("notifications.refresh", function()
  refresh(popover and popover.visible)
end)
app.command("notifications.open", function()
  if button and signed_in() then button:popup() end
end)
app.command("notifications.more", function()
  if state.continuation and not state.busy then
    request("more", state.continuation, function(result) load_page(result, true) end)
  end
end)
app.command("notifications.activate", function(args)
  local item = args.item
  if not item or state.busy then return end
  if item.record_click then
    request("interact", item.record_click, function()
      item.read = true
      for _, row in ipairs(state.items) do if row.id == item.id then row.read = true end end
      app.emit("notifications.read", { id = item.id })
    end)
  end
  if popover then popover:popdown() end
  local target = item.destination or {}
  if target.link then app.run("link.open", { link = target.link })
  elseif target.url then open_url(target.url) end
end)
app.command("notifications.action", function(args)
  if not args.action then return end
  request("interact", args.action, function()
    -- Re-read the inbox after YouTube applies its action (hide, turn off...).
    app.idle(function() refresh(true) end)
  end)
end)

app.component("notification", { build = function(item)
  local row = ui.box { orientation = "horizontal", spacing = 8, class = "notification-row",
    name = "notification-" .. (item.id or ""), margin_start = 12, margin_end = 8, margin_top = 6, margin_bottom = 6 }
  local dot = ui.label("●", { class = "notification-dot", width_request = 8, opacity = item.read and 0 or 1 })
  row:append(dot)
  local content = ui.box { orientation = "horizontal", spacing = 12, hexpand = true }
  if item.avatar then
    content:append(picture(item.avatar, 40, 40, "notification-avatar"))
  end
  local words = ui.box { orientation = "vertical", spacing = 6, hexpand = true, valign = "center" }
  words:append(ui.label(item.message or "", { wrap = true, wrap_mode = "word-char", lines = 3,
    ellipsize = "end", xalign = 0, max_width_chars = 32, hexpand = true, class = "notification-message" }))
  words:append(ui.label(item.sent or "", { xalign = 0, class = "dim notification-time" }))
  content:append(words)
  if item.thumbnail then
    content:append(picture(item.thumbnail, 86, 48, "notification-thumbnail"))
  end
  local activate = Gtk.Button { child = content, hexpand = true, css_classes = { "flat", "notification-open" },
    tooltip_text = item.message or "Open notification" }
  activate:on("clicked", function() app.run("notifications.activate", { item = item }) end)
  row:append(activate)
  if item.menu and #item.menu > 0 then
    local actions = ui.box { orientation = "vertical" }
    local menu_popover = Gtk.Popover { child = actions, has_arrow = false }
    menu_popover:add_css_class("yt-menu-popover")
    for _, entry in ipairs(item.menu) do
      actions:append(ui.button(entry.label, function()
        menu_popover:popdown()
        app.run("notifications.action", { action = entry.action })
      end, { class = "flat", halign = "fill" }))
    end
    row:append(Gtk.MenuButton { icon_name = "view-more-symbolic", popover = menu_popover,
      tooltip_text = "More actions", valign = "start", css_classes = { "flat", "circular" } })
  end
  return row
end })

render = function()
  if not rows then return end
  clear(rows)
  clear(footer)
  source.label = state.browser and ("From " .. state.browser:gsub("^%l", string.upper)) or "From your default browser"
  if state.status ~= "ok" and state.status ~= "loading" then
    local notice = ui.box { orientation = "vertical", spacing = 12, class = "notification-notice",
      name = "notifications-notice", margin_start = 24, margin_end = 24, margin_top = 24, margin_bottom = 24 }
    notice:append(ui.label(state.status == "sign_in_required" and "Sign in to YouTube" or "Notifications unavailable",
      { class = "title-3", wrap = true }))
    notice:append(ui.label(state.message or "Try again in a moment.", { wrap = true, xalign = 0, max_width_chars = 44 }))
    local buttons = ui.box { orientation = "horizontal", spacing = 8, halign = "center" }
    buttons:append(ui.button("Open YouTube", function() open_url("https://www.youtube.com/") end,
      { name = "notifications-sign-in", class = "suggested-action" }))
    buttons:append(ui.button("Try again", function() refresh(true) end,
      { name = "notifications-retry", sensitive = not state.busy }))
    notice:append(buttons)
    rows:append(notice)
  end
  for _, item in ipairs(state.items) do rows:append(app.build("notification", item)) end
  if state.status == "ok" and #state.items == 0 and not state.busy then
    rows:append(ui.box { orientation = "vertical", spacing = 12, margin_top = 60, margin_bottom = 60,
      children = { ui.label("Your notifications live here", { class = "title-3" }),
        ui.label("Subscribe to your favorite channels to get notified about their latest videos.",
          { wrap = true, justify = "center", max_width_chars = 42, class = "dim" }) } })
  end
  if state.busy then
    footer:append(Gtk.Spinner { spinning = true, halign = "center", margin_top = 12, margin_bottom = 12 })
  elseif state.continuation then
    footer:append(ui.button("Show more", function() app.run("notifications.more") end,
      { name = "notifications-more", class = "flat" }))
  end
end

app.component("notifications_button", { build = function()
  badge = ui.label("", { class = "notification-badge", halign = "end", valign = "start", visible = false })
  local overlay = Gtk.Overlay { child = Gtk.Image { icon_name = "yt-bell-symbolic", pixel_size = 24 },
    width_request = 28, height_request = 28 }
  overlay:add_overlay(badge)
  local column = ui.box { orientation = "vertical", width_request = 460 }
  local heading = ui.box { orientation = "horizontal", spacing = 8, margin_start = 16, margin_end = 8,
    margin_top = 8, margin_bottom = 8 }
  heading:append(ui.label("Notifications", { xalign = 0, hexpand = true, class = "title-3" }))
  heading:append(icon_button("view-refresh-symbolic", "Refresh notifications", function() refresh(true) end))
  heading:append(icon_button("yt-settings-symbolic", "Notification settings", function()
    open_url("https://www.youtube.com/account_notifications")
  end))
  column:append(heading)
  source = ui.label("From your default browser", { xalign = 0, class = "dim", margin_start = 16, margin_bottom = 8 })
  column:append(source)
  column:append(Gtk.Separator { orientation = "horizontal" })
  rows, footer = ui.box { orientation = "vertical" }, ui.box { orientation = "vertical" }
  local body = ui.box { orientation = "vertical", children = { rows, footer } }
  local scroller = ui.scrolled(body, { hscrollbar_policy = "never", min_content_height = 220,
    max_content_height = 560, propagate_natural_height = true })
  column:append(scroller)
  popover = Gtk.Popover { child = column, has_arrow = false, halign = "end", name = "notifications-popover" }
  popover:add_css_class("yt-menu-popover")
  popover:add_css_class("notifications-popover")
  popover:on("show", function()
    local window = app.window()
    column.width_request = math.max(260, math.min(460, window:get_width() - 32))
    scroller.max_content_height = math.max(220, math.min(560, window:get_height() - 180))
    refresh(true)
  end)
  button = Gtk.MenuButton { child = overlay, popover = popover, name = "notifications-button",
    tooltip_text = "Notifications", visible = signed_in(), valign = "center", css_classes = { "flat", "circular" } }
  update_badge()
  render()
  return button
end })
app.slot("topbar.end", function() return app.build("notifications_button") end, 100)

app.on("auth.signed_out", function()
  generation = generation + 1
  if cancel then cancel(); cancel = nil end
  state = { count = 0, items = {}, status = "loading", busy = false }
  if popover then popover:popdown() end
  if button then button.visible = false end
  update_badge()
  render()
end)
app.on("auth.signed_in", function()
  if button then button.visible = true end
  refresh(false)
end)
app.on("app.ready", function()
  refresh(false)
  local window = app.window()
  window:on("notify::is-active", function()
    if window.is_active and app.now() - last_check >= 180 then refresh(false) end
  end)
  app.timer(180000, function()
    if window.is_active then refresh(false) end
    return true
  end, true)
end)

app.css [[
.notification-badge { background: #c00; color: white; border-radius: 10px; font-size: 10px; font-weight: 600; min-width: 14px; padding: 0 3px; }
.notification-dot { color: #3ea6ff; font-size: 10px; }
.notification-message { font-size: 14px; }
.notification-time { font-size: 12px; }
.notification-open { padding: 6px 0; border-radius: 8px; font-weight: normal; }
.notification-avatar { border-radius: 50%; }
.notification-thumbnail { border-radius: 6px; }
.notifications-popover > contents { padding: 0; }
]]
