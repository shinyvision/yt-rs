-- Widgets, structs, out arguments, subclasses with virtual functions and
-- interfaces. Needs a display; opens no window.
local Gtk = gi.require("Gtk", "4.0")
local Gdk = gi.require("Gdk", "4.0")
local Gsk = gi.require("Gsk", "4.0")
local GObject = gi.require("GObject", "2.0")
local Gio = gi.require("Gio", "2.0")
local Graphene = gi.require("Graphene", "1.0")
local Pango = gi.require("Pango", "1.0")
Gtk.init()

local function eq(a, b, what) if a ~= b then error(string.format("%s: expected %s, got %s", what, tostring(b), tostring(a)), 2) end end

-- Constructors, properties, methods.
local b = Gtk.Button.new_with_label("one")
eq(b.label, "one", "constructor")
b:set_label("two")
eq(b:get_label(), "two", "method")
local l = Gtk.Label { label = "props", wrap = true }
eq(l.label, "props", "class call with properties")
eq(l:get_wrap(), true, "boolean")
eq(Gtk.Orientation.VERTICAL, "vertical", "enum")
local box = Gtk.Box.new(Gtk.Orientation.VERTICAL, 6)
eq(box:get_orientation(), "vertical", "enum out")
box:append(l)
eq(l:get_parent(), box, "object out, identity")
eq(box:get_first_child(), l, "first child")

-- Static functions, interfaces' methods.
local display = Gdk.Display.get_default()
assert(display, "display")
local theme = Gtk.IconTheme.get_for_display(display)
assert(theme:is_a("GtkIconTheme"), "icon theme")
assert(type(theme:get_icon_names()) == "table", "string array out")

-- Out arguments.
local min, nat = l:measure(Gtk.Orientation.HORIZONTAL, -1)
assert(min > 0 and nat >= min, "measure: " .. tostring(min) .. " " .. tostring(nat))

-- Structs.
local rgba = Gdk.RGBA()
assert(rgba:parse("#ff0000"), "parse")
eq(rgba.red, 1, "field")
rgba.alpha = 0.5
eq(rgba:to_string(), "rgba(255,0,0,0.5)", "to_string")
local c2 = Gdk.RGBA("blue")
eq(c2.blue, 1, "struct from a string")
local c3 = Gdk.RGBA { red = 0.5, alpha = 1 }
eq(c3.red, 0.5, "struct from fields")
local rect = Graphene.Rect():init(1, 2, 30, 40)
eq(rect:get_width(), 30, "graphene")
eq(rect.size.height, 40, "embedded struct field")
local desc = Pango.FontDescription.from_string("Sans Bold 12")
eq(desc:get_size(), 12 * Pango.SCALE, "pango")

-- Properties holding structs and objects.
local label2 = Gtk.Label.new("x")
label2:set_attributes(Pango.AttrList.from_string("0 1 weight bold"))
assert(label2:get_attributes(), "boxed property out")

-- Signals with typed arguments.
local seen
l:on("notify::label", function(_, pspec) seen = pspec:get_name() end)
l.label = "changed"
eq(seen, "label", "pspec in a signal")

-- Overrides.
function Gtk.Widget:hello() return "hello " .. self:type_name() end
eq(Gtk.Label.new("x"):hello(), "hello GtkLabel", "a Lua method on a class")

-- A subclass with properties, signals, vfuncs and instance fields.
local drawn = 0
local Swatch = gi.subclass(Gtk.Widget, {
  name = "TestSwatch",
  properties = { GObject.param_spec_double("level", nil, nil, 0, 1, 0.5, GObject.ParamFlags.READWRITE) },
  signals = { tapped = { params = { "gint" } } },
  init = function(self) self.inited = true end,
  vfuncs = {
    measure = function(self, orientation, for_size) return 24, 30, -1, -1 end,
    snapshot = function(self, snapshot)
      local color = Gdk.RGBA("blue")
      snapshot:append_color(color, Graphene.Rect():init(0, 0, 10, 10))
      drawn = drawn + 1
    end,
  },
})
local s = Swatch()
eq(s.inited, true, "init ran")
eq(s.level, 0.5, "property default")
s.level = 0.25
eq(s.level, 0.25, "property set")
local s2 = Swatch { level = 0.75 }
eq(s2.level, 0.75, "construct property")
eq(s.level, 0.25, "per-instance values")
local tapped
s:on("tapped", function(_, n) tapped = n end)
s:emit("tapped", 3)
eq(tapped, 3, "custom signal")
s.items = { 1, 2 }
eq(#s.items, 2, "instance field")
eq(s2.items, nil, "fields are per instance")
local m1, m2 = s:measure(Gtk.Orientation.HORIZONTAL, -1)
eq(m1, 24, "measure vfunc min")
eq(m2, 30, "measure vfunc nat")
local snapshot = Gtk.Snapshot.new()
Swatch.vfunc_snapshot(s, snapshot)
eq(drawn, 1, "snapshot vfunc")
local node = snapshot:to_node()
assert(node and node:get_node_type() == "color-node", "render node: " .. tostring(node and node:get_node_type()))
function Swatch:double_level() return self.level * 2 end
eq(s:double_level(), 0.5, "methods on a Lua class")

-- Class setup, and describing a type.
local Named = gi.subclass(Gtk.Widget, {
  name = "TestNamed",
  class_init = function(class) class:set_css_name("named") end,
})
eq(Named():get_css_name(), "named", "class_init")
local d = gi.describe(Gtk.Label)
local has = {}
for _, m in ipairs(d.methods) do has[m] = true end
assert(has.set_label and has.get_parent, "describe lists methods, inherited ones too")
local props = {}
for _, p in ipairs(d.properties) do props[p] = true end
assert(props.label and props.wrap, "describe lists properties")

-- A subclass of a Lua subclass.
local Big = gi.subclass(Swatch, {
  name = "TestBigSwatch",
  vfuncs = { measure = function(self, o, f) local a, b2 = Swatch.vfunc_measure(self, o, f) return a * 2, b2 * 2, -1, -1 end },
})
local big = Big()
eq(big:measure(Gtk.Orientation.HORIZONTAL, -1), 48, "chain up to a Lua class")
eq(big.inited, true, "parent's init")
eq(big:double_level(), 1, "inherited Lua method")

-- Chain up to a native class.
local Wide = gi.subclass(Gtk.Label, {
  name = "TestWideLabel",
  vfuncs = {
    measure = function(self, o, f)
      local a, b2 = Gtk.Label.vfunc_measure(self, o, f)
      return a + 100, b2 + 100
    end,
  },
})
local plain = Gtk.Label.new("abc")
local wide = Wide { label = "abc" }
eq(wide:measure(Gtk.Orientation.HORIZONTAL, -1), plain:measure(Gtk.Orientation.HORIZONTAL, -1) + 100, "chain up")

-- An interface: a list model.
local Numbers = gi.subclass(GObject.Object, {
  name = "TestNumbers",
  interfaces = { Gio.ListModel },
  vfuncs = {
    get_item_type = function() return Gtk.StringObject._gtype end,
    get_n_items = function() return 5 end,
    get_item = function(_, i) return Gtk.StringObject.new("item " .. i) end,
  },
})
local model = Gtk.SingleSelection.new(Numbers())
eq(model:get_n_items(), 5, "n items through GTK")
eq(model:get_item(2):get_string(), "item 2", "item through GTK")
local list = Gtk.ListView.new(model, nil)
assert(list, "a list view on a Lua model")

-- Callbacks: sorting with a Lua comparison (scope call).
local strings = Gtk.StringList.new({ "b", "c", "a" })
local sorter = Gtk.CustomSorter.new(function(a, b2)
  -- Items come as plain pointers (gconstpointer): wrap them.
  local x, y = gi.wrap(a):get_string(), gi.wrap(b2):get_string()
  return x < y and -1 or (x > y and 1 or 0)
end)
local sorted = Gtk.SortListModel.new(strings, sorter)
eq(sorted:get_item(0):get_string(), "a", "sorted by a Lua callback")

-- Draw functions get a cairo context.
local area = Gtk.DrawingArea.new()
area:set_draw_func(function(_, cr, w, h) cr:set_source_rgb(1, 0, 0) end)

-- Pointers bridge.
local same = gi.type("GtkLabel")
eq(same._name, "Gtk.Label", "gi.type by name")

-- Wrong arguments.
local ok, err = pcall(Gtk.Box.new, "sideways", 0)
assert(not ok and tostring(err):find("sideways"), tostring(err))
ok, err = pcall(function() box:append("nope") end)
assert(not ok and tostring(err):find("Gtk.Widget"), tostring(err))
ok, err = pcall(function() Gtk.Widget.set_visible("x", true) end)
assert(not ok and tostring(err):find("method"), tostring(err))
