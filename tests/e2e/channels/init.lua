-- All subscriptions (youtube.com's feed/channels), signed in, in a dry run
-- (YT_RS_DRY_RUN=1: unsubscribing sends nothing): the sidebar heading opens
-- it, every subscription is a row, A to Z sorts them, unsubscribing keeps
-- the row with "Subscribe", and a row opens its channel. See ../README.md.
local shots = os.getenv("E2E_SHOTS") or "/tmp"
local function log(...) app.log("[channels]", ...) end
app.player.set("ao", "null"); app.player.set("mute", "yes")
app.run("watch.set_tracking", { enabled = false })
local n = 0
local function shot(name) n = n + 1; app.screenshot(string.format("%s/%02d-%s.png", shots, n, name)) end
local results = { pass = 0, fail = 0 }
local function check(label, ok, detail)
  if ok then results.pass = results.pass + 1 else results.fail = results.fail + 1 end
  log(ok and "PASS" or "FAIL", label, detail and ("| " .. tostring(detail)) or "")
end
local function rows()
  local page = app.find("page-channels"); if not page then return {} end
  local list = {}
  local function walk(w)
    if w.name:sub(1, 9) == "channels:" then table.insert(list, w) end
    for _, c in ipairs(w:children()) do walk(c) end
  end
  walk(page)
  return list
end
local function names()
  local out = {}
  for _, row in ipairs(rows()) do
    local label = row:find(".channels-name")
    table.insert(out, label and label.label or "?")
  end
  return out
end
local subscriptions = 0
local steps = {
  { 4000, function()
      subscriptions = #app.run("channels.subscriptions")
      local heading = app.find("nav:channels")
      check("the sidebar's Subscriptions heading opens the page", heading ~= nil)
      if heading then heading:emit("clicked") end
    end },
  { 1500, function()
      check("on the page", app.current_page() == "channels", app.current_page())
      check("a row for every subscription", #rows() == subscriptions and subscriptions > 0, #rows() .. " of " .. subscriptions)
      _G.first = names()
      shot("relevant")
    end },
  -- A screenshot shows the next frame: what is shot waits a step.
  { 500, function()
      local sort = app.find("page-channels"):find(".channels-sort")
      if sort then sort.selected = 1 end
    end },
  { 800, function()
      local sorted = names()
      local ok = #sorted > 1
      for i = 2, #sorted do if sorted[i - 1]:lower() > sorted[i]:lower() then ok = false end end
      check("A to Z sorts them", ok, table.concat(sorted, ", "):sub(1, 120))
      shot("a-z")
    end },
  { 500, function()
      local button = rows()[1]:find(".subscribe-button")
      _G.unsub = rows()[1].name
      if button then button:emit("clicked") end
    end },
  { 800, function()
      -- Unsubscribing asks first; confirmed, the dry run answers.
      local confirm = app.find("dialog-confirm")
      check("asks before unsubscribing", confirm ~= nil)
      if confirm then confirm:emit("clicked") end
    end },
  { 2500, function()
      local row = app.find(_G.unsub)
      local button = row and row:find(".subscribe-button")
      check("unsubscribed, the row stays, saying Subscribe", button and button.label == "Subscribe",
        button and tostring(button.label))
      shot("unsubscribed")
      -- A click on its name opens the channel.
      local text = row:find(".channels-name"):parent():parent()
      for _, c in ipairs(text:controllers()) do
        if c:is_a("GtkGestureClick") then c:emit("released", 1, 5.0, 5.0) end
      end
    end },
  { 4000, function()
      check("a row opens its channel", app.current_page() == "channel", app.current_page())
      log(string.format("SUMMARY %d passed, %d failed", results.pass, results.fail))
      app.quit()
    end },
}
app.on("app.ready", function()
  local i = 0
  local function next_step()
    i = i + 1
    local step = steps[i]
    if not step then return end
    app.timer(step[1], function()
      local ok, err = pcall(step[2])
      if not ok then log("STEP FAILED", i, err) end
      next_step()
    end)
  end
  next_step()
end)
app.timer(60000, function() log("timeout"); app.quit() end)
