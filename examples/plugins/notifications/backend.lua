-- Browser access is a child process: keyring/database work and HTTP never
-- block GTK. Only sanitized JSON progress and results cross this boundary.
local Gio = gi.require("Gio", "2.0")
local GLib = gi.require("GLib", "2.0")
local M = {}

function M.request(dir, settings, request, done, progress)
  local browser = Gio.AppInfo.get_default_for_uri_scheme("https")
  if browser then
    request.desktop = (browser:get_id() or "") .. " " .. (browser:get_executable() or "")
  end
  request.profile = settings.profile
  local python = settings.python
  if not python then
    for _, relative in ipairs({ "/.venv/bin/python", "/.venv/Scripts/python.exe" }) do
      local path = dir .. relative
      if Gio.File.new_for_path(path):query_exists(nil) then python = path; break end
    end
  end
  python = python or (package.config:sub(1, 1) == "\\" and "python" or "python3")
  local flags = Gio.SubprocessFlags
  local ok, process = pcall(Gio.Subprocess.new,
    { python, "-I", dir .. "/browser.py" },
    flags.STDIN_PIPE + flags.STDOUT_PIPE + flags.STDERR_SILENCE)
  if not ok then
    done({ status = "dependency_missing", message = "Install Python and yt-dlp to use browser notifications." })
    return function() end
  end
  local settled, timer = false, nil
  local function finish(result)
    if settled then return end
    settled = true
    if timer then timer:cancel() end
    done(result)
  end
  timer = app.timer(55000, function()
    timer = nil
    process:force_exit()
    finish({ status = "timeout", message = "Your browser or YouTube took too long to respond. Try again." })
  end)
  local function failed()
    if settled then return end
    process:force_exit()
    finish({ status = "service_error", message = "Couldn't load notifications. Try again." })
  end
  local stdout = Gio.DataInputStream.new(process:get_stdout_pipe())
  local response
  local function read_line()
    stdout:read_line_async(0, nil, function(stream, result)
      local ok, line = pcall(stream.read_line_finish_utf8, stream, result)
      if settled then return end
      if not ok then failed(); return end
      if line == nil then
        process:wait_check_async(nil, function(p, result)
          local ok, success = pcall(p.wait_check_finish, p, result)
          if settled then return end
          if ok and success and response then finish(response) else failed() end
        end)
        return
      end
      local decoded, event = pcall(app.json.decode, line)
      if not decoded or type(event) ~= "table" or type(event.status) ~= "string" then failed(); return end
      if event.status == "progress" then
        if progress then progress(event) end
      else
        response = event
      end
      read_line()
    end)
  end
  read_line()
  local stdin, payload, offset = process:get_stdin_pipe(), app.json.encode(request), 1
  local function write()
    -- Gio owns the GBytes during this async call; a temporary byte buffer
    -- passed to write_all_async would have to outlive the GI invocation.
    stdin:write_bytes_async(GLib.Bytes.new(payload:sub(offset)), 0, nil, function(stream, result)
      local ok, count = pcall(stream.write_bytes_finish, stream, result)
      if settled then return end
      if not ok or count == 0 then failed(); return end
      offset = offset + count
      if offset <= #payload then write(); return end
      stream:close_async(0, nil, function(stream, result)
        local ok = pcall(stream.close_finish, stream, result)
        if not ok then failed() end
      end)
    end)
  end
  write()
  return function()
    if settled then return end
    settled = true
    timer:cancel()
    process:force_exit()
  end
end

return M
