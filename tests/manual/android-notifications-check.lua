-- Run as init.lua in a separate test profile containing the notifications modules (without init.lua).
-- ANDROID_PROBE_PROFILE selects the existing prototype's keyring entry.
-- Never prints credentials, notification text, URLs, or raw server responses.
local profile = assert(os.getenv("ANDROID_PROBE_PROFILE"), "Set ANDROID_PROBE_PROFILE")
assert(os.getenv("YT_RS_DRY_RUN") ~= "1", "Live inbox checks are disabled in dry-run mode")
local transport = require("notifications.transport").new()
local keyring = require("notifications.keyring").new(profile)
local P = require("notifications.protocol")
local function report(...)
  app.log("[android-account-check]", ...)
end
local function http(url, kind, body, headers, done)
  return transport(url, kind, body, headers, function(response, err)
    if response then
      local stage = url == P.auth_url and "access token" or "inbox"
      report(stage, "HTTP", response.status)
      if stage == "inbox" then
        local ok, data = pcall(app.json.decode, response.body)
        if ok and type(data) == "table" then
          local shapes = {}
          local function walk(value)
            if type(value) ~= "table" then return end
            for key, child in pairs(value) do
              if type(key) == "string" and (key:match("^[%a]+Renderer$") or key:match("^[%a]+ViewModel$")) then shapes[key] = true end
              walk(child)
            end
          end
          walk(data)
          local names = {}
          for shape in pairs(shapes) do names[#names + 1] = shape end
          table.sort(names)
          report("response shapes:", table.concat(names, ", "))
          local code = (data.error or {}).status
          if type(code) == "string" and code:match("^[A-Z_]+$") then report("server status:", code) end
        end
      end
    end
    done(response, err)
  end)
end
local backend = require("notifications.android").new(http, keyring, app.json, os.time)
app.on("app.ready", function()
  app.api.mpv.set_property("ao", "null")
  app.api.mpv.set_property("mute", "yes")
  app.run("watch.set_tracking", { enabled = false })
  local jobs = { { "ANDROID", true, "after restart" }, { "ANDROID", true, "forced renewal" },
    { "WEB", true, "WEB compatibility" } }
  local function next_job()
    local job = table.remove(jobs, 1)
    if not job then app.quit(); return end
    report("checking", job[1], job[3], "with a freshly issued token")
    backend.fetch(job[1], nil, job[2], function(page, err)
      if page then
        report(job[1], "notifications:", #page.items, "has another page:", page.continuation ~= nil)
        local videos = 0
        for _, item in ipairs(page.items) do if item.video_id then videos = videos + 1 end end
        report(job[1], "in-app video destinations:", videos)
        if page.continuation then
          backend.fetch(job[1], page.continuation, false, function(more, failure)
            report(job[1], more and ("next page notifications: " .. #more.items) or failure)
            next_job()
          end)
          return
        end
      else report(job[1], err) end
      next_job()
    end)
  end
  backend.restore(function(found, err)
    if not found then report(err or "No saved Android credential yet."); app.quit(); return end
    report("restored account credential from keyring")
    next_job()
  end)
end)
