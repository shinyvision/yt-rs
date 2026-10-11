-- Looks at the Downloads page with a download listed: the sizes of the
-- window's parts, and a screenshot.
local function log(...) app.log("[dlprobe]", ...) end
local shots = os.getenv("E2E_SHOTS") or "/tmp"
local function sizes(tag)
  local parts = {}
  for _, name in ipairs({ "shell", "content", "page-host", "sidebar", "sidebar-mini", "page-downloads" }) do
    local w = app.find(name)
    if w then
      local x, y = w:size()
      table.insert(parts, name .. " " .. x .. "x" .. y .. (w:mapped() and "" or " (unmapped)"))
    else
      table.insert(parts, name .. " missing")
    end
  end
  log(tag, table.concat(parts, " | "), "| page", app.current_page())
end
app.on("app.ready", function()
  local early = tonumber(os.getenv("E2E_AT") or "1000")
  app.timer(early, function() sizes("home"); app.run("app.navigate", { page = "downloads" }) end)
  for _, t in ipairs({ 300, 1500, 4000, 8000 }) do
    app.timer(early + t, function() sizes("downloads +" .. t); app.screenshot(shots .. "/dl-" .. t .. ".png") end)
  end
  app.timer(early + 9000, function() app.quit() end)
end)
