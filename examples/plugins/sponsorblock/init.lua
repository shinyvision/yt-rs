-- SponsorBlock: skips sponsor segments and other filler that the
-- SponsorBlock community has marked in a video (https://sponsor.ajay.app).
--
-- Shows: reacting to player events, app.http for web APIs, controlling
-- playback through commands, adding a button to the player controls.
--
-- Privacy note: this asks the API by video ID. SponsorBlock also offers a
-- lookup by a hash prefix of the ID, if you'd rather not share which video
-- you watch.

local CATEGORIES = { "sponsor", "selfpromo", "interaction", "intro", "outro" }
local enabled = true
local segments, skipped = {}, {}

local function url_encode(text)
  return (text:gsub("[^%w%-_%.~]", function(c) return string.format("%%%02X", c:byte()) end))
end

app.on("player.loading", function(event)
  segments, skipped = {}, {}
  local id = event.video_id
  local url = "https://sponsor.ajay.app/api/skipSegments?videoID=" .. id
    .. "&categories=" .. url_encode(app.json.encode(CATEGORIES))
  app.http({ url = url }, function(response, err)
    if response and response.status == 200 and app.player.state().video_id == id then
      segments = app.json.decode(response.body) or {}
      app.log("sponsorblock:", #segments, "segments for", id)
    elseif response and response.status ~= 404 then
      app.log("sponsorblock: lookup failed", err or response.status)
    end
  end)
end)

app.on("player.position", function(event)
  if not enabled then return end
  for _, s in ipairs(segments) do
    local from, to = s.segment[1], s.segment[2]
    if not skipped[s.UUID] and event.position >= from and event.position < to - 0.3 then
      skipped[s.UUID] = true
      app.log("sponsorblock: skipped", s.category, from, "->", to)
      app.run("player.seek", { position = to })
      app.toast("Skipped " .. s.category:gsub("selfpromo", "self-promotion"))
    end
  end
end)

-- A toggle in the player controls.
app.slot("player.controls.end", function()
  local button = ui.button(enabled and "SB on" or "SB off", nil, { class = "flat", tooltip_text = "SponsorBlock" })
  button:on("clicked", function(self)
    enabled = not enabled
    self.label = enabled and "SB on" or "SB off"
  end)
  return button
end)
