-- Writes docs/api.md, the reference of every function plugins can call
-- and hook (app.functions()), to E2E_OUT. Regenerate it after changing
-- what the services export.
local out = assert(os.getenv("E2E_OUT"), "E2E_OUT: where to write the reference")

local services, order = {}, {}
for _, f in ipairs(app.functions()) do
  local service, name = f.name:match("^([^.]+)%.(.+)$")
  if not services[service] then
    services[service] = {}
    table.insert(order, service)
  end
  table.insert(services[service], { name = name, f = f })
end
table.sort(order)

local lines = {
  "# The app's functions",
  "",
  "Every function of the app's services, as plugins call them",
  "(`app.api.<service>.<function>(...)`) and hook them",
  "(`app.hook(\"<service>.<function>\", fn)`); see [plugins.md](plugins.md).",
  "Async functions take a callback last: `fn(result, error)`.",
  "",
  "Generated from the app by `tests/e2e/api-doc`; types are the Rust ones",
  "(`&str` and `String` are strings, `Option<T>` may be nil, `Vec<T>` is a",
  "list, structs are tables).",
  "",
}
local function add(s) table.insert(lines, s) end
for _, service in ipairs(order) do
  add("## " .. service)
  add("")
  for _, entry in ipairs(services[service]) do
    local f = entry.f
    local params = {}
    for _, p in ipairs(f.params) do table.insert(params, p.name .. ": " .. p.type) end
    local returns = f.returns ~= "()" and (" → " .. f.returns) or ""
    add(string.format("- `%s(%s)`%s%s", entry.name, table.concat(params, ", "), returns, f.async and " *(async)*" or ""))
    if f.doc ~= "" then
      add("  " .. f.doc:gsub("\n", " "))
    end
  end
  add("")
end
local file = assert(io.open(out, "w"))
file:write(table.concat(lines, "\n"))
file:close()
app.log("[api-doc] wrote " .. #app.functions() .. " functions to " .. out)
app.quit()
