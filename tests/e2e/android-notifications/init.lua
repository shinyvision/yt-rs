-- Install next to android-notifications in a throwaway profile with no token.
-- Fixtures cover account exchange and inbox access. Optional live mode only
-- opens the public Google sign-in page; it never enters account details.
local P = require("android-notifications.protocol")
local Backend = require("android-notifications.backend")
local passed = 0
local function check(label, condition)
  assert(condition, label)
  passed = passed + 1
  app.log("[android-notifications] PASS", label)
end
local function fixture_tests()
  check("form values cannot inject fields", P.form({ Token = "a+b&c=\n%", Email = "a@b" }) == "Email=a%40b&Token=a%2Bb%26c%3D%0A%25")
  check("response parser preserves equals and final line", P.fields("Auth=a=b==\r\nEmail=a@b").Auth == "a=b==")
  check("auth errors do not expose response contents", P.auth_error({ Error = "secret-value", Url = "secret" }, 403) == "Google rejected the Android authentication request.")
  check("DroidGuard is an explicit failure", P.auth_error({ Error = "MissingDroidguard" }, 200):find("DroidGuard", 1, true))
  local clock, saved, requests, pending = 1700000000, nil, {}, {}
  local storage = {
    read = function(done) done(saved) end,
    write = function(value, done) saved = value; done(true) end,
    clear = function(done) saved = nil; done(true) end,
  }
  local function http(url, kind, body, headers, done)
    requests[#requests + 1] = { url = url, body = body, headers = headers, kind = kind }
    pending[#pending + 1] = done
    return function() end -- Deliberately deliver stale callbacks after cancel.
  end
  local function response(status, body)
    table.remove(pending, 1)({ status = status, body = type(body) == "table" and app.json.encode(body) or body })
  end
  local inbox = { contents = { notificationRenderer = { notificationId = "one", shortMessage = { runs = {
    { text = 'You got 2 likes: "All fun and games ' }, { text = 'until you get arrested."' },
  } }, navigationEndpoint = { getCommentsFromInboxCommand = { videoId = "dQw4w9WgXcQ" } } } } }
  local backend = Backend.new(http, storage, app.json, function() return clock end)
  backend.restore(function(found, err) check("fresh profile is signed out", not found and not err) end)
  backend.sign_in({ token = "temporary-handoff" }, function(ok, err) check("credential exchange succeeds", ok and not err) end)
  check("exchange is restricted to Android auth endpoint", requests[1].url == P.auth_url and requests[1].body:find("ACCESS_TOKEN=1", 1, true))
  response(200, "Token=persistent-master\nEmail=fixture@example.invalid\n")
  check("only persistent account credential enters keyring", saved and not saved:find("temporary-handoff", 1, true))
  local result
  backend.fetch("ANDROID", nil, false, function(page, err) assert(not err); result = page end)
  check("access token requests YouTube scope", requests[2].body:find("service=oauth2%%3Ahttps%%3A%%2F%%2Fwww.googleapis.com%%2Fauth%%2Fyoutube"))
  response(200, "Auth=short-lived-access\nExpires=3600\n")
  check("inbox uses Android access token", requests[3].headers.Authorization == "Bearer short-lived-access")
  check("inbox uses full feed", app.json.decode(requests[3].body).browseId == "FEnotifications_inbox")
  response(200, inbox)
  check("exact notification text and comment destination", result.items[1].message == 'You got 2 likes: "All fun and games until you get arrested."' and result.items[1].video_id == "dQw4w9WgXcQ")
  backend.fetch("ANDROID", "next-page", false, function(page) assert(page) end)
  check("unexpired access token is reused", #requests == 4 and app.json.decode(requests[4].body).continuation == "next-page")
  response(200, inbox)
  clock = clock + 3400
  backend.fetch("ANDROID", nil, false, function(page) assert(page) end)
  check("expiring token is renewed", requests[5].url == P.auth_url)
  response(200, "Auth=renewed-access\nExpires=3600\n")
  response(200, inbox)
  backend.fetch("ANDROID", nil, true, function(page) assert(page) end)
  check("forced renewal ignores cached token", requests[7].url == P.auth_url)
  response(200, "Auth=forced-access\nExpires=3600\n")
  response(200, inbox)
  backend = Backend.new(http, storage, app.json, function() return clock end)
  backend.restore(function(found, err) check("restart restores persistent credential", found and not err) end)
  backend.fetch("ANDROID", nil, false, function(page) assert(page) end)
  check("restart obtains fresh access without login window", requests[9].url == P.auth_url)
  response(200, "Auth=after-restart\nExpires=3600\n")
  response(200, inbox)
  backend.fetch("WEB", nil, false, function(page, err) check("HTTP 200 without inbox is not success", not page and err:find("not yet verified", 1, true)) end)
  response(200, { responseContext = {} })
  backend.fetch("ANDROID", nil, false, function(page, err) check("signed-out response rejected", not page and err:find("signed-out", 1, true)) end)
  response(200, { responseContext = { mainAppWebResponseContext = { loggedOut = true } } })
  backend.fetch("ANDROID", nil, false, function() error("stale callback ran") end)
  backend.forget(function(ok) check("forget removes keyring entry", ok and saved == nil) end)
  response(200, inbox)
  check("cancelled request cannot restore signed-out UI", true)
  backend.fetch("ANDROID", nil, false, function(page, err) check("forgotten account cannot fetch", not page and err ~= nil) end)
  local ordered = { contents = {} }
  for n = 1, 40 do ordered.contents[n] = { notificationRenderer = { notificationId = tostring(n), shortMessage = { simpleText = tostring(n) } } } end
  local parsed = P.inbox(ordered)
  for n = 1, 40 do assert(parsed.items[n].id == tostring(n), "notification order") end
  check("all notification rows retain server order", true)
  local android = { contents = {
    { elementRenderer = { newElement = { type = { componentType = { model = { shelfHeaderModel = { title = "Today" } } } } } } },
    { elementRenderer = { newElement = { type = { componentType = { model = { inboxNotificationModel = { inboxNotification = {
      performOnceIdentifier = "android-one", metadata = { header = { content = "You got 2 likes" },
        shortMessageFormatted = { content = '"An exact quoted comment."' }, sentTimeText = "10 hours ago" },
      onTap = { innertubeCommand = { browseEndpoint = { browseId = "FEcommentHistory" } } },
      inboxNotificationThumbnail = { thumbnail = { image = { sources = { { url = "https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg" } } } } },
    } } } } } } } },
  }, continuations = { { nextContinuationData = { continuation = "android-next" } } } }
  local android_page = P.inbox(android)
  check("Android element model preserves complete notification text", #android_page.items == 1 and android_page.items[1].message == 'You got 2 likes: "An exact quoted comment."')
  check("Android comment notification opens its video", android_page.items[1].video_id == "dQw4w9WgXcQ" and android_page.items[1].sent == "10 hours ago")
  check("Android continuation is retained", android_page.continuation == "android-next")
  local metadata = P.nodes(android, "inboxNotificationModel")[1].inboxNotification.metadata
  metadata.header.content = "@fixture"
  metadata.shortMessageFormatted.content = 'replied: "A reply."'
  check("Android reply joins the name and action correctly", P.inbox(android).items[1].message == '@fixture replied: "A reply."')
  local bad_store = { write = function(_, done) done(false, "keyring locked") end }
  backend = Backend.new(http, bad_store, app.json, function() return clock end)
  backend.sign_in({ token = "temporary" }, function(ok, err) check("keyring failure is not successful login", not ok and err == "keyring locked") end)
  response(200, "Token=persistent\nEmail=fixture@example.invalid\n")
end

local function handoff_test(done)
  local Login = require("android-notifications.login")
  local Soup = gi.require("Soup", "3.0")
  local original_url = P.login_url
  P.login_url = "about:blank" -- Fixture credentials never reach the network.
  local view, close
  view, close = Login.new(app.api.paths.config_dir() .. "/plugins/android-notifications", function(handoff)
    check("path-scoped Android handoff is received", handoff.token == "fixture-handoff")
    close() -- Closing twice must be harmless.
    app.timer(300, function()
      local cookies = view:get_network_session():get_cookie_manager()
      cookies:get_all_cookies(nil, function(source, result)
        check("handoff clears the ephemeral cookie store", #source:get_all_cookies_finish(result) == 0)
        done()
      end)
    end)
  end, function() end)
  P.login_url = original_url
  local cookie = Soup.Cookie.new("oauth_token", "fixture-handoff", "accounts.google.com", "/o/oauth2/programmatic_auth", -1)
  cookie:set_secure(true)
  view:get_network_session():get_cookie_manager():add_cookie(cookie, nil, function(source, result)
    assert(source:add_cookie_finish(result))
  end)
end

app.api.mpv.set_property("ao", "null")
app.api.mpv.set_property("mute", "yes")
app.run("watch.set_tracking", { enabled = false })
app.on("app.ready", function()
  fixture_tests()
  app.timer(200, function() app.navigate("android-notifications") end)
  app.timer(1000, function()
    check("prototype page loads", app.find("android-notifications-status") ~= nil)
    local keyring = require("android-notifications.keyring").new(app.api.paths.config_dir() .. "/fixture")
    keyring.write("fixture-secret", function(ok, err)
      assert(ok, err)
      keyring.read(function(value, err)
        check("real system keyring round trip", value == "fixture-secret" and not err)
        keyring.clear(function(ok, err)
          assert(ok, err)
          keyring.read(function(value, err)
            check("real system keyring removal", value == nil and not err)
            if os.getenv("E2E_LIVE_LOGIN") == "1" then
              local http = require("android-notifications.transport").new()
              local transported = false
              check("transport rejects other credential destinations", not pcall(http, "https://example.com/auth", "text/plain", "fixture", {}, function() end))
              http("https://www.youtube.com/youtubei/v1/browse?prettyPrint=false", "application/json",
                app.json.encode({ context = P.context("ANDROID"), browseId = "FEnotifications_inbox" }), {}, function(response, err)
                  assert(response, err)
                  check("transport returns a numeric HTTP status", type(response.status) == "number" and response.status >= 100 and response.status < 600)
                  check("anonymous HTTP response crosses GI intact", type(response.body) == "string" and type(app.json.decode(response.body)) == "table")
                  transported = true
                end)
              app.run("android-notifications.sign_in")
              app.timer(12000, function()
                local view = app.find("android-google-login")
                check("login uses an ephemeral session", view and view:get_network_session().is_ephemeral)
                view:evaluate_javascript("JSON.stringify({email:!!document.querySelector('input[type=email],input[autocomplete=username],#identifierId'),password:!!document.querySelector('input[type=password]'),bridge:!!window.mm,unsupported:['getDroidGuardResult','addAccount','getDeviceDataVersionInfo','setKeyData'].some(k=>k in window.mm)})", -1, nil, nil, nil, function(web, result)
                  local value = web:evaluate_javascript_finish(result)
                  local data = app.json.decode(value:to_string())
                  check("Google sign-in form and Android bridge load", data.email and data.bridge and not data.password)
                  check("unavailable native capabilities are not advertised", not data.unsupported)
                  check("anonymous transport completed", transported)
                  app.screenshot((os.getenv("E2E_SHOTS") or "/tmp") .. "/android-notifications-login.png")
                  app.timer(700, function()
                    handoff_test(function() app.log("[android-notifications]", passed, "checks passed"); app.quit() end)
                  end)
                end)
              end)
            else
              app.log("[android-notifications]", passed, "checks passed")
              app.quit()
            end
          end)
        end)
      end)
    end)
  end)
end)
