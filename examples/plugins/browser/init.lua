-- A web browser page in the sidebar, using WebKitGTK through LuaJIT's FFI.
--
-- Shows: calling any C library with LuaJIT's ffi, turning its widgets into
-- app widgets (ui.from_ffi), adding a page and a sidebar entry, declarative
-- UI with ui.build. Needs webkitgtk-6.0 (the GTK 4 version of WebKit).

local ffi = require("ffi")
ffi.cdef [[
  void *webkit_web_view_new(void);
  void webkit_web_view_load_uri(void *web_view, const char *uri);
  const char *webkit_web_view_get_uri(void *web_view);
  void webkit_web_view_go_back(void *web_view);
  void webkit_web_view_reload(void *web_view);
]]

local ok, webkit = pcall(ffi.load, "libwebkitgtk-6.0.so.4")
if not ok then
  app.log("browser plugin: WebKitGTK 6.0 isn't installed")
  return
end

local HOME = "https://duckduckgo.com"

app.page {
  name = "browser",
  title = "Browser",
  build = function()
    local view_ptr = webkit.webkit_web_view_new()
    local view = ui.from_ffi(view_ptr)
    view.hexpand = true
    view.vexpand = true

    local address = ui.build { "GtkEntry", hexpand = true, placeholder_text = "Search or enter an address" }
    local function go(text)
      if not text:match("^%a+://") then
        text = text:match("%.") and ("https://" .. text) or (HOME .. "/?q=" .. text)
      end
      webkit.webkit_web_view_load_uri(view_ptr, text)
    end
    address:on("activate", function(entry) go(entry.text) end)
    -- Keep the address bar in sync with the page.
    view:on("notify::uri", function()
      local uri = webkit.webkit_web_view_get_uri(view_ptr)
      if uri ~= nil then address.text = ffi.string(uri) end
    end)

    local page = ui.box {
      orientation = "vertical",
      spacing = 8,
      class = "browser-page",
      children = {
        ui.box {
          spacing = 8,
          children = {
            ui.button("←", function() webkit.webkit_web_view_go_back(view_ptr) end, { class = "flat" }),
            ui.button("⟳", function() webkit.webkit_web_view_reload(view_ptr) end, { class = "flat" }),
            address,
          },
        },
        view,
      },
    }
    webkit.webkit_web_view_load_uri(view_ptr, HOME)
    return page
  end,
}

app.nav { id = "browser", title = "Browser", icon = "web-browser-symbolic", page = "browser", section = "library", order = 50 }
app.css [[ .browser-page { padding: 12px 24px 24px 24px; } ]]
