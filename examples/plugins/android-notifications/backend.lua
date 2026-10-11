local P = require("android-notifications.protocol")
local M = {}

-- Dependency injection lets the complete exchange/renew/restart sequence run
-- with fixtures, without either an account or a live Google request.
function M.new(http, keyring, json, now)
  local self = {}
  local credential, token, expires, cancel
  local generation = 0
  local function stop()
    generation = generation + 1
    if cancel then cancel(); cancel = nil end
  end
  local function request(url, kind, body, headers, done)
    local current = generation
    cancel = http(url, kind, body, headers, function(response, err)
      if generation ~= current then return end
      cancel = nil
      done(response, err)
    end)
  end
  local function auth(fields, done)
    request(P.auth_url, "application/x-www-form-urlencoded", P.form(fields),
      { ["User-Agent"] = P.auth_agent, app = fields.app }, function(response, err)
        if not response then done(nil, err); return end
        local fields = P.fields(response.body)
        local failure = P.auth_error(fields, response.status)
        if failure then done(nil, failure); return end
        done(fields)
      end)
  end
  function self.restore(done)
    stop()
    local current = generation
    keyring.read(function(value, err)
      if current ~= generation then return end
      if err then done(false, err); return end
      if not value then done(false); return end
      local ok, data = pcall(json.decode, value)
      if not ok or type(data) ~= "table" or type(data.master) ~= "string" or type(data.email) ~= "string" then
        done(false, "The saved credential is invalid. Sign in again."); return
      end
      credential, token, expires = data, nil, nil
      done(true)
    end)
  end
  function self.sign_in(handoff, done)
    stop()
    token, expires = nil, nil
    local current = generation
    auth(P.exchange(handoff.token, handoff.email), function(fields, err)
      if not fields then done(false, err); return end
      if not fields.Token or not fields.Email then done(false, "Google did not return an account credential."); return end
      local account = { master = fields.Token, email = fields.Email }
      keyring.write(json.encode(account), function(ok, failure)
        if current ~= generation then return end
        if ok then credential = account end
        done(ok, failure)
      end)
    end)
  end
  local function access(force, done)
    if not credential then done(nil, "Sign in to test Android notifications."); return end
    if not force and token and now() < expires then done(token); return end
    token, expires = nil, nil
    auth(P.access(credential), function(fields, err)
      if not fields then done(nil, err); return end
      if not fields.Auth then done(nil, "Google did not return a YouTube access token."); return end
      token = fields.Auth:gsub("^oauth2:", "")
      -- Expires is a lifetime on some responses and an epoch on others.
      local lifetime = tonumber(fields.Expires) or 3600
      if lifetime > now() then lifetime = lifetime - now() end
      expires = now() + math.max(0, math.min(lifetime, 3600) - 300)
      done(token)
    end)
  end
  function self.fetch(client, continuation, force, done)
    stop()
    access(force, function(bearer, err)
      if not bearer then done(nil, err); return end
      local context, number = P.context(client)
      local payload = { context = context }
      if continuation then payload.continuation = continuation else payload.browseId = "FEnotifications_inbox" end
      request("https://www.youtube.com/youtubei/v1/browse?prettyPrint=false", "application/json", json.encode(payload),
        { Authorization = "Bearer " .. bearer, ["X-Youtube-Client-Name"] = number,
          ["X-Youtube-Client-Version"] = context.client.clientVersion,
          ["User-Agent"] = context.client.userAgent or "Mozilla/5.0", ["Origin"] = "https://www.youtube.com" },
        function(response, failure)
          if not response then done(nil, failure); return end
          if response.status ~= 200 then
            if response.status == 401 then token, expires = nil, nil end
            done(nil, client .. " inbox returned HTTP " .. response.status .. "."); return
          end
          local ok, data = pcall(json.decode, response.body)
          if not ok or type(data) ~= "table" or data.error then done(nil, "YouTube returned an invalid inbox response."); return end
          local page = P.inbox(data)
          if page.logged_out then done(nil, "YouTube returned a signed-out response."); return end
          if #page.items == 0 then
            done(nil, "YouTube returned HTTP 200 but no recognized notifications. Inbox access is not yet verified."); return
          end
          done(page)
        end)
    end)
  end
  function self.forget(done)
    stop()
    credential, token, expires = nil, nil, nil
    keyring.clear(done)
  end
  self.cancel = stop
  return self
end
return M
