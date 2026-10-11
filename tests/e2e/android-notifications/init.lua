-- Install next to notifications in a throwaway profile with no token.
-- Fixtures cover account exchange and inbox access. Optional live mode only
-- opens the public Google sign-in page; it never enters account details.
local P = require("notifications.protocol")
local Backend = require("notifications.android")
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
  check("DroidGuard is an explicit failure", P.auth_error({ Error = "MissingDroidguard" }, 200):find("device verification", 1, true))
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
  backend.fetch("WEB", nil, false, function(page, err) check("HTTP 200 without inbox is not success", not page and err:find("unrecognized", 1, true)) end)
  response(200, { responseContext = {} })
  backend.fetch("ANDROID", nil, false, function(page, err) check("signed-out response rejected", not page and err:find("signed-out", 1, true)) end)
  response(200, { responseContext = { mainAppWebResponseContext = { loggedOut = true } } })
  backend.restore(function(found) assert(found) end)
  backend.fetch("ANDROID", nil, false, function() error("stale callback ran") end)
  response(200, "Auth=restored-access\nExpires=3600\n")
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
  metadata.shortMessageFormatted.content = 'Replied: "A reply."'
  check("Android reply joins the name and action correctly", P.inbox(android).items[1].message == '@fixture replied: "A reply."')
  local bad_store = { write = function(_, done) done(false, "keyring locked") end }
  backend = Backend.new(http, bad_store, app.json, function() return clock end)
  backend.sign_in({ token = "temporary" }, function(ok, err) check("keyring failure is not successful login", not ok and err == "keyring locked") end)
  response(200, "Token=persistent\nEmail=fixture@example.invalid\n")

  local counts = { actions = {
    { updateBrowseTabNewContentAction = { tabIdentifier = "FEconnections_inbox", unseenItemCount = "99" } },
    { updateBrowseTabNewContentAction = { tabIdentifier = "FEnotifications_inbox", unseenItemCount = "7" } },
  } }
  check("badge selects the notifications tab count", P.count(counts) == 7)
  check("missing count is not fabricated as zero", P.count({}) == nil)
  check("count requests only home metadata", P.url("count"):find("fields=onResponseReceivedActions", 1, true) ~= nil)
  check("unknown credential destination is rejected", not pcall(P.url, "https://example.org"))
  local action = P.action({ commandExecutorCommand = { commands = {
    { elementsCommand = {} }, { recordNotificationInteractionsEndpoint = { serializedInteractionsRequest = "fixture-hide" } },
  } } })
  check("Android menu action unwraps command executor", action and action.endpoint == "notification/record_interactions" and action.body.serializedInteractionsRequest == "fixture-hide")
  check("unrecognized menu actions are ignored", P.action({ urlEndpoint = { url = "https://example.org" } }) == nil)
  check("unrecognized feed is not an empty inbox", not P.inbox({ contents = { sectionListRenderer = { contents = {} } } }).recognized)
  check("explicit empty notification feed is recognized", P.inbox({
    header = { feedTabbedHeaderRenderer = { title = { runs = { { text = "Notifications" } } } } },
    contents = { sectionListRenderer = { contents = {} } },
  }).recognized)

  saved = app.json.encode({ master = "persistent", email = "fixture@example.invalid" })
  backend = Backend.new(http, storage, app.json, function() return clock end)
  backend.restore(function(found) assert(found) end)
  local recovered
  backend.fetch("ANDROID", nil, false, function(page) recovered = page ~= nil end)
  response(200, "Auth=expired-access\nExpires=3600\n")
  response(401, {})
  check("HTTP 401 renews the access token", requests[#requests].url == P.auth_url)
  response(200, "Auth=replacement-access\nExpires=3600\n")
  response(200, inbox)
  check("request succeeds after renewal", recovered)
  local expired
  backend.fetch("ANDROID", nil, false, function(page, err, code) expired = not page and code == "sign_in_required" end)
  response(401, {})
  response(200, "Auth=still-rejected-access\nExpires=3600\n")
  response(401, {})
  check("repeated rejection returns to sign-in without a loop", expired and not backend.connected() and #pending == 0)

  local save_callback, delayed_value, forgot
  local delayed_store = {
    write = function(value, done) delayed_value, save_callback = value, done end,
    clear = function(done) delayed_value = nil; done(true) end,
  }
  backend = Backend.new(http, delayed_store, app.json, function() return clock end)
  backend.sign_in({ token = "temporary" }, function() error("cancelled sign-in completed") end)
  response(200, "Token=persistent\nEmail=fixture@example.invalid\n")
  backend.forget(function(ok) forgot = ok end)
  check("sign-out waits for an in-flight keyring save", not forgot)
  save_callback(true)
  check("late keyring save cannot restore a signed-out account", forgot and not delayed_value and not backend.connected())
end

local function adapter_tests()
  local names = { "notifications.android", "notifications.transport", "notifications.keyring" }
  local originals = {}
  for _, name in ipairs(names) do originals[name] = package.loaded[name] end
  local calls, connected = 0, true
  local fake = {
    connected = function() return connected end,
    restore = function(done) done(true) end,
    cancel = function() end,
    fetch = function() error("dry-run inbox request escaped") end,
    call = function(client, endpoint, body, force, done)
      assert(client == "ANDROID" and endpoint == "count" and body.browseId == "FEwhat_to_watch")
      calls = calls + 1
      done({ actions = { { updateBrowseTabNewContentAction = { tabIdentifier = "FEnotifications_inbox", unseenItemCount = "5" } } } })
    end,
  }
  package.loaded[names[1]] = { new = function() return fake end }
  package.loaded[names[2]] = { new = function() end }
  package.loaded[names[3]] = { new = function() end }
  local adapter = assert(loadfile(app.api.paths.config_dir() .. "/plugins/notifications/backend.lua"))()
  adapter.restore(function() end)
  for _, name in ipairs(names) do package.loaded[name] = originals[name] end
  for _, operation in ipairs({ "menu", "more", "interact" }) do
    adapter.request({ operation = operation }, function(result)
      check("dry-run blocks " .. operation, result.status == "dry_run" and calls == 0)
    end)
  end
  adapter.request({ operation = "count" }, function(result)
    check("dry-run permits only the native count metadata request", result.status == "ok" and result.count == 5 and calls == 1)
  end)
  connected = false
  adapter.request({ operation = "count" }, function(result)
    check("disconnected bell requests sign-in without HTTP", result.status == "sign_in_required" and calls == 1)
  end)
end

local function handoff_test(done)
  local Login = require("notifications.login")
  local Soup = gi.require("Soup", "3.0")
  local original_url = P.login_url
  P.login_url = "about:blank" -- Fixture credentials never reach the network.
  local view, close
  view, close = Login.new(app.api.paths.config_dir() .. "/plugins/notifications", function(handoff)
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
  adapter_tests()
  app.timer(1000, function()
    check("notification bell loads", app.find("notifications-button") ~= nil)
    local keyring = require("notifications.keyring").new(app.api.paths.config_dir() .. "/fixture")
    keyring.write("fixture-secret", function(ok, err)
      assert(ok, err)
      keyring.read(function(value, err)
        check("real system keyring round trip", value == "fixture-secret" and not err)
        keyring.clear(function(ok, err)
          assert(ok, err)
          keyring.read(function(value, err)
            check("real system keyring removal", value == nil and not err)
            if os.getenv("E2E_LIVE_LOGIN") == "1" then
              local http = require("notifications.transport").new()
              local transported = false
              check("transport rejects other credential destinations", not pcall(http, "https://example.com/auth", "text/plain", "fixture", {}, function() end))
              http("https://www.youtube.com/youtubei/v1/browse?prettyPrint=false", "application/json",
                app.json.encode({ context = P.context("ANDROID"), browseId = "FEnotifications_inbox" }), {}, function(response, err)
                  assert(response, err)
                  check("transport returns a numeric HTTP status", type(response.status) == "number" and response.status >= 100 and response.status < 600)
                  check("anonymous HTTP response crosses GI intact", type(response.body) == "string" and type(app.json.decode(response.body)) == "table")
                  transported = true
                end)
              app.run("notifications.sign_in")
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
