-- Drive the real TV sign-in UI with dummy OAuth results and fixture feeds.
-- No account credential is read/written and no authenticated HTTP is sent.
local signed_in, populated, calls = false, false, 0
local passed, failed = 0, 0
app.api.mpv.set_property("ao", "null")
app.run("watch.set_tracking", { enabled = false })
app.hook("session.is_signed_in", function() return signed_in end)
app.hook("session.sign_in", function() signed_in = true end)
app.hook("session.sign_out", function() signed_in = false end)
app.hook("oauth.request_device_code", function()
  return { device_code = "fixture-device", user_code = "TEST-CODE", verification_url = "https://example.invalid", expires_in = 600, interval = 1 }
end)
app.hook("oauth.wait_for_approval", function()
  return { refresh_token = "fixture-refresh", access_token = "fixture-access", expires_at = 9999999999 }
end)
app.hook("youtube.account", function() return { name = "Fixture account" } end)
app.hook("youtube.subscriptions", function() return app.array{} end)
app.hook("youtube.browse", function()
  calls = calls + 1
  return { shelves = app.array{} }
end)
app.filter("feed.page", function(event)
  event.page.shelves = populated and signed_in and app.array{
    { title = "Fixture recommendations", videos = app.array{ { id = "abcdefghijk", title = "Account-only fixture" } } }
  } or app.array{}
  event.page.continuation = nil
  return event
end)
local function check(name, condition)
  if condition then passed = passed + 1 else failed = failed + 1 end
  app.log("[auth-ui]", condition and "PASS" or "FAIL", name)
end
local function home() return app.find("feed-content") end
local function sign_in_button() return home():find(".signin-button") end
local function login()
  app.navigate("signin")
  app.find("page-signin"):find(".suggested-action"):emit("clicked")
end
local checkpoint
local steps = {
  { 800, function() app.run("feed.refresh") end },
  { 600, function()
    check("signed-out home offers sign-in", home().visible_child_name == "status" and sign_in_button():get_mapped())
    checkpoint = calls
    login()
  end },
  { 600, function()
    check("TV sign-in returns to home with signed-in state", app.api.app.current_page() == "home" and app.api.social.signed_in())
    check("cached home is reloaded after sign-in", calls > checkpoint)
    check("signed-in empty home has no sign-in prompt", home().visible_child_name == "status" and not sign_in_button():get_mapped())
    app.screenshot((os.getenv("E2E_SHOTS") or "/tmp") .. "/signed-in-empty-home.png")
    populated = true
    app.run("feed.refresh")
  end },
  { 600, function()
    check("signed-in recommendations load", home().visible_child_name == "grid")
    app.navigate("history")
  end },
  { 500, function()
    app.run("auth.sign_out")
  end },
  { 500, function()
    check("sign-out updates shared account state", not app.api.social.signed_in())
    checkpoint = calls
    app.navigate("home")
  end },
  { 600, function()
    check("hidden cached home reloads after sign-out", calls > checkpoint)
    check("previous account recommendations are removed", home().visible_child_name == "status" and sign_in_button():get_mapped())
    populated = false
    login()
  end },
  { 600, function()
    check("sign-in works again without restarting", signed_in and not sign_in_button():get_mapped())
    app.log("[auth-ui]", string.format("SUMMARY %d passed, %d failed", passed, failed))
    app.quit()
  end },
}
local function run(index)
  local step = steps[index]
  if step then app.timer(step[1], function()
    local ok, err = pcall(step[2])
    if not ok then check("step " .. index .. ": " .. tostring(err), false) end
    run(index + 1)
  end) end
end
app.on("app.ready", function() run(1) end)
