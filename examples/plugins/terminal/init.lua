-- A terminal page in the sidebar, using VTE (the GTK 4 build) through
-- GObject introspection. Needs the `vte4` package.
--
-- Shows: embedding a widget from a library the app knows nothing about,
-- with its own types (flags, callbacks) through `gi`.

local ok, Vte = pcall(gi.require, "Vte", "3.91")
if not ok then
  app.log("terminal plugin: VTE for GTK 4 isn't installed (package vte4)")
  return
end
local GLib = gi.require("GLib", "2.0")

app.page {
  name = "terminal",
  title = "Terminal",
  build = function()
    local terminal = Vte.Terminal { hexpand = true, vexpand = true }
    terminal:spawn_async(
      Vte.PtyFlags.DEFAULT,
      os.getenv("HOME"),
      { os.getenv("SHELL") or "/bin/sh" },
      nil, -- the environment: the app's
      GLib.SpawnFlags.SEARCH_PATH,
      nil, -- no child setup
      -1, -- the default timeout
      nil, -- not cancellable
      function(_, pid, err)
        if err then app.log("terminal plugin: " .. tostring(err)) end
      end
    )
    return ui.box { orientation = "vertical", class = "terminal-page", children = { terminal } }
  end,
}

app.nav { id = "terminal", title = "Terminal", icon = "utilities-terminal-symbolic", page = "terminal", section = "library", order = 40 }
app.css [[ .terminal-page { padding: 12px 24px 24px 24px; } ]]
