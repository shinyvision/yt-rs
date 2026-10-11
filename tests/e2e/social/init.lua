-- End-to-end check of watch details, comments, channels, subscriptions and
-- the account menu. Reads are live; every write command is replaced by a
-- fake that reports success, so nothing is posted, rated or subscribed on
-- the account. See ../README.md.
local shots = os.getenv("E2E_SHOTS") or "/tmp"
local video = { id = "dQw4w9WgXcQ", title = "Rick Astley - Never Gonna Give You Up (Official Video)", channel = "Rick Astley", duration = "3:34" }
local function log(...) app.log("[social]", ...) end

-- Silent, and no watch history.
app.player.set("ao", "null")
app.player.set("mute", "yes")
app.run("watch.set_tracking", { enabled = false })

local account = nil
app.on("account.loaded", function(e) account = e.account; log("account:", e.account.name, e.account.handle, e.account.photo and "photo" or "no photo") end)
app.on("channels.updated", function(e) log("subscriptions:", #e.subscriptions) end)
app.on("watch.loaded", function(e)
  local p = e.page
  log("watch:", p.title, "| owner:", p.owner and p.owner.name, p.owner and p.owner.id, "| likes:", p.likes, "| vote:", p.vote,
    "| subscribed:", p.subscription and tostring(p.subscription.subscribed), "| comments:", p.comments and p.comments.count, "| related:", p.related and #p.related)
end)
app.on("comments.loaded", function(e) log("comments page:", #e.page.comments, "next:", e.page.next and "yes" or "no") end)
app.on("channel.loaded", function(e) local c = e.channel; log("channel:", c.channel.name, c.channel.handle, c.channel.subscribers, "| tabs:", #c.tabs, "| home shelves:", #c.home.shelves) end)
app.on("page.changed", function(e) log("page:", e.page) end)
for _, name in ipairs({ "watch.failed", "comment.posted.failed", "comment.vote.failed", "comment.deleted.failed", "video.rated.failed", "channel.subscription.failed", "player.error" }) do
  app.on(name, function(e) log("FAILED", name, e.error or e.message) end)
end
app.on("comment.posted", function(e) log("posted:", e.comment.id, "parent:", e.parent_id) end)
app.on("comment.voted", function(e) log("voted:", e.comment_id, e.from, "->", e.vote) end)
app.on("comment.deleted", function(e) log("deleted:", e.comment_id) end)
app.on("video.rated", function(e) log("rated:", e.video_id, e.rating) end)
app.on("channel.subscription", function(e) log("subscription:", e.channel_id, e.subscribed) end)

-- Fake writes: the same events the real commands emit, a moment later.
local fake_id = 0
local function later(f) app.timer(400, f) end
local function fake_comment(args)
  fake_id = fake_id + 1
  return { id = (args.parent_id and (args.parent_id .. ".") or "") .. "fake" .. fake_id,
    author = { name = account and account.handle or "@me", handle = account and account.handle, avatar = account and account.photo },
    text = { runs = { { text = args.text } } }, published = "just now", vote = "none", reply_count = 0 }
end
app.command("comment.post", function(a) later(function() app.emit("comment.posted", { video_id = a.video_id, comment = fake_comment(a) }) end) end)
app.command("comment.reply", function(a) later(function() app.emit("comment.posted", { video_id = a.video_id, parent_id = a.parent_id, comment = fake_comment(a) }) end) end)
app.command("comment.vote", function(a) later(function() app.emit("comment.voted", { comment_id = a.comment_id, from = a.from, vote = a.to }) end) end)
app.command("comment.delete", function(a) later(function() app.emit("comment.deleted", { video_id = a.video_id, comment_id = a.comment_id }) end) end)
app.command("video.rate", function(a) later(function() app.emit("video.rated", { video_id = a.video_id, rating = a.rating }) end) end)
app.command("channel.subscribe", function(a) later(function() app.emit("channel.subscription", { channel_id = a.subscription.channel_id, subscribed = true }) end) end)
app.command("channel.unsubscribe", function(a) later(function() app.emit("channel.subscription", { channel_id = a.subscription.channel_id, subscribed = false }) end) end)

-- Widget helpers.
local function find_all(root, class, out)
  out = out or {}
  if root:has_class(class) then table.insert(out, root) end
  for _, c in ipairs(root:children()) do find_all(c, class, out) end
  return out
end
local function first_visible(class, root)
  for _, w in ipairs(find_all(root or app.window(), class)) do
    if w:mapped() then return w end
  end
end
local function shot(name) app.screenshot(shots .. "/" .. name .. ".png"); log("screenshot", name) end
local function click(class, root)
  local w = first_visible(class, root)
  if w then w:emit("clicked") else log("NOT FOUND", class) end
  return w
end

-- Scrolls the watch page to `value` pixels below the player (0: the top).
local function scroll(value)
  local _, player = app.find("watch-player"):size()
  app.find("watch-info").vadjustment.value = value == 0 and 0 or player + value
end
local steps = {
  { 8000, function() shot("01-home") end },
  { 200, function() app.run("video.play", { video = video }) end },
  { 8000, function() shot("02-watch") end },
  { 200, function() click("description-more") end },
  { 1000, function() scroll(150); shot("03-description") end },
  { 200, function() click("description-more"); scroll(330) end },
  { 1500, function() shot("04-comments") end },
  { 200, function() click("replies-toggle") end },
  { 3000, function() shot("05-replies") end },
  { 200, function()
      click("vote-button")
      first_visible("rating"):children()[1]:emit("clicked")
      local input = first_visible("composer-input"); input:grab_focus(); input.buffer.text = "Testing the composer from yt-rs (fake post)"
    end },
  { 1500, function() shot("06-composer") end },
  { 200, function() click("suggested-action", first_visible("comment-composer")) end },
  { 1500, function() shot("07-posted") end },
  { 200, function() scroll(0); click("subscribe-button") end },
  { 1500, function() click("subscribe-button") end },
  { 1500, function() shot("08-unsubscribe-dialog") end },
  { 200, function()
      -- Keep the subscription: press Cancel.
      local function cancel(w)
        if w:is_a("GtkButton") and w.label == "Cancel" and w:mapped() then w:emit("clicked"); return true end
        for _, c in ipairs(w:children()) do if cancel(c) then return true end end
      end
      if not cancel(app.window()) then log("NOT FOUND", "Cancel") end
    end },
  { 200, function() app.run("channel.open", { name = video.channel, video_id = video.id }) end },
  { 6000, function() shot("09-channel") end },
  { 200, function()
      for _, tab in ipairs(find_all(app.window(), "channel-tab")) do
        if tab.label == "Videos" then tab.active = true end
      end
    end },
  { 5000, function() shot("10-channel-videos") end },
  { 200, function() local grid = first_visible("video-grid", app.find("page-channel")); grid:parent().vadjustment.value = 400 end },
  { 1500, function() shot("11-channel-scrolled") end },
  { 200, function() click("channel-item") end },
  { 6000, function() shot("12-sidebar-channel") end },
  { 200, function()
      -- The account menu's first item opens your own channel.
      for _, item in ipairs(find_all(app.window():find(".account-button"), "account-item")) do
        item:emit("clicked"); break
      end
    end },
  { 6000, function() shot("13-own-channel") end },
  { 2000, function() app.quit() end },
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
app.timer(150000, function() log("timeout"); app.quit() end)
