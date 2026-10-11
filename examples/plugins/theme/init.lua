-- A different look: purple accent, larger titles, square thumbnails, and
-- the logo replaced.
--
-- Shows: CSS overrides (every color and size in the built-in theme is a CSS
-- variable or class) and changing built-in widgets once the window exists.

app.css [[
  :root {
    --yt-accent: #a855f7;
    --accent-bg-color: #a855f7;
    --yt-radius: 4px;
  }
  .video-card .title { font-size: 17px; }
]]

-- The name in the logo becomes "my tube" (the play button stays).
app.on("app.ready", function()
  local topbar = app.find("topbar")
  local name = topbar and topbar:find(".logo")
  if not name then return end
  name.visible = false
  name:get_parent():append(ui.label("my tube", { class = "my-logo" }))
end)
app.css [[ .my-logo { font-size: 20px; font-weight: 700; letter-spacing: -1px; } ]]
