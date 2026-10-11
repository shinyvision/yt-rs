-- The plugins that come with the app: on by default, off with their switch
-- in Settings from the next start, and replaced by a plugin of the same name
-- in the plugins folder. Three runs on one profile, in this order:
-- E2E_PHASE=on (a fresh profile: both load, turns SponsorBlock off),
-- E2E_PHASE=off (SponsorBlock doesn't load; turns it back on), and
-- E2E_PHASE=replace (with a copy of plugins/sponsorblock in the plugins
-- folder). Signed out works.
local phase = os.getenv("E2E_PHASE") or "on"
local function log(...) app.log("[bundled-plugins]", ...) end
local results = { pass = 0, fail = 0 }
local function check(label, ok, detail)
  if ok then results.pass = results.pass + 1 else results.fail = results.fail + 1 end
  log(ok and "PASS" or "FAIL", label, detail and ("| " .. tostring(detail)) or "")
end
local function has(list, item)
  for _, x in ipairs(list) do if x == item then return true end end
  return false
end

-- Where `require` looks: the plugins folder, then the bundled plugins.
local user_dir = app.api.paths.plugins_dir()
local bundled_dir = package.path:match("(" .. app.api.paths.cache_dir():gsub("%p", "%%%0") .. "/plugins/[^/;]+)/%?%.lua")

local function switch_row(name)
  local function walk(w)
    if w:is_a("AdwSwitchRow") and w:get_title() == name then return w end
    for _, c in ipairs(w:children()) do
      local found = walk(c)
      if found then return found end
    end
  end
  local page = app.find("page-settings")
  return page and walk(page)
end
local function subtitle(name)
  local row = switch_row(name)
  return row and row:get_subtitle()
end
local function disabled() return app.api.settings.get().disabled_plugins or {} end
local function file_says(path, text)
  local f = io.open(path)
  if not f then return false end
  local contents = f:read("*a")
  f:close()
  return contents:find(text, 1, true) ~= nil
end

local steps = {}
local function step(delay, f) steps[#steps + 1] = { delay, f } end

step(1500, function()
  local loaded = app.run("app.plugins")
  check("the bundled plugins have a directory", bundled_dir ~= nil, bundled_dir or package.path)
  check("written out in full", bundled_dir and file_says(bundled_dir .. "/sponsorblock/init.lua", "SponsorBlock")
    and file_says(bundled_dir .. "/notifications/init.lua", "WebKit"))
  check("the plugins folder comes first for require",
    package.path:find(user_dir, 1, true) < (package.path:find(bundled_dir or "", 1, true) or 0))
  check("notifications loads", has(loaded, "notifications"), table.concat(loaded, ", "))
  check("its bell is in the top bar", app.find("notifications-button") ~= nil)
  if phase == "off" then
    check("SponsorBlock, turned off, doesn't load", not has(loaded, "sponsorblock"), table.concat(loaded, ", "))
    check("its commands aren't there", not has(app.commands(), "sponsorblock.segments"))
  else
    check("SponsorBlock loads", has(loaded, "sponsorblock"), table.concat(loaded, ", "))
    check("with its commands", has(app.commands(), "sponsorblock.segments"))
  end
  app.run("app.navigate", { page = "settings" })
end)

step(1500, function()
  check("Settings has a switch for each", switch_row("sponsorblock") ~= nil and switch_row("notifications") ~= nil)
  check("and for a plugin in the plugins folder", subtitle("bundled-plugins") == "From your plugins folder",
    subtitle("bundled-plugins"))
  check("notifications comes with yt-rs", subtitle("notifications") == "Comes with yt-rs", subtitle("notifications"))
  local shots = os.getenv("E2E_SHOTS")
  if shots then app.screenshot(shots .. "/settings-" .. phase .. ".png") end
  local row = switch_row("sponsorblock")
  if phase == "replace" then
    check("a copy in the plugins folder replaces SponsorBlock",
      subtitle("sponsorblock") == "Your copy, in place of the one that comes with yt-rs", subtitle("sponsorblock"))
    check("and runs", has(app.run("app.plugins"), "sponsorblock"))
    return
  end
  check("SponsorBlock comes with yt-rs", subtitle("sponsorblock") == "Comes with yt-rs", subtitle("sponsorblock"))
  if phase == "on" then
    check("its switch is on", row:get_active())
    row:set_active(false)
  else
    check("its switch is off", not row:get_active())
    row:set_active(true)
  end
end)

step(1500, function()
  local settings = app.api.paths.config_dir() .. "/settings.json"
  if phase == "on" then
    check("switching it off remembers it", has(disabled(), "sponsorblock"), table.concat(disabled(), ", "))
    check("in settings.json", file_says(settings, '"sponsorblock"'))
    check("while it keeps running until the next start", has(app.run("app.plugins"), "sponsorblock"))
  elseif phase == "off" then
    check("switching it on again forgets it", not has(disabled(), "sponsorblock"), table.concat(disabled(), ", "))
    check("in settings.json", not file_says(settings, '"sponsorblock"'))
  end
  log(string.format("SUMMARY %d passed, %d failed", results.pass, results.fail))
  app.quit()
end)

app.on("app.ready", function()
  local i = 0
  local function next_step()
    i = i + 1
    local s = steps[i]
    if not s then return end
    app.timer(s[1], function()
      local ok, err = pcall(s[2])
      if not ok then log("STEP FAILED", i, err) end
      next_step()
    end)
  end
  next_step()
end)
