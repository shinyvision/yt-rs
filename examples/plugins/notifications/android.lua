local P = require("notifications.protocol")
local M = {}

-- All network and storage dependencies are injectable for account-free tests.
function M.new(http, keyring, json, now)
  local self = {}
  local credential, token, expires, cancel
  local generation = 0
  local saving, clearing, forgetters = false, false, {}
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
        if not response then done(nil, err, "network_error"); return end
        local values = P.fields(response.body)
        local failure, code = P.auth_error(values, response.status)
        if failure then
          if code == "sign_in_required" then credential, token, expires = nil, nil, nil end
          done(nil, failure, code or "service_error"); return
        end
        done(values)
      end)
  end
  local function clear_keyring()
    if saving or clearing then return end
    clearing = true
    keyring.clear(function(ok, err)
      clearing = false
      local callbacks = forgetters
      forgetters = {}
      for _, done in ipairs(callbacks) do done(ok, err) end
    end)
  end
  function self.restore(done)
    stop()
    credential, token, expires = nil, nil, nil
    local current = generation
    keyring.read(function(value, err)
      if current ~= generation then return end
      if err then done(false, err); return end
      if not value then done(false); return end
      local ok, data = pcall(json.decode, value)
      if not ok or type(data) ~= "table" or type(data.master) ~= "string" or data.master == "" or
        type(data.email) ~= "string" or data.email == "" then
        done(false, "The saved sign-in is invalid. Sign in again.", "sign_in_required"); return
      end
      credential = data
      done(true)
    end)
  end
  function self.sign_in(handoff, done)
    if saving or clearing then done(false, "The account is still being updated. Try again.", "busy"); return end
    stop()
    credential, token, expires = nil, nil, nil
    local current = generation
    auth(P.exchange(handoff.token, handoff.email), function(fields, err, code)
      if not fields then done(false, err, code); return end
      if not fields.Token or not fields.Email then done(false, "Google did not return an account credential.", "service_error"); return end
      local account = { master = fields.Token, email = fields.Email }
      saving = true
      keyring.write(json.encode(account), function(ok, failure)
        saving = false
        -- Sign-out during an asynchronous save must also remove that save.
        if #forgetters > 0 then clear_keyring() end
        if current ~= generation then return end
        if ok then credential = account end
        done(ok, failure, not ok and "keyring_error" or nil)
      end)
    end)
  end
  local function access(force, done)
    if not credential then done(nil, "Sign in to YouTube to see your notifications.", "sign_in_required"); return end
    if not force and token and now() < expires then done(token); return end
    token, expires = nil, nil
    auth(P.access(credential), function(fields, err, code)
      if not fields then done(nil, err, code); return end
      if not fields.Auth then done(nil, "Google did not return a YouTube access token.", "service_error"); return end
      token = fields.Auth:gsub("^oauth2:", "")
      local lifetime = tonumber(fields.Expires) or 3600
      if lifetime > now() then lifetime = lifetime - now() end
      expires = now() + math.max(0, math.min(lifetime, 3600) - 300)
      done(token)
    end)
  end
  function self.call(client, endpoint, body, force, done)
    local url = P.url(endpoint) -- Validate before requesting any credentials.
    stop()
    local function perform(retry)
      access(force or retry, function(bearer, err, code)
        if not bearer then done(nil, err, code); return end
        local context, number = P.context(client)
        local payload = {}
        for key, value in pairs(body or {}) do payload[key] = value end
        payload.context = context
        request(url, "application/json", json.encode(payload),
          { Authorization = "Bearer " .. bearer, ["X-Youtube-Client-Name"] = number,
            ["X-Youtube-Client-Version"] = context.client.clientVersion,
            ["User-Agent"] = context.client.userAgent or "Mozilla/5.0", Origin = "https://www.youtube.com" },
          function(response, failure)
            if not response then done(nil, failure, "network_error"); return end
            if response.status == 401 then
              token, expires = nil, nil
              if not retry then perform(true); return end
              credential = nil
              done(nil, "Your YouTube sign-in has expired. Sign in again.", "sign_in_required"); return
            end
            if response.status ~= 200 then done(nil, "YouTube returned HTTP " .. response.status .. ". Try again.", "service_error"); return end
            local ok, data = pcall(json.decode, response.body)
            if not ok or type(data) ~= "table" or data.error then done(nil, "YouTube returned an invalid response.", "service_error"); return end
            for _, logged_out in ipairs(P.nodes(data, "loggedOut")) do
              if logged_out == true then
                credential, token, expires = nil, nil, nil
                done(nil, "YouTube returned a signed-out response. Sign in again.", "sign_in_required"); return
              end
            end
            done(data)
          end)
      end)
    end
    perform(false)
  end
  function self.fetch(client, continuation, force, done)
    local body = continuation and { continuation = continuation } or { browseId = "FEnotifications_inbox" }
    self.call(client, "browse", body, force, function(data, err, code)
      if not data then done(nil, err, code); return end
      local page = P.inbox(data)
      if not page.recognized then done(nil, "YouTube returned an unrecognized notification inbox. Try again.", "service_error"); return end
      done(page)
    end)
  end
  function self.forget(done)
    stop()
    credential, token, expires = nil, nil, nil
    forgetters[#forgetters + 1] = done
    clear_keyring()
  end
  function self.connected() return credential ~= nil end
  function self.account() return credential and credential.email end
  self.cancel = stop
  return self
end
return M
