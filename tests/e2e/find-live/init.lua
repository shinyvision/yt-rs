-- Finds live streams with fast chats, for the tests that need one (live
-- streams end within hours, so no fixture lasts): searches a few topics
-- filtered to Live and ranks the streams by chat messages a minute. Run
-- it signed out (SIGNED_OUT=1): signed in, searches land in the account's
-- search history. Prints the six fastest, then quits.
local function log(...) app.log("[find-live]", ...) end
local filters = { features = { "live" }, sort = "relevance", exact = false }
local queries = { "lofi radio", "live gaming", "24/7 music live", "live stream" }
local found = {}
local function rate(video, channel, done)
  if not channel then return done(nil) end
  app.api.youtube.live_chat(app.api.live_chat.token(channel, video, "live"), function(chat, err)
    if err then return done(nil) end
    local times = {}
    for _, a in ipairs(chat.actions or {}) do
      local m = a.message
      if m and m.time_usec then table.insert(times, m.time_usec) end
    end
    if #times < 10 then return done(nil) end
    table.sort(times)
    local span = (times[#times] - times[1]) / 1e6
    done(#times / math.max(span, 1) * 60, os.time() - times[#times] / 1e6)
  end)
end
local function search(qi)
  local q = queries[qi]
  if not q then
    table.sort(found, function(a, b) return a.rate > b.rate end)
    for i = 1, math.min(6, #found) do
      local f = found[i]
      log(string.format("%s %.0f/min newest %.0fs ago | %s", f.id, f.rate, f.age, f.title))
    end
    return app.quit()
  end
  app.api.youtube.search(q, filters, nil, function(page, err)
    if err then log("search error", err); return search(qi + 1) end
    local videos = {}
    for _, item in ipairs(page.items or {}) do
      if item.kind == "video" and #videos < 6 then table.insert(videos, item) end
    end
    local i = 0
    local function next_one()
      i = i + 1
      local v = videos[i]
      if not v then return search(qi + 1) end
      rate(v.id, v.owner and v.owner.id, function(r, age)
        if r then table.insert(found, { id = v.id, rate = r, age = age, title = v.title }) end
        next_one()
      end)
    end
    next_one()
  end)
end
app.on("app.ready", function() search(1) end)
app.timer(110000, function() log("timeout"); app.quit() end)
