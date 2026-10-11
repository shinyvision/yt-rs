-- What a plugin must be able to do, checked end to end in the running app.
-- Each check is something a real plugin needs; none may need a binding
-- written for it. Logs "[capabilities] ok <name>" or "FAIL <name>: why",
-- then a summary, and quits.
local name, dir = ...
local function log(...) app.log("[capabilities]", ...) end

local checks = {}
-- A check runs `f(done)`. Synchronous checks just return; asynchronous
-- ones return "async" and call done(error_or_nil) later.
local function check(label, f) table.insert(checks, { label = label, f = f }) end
local function eq(a, b, what)
  if a ~= b then error(string.format("%s: expected %s, got %s", what or "value", tostring(b), tostring(a)), 2) end
end
local function yes(v, what) if not v then error((what or "condition") .. " is false", 2) end end

local tmp = os.getenv("XDG_RUNTIME_DIR") or "/tmp"
tmp = tmp .. "/yt-rs-capabilities-" .. tostring(os.time())
os.execute("mkdir -p '" .. tmp .. "/icons/hicolor/scalable/actions'")

-- The platform: every introspected library -------------------------------

local Gtk, Gdk, Gio, GLib, GObject, Adw, Pango, Graphene, Gsk
check("namespaces load", function()
  Gtk = gi.require("Gtk", "4.0")
  Gdk = gi.require("Gdk", "4.0")
  Gsk = gi.require("Gsk", "4.0")
  Gio = gi.require("Gio", "2.0")
  GLib = gi.require("GLib", "2.0")
  GObject = gi.require("GObject", "2.0")
  Adw = gi.require("Adw", "1")
  Pango = gi.require("Pango", "1.0")
  Graphene = gi.require("Graphene", "1.0")
end)

check("constants", function()
  eq(Gtk.MAJOR_VERSION, 4, "Gtk.MAJOR_VERSION")
  yes(type(GLib.PRIORITY_DEFAULT) == "number", "GLib.PRIORITY_DEFAULT is a number")
end)

check("enums and flags", function()
  eq(Gtk.Orientation.VERTICAL, "vertical", "Gtk.Orientation.VERTICAL")
  eq(Gtk.WrapMode.WORD_CHAR, "word-char", "Gtk.WrapMode.WORD_CHAR")
  yes(type(Gtk.StateFlags.ACTIVE) == "number", "flags are numbers")
  local box = Gtk.Box.new(Gtk.Orientation.VERTICAL, 4)
  eq(box:get_orientation(), "vertical", "enum return value")
  eq(box.orientation, "vertical", "enum property")
end)

check("constructors and properties", function()
  local b = Gtk.Button.new_with_label("one")
  eq(b.label, "one", "label from constructor")
  local l = Gtk.Label { label = "two", wrap = true }
  eq(l:get_label(), "two", "label from properties")
  yes(l.wrap, "wrap set")
  eq(l:type_name(), "GtkLabel", "type name")
end)

check("methods on app widgets", function()
  local button = ui.new("GtkMenuButton", {})
  local popover = Gtk.Popover.new()
  button:set_popover(popover)
  eq(button:get_popover(), popover, "popover round trip")
  yes(type(popover.popup) == "function", "popover:popup is a method")
end)

check("static functions", function()
  local display = Gdk.Display.get_default()
  yes(display, "a display")
  local theme = Gtk.IconTheme.get_for_display(display)
  yes(theme:is_a("GtkIconTheme"), "icon theme")
end)

check("a plugin ships its own icons", function()
  local svg = '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#000"/></svg>'
  local f = assert(io.open(tmp .. "/icons/hicolor/scalable/actions/cap-test-symbolic.svg", "w"))
  f:write(svg)
  f:close()
  local theme = Gtk.IconTheme.get_for_display(Gdk.Display.get_default())
  theme:add_search_path(tmp .. "/icons")
  yes(theme:has_icon("cap-test-symbolic"), "the icon is found")
  local image = Gtk.Image.new_from_icon_name("cap-test-symbolic")
  eq(image:get_icon_name(), "cap-test-symbolic", "image icon")
end)

check("out arguments", function()
  local label = Gtk.Label.new("measure me")
  local min, nat = label:measure(Gtk.Orientation.HORIZONTAL, -1)
  yes(type(min) == "number" and nat >= min and nat > 0, "measure returns sizes")
end)

check("structs: create, fields, methods", function()
  local rgba = Gdk.RGBA()
  yes(rgba:parse("#ff0000"), "parse")
  eq(rgba.red, 1, "red field")
  eq(rgba.green, 0, "green field")
  rgba.alpha = 0.5
  eq(rgba:to_string(), "rgba(255,0,0,0.5)", "to_string")
  local rect = Graphene.Rect():init(0, 0, 10, 20)
  eq(rect:get_height(), 20, "graphene rect")
  local desc = Pango.FontDescription.from_string("Sans Bold 12")
  eq(desc:get_size(), 12 * Pango.SCALE, "font size")
end)

check("errors are catchable, with domain and code", function()
  local ok, err = pcall(GLib.file_get_contents, tmp .. "/missing")
  yes(not ok, "reading a missing file fails")
  yes(err.message and err.message:find("missing"), "message")
  eq(err.code, 4, "code (G_FILE_ERROR_NOENT)")
end)

check("strings, string arrays and bytes", function()
  local list = Gtk.StringList.new({ "a", "b", "c" })
  eq(list:get_n_items(), 3, "string list")
  eq(list:get_string(1), "b", "item")
  local f = assert(io.open(tmp .. "/data", "wb"))
  f:write("bin\0ary")
  f:close()
  local ok, contents = GLib.file_get_contents(tmp .. "/data")
  yes(ok, "read")
  eq(contents, "bin\0ary", "binary contents")
  local bytes = GLib.Bytes.new("xyz")
  eq(bytes:get_size(), 3, "bytes")
end)

check("variants", function()
  eq(GLib.Variant.new_string("hi"):get_string(), "hi", "string variant")
  local action = Gio.SimpleAction.new("cap", GLib.VariantType.new("s"))
  local got
  action:on("activate", function(_, param) got = param:get_string() end)
  action:activate(GLib.Variant.new_string("x"))
  eq(got, "x", "action parameter")
end)

check("signal arguments are full values", function()
  local label = Gtk.Label.new("")
  local seen
  label:on("notify::label", function(_, pspec) seen = pspec:get_name() end)
  label.label = "changed"
  eq(seen, "label", "pspec name")
end)

check("callbacks: idle", function(done)
  GLib.idle_add(GLib.PRIORITY_DEFAULT, function()
    done()
    return false
  end)
  return "async"
end)

check("async I/O", function(done)
  local file = Gio.File.new_for_path(tmp .. "/data")
  file:load_contents_async(nil, function(source, result)
    local ok, contents = source:load_contents_finish(result)
    if ok and contents == "bin\0ary" then done() else done("wrong contents: " .. tostring(contents)) end
  end)
  return "async"
end)

check("subprocesses", function(done)
  local proc = Gio.Subprocess.new({ "sh", "-c", "printf hello" }, Gio.SubprocessFlags.STDOUT_PIPE)
  proc:communicate_utf8_async(nil, nil, function(p, result)
    local ok, out = p:communicate_utf8_finish(result)
    if ok and out == "hello" then done() else done("got " .. tostring(out)) end
  end)
  return "async"
end)

check("cairo drawing", function(done)
  local area = Gtk.DrawingArea { content_width = 20, content_height = 20 }
  area:set_draw_func(function(_, cr, w, h)
    cr:set_source_rgb(1, 0, 0)
    cr:rectangle(0, 0, w, h)
    cr:fill()
    done()
  end)
  local overlay = Gtk.Window { child = area, default_width = 20, default_height = 20 }
  overlay:present()
  GLib.timeout_add(GLib.PRIORITY_DEFAULT, 3000, function() overlay:destroy() return false end)
  return "async"
end)

check("overrides: plugins add methods to any class", function()
  function Gtk.Widget:cap_hello() return "hello from " .. self:type_name() end
  eq(Gtk.Label.new("x"):cap_hello(), "hello from GtkLabel", "added method")
end)

check("subclass a widget with vfuncs, properties and signals", function(done)
  local drawn = false
  local Swatch = gi.subclass(Gtk.Widget, {
    name = "PluginCapSwatch",
    properties = {
      GObject.param_spec_double("level", nil, nil, 0, 1, 0.5, GObject.ParamFlags.READWRITE),
    },
    signals = { tapped = { params = { "gint" } } },
    vfuncs = {
      measure = function(self, orientation, for_size) return 24, 24, -1, -1 end,
      snapshot = function(self, snapshot)
        local rect = Graphene.Rect():init(0, 0, self:get_width(), self:get_height())
        local color = Gdk.RGBA()
        color:parse("blue")
        snapshot:append_color(color, rect)
        drawn = true
      end,
    },
  })
  local swatch = Swatch()
  eq(swatch.level, 0.5, "property default")
  swatch.level = 0.75
  eq(swatch.level, 0.75, "property set")
  local tapped
  swatch:on("tapped", function(_, n) tapped = n end)
  swatch:emit("tapped", 3)
  eq(tapped, 3, "custom signal")
  swatch.counter = 1
  eq(swatch.counter, 1, "instance fields")
  local min = swatch:measure(Gtk.Orientation.HORIZONTAL, -1)
  eq(min, 24, "measure vfunc")
  local window = Gtk.Window { child = swatch, default_width = 40, default_height = 40 }
  window:present()
  GLib.timeout_add(GLib.PRIORITY_DEFAULT, 100, function()
    if drawn then
      window:destroy()
      done()
      return false
    end
    return true
  end)
  return "async"
end)

check("implement an interface (a list model)", function()
  local Numbers = gi.subclass(GObject.Object, {
    name = "PluginCapNumbers",
    interfaces = { Gio.ListModel },
    vfuncs = {
      get_item_type = function() return Gtk.StringObject end,
      get_n_items = function() return 5 end,
      get_item = function(_, i) return Gtk.StringObject.new("item " .. i) end,
    },
  })
  local model = Gtk.SingleSelection.new(Numbers())
  eq(model:get_n_items(), 5, "n items through GTK")
  eq(model:get_item(2):get_string(), "item 2", "item through GTK")
end)

check("chain up to the parent class", function()
  local Label = gi.subclass(Gtk.Label, {
    name = "PluginCapLabel",
    vfuncs = {
      measure = function(self, orientation, for_size)
        -- (Baselines not returned stay as GTK set them.)
        local min, nat = Gtk.Label.vfunc_measure(self, orientation, for_size)
        return min + 100, nat + 100
      end,
    },
  })
  local plain = Gtk.Label.new("abc")
  local wide = Label { label = "abc" }
  local pmin = plain:measure(Gtk.Orientation.HORIZONTAL, -1)
  local wmin = wide:measure(Gtk.Orientation.HORIZONTAL, -1)
  eq(wmin, pmin + 100, "chained measure")
end)

check("libadwaita", function()
  local toast = Adw.Toast.new("hello")
  eq(toast:get_title(), "hello", "toast title")
end)

check("other libraries (HTTP with libsoup)", function()
  local Soup = gi.require("Soup", "3.0")
  local msg = Soup.Message.new("GET", "https://example.com/")
  eq(msg:get_method(), "GET", "soup message")
end)

check("raw pointers bridge to LuaJIT's ffi", function()
  local label = Gtk.Label.new("p")
  local same = ui.wrap(label:ptr())
  eq(same, label, "same object")
end)

check("clear errors for wrong arguments", function()
  local ok, err = pcall(function() Gtk.Box.new("sideways", 0) end)
  yes(not ok and tostring(err):find("sideways"), "names the bad value: " .. tostring(err))
  ok, err = pcall(function() Gtk.Label.new("x"):set_label(42, 43) end)
  yes(ok or tostring(err):find("argument"), "too many arguments are reported: " .. tostring(err))
end)

-- Runner ------------------------------------------------------------------

local passed, failed = 0, 0
local function finish()
  log(string.format("%d passed, %d failed", passed, failed))
  os.execute("rm -rf '" .. tmp .. "'")
  app.quit()
end

local function run(i)
  local c = checks[i]
  if not c then return finish() end
  local settled = false
  local function settle(err)
    if settled then return end
    settled = true
    if err then
      failed = failed + 1
      log("FAIL " .. c.label .. ": " .. tostring(err))
    else
      passed = passed + 1
      log("ok " .. c.label)
    end
    app.idle(function() run(i + 1) end)
  end
  local ok, result = pcall(c.f, settle)
  if not ok then return settle(result) end
  if result == "async" then
    app.timer(5000, function() settle("timed out") end)
  else
    settle(nil)
  end
end

app.on("app.ready", function() app.timer(500, function() run(1) end) end)
