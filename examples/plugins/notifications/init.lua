-- YouTube's real inbox, using the default browser's session. All browser
-- integration belongs to this plugin; the app's TV sign-in stays separate.
local NAME, DIR = ...
local backend = require(NAME .. ".backend")
local Gtk = gi.require("Gtk", "4.0")
local Adw = gi.require("Adw", "1")
local Gdk = gi.require("Gdk", "4.0")
local Pango = gi.require("Pango", "1.0")
local PangoCairo = gi.require("PangoCairo", "1.0")
local font_map = PangoCairo.FontMap.new()
font_map:add_font_file(DIR .. "/fonts/Roboto.ttf")
Gtk.IconTheme.get_for_display(Gdk.Display.get_default()):add_search_path(DIR .. "/icons")
local settings = app.storage(NAME)
local state = { count = 0, items = {}, status = "loading", busy = false }
local button, badge, popover, rows, footer, stack, loading_label
local cancel, generation, last_check = nil, 0, 0
local refresh, render, update_badge

local function signed_in() return app.api.social.signed_in() end
local function open_url(url) app.run("app.open_url", { url = url }) end
local function clear(box)
  for _, child in ipairs(box:children()) do box:remove(child) end
end
local function icon_button(icon, tooltip, click)
  local widget = Gtk.Button { child = Gtk.Image { icon_name = icon, pixel_size = 24 },
    tooltip_text = tooltip, css_classes = { "notification-icon-button" } }
  widget:on("clicked", click)
  return widget
end
local function picture(url, width, height, class)
  -- Overlay pictures don't contribute their texture's natural size to the
  -- row. The fixed frame keeps avatars and thumbnails from squeezing text.
  local frame = Gtk.Overlay { width_request = width, height_request = height,
    valign = "start", overflow = "hidden", css_classes = { class } }
  frame:add_overlay(ui.image(url, width))
  return frame
end
local function line_height(pixels)
  local attrs = Pango.AttrList.new()
  attrs:insert(Pango.attr_line_height_new_absolute(pixels * Pango.SCALE))
  return attrs
end
local message_attrs, time_attrs = line_height(20), line_height(18)

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
  state.busy, state.operation, state.progress = true, operation, "Checking your default browser"
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
  end, function(event)
    if current ~= generation then return end
    if event.stage == "fetching" and event.browser then
      local names = { chrome = "Chrome", chromium = "Chromium", firefox = "Firefox", edge = "Edge",
        brave = "Brave", vivaldi = "Vivaldi", opera = "Opera", whale = "Whale", safari = "Safari" }
      state.progress = "Fetching notifications from " .. (names[event.browser] or event.browser)
    else
      state.progress = "Checking your default browser"
    end
    if loading_label then loading_label.label = state.progress end
    app.emit("notifications.progress", { message = state.progress })
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
  local row = Gtk.Overlay { css_classes = { "notification-row" }, name = "notification-" .. (item.id or "") }
  local content = ui.box { orientation = "horizontal", hexpand = true, margin_end = 24 }
  local dot = ui.box { class = "notification-dot", width_request = 4, height_request = 4,
    margin_start = 6, margin_end = 6, margin_top = 22, valign = "start", opacity = item.read and 0 or 1 }
  content:append(dot)
  local avatar = ui.box { width_request = 48, height_request = 48, valign = "start", margin_end = 16 }
  if item.avatar then
    avatar:append(picture(item.avatar, 48, 48, "notification-avatar"))
  end
  content:append(avatar)
  local words = ui.box { orientation = "vertical", spacing = 8, hexpand = true, valign = "start" }
  words:append(ui.label(item.message or "", { wrap = true, wrap_mode = "word-char", lines = 6,
    ellipsize = "end", xalign = 0, max_width_chars = 32, width_chars = 1, hexpand = true,
    attributes = message_attrs, class = "notification-message" }))
  words:append(ui.label(item.sent or "", { xalign = 0, attributes = time_attrs, class = "notification-time" }))
  content:append(words)
  if item.thumbnail then
    local thumb = picture(item.thumbnail, 86, 48, "notification-thumbnail")
    thumb.margin_start, thumb.margin_end = 16, 16
    content:append(thumb)
  end
  local activate = Gtk.Button { child = content, hexpand = true, css_classes = { "notification-open" },
    tooltip_text = item.message or "Open notification" }
  activate:on("clicked", function() app.run("notifications.activate", { item = item }) end)
  row:set_child(activate)
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
    row:add_overlay(Gtk.MenuButton { child = Gtk.Image { icon_name = "notifications-more-symbolic", pixel_size = 24 },
      popover = menu_popover, tooltip_text = "More actions", valign = "start", halign = "end",
      margin_top = 8, margin_end = 8, css_classes = { "notification-actions" } })
  end
  return row
end })

render = function()
  if not rows then return end
  clear(rows)
  clear(footer)
  local loading = state.busy and (state.operation == "menu" or #state.items == 0)
  stack.visible_child_name = loading and "loading" or "list"
  loading_label.label = state.progress or "Checking your default browser"
  if loading then return end
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
    footer:append(Adw.Spinner { width_request = 24, height_request = 24,
      halign = "center", margin_top = 12, margin_bottom = 12 })
  end
end

app.component("notifications_button", { build = function()
  badge = ui.label("", { class = "notification-badge", halign = "end", valign = "start", visible = false })
  local overlay = Gtk.Overlay { child = Gtk.Image { icon_name = "notifications-bell-symbolic", pixel_size = 24 },
    width_request = 24, height_request = 24 }
  overlay:add_overlay(badge)
  local column = ui.box { orientation = "vertical", width_request = 480, height_request = 642 }
  column:set_font_map(font_map)
  local heading = ui.box { orientation = "horizontal", class = "notifications-heading" }
  heading:append(ui.label("Notifications", { xalign = 0, hexpand = true, margin_start = 16, class = "notifications-title" }))
  local gear = icon_button("notifications-settings-symbolic", "Notification settings", function()
    open_url("https://www.youtube.com/account_notifications")
  end)
  gear.margin_end = 8
  heading:append(gear)
  column:append(heading)
  rows, footer = ui.box { orientation = "vertical" }, ui.box { orientation = "vertical" }
  local body = ui.box { orientation = "vertical", children = { rows, footer } }
  local scroller = ui.scrolled(body, { hscrollbar_policy = "never", vscrollbar_policy = "automatic",
    overlay_scrolling = false, vexpand = true, class = "notifications-scroll" })
  scroller:on("edge-reached", function(_, edge)
    if edge == "bottom" and state.status == "ok" then app.run("notifications.more") end
  end)
  loading_label = ui.label("Checking your default browser", { justify = "center", wrap = true,
    class = "notification-loading-message", name = "notifications-loading-message" })
  local loading = ui.box { orientation = "vertical", spacing = 16, halign = "center", valign = "center",
    name = "notifications-loading", children = {
      Adw.Spinner { width_request = 32, height_request = 32, halign = "center",
        name = "notifications-spinner" }, loading_label } }
  stack = Gtk.Stack { vexpand = true }
  stack:add_named(scroller, "list")
  stack:add_named(loading, "loading")
  column:append(stack)
  popover = Gtk.Popover { child = column, has_arrow = false, halign = "end", name = "notifications-popover" }
  popover:set_offset(0, 8)
  popover:add_css_class("notifications-popover")
  popover:on("show", function()
    local window = app.window()
    column.width_request = math.max(280, math.min(480, window:get_width() - 32))
    column.height_request = math.max(240, math.min(642, window:get_height() - 80))
    scroller.vadjustment.value = 0
    refresh(true)
  end)
  button = Gtk.MenuButton { child = overlay, popover = popover, name = "notifications-button",
    tooltip_text = "Notifications", visible = signed_in(), valign = "center", css_classes = { "notifications-bell" } }
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
.notification-dot { background: #3ea6ff; border-radius: 2px; }
.notifications-popover { font-family: "Roboto", Arial, sans-serif; color: #f1f1f1; }
.notification-message { font-size: 14px; }
.notification-time { font-size: 12px; color: #aaa; }
.notification-open { padding: 16px 16px 16px 0; margin: 0; border: 0; border-radius: 0; min-height: 0; min-width: 0; font-weight: 400; background: transparent; box-shadow: none; }
.notification-row:hover { background: rgba(255,255,255,.1); }
.notification-actions > button, .notification-icon-button, .notifications-bell > button { min-width: 40px; min-height: 40px; padding: 0; margin: 0; border: 0; border-radius: 50%; background: transparent; box-shadow: none; }
.notification-actions > button:hover, .notification-icon-button:hover, .notifications-bell > button:hover { background: rgba(255,255,255,.1); }
.notifications-heading { min-height: 48px; border-bottom: 1px solid rgba(255,255,255,.2); }
.notifications-title { font-size: 16px; font-weight: 400; }
.notification-loading-message { font-size: 14px; color: #aaa; }
.notification-avatar { border-radius: 50%; }
.notification-thumbnail { border-radius: 4px; }
.notifications-popover > contents { padding: 0; border: 0; border-radius: 12px; background: #282828; box-shadow: 0 4px 32px rgba(0,0,0,.1); }
.notifications-scroll scrollbar.vertical { min-width: 8px; padding: 0; margin: 0; border: 0; background: transparent; }
.notifications-scroll scrollbar.vertical range, .notifications-scroll scrollbar.vertical trough { min-width: 8px; padding: 0; margin: 0; border: 0; }
.notifications-scroll scrollbar.vertical slider { min-width: 8px; min-height: 56px; padding: 0; margin: 0; border: 0; border-radius: 4px; background: transparent; }
.notifications-scroll:hover scrollbar.vertical slider { background: #717171; }
]]
