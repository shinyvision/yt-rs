-- UI fixtures: no browser, subprocess, keyring or authenticated network.
local M = { mode = "missing", connected = false, requests = {}, closed = 0, restores = 0, forgets = 0 }
local Gtk = gi.require("Gtk", "4.0")
package.loaded["notifications.login"] = { new = function(_, done)
  M.complete = function() done({ token = "fixture-handoff" }) end
  return Gtk.Box { height_request = 300, name = "fixture-google-login" }, function() M.closed = M.closed + 1 end
end }
local backend = require("notifications.backend")
backend.restore = function(done)
  M.restores = M.restores + 1
  app.timer(20, function() done(M.connected) end)
end
backend.connected = function() return M.connected end
backend.account = function() return M.connected and "fixture@example.invalid" end
backend.sign_in = function(_, done)
  app.timer(20, function() M.connected = true; M.mode = "ok"; done(true) end)
end
backend.forget = function(done)
  M.connected = false
  M.forgets = M.forgets + 1
  app.timer(20, function() done(not M.locked, M.locked and "Couldn't remove the account from the system keyring." or nil) end)
end
backend.cancel = function() end
local function rows(mode, operation)
  local action = { endpoint = "notification/record_interactions", body = { serializedInteractionsRequest = "fixture" } }
  local function upload(id)
    return { id = id, message = 'You got 2 likes: "All fun and games until you get arrested with "one honest caveat" for doing a mission called "two things worth mentioning"."',
      sent = "10 hours ago", read = false, avatar = "fixture://avatar", thumbnail = "fixture://thumbnail",
      destination = { link = { kind = "video", video_id = "abcdefghijk" } }, record_click = action,
      menu = { { label = "Hide this notification", action = action } } }
  end
  local items = { upload("upload"), { id = "reply", message = "Someone replied to your comment: Thanks for watching!", sent = "3 hours ago",
    read = true, avatar = "fixture://avatar", thumbnail = "fixture://thumbnail", record_click = action,
    destination = { url = "https://www.youtube.com/watch?v=abcdefghijk&lc=comment" } } }
  if mode == "geometry" and operation == "menu" then
    for n = 1, 6 do items[#items + 1] = upload("geometry-" .. n) end
  end
  if operation == "more" then
    items = { upload("upload"), { id = "post", message = "A channel shared a community post", sent = "Yesterday", read = false,
      destination = { url = "https://www.youtube.com/post/example" } } }
  end
  return items
end
backend.request = function(request, done, progress)
  local mode, operation = M.mode, request.operation
  M.requests[#M.requests + 1] = operation
  app.timer(100, function() progress({ message = "Fetching notifications…" }) end)
  local timer = app.timer(mode == "slow" and 1000 or 200, function()
    if mode == "missing" then done({ status = "sign_in_required", message = "Sign in once to see your YouTube notifications here." })
    elseif mode == "expired" then M.connected = false; done({ status = "sign_in_required", message = "Your YouTube sign-in has expired. Sign in again." })
    elseif mode == "error" then done({ status = "network_error", message = "Couldn't connect to YouTube. Try again." })
    elseif operation == "count" then done({ status = "ok", count = 12 })
    elseif operation == "interact" then done({ status = "ok" })
    else done({ status = "ok", items = mode == "empty" and {} or rows(mode, operation),
      continuation = operation == "menu" and mode ~= "empty" and { endpoint = "browse", body = { continuation = "next" } } or nil }) end
  end)
  return function() if mode ~= "slow" then timer:cancel() end end
end
return M
