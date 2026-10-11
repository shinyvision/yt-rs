-- A web browser page in the sidebar, using WebKitGTK through GObject
-- introspection.
--
-- Shows: using any introspected library with `gi` (WebKit's web view, its
-- methods, properties and signals), adding a page and a sidebar entry,
-- declarative UI with ui.build. Needs webkitgtk-6.0 (the GTK 4 version of
-- WebKit).

local ok, WebKit = pcall(gi.require, "WebKit", "6.0")
if not ok then
  app.log("browser plugin: WebKitGTK 6.0 isn't installed")
  return
end

local HOME = "https://duckduckgo.com"

app.page {
  name = "browser",
  title = "Browser",
  build = function()
    local view = WebKit.WebView { hexpand = true, vexpand = true }

    local address = ui.build { "GtkEntry", hexpand = true, placeholder_text = "Search or enter an address" }
    local function go(text)
      if not text:match("^%a+://") then
        text = text:match("%.") and ("https://" .. text) or (HOME .. "/?q=" .. text)
      end
      view:load_uri(text)
    end
    address:on("activate", function(entry) go(entry.text) end)
    -- Keep the address bar in sync with the page.
    view:on("notify::uri", function() address.text = view.uri or "" end)

    local page = ui.box {
      orientation = "vertical",
      spacing = 8,
      class = "browser-page",
      children = {
        ui.box {
          spacing = 8,
          children = {
            ui.button("←", function() view:go_back() end, { class = "flat" }),
            ui.button("⟳", function() view:reload() end, { class = "flat" }),
            address,
          },
        },
        view,
      },
    }
    view:load_uri(HOME)
    return page
  end,
}

app.nav { id = "browser", title = "Browser", icon = "web-browser-symbolic", page = "browser", section = "library", order = 50 }
app.css [[ .browser-page { padding: 12px 24px 24px 24px; } ]]
