#!/bin/bash
# The media controls (MPRIS) end to end, on a private D-Bus session, so the
# desktop's own media widget never sees the test player: starts the app
# with the mpris plugin, then reads and drives it with gdbus and playerctl.
# Usage: E2E=<command that runs a test plugin> mpris.sh; prints PASS/FAIL
# lines and a summary.
set -u
# Nothing in the private session may start session services that share
# paths with the real ones: its accessibility bus once took over the
# desktop's own socket (/run/user/UID/at-spi/bus_0) and left it dead.
export GTK_A11Y=none NO_AT_BRIDGE=1 GDK_DEBUG=no-portals
exec dbus-run-session -- bash -c '
  set -u
  $E2E > "$LOG" 2>&1 &
  app=$!
  pass=0; fail=0
  check() { if [ "$2" = "$3" ]; then pass=$((pass+1)); echo "PASS $1"; else fail=$((fail+1)); echo "FAIL $1: got [$2], wanted [$3]"; fi; }
  for i in $(seq 1 40); do
    name=$(gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus \
      --method org.freedesktop.DBus.ListNames 2>/dev/null | grep -o "org.mpris.MediaPlayer2[^'"'"']*" | head -1)
    [ -n "$name" ] && break; sleep 0.5
  done
  check "the app is on the bus" "${name:+yes}" "yes"
  sleep 12
  prop() { gdbus call --session --dest "$name" --object-path /org/mpris/MediaPlayer2 \
    --method org.freedesktop.DBus.Properties.Get org.mpris.MediaPlayer2.Player "$1" 2>&1 | sed -E "s/^\(<(.*)>,\)$/\1/"; }
  call() { gdbus call --session --dest "$name" --object-path /org/mpris/MediaPlayer2 --method "org.mpris.MediaPlayer2.Player.$1" "${@:2}" >/dev/null 2>&1; }
  setprop() { gdbus call --session --dest "$name" --object-path /org/mpris/MediaPlayer2 \
    --method org.freedesktop.DBus.Properties.Set org.mpris.MediaPlayer2.Player "$1" "$2" >/dev/null 2>&1; }
  root() { gdbus call --session --dest "$name" --object-path /org/mpris/MediaPlayer2 \
    --method org.freedesktop.DBus.Properties.Get org.mpris.MediaPlayer2 "$1" 2>&1 | sed -E "s/^\(<(.*)>,\)$/\1/"; }
  check "Identity" "$(root Identity)" "'"'"'yt-rs'"'"'"
  check "playing" "$(prop PlaybackStatus)" "'"'"'Playing'"'"'"
  meta=$(prop Metadata)
  check "the title is known" "$(echo "$meta" | grep -o "Rick Astley - Never Gonna Give You Up" | head -1)" "Rick Astley - Never Gonna Give You Up"
  check "the channel is the artist" "$(echo "$meta" | grep -o "xesam:artist.: <\[.Rick Astley.\]>" | head -1 | grep -c Rick)" "1"
  check "the length is the video'"'"'s" "$(echo "$meta" | grep -oE "mpris:length.: <int64 [0-9]+" | grep -oE "[0-9]+$" | cut -c1-3)" "213"
  check "playerctl sees it" "$(playerctl -p "${name#org.mpris.MediaPlayer2.}" status 2>&1)" "Playing"
  mon=$(mktemp)
  gdbus monitor --session --dest "$name" --object-path /org/mpris/MediaPlayer2 > "$mon" 2>&1 &
  monitor=$!
  sleep 1
  call Pause; sleep 1
  check "Pause" "$(prop PlaybackStatus)" "'"'"'Paused'"'"'"
  kill $monitor 2>/dev/null
  check "pausing says so (PropertiesChanged)" "$(grep -c "PropertiesChanged.*PlaybackStatus.: <.Paused.>" "$mon" | head -1 | sed "s/^[1-9][0-9]*$/yes/")" "yes"
  rm -f "$mon"
  call PlayPause; sleep 1
  check "PlayPause" "$(prop PlaybackStatus)" "'"'"'Playing'"'"'"
  before=$(prop Position | grep -oE "[0-9]+$"); call Seek 60000000; sleep 2
  after=$(prop Position | grep -oE "[0-9]+$")
  check "Seek a minute on" "$(( (after - before) / 1000000 >= 58 && (after - before) / 1000000 <= 66 ))" "1"
  track=$(echo "$meta" | grep -oE "/dev/ytrs/video/v[0-9a-f]+" | head -1)
  call SetPosition "$track" 20000000; sleep 2
  pos=$(prop Position | grep -oE "[0-9]+$")
  check "SetPosition to 0:20" "$(( pos / 1000000 >= 19 && pos / 1000000 <= 24 ))" "1"
  call SetPosition /dev/ytrs/video/vother 100000000; sleep 1
  pos2=$(prop Position | grep -oE "[0-9]+$")
  check "SetPosition for another track does nothing" "$(( pos2 / 1000000 < 40 ))" "1"
  setprop Volume "<0.5>"; sleep 1
  check "Volume" "$(prop Volume)" "0.5"
  setprop Rate "<1.5>"; sleep 1
  check "Rate" "$(prop Rate)" "1.5"
  setprop Rate "<1.0>"
  setprop LoopStatus "<'"'"'Track'"'"'>"; sleep 1
  check "LoopStatus Track" "$(prop LoopStatus)" "'"'"'Track'"'"'"
  setprop LoopStatus "<'"'"'None'"'"'>"
  check "can go next" "$(prop CanGoNext)" "true"
  call Next; sleep 6
  meta2=$(prop Metadata)
  check "Next plays another video" "$(echo "$meta2" | grep -c "dQw4w9WgXcQ")" "0"
  call Stop; sleep 1
  check "Stop" "$(prop PlaybackStatus)" "'"'"'Stopped'"'"'"
  echo "SUMMARY $pass passed, $fail failed"
  kill $app 2>/dev/null; wait $app 2>/dev/null
'
