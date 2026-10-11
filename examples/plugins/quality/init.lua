-- Plays videos at 720p at most, AV1 when there is a choice, without the
-- player knowing: a hook on the stream resolver changes which streams it
-- picks. The other qualities stay in the quality menu.
--
-- Shows: app.hook (around any of the app's functions, whoever calls them),
-- arguments and results as tables, and app.functions() to find what there
-- is to hook.

local MAX_HEIGHT = 720

local function better(a, b)
  if not b then return true end
  if a.height ~= b.height then return a.height > b.height end
  return (a.vcodec or ""):find("^av01") ~= nil and not (b.vcodec or ""):find("^av01")
end

app.hook("streams.resolve", function(args, next)
  local set = next(args)
  local pick
  for _, stream in ipairs(set.qualities or {}) do
    if stream.height and stream.height <= MAX_HEIGHT and better(stream, pick) then pick = stream end
  end
  if pick then set.video = pick end
  return set
end)
