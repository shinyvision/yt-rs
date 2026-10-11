-- Notification operations over the plugin's Android account session.
local P = require("notifications.protocol")
local M = {}
local service
local function session()
  if service then return service end
  local ok, result = pcall(function()
    return require("notifications.android").new(require("notifications.transport").new(),
      require("notifications.keyring").new(app.api.paths.config_dir()), app.json, os.time)
  end)
  if not ok then return nil, "Notifications need libsecret and libsoup 3 installed." end
  service = result
  return service
end
function M.restore(done)
  local account, err = session()
  if not account then done(false, err, "dependency_missing"); return end
  account.restore(done)
end
function M.connected() return service and service.connected() or false end
function M.account() return service and service.account() end
function M.cancel() if service then service.cancel() end end
function M.sign_in(handoff, done)
  local account, err = session()
  if not account then done(false, err, "dependency_missing"); return end
  account.sign_in(handoff, done)
end
function M.forget(done)
  local account, err = session()
  if not account then done(false, err); return end
  account.forget(done)
end
function M.request(request, done, progress)
  local account, err = session()
  if not account then done({ status = "dependency_missing", message = err }); return function() end end
  local operation = request.operation
  assert(operation == "count" or operation == "menu" or operation == "more" or operation == "interact", "Invalid notification operation")
  if not account.connected() then done({ status = "sign_in_required", message = "Sign in once to see your YouTube notifications here." }); return function() end end
  if os.getenv("YT_RS_DRY_RUN") == "1" and operation ~= "count" then
    done({ status = "dry_run", message = "Notification inbox and account changes are disabled in dry-run mode." }); return function() end
  end
  local function finish(data, failure, code)
    done(data or { status = code or "service_error", message = failure })
  end
  if progress then progress({ message = operation == "count" and "Checking notifications…" or "Fetching notifications…" }) end
  if operation == "menu" or operation == "more" then
    local token
    if operation == "more" then
      assert(type(request.action) == "table" and request.action.endpoint == "browse" and
        type((request.action.body or {}).continuation) == "string", "Invalid notification continuation")
      token = request.action.body.continuation
    end
    account.fetch("ANDROID", token, false, function(page, failure, code)
      if not page then finish(nil, failure, code); return end
      finish({ status = "ok", items = page.items,
        continuation = page.continuation and { endpoint = "browse", body = { continuation = page.continuation } } })
    end)
  elseif operation == "count" then
    account.call("ANDROID", "count", { browseId = "FEwhat_to_watch" }, false, function(data, failure, code)
      if not data then finish(nil, failure, code); return end
      local count = P.count(data)
      if not count then finish(nil, "YouTube didn't return a notification count. Try again."); return end
      finish({ status = "ok", count = math.max(0, math.floor(count)) })
    end)
  else
    local action = request.action or {}
    assert(action.endpoint == "notification/record_interactions" or action.endpoint == "notification/modify_channel_preference", "Invalid notification action")
    account.call("ANDROID", action.endpoint, action.body, false, function(data, failure, code)
      if data then finish({ status = "ok" }) else finish(nil, failure, code) end
    end)
  end
  return account.cancel
end
return M
