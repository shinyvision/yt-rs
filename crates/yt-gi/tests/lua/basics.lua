-- Namespaces, functions, constants, enums, structs, errors: no display.
local GLib = gi.require("GLib", "2.0")
local Gio = gi.require("Gio", "2.0")
local GObject = gi.require("GObject", "2.0")

assert(type(GLib.MAJOR_VERSION) == "number" and GLib.MAJOR_VERSION == 2, "GLib.MAJOR_VERSION")
assert(GLib.get_user_name() == os.getenv("USER"), "a plain function")
assert(GLib.path_get_basename("/a/b/c.txt") == "c.txt", "strings in and out")
assert(GLib.build_filenamev({ "a", "b", "c" }) == "a/b/c", "a string array in")
local env = GLib.listenv()
assert(type(env) == "table" and #env > 0, "a string array out")

-- Errors.
local ok, err = pcall(GLib.file_get_contents, "/definitely/missing")
assert(not ok, "a missing file fails")
assert(err.code == 4, "error code: " .. tostring(err.code))
assert(err.domain == "g-file-error-quark", "error domain: " .. tostring(err.domain))
assert(tostring(err):find("missing"), "error message: " .. tostring(err))

-- Bytes, binary contents.
local path = os.tmpname()
local f = io.open(path, "wb") f:write("a\0b") f:close()
local ok2, contents = GLib.file_get_contents(path)
assert(ok2 and contents == "a\0b", "binary contents")
os.remove(path)

-- Objects from Gio, methods, out values.
local file = Gio.File.new_for_path("/tmp/x.txt")
assert(file:get_basename() == "x.txt", "interface method on an object")
assert(file:get_path() == "/tmp/x.txt", "get_path")

-- Enums and flags.
assert(Gio.FileType.DIRECTORY == "directory", "enum nick: " .. tostring(Gio.FileType.DIRECTORY))
assert(type(Gio.FileQueryInfoFlags.NOFOLLOW_SYMLINKS) == "number", "flags are numbers")
local info = Gio.File.new_for_path("/"):query_info("standard::type", Gio.FileQueryInfoFlags.NONE, nil)
assert(info:get_file_type() == "directory", "enum return: " .. tostring(info:get_file_type()))

-- Variants.
local v = GLib.Variant.new_string("hi")
assert(v:get_string() == "hi", "variant string")
assert(GLib.Variant.new_int32(7):get_int32() == 7, "variant int")

-- Struct with fields made from Lua (GLib.Date is boxed).
local d = GLib.Date.new_dmy(15, GLib.DateMonth.MARCH, 2024)
assert(d:get_year() == 2024 and d:get_day() == 15, "boxed struct methods")
assert(d:get_month() == "march", "enum from a struct method: " .. tostring(d:get_month()))

-- GObject properties and signals on a plain object.
local action = Gio.SimpleAction.new("x", nil)
assert(action.name == "x", "property")
local hit = 0
action:on("activate", function() hit = hit + 1 end)
action:activate(nil)
assert(hit == 1, "signal")

-- Async callbacks with a main loop.
local loop = GLib.MainLoop.new(nil, false)
local fired = false
GLib.timeout_add(GLib.PRIORITY_DEFAULT, 10, function() fired = true; loop:quit(); return false end)
GLib.timeout_add(GLib.PRIORITY_DEFAULT, 2000, function() loop:quit(); return false end)
loop:run()
assert(fired, "timeout callback")

-- Async I/O.
local p2 = os.tmpname()
f = io.open(p2, "wb") f:write("async!") f:close()
local got
Gio.File.new_for_path(p2):load_contents_async(nil, function(source, result)
  local ok3, data = source:load_contents_finish(result)
  got = ok3 and data
  loop:quit()
end)
loop:run()
os.remove(p2)
assert(got == "async!", "async load: " .. tostring(got))

-- Subprocess.
local proc = Gio.Subprocess.new({ "printf", "out" }, Gio.SubprocessFlags.STDOUT_PIPE)
local out
proc:communicate_utf8_async(nil, nil, function(p, res)
  local ok4, stdout = p:communicate_utf8_finish(res)
  out = stdout
  loop:quit()
end)
loop:run()
assert(out == "out", "subprocess output: " .. tostring(out))

-- Wrong arguments are explained.
local ok5, err5 = pcall(GLib.path_get_basename, {})
assert(not ok5 and tostring(err5):find("expected a string"), tostring(err5))
local ok6, err6 = pcall(GLib.path_get_basename, "a", "b")
assert(not ok6 and tostring(err6):find("takes 1 argument"), tostring(err6))
