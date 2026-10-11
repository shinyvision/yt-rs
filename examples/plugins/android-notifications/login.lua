local WebKit = gi.require("WebKit", "6.0")
local P = require("android-notifications.protocol")
local M = {}

function M.new(dir, done, status)
  local file = assert(io.open(dir .. "/bridge.js", "r"))
  local script = file:read("*a")
  file:close()
  local session = WebKit.NetworkSession.new_ephemeral()
  local cookies = session:get_cookie_manager()
  local manager = WebKit.UserContentManager.new()
  local view = WebKit.WebView { network_session = session, user_content_manager = manager,
    hexpand = true, vexpand = true, height_request = 560, name = "android-google-login" }
  view:get_settings():set_user_agent(P.login_agent)
  local active, reading, email, timer = true, false, nil, nil
  local function close()
    if not active then return end
    active = false
    if timer then timer:cancel(); timer = nil end
    view:stop_loading()
    manager:unregister_script_message_handler("androidSetup", nil)
    manager:remove_all_scripts()
    session:get_website_data_manager():clear(WebKit.WebsiteDataTypes.ALL, 0, nil, function(source, result)
      pcall(source.clear_finish, source, result)
    end)
    -- Only this ephemeral session is affected, never the user's browser.
    view:load_uri("about:blank")
  end
  local function check()
    if not active or reading then return end
    reading = true
    -- The handoff may be scoped to /EmbeddedSetup or to
    -- /o/oauth2/programmatic_auth. This is only our ephemeral login session.
    cookies:get_all_cookies(nil, function(source, result)
      local ok, list = pcall(source.get_all_cookies_finish, source, result)
      reading = false
      if not active or not ok then return end
      for _, cookie in ipairs(list or {}) do
        local domain = (cookie:get_domain() or ""):gsub("^%.", "")
        if (domain == "accounts.google.com" or domain == "google.com") and
          cookie:get_name() == "oauth_token" and cookie:get_value() ~= "" and cookie:get_secure() then
          local handoff = { token = cookie:get_value(), email = email }
          close()
          done(handoff)
          return
        end
      end
    end)
  end
  manager:on("script-message-received::androidSetup", function(_, value)
    if not active or not (view.uri or ""):match("^https://accounts%.google%.com/") then return end
    local ok, message = pcall(app.json.decode, value:to_string())
    if not ok or type(message) ~= "table" then return end
    if message.event == "account" and type(message.value) == "string" and #message.value < 320 then
      email = message.value
    elseif message.event == "close" then
      status("Waiting for Google's Android sign-in credential…")
      check()
    end
  end)
  assert(manager:register_script_message_handler("androidSetup", nil))
  manager:add_script(WebKit.UserScript.new(script, "top-frame", "start", { "https://accounts.google.com/*" }, nil))
  cookies:on("changed", check)
  view:on("load-changed", function(_, event)
    if active and event == "finished" then check() end
  end)
  view:on("load-failed", function()
    if active then status("The Google sign-in page couldn't load. Retry sign-in.") end
    return false
  end)
  view:on("decide-policy", function(_, decision, kind)
    if kind == "navigation-action" or kind == "new-window-action" then
      local uri = decision:get_navigation_action():get_request():get_uri()
      if uri == "about:blank" or uri:match("^https://accounts%.google%.com/") then return false end
      decision:ignore()
      if active then status("Google requested a page outside this sign-in window. This sign-in method isn't supported yet.") end
      return true
    end
    return false
  end)
  view:on("permission-request", function(_, request) request:deny(); return true end)
  timer = app.timer(1000, check, true)
  view:load_uri(P.login_url)
  return view, close
end
return M
