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

app.on("app.ready", function()
  local topbar = app.find("topbar")
  local logo = topbar and topbar:find(".logo")
  if logo then logo.label = "my tube" end
end)
