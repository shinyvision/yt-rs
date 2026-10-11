#!/usr/bin/env bash
# Test the real notification bell in an isolated app profile.
set -eu
repo=$(cd -- "$(dirname -- "$0")/../.." && pwd)
probe_profile="${XDG_CONFIG_HOME:-$HOME/.config}/yt-rs-android-probe"
mkdir -p "$probe_profile/plugins"
# The notifications plugin comes with the app; an older copy this script put
# here would take its place.
rm -rf "$probe_profile/plugins/notifications"
# Retire only the old prototype entry point in this script's own test profile.
rm -f "$probe_profile/plugins/android-notifications/init.lua"
cat > "$probe_profile/init.lua" <<'LUA'
app.on("app.ready", function()
  app.window().title = "yt-rs · notification test"
  app.api.mpv.set_property("ao", "null")
  app.api.mpv.set_property("mute", "yes")
  app.run("watch.set_tracking", { enabled = false })
  app.timer(400, function() app.run("notifications.open") end)
end)
app.on("notifications.failed", function(event) app.log("[notifications]", event.status, event.message) end)
app.on("notifications.loaded", function(event) app.log("[notifications]", #event.items, "notifications loaded") end)
LUA
exec env -u YT_RS_DRY_RUN YT_RS_CONFIG_DIR="$probe_profile" \
  YT_RS_APP_ID=dev.ytrs.NotificationsTest YT_RS_LOG=warn,plugin=info \
  "$repo/target/release/yt-rs"
