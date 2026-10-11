-- A terminal page in the sidebar, using VTE (the GTK 4 build) through
-- LuaJIT's FFI. Needs the `vte4` package.
--
-- Shows: embedding a widget from a C library that has no Lua bindings.

local ffi = require("ffi")
ffi.cdef [[
  void *vte_terminal_new(void);
  void vte_terminal_spawn_async(void *terminal, int pty_flags, const char *working_directory,
                                char **argv, char **envv, int spawn_flags,
                                void *child_setup, void *child_setup_data, void *child_setup_data_destroy,
                                int timeout, void *cancellable, void *callback, void *user_data);
  void vte_terminal_set_font_scale(void *terminal, double scale);
]]

local ok, vte = pcall(ffi.load, "libvte-2.91-gtk4.so.0")
if not ok then
  app.log("terminal plugin: VTE for GTK 4 isn't installed (package vte4)")
  return
end

local G_SPAWN_SEARCH_PATH = 4

app.page {
  name = "terminal",
  title = "Terminal",
  build = function()
    local term_ptr = vte.vte_terminal_new()
    local shell = os.getenv("SHELL") or "/bin/sh"
    -- Keep the strings alive while VTE reads them.
    local shell_c = ffi.new("char[?]", #shell + 1, shell)
    local argv = ffi.new("char*[2]", { shell_c, nil })
    vte.vte_terminal_spawn_async(term_ptr, 0, os.getenv("HOME"), argv, nil, G_SPAWN_SEARCH_PATH,
      nil, nil, nil, -1, nil, nil, nil)
    local terminal = ui.from_ffi(term_ptr)
    terminal.hexpand, terminal.vexpand = true, true
    return ui.box { orientation = "vertical", class = "terminal-page", children = { terminal } }
  end,
}

app.nav { id = "terminal", title = "Terminal", icon = "utilities-terminal-symbolic", page = "terminal", section = "library", order = 40 }
app.css [[ .terminal-page { padding: 12px 24px 24px 24px; } ]]
