-- A private HTTP session: no browser cookie jar and no credential redirects.
local Soup = gi.require("Soup", "3.0")
local GLib = gi.require("GLib", "2.0")
local Gio = gi.require("Gio", "2.0")
local M = {}
function M.new()
  local session = Soup.Session { timeout = 30, max_conns = 2 }
  return function(url, content_type, body, headers, done)
    assert(url == "https://android.clients.google.com/auth" or
      url == "https://www.youtube.com/youtubei/v1/browse?prettyPrint=false", "Unexpected credential destination")
    local cancel = Gio.Cancellable.new()
    local message = Soup.Message.new("POST", url)
    message:set_flags(Soup.MessageFlags.NO_REDIRECT)
    for key, value in pairs(headers) do message:get_request_headers():replace(key, value) end
    message:set_request_body_from_bytes(content_type, GLib.Bytes.new(body))
    session:send_and_read_async(message, 0, cancel, function(source, result)
      local ok, bytes = pcall(source.send_and_read_finish, source, result)
      if cancel:is_cancelled() then return end
      if not ok then done(nil, "The network request failed. Check your connection and retry."); return end
      if bytes:get_size() > 8 * 1024 * 1024 then done(nil, "The server response was too large."); return end
      -- get_status() is a Soup.Status enum ("ok", "unauthorized", ...)
      -- in Lua; the property exposes the numeric HTTP code.
      done({ status = message.status_code, body = bytes:get_data() })
    end)
    return function() cancel:cancel() end
  end
end
return M
