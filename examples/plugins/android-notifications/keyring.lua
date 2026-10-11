-- Linux proof of concept. An unavailable/locked keyring is an error, never a
-- reason to fall back to a plaintext plugin-data file.
local Secret = gi.require("Secret", "1")
local M = {}
function M.new(profile)
  local schema = Secret.Schema.new("dev.ytrs.AndroidNotifications", 0, { profile = Secret.SchemaAttributeType.STRING })
  local attributes = { profile = profile }
  return {
    read = function(done)
      Secret.password_lookup(schema, attributes, nil, function(_, result)
        local ok, value = pcall(Secret.password_lookup_finish, result)
        if not ok then done(nil, "Couldn't unlock the system keyring."); return end
        done(value)
      end)
    end,
    write = function(value, done)
      Secret.password_store(schema, attributes, nil, "yt-rs Android notifications", value, nil, function(_, result)
        local ok, stored = pcall(Secret.password_store_finish, result)
        if ok and stored then done(true) else done(false, "Couldn't save the account in the system keyring.") end
      end)
    end,
    clear = function(done)
      Secret.password_clear(schema, attributes, nil, function(_, result)
        local ok = pcall(Secret.password_clear_finish, result)
        if ok then done(true) else done(false, "Couldn't remove the account from the system keyring.") end
      end)
    end,
  }
end
return M
