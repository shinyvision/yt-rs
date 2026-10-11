-- Hides videos by channel or title keyword, hides Shorts, and marks long
-- videos on their cards.
--
-- Shows: filters (changing data before it's shown), wrapping a built-in
-- component (changing every card after it's built), adding card menu items.

local HIDDEN_CHANNELS = { ["Some Channel"] = true }
local HIDDEN_WORDS = { "reaction", "prank" }

local function hidden(video)
  if video.short or HIDDEN_CHANNELS[video.channel or ""] then return true end
  local title = (video.title or ""):lower()
  for _, word in ipairs(HIDDEN_WORDS) do
    if title:find(word, 1, true) then return true end
  end
  return false
end

app.filter("feed.page", function(data)
  for _, shelf in ipairs(data.page.shelves) do
    local kept = app.array {}
    for _, video in ipairs(shelf.videos) do
      if not hidden(video) then table.insert(kept, video) end
    end
    shelf.videos = kept
  end
  return data
end)

-- A "Long" badge on videos over an hour. Cards are reused while scrolling,
-- so the badge is made once ("build") and updated on every refill ("update").
app.wrap("video_card", function(card, video, phase)
  local badge = card:find(".long-badge")
  if phase == "build" or not badge then
    badge = ui.label("Long", { class = "long-badge", halign = "start", valign = "start" })
    card:find(".thumb"):children()[1]:append(badge) -- the thumbnail overlay
  end
  local hours = (video.duration or ""):match("^(%d+):%d+:%d+$")
  badge.visible = hours ~= nil
end)

app.css [[
  .long-badge { background: rgba(255, 0, 51, 0.9); color: white; font-weight: 700;
                font-size: 11px; border-radius: 4px; padding: 1px 6px; margin: 8px; }
]]

-- An extra item in every card's menu.
app.filter("video.menu", function(menu)
  table.insert(menu.items, { label = "Search on DuckDuckGo", command = "feed_filter.search" })
  return menu
end)
app.command("feed_filter.search", function(args)
  local query = (args.video.title or ""):gsub(" ", "+")
  os.execute("xdg-open 'https://duckduckgo.com/?q=" .. query:gsub("'", "") .. "' &")
end)
