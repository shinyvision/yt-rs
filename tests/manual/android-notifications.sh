#!/usr/bin/env bash
# A separate app/profile for an interactive feasibility test. No TV token or
# browser profile is copied. The Google credential lives only in the keyring.
set -eu
repo=$(cd -- "$(dirname -- "$0")/../.." && pwd)
probe_profile="${XDG_CONFIG_HOME:-$HOME/.config}/yt-rs-android-probe"
mkdir -p "$probe_profile/plugins"
cp -R "$repo/examples/plugins/android-notifications" "$probe_profile/plugins/"
cat > "$probe_profile/init.lua" <<'LUA'
app.component("shell", { build = function()
  return ui.box { orientation = "vertical", children = {
    ui.new("GtkHeaderBar", { title_widget = ui.label("Android notifications · prototype") }),
    app.page_host(),
  } }
end })
app.on("app.ready", function()
  app.window().title = "Android notifications · prototype"
  app.api.mpv.set_property("ao", "null")
  app.api.mpv.set_property("mute", "yes")
  app.run("watch.set_tracking", { enabled = false })
  app.timer(200, function() app.navigate("android-notifications") end)
end)
local first = true
app.on("android-notifications.status", function(event)
  app.log("[android-notifications]", event.message)
  if first and event.message == "Sign in to test Android notifications." then
    first = false
    app.idle(function() app.run("android-notifications.sign_in") end)
  elseif first and event.message:find("Account restored from the system keyring.", 1, true) == 1 then
    first = false
    app.idle(function() app.run("android-notifications.fetch") end)
  end
end)
LUA
exec env -u YT_RS_DRY_RUN YT_RS_CONFIG_DIR="$probe_profile" \
  YT_RS_APP_ID=dev.ytrs.AndroidNotificationsProbe YT_RS_LOG=warn,plugin=info \
  "$repo/target/release/yt-rs"
