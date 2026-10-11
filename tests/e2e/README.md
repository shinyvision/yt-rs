# End-to-end checks

Each directory is a test plugin that drives the real app: it runs commands,
follows events, logs what it sees (`[social] ...`) and takes screenshots.

```sh
cargo build --release
E=$(mktemp -d)
mkdir -p "$E/plugins" && cp -r tests/e2e/social "$E/plugins/"
install -m 600 ~/.config/yt-rs/token.json "$E/token.json"   # your sign-in
mkdir -p /tmp/yt-rs-shots
E2E_SHOTS=/tmp/yt-rs-shots YT_RS_CONFIG_DIR="$E" YT_RS_APP_ID=dev.ytrs.YtRsTest YT_RS_LOG=warn,plugin=info \
  G_DEBUG=fatal-criticals ./target/release/yt-rs
```

`YT_RS_DRY_RUN=1` sends nothing that would change the account (rating,
subscribing, playlists, comments, feedback): the app acts as if it had. The
`playlists` test needs it.

`YT_RS_APP_ID` gives the test window its own class, so a window-manager
rule can move it out of the way without catching a yt-rs you're using. The
plugins are silent (mpv's null audio output, muted) and don't report
watch time, so they leave your history alone. `social` replaces the commands
that would change your account (commenting, voting, rating, subscribing) with
fakes that emit the same success events, so the whole UI flow runs without
posting anything. `G_DEBUG=fatal-criticals` turns GTK warnings about misused
widgets into crashes, so they can't go unnoticed.

`search` checks the search box, results, loading more, chips, filters and
the filters dialog, and what plugins can change there. Signed in, a search
lands in the account's YouTube search history, so the default run only
searches "hello"; `E2E_SEARCH=signed_out` (with no `token.json`) runs the
other queries (an artist, a misspelling, a narrow window) anonymously.

`playback` plays a video YouTube used to bot-check (with chapters), seeks,
switches quality, turns captions on, plays a video with scrambled
signatures, and a live stream, and logs what the player reports.

## Media controls

`mpris.sh` runs the app on a private D-Bus session (`dbus-run-session`), so
the desktop's own media widget never sees the test player, and reads and
drives it from outside with `gdbus` and `playerctl`: status, metadata,
pause, play, seek, set position, volume, rate, loop, next, stop, and the
`PropertiesChanged` signal. Give it the command that runs the `mpris`
plugin:

```sh
E2E="<run the mpris plugin as above>" LOG=/tmp/mpris.log tests/e2e/mpris.sh
```

It keeps the private session from starting an accessibility bus or desktop
portals (`GTK_A11Y=none NO_AT_BRIDGE=1 GDK_DEBUG=no-portals`): one started
there takes over the desktop's own accessibility socket and leaves it dead.

## SponsorBlock

`sponsorblock-test` drives the plugin in `examples/plugins/sponsorblock`
against a stand-in server (`sponsorblock_server.py`, never the real one):
install both plugins, start the server, and point the plugin at it through
its storage:

```sh
python3 -I tests/e2e/sponsorblock_server.py 18761 /tmp/sb.log &
mkdir -p "$E/plugin-data" && echo '{"server": "http://127.0.0.1:18761", "min_duration": 3}' > "$E/plugin-data/sponsorblock.json"
cp -r examples/plugins/sponsorblock tests/e2e/sponsorblock-test "$E/plugins/"
```

The server's log then shows the counted skips, the vote and the submission.

## Downloads

`downloads` runs twice on one profile: `E2E_PHASE=online` (downloads, the
page, playing from the files, stopping and resuming a long download), then
`E2E_PHASE=offline` with YouTube out of reach
(`HTTPS_PROXY=http://127.0.0.1:9 HTTP_PROXY=http://127.0.0.1:9`): the
download still plays, and deleting it empties the page.

## Channel orders

`channel-order` opens Rick Astley's Videos and checks the Latest, Popular
and Oldest chips: there before the videos load, each order loading in its
order (Popular and Oldest from the web client), more loading on scrolling,
back to the top on another order, and no chips on the other tabs. Reads
only.
