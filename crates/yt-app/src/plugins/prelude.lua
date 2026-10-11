-- Helpers written in Lua on top of the core API, loaded before any plugin.

-- ui.build: declarative widget trees. Keys are properties ("_" may stand in
-- for "-"), plus:
--   [1]      the GObject type name, e.g. "GtkBox"
--   class    CSS classes, space-separated
--   on       signal handlers, { clicked = function(self) ... end }
--   children child specs or widgets
--
--   ui.build{ "GtkBox", orientation = "vertical", spacing = 8, class = "card",
--     children = {
--       { "GtkLabel", label = "Hello" },
--       ui.button("Close", function() app.back() end),
--     } }
local special = { class = true, on = true, children = true }

function ui.build(spec)
  if type(spec) ~= "table" then
    return spec
  end
  local props = {}
  for key, value in pairs(spec) do
    if type(key) == "string" and not special[key] then
      props[key] = value
    end
  end
  local widget = ui.new(spec[1] or "GtkBox", props)
  if spec.class then
    for class in tostring(spec.class):gmatch("%S+") do
      widget:add_class(class)
    end
  end
  for signal, handler in pairs(spec.on or {}) do
    widget:on(signal, handler)
  end
  for _, child in ipairs(spec.children or {}) do
    widget:append(ui.build(child))
  end
  return widget
end

local function with_type(spec, type_name)
  spec = spec or {}
  spec[1] = type_name
  return spec
end

function ui.box(spec)
  return ui.build(with_type(spec, "GtkBox"))
end

function ui.label(text, spec)
  spec = with_type(spec, "GtkLabel")
  spec.label = text
  return ui.build(spec)
end

function ui.button(label, on_click, spec)
  spec = with_type(spec, "GtkButton")
  spec.label = label
  if on_click then
    spec.on = spec.on or {}
    spec.on.clicked = on_click
  end
  return ui.build(spec)
end

function ui.scrolled(child, spec)
  local scroller = ui.build(with_type(spec, "GtkScrolledWindow"))
  scroller:set_child(ui.build(child))
  return scroller
end

-- ui.from_ffi(pointer): a widget (or any GObject) created through LuaJIT's
-- FFI, e.g. ui.from_ffi(webkit.webkit_web_view_new()).
function ui.from_ffi(pointer)
  local ffi = require("ffi")
  return ui.wrap(tonumber(ffi.cast("intptr_t", pointer)))
end

-- app.bind(key, fn, scope): runs a Lua function on a key, as a command
-- named "plugin.key.<key>", so it shows up in app.keymap.list().
function app.bind(key, fn, scope)
  local name = "plugin.key." .. key .. (scope and ("." .. scope) or "")
  app.command(name, function(args)
    return fn(args)
  end)
  app.keymap.set(key, name, nil, scope)
end
