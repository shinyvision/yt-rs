-- Android account and notification protocol. No credentials or I/O here.
local M = {}
M.auth_url = "https://android.clients.google.com/auth"
M.login_url = "https://accounts.google.com/EmbeddedSetup?source=android&xoauth_display_name=Android%20Device&lang=en&cc=us&langCountry=en_us&hl=en-US&tmpl=new_account"
M.login_agent = "Mozilla/5.0 (Linux; Android 11; Pixel 5 Build/RQ3A.211001.001; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/131.0.0.0 Mobile Safari/537.36 MinuteMaid"
M.auth_agent = "GoogleAuth/1.4 (redfin RQ3A.211001.001); gzip"
M.youtube_version = "21.26.364"
local endpoints = { browse = true, count = true,
  ["notification/record_interactions"] = true, ["notification/modify_channel_preference"] = true }
function M.url(endpoint)
  assert(endpoints[endpoint], "Unsupported notification endpoint")
  if endpoint == "count" then
    -- Android includes each tab's unseen count with the home response.
    -- Request just its metadata, without browsing/marking the inbox seen.
    return "https://www.youtube.com/youtubei/v1/browse?prettyPrint=false&fields=onResponseReceivedActions"
  end
  return "https://www.youtube.com/youtubei/v1/" .. endpoint .. "?prettyPrint=false"
end

function M.count(response)
  for _, action in ipairs(M.nodes(response, "updateBrowseTabNewContentAction")) do
    if action.tabIdentifier == "FEnotifications_inbox" then
      local count = tonumber(action.unseenItemCount)
      if count then return math.max(0, math.floor(count)) end
    end
  end
end
function M.allowed_url(url)
  if url == M.auth_url then return true end
  for endpoint in pairs(endpoints) do if url == M.url(endpoint) then return true end end
  return false
end

local function escape(value)
  return tostring(value):gsub("[^%w%-._~]", function(c) return string.format("%%%02X", c:byte()) end)
end
function M.form(fields)
  local keys, parts = {}, {}
  for key in pairs(fields) do keys[#keys + 1] = key end
  table.sort(keys)
  for _, key in ipairs(keys) do parts[#parts + 1] = escape(key) .. "=" .. escape(fields[key]) end
  return table.concat(parts, "&")
end
function M.fields(body)
  local result = {}
  for line in (body .. "\n"):gmatch("([^\n]*)\n") do
    local key, value = line:gsub("\r$", ""):match("^([%w_]+)=(.*)$")
    if key then result[key] = value end
  end
  return result
end

local function base()
  return { accountType = "HOSTED_OR_GOOGLE", source = "android", sdk_version = 30,
    device_country = "us", operatorCountry = "us", lang = "en_US",
    google_play_services_version = 240913000, has_permission = 1 }
end
function M.exchange(token, email)
  local fields = base()
  fields.Token, fields.Email = token, email
  fields.ACCESS_TOKEN, fields.add_account, fields.get_accountid = 1, 1, 1
  fields.app, fields.callerPkg = "com.google.android.gms", "com.google.android.gms"
  fields.client_sig, fields.callerSig = "38918a453d07199354f8b19af05ec6562ced5788", "38918a453d07199354f8b19af05ec6562ced5788"
  fields.service, fields.droidguard_results = "ac2dm", "null"
  return fields
end
function M.access(credential)
  local fields = base()
  fields.Token, fields.Email = credential.master, credential.email
  fields.app, fields.callerPkg = "com.google.android.youtube", "com.google.android.youtube"
  fields.client_sig, fields.callerSig = "24bb24c05e47e0aefa68a58a766179d9b613a600", "24bb24c05e47e0aefa68a58a766179d9b613a600"
  fields.service = "oauth2:https://www.googleapis.com/auth/youtube"
  return fields
end

local errors = {
  BadAuthentication = "Google rejected the credential. Sign in again.",
  NeedsBrowser = "Google requires another interactive sign-in.",
  MissingDroidguard = "Google requires device verification that this sign-in method doesn't support.",
  DeviceManagementRequired = "This account requires device management.",
  ServiceDisabled = "YouTube access is disabled for this account.",
  CAPTCHA = "Google requires an interactive verification.",
}
function M.auth_error(fields, status)
  if fields.Error then return errors[fields.Error] or "Google rejected the Android authentication request.",
    (fields.Error == "BadAuthentication" or fields.Error == "NeedsBrowser") and "sign_in_required" or "service_error" end
  if status ~= 200 then return "Google authentication returned HTTP " .. tostring(status) .. "." end
end

function M.context(client)
  if client == "ANDROID" then
    return { client = { clientName = client, clientVersion = M.youtube_version,
      androidSdkVersion = 30, osName = "Android", osVersion = "11", hl = "en", gl = "US",
      userAgent = "com.google.android.youtube/" .. M.youtube_version .. " (Linux; U; Android 11) gzip" } }, "3"
  end
  assert(client == "WEB", "Unsupported inbox client")
  return { client = { clientName = client, clientVersion = "2.20260708.00.00", hl = "en", gl = "US" } }, "1"
end

function M.nodes(value, key, out)
  out = out or {}
  if type(value) ~= "table" then return out end
  if value[key] ~= nil then out[#out + 1] = value[key] end
  -- Array order is the inbox order; pairs() is not sufficient for arrays.
  if #value > 0 then
    for _, child in ipairs(value) do M.nodes(child, key, out) end
  else
    for _, child in pairs(value) do M.nodes(child, key, out) end
  end
  return out
end
function M.text(value)
  if type(value) == "string" then return value end
  if type(value) ~= "table" then return "" end
  if value.content then return value.content end
  if value.simpleText then return value.simpleText end
  local parts = {}
  for _, run in ipairs(value.runs or {}) do parts[#parts + 1] = run.text or "" end
  return table.concat(parts)
end

function M.action(command)
  command = command or {}
  command = command.innertubeCommand or command
  local record = M.nodes(command, "recordNotificationInteractionsEndpoint")[1]
  if record and type(record.serializedInteractionsRequest) == "string" then
    return { endpoint = "notification/record_interactions", body = { serializedInteractionsRequest = record.serializedInteractionsRequest } }
  end
  local preference = M.nodes(command, "modifyChannelNotificationPreferenceEndpoint")[1]
  if preference then return { endpoint = "notification/modify_channel_preference", body = preference } end
end

local function image_url(value)
  local sources = M.nodes(value, "sources")[1] or M.nodes(value, "thumbnails")[1]
  return sources and sources[#sources] and sources[#sources].url
end
local function destination(command, thumbnail)
  command = command or {}
  local video = command.watchEndpoint or command.getCommentsFromInboxCommand
  local id = video and video.videoId
  if not id and type(thumbnail) == "string" then
    id = thumbnail:match("^https://i%.ytimg%.com/vi/([%w_-]+)/") or thumbnail:match("^https://i%.ytimg%.com/vi_webp/([%w_-]+)/")
  end
  if type(id) == "string" and #id == 11 and id:match("^[%w_-]+$") then
    return { link = { kind = "video", video_id = id, start = video and video.startTimeSeconds } }, id
  end
  local url = (command.urlEndpoint or {}).url or (((command.commandMetadata or {}).webCommandMetadata or {}).url)
  if type(url) == "string" then
    if url:sub(1, 1) == "/" and url:sub(1, 2) ~= "//" then url = "https://www.youtube.com" .. url end
    if url:match("^https://www%.youtube%.com/") then return { url = url } end
  end
  return {}
end
local function menu_items(value)
  local menu = {}
  for _, entry in ipairs(M.nodes(value, "menuServiceItemRenderer")) do
    local action = M.action(entry.serviceEndpoint)
    if action then menu[#menu + 1] = { label = M.text(entry.text), action = action } end
  end
  return menu
end
function M.inbox(response)
  local items, seen = {}, {}
  local function append(item)
    if item.id and seen[item.id] then return end
    if item.id then seen[item.id] = true end
    items[#items + 1] = item
  end
  for _, row in ipairs(M.nodes(response, "notificationRenderer")) do
    local id = row.notificationId
    local endpoint = row.navigationEndpoint or {}
    local thumbnail = image_url(row.videoThumbnail)
    local target, video_id = destination(endpoint, thumbnail)
    append({ id = id, message = M.text(row.shortMessage),
      sent = M.text(row.sentTimeText), video_id = video_id, destination = target,
      avatar = image_url(row.thumbnail), thumbnail = thumbnail, read = row.read == true,
      record_click = M.action(row.recordClickEndpoint), menu = menu_items(row.contextualMenu) })
  end
  for _, model in ipairs(M.nodes(response, "inboxNotificationModel")) do
    local row = model.inboxNotification
    if type(row) == "table" then
      local metadata = row.metadata or {}
      local header, short = M.text(metadata.header), M.text(metadata.shortMessageFormatted)
      -- Match the English web inbox's connective text, preserving the
      -- actual quoted comment exactly as supplied by YouTube.
      if header == "You got 1 like" then header = "You got a like" end
      short = short:gsub("^Replied:", "replied:")
      -- Likes put the quoted comment under the heading; replies put the
      -- action (e.g. 'replied: "..."') after the account name.
      local separator = short:sub(1, 1) == '"' and ": " or " "
      local message = header ~= "" and short ~= "" and (header .. separator .. short) or (header .. short)
      local command = (row.onTap or {}).innertubeCommand or {}
      local thumbnail = image_url(row.inboxNotificationThumbnail)
      -- Android sends comment-inbox browse commands for likes/replies. The
      -- accompanying YouTube video thumbnail identifies the video to open.
      local target, video_id = destination(command, thumbnail)
      append({ id = row.performOnceIdentifier, message = message,
        sent = M.text(metadata.sentTimeText), video_id = video_id,
        thumbnail = thumbnail, avatar = image_url(row.avatar), read = row.read == true,
        destination = target, record_click = M.action(row.recordClickEndpoint), menu = menu_items(row.menuOnTap) })
    end
  end
  local continuation
  for _, row in ipairs(M.nodes(response, "continuationItemRenderer")) do
    local command = (row.continuationEndpoint or {}).continuationCommand
    if command then continuation = command.token; break end
  end
  if not continuation then
    local next_page = M.nodes(response, "nextContinuationData")[1]
    continuation = next_page and next_page.continuation
  end
  local recognized = #items > 0 or #M.nodes(response, "notificationSectionContinuation") > 0 or
    #M.nodes(response, "multiPageMenuRenderer") > 0
  for _, action in ipairs(M.nodes(response, "appendContinuationItemsAction")) do
    if type(action.continuationItems) == "table" and #action.continuationItems == 0 then recognized = true end
  end
  local header = M.nodes(response, "feedTabbedHeaderRenderer")[1]
  if header and M.text(header.title) == "Notifications" then
    for _, section in ipairs(M.nodes(response, "sectionListRenderer")) do
      if type(section.contents) == "table" and #section.contents == 0 then recognized = true end
    end
  end
  return { items = items, continuation = continuation, recognized = recognized,
    logged_out = #M.nodes(response, "loggedOut") > 0 and M.nodes(response, "loggedOut")[1] == true }
end
return M
