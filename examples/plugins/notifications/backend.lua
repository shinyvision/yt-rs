-- Browser access is a child process: keyring/database work and HTTP never
-- block GTK. The only process output is the helper's sanitized JSON result.
local Gio = gi.require("Gio", "2.0")
local M = {}

function M.request(dir, settings, request, done)
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
  process:communicate_utf8_async(app.json.encode(request), nil, function(p, result)
    if settled then return end
    local success, communicated, stdout = pcall(p.communicate_utf8_finish, p, result)
    local decoded, response = false, nil
    if success and communicated and p:get_successful() then
      decoded, response = pcall(app.json.decode, stdout or "")
    end
    if not decoded or type(response) ~= "table" or type(response.status) ~= "string" then
      response = { status = "service_error", message = "Couldn't load notifications. Try again." }
    end
    finish(response)
  end)
  return function()
    if settled then return end
    settled = true
    timer:cancel()
    process:force_exit()
  end
end

return M
