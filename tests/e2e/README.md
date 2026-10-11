# End-to-end checks

Each directory is a test plugin that drives the real app: it runs commands,
follows events, logs what it sees (`[social] ...`) and takes screenshots.

```sh
cargo build --release
E=$(mktemp -d)
mkdir -p "$E/plugins" && cp -r tests/e2e/social "$E/plugins/"
# For tests that need an account, first run yt-rs with this profile and sign
# in normally. Its keyring entry is separate from your everyday profile.
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
searches "hello"; `E2E_SEARCH=signed_out` (in a fresh signed-out profile) runs the
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

`sponsorblock-test` drives the plugin in `plugins/sponsorblock`, which comes
with the app, against a stand-in server (`sponsorblock_server.py`, never
the real one): install the test, start the server, and point the plugin at
it through its storage:

```sh
python3 -I tests/e2e/sponsorblock_server.py 18761 /tmp/sb.log &
mkdir -p "$E/plugin-data" && echo '{"server": "http://127.0.0.1:18761", "min_duration": 3}' > "$E/plugin-data/sponsorblock.json"
cp -r tests/e2e/sponsorblock-test "$E/plugins/"
```

The server's log then shows the counted skips, the vote and the submission.

## The plugins that come with the app

`bundled-plugins` runs three times on one fresh profile, with
`E2E_PHASE=on`, `off` and `replace`. It checks that SponsorBlock and
notifications load without being installed and that `require` looks in the
plugins folder first. Then it checks that each has a switch in Settings,
that switching SponsorBlock off is saved and keeps it from loading at the
next start, and that a copy of `plugins/sponsorblock` in the plugins folder
(installed for the `replace` run) takes the bundled one's place.

## Notifications

Install `tests/e2e/notifications` as `plugins/notifications-test` in a
throwaway profile, with no token file: a folder named `notifications` would
take the place of the plugin that comes with the app. Run in a separate headless Sway compositor as
for other GUI tests, with `YT_RS_DRY_RUN=1` and `G_DEBUG=fatal-criticals`.
Account and network operations use Lua fixtures. The real UI, embedded
sign-in dialog, link router, commands, filters and events are exercised.
Playback and external URLs are intercepted. The 47 checks cover sign-in and
cancellation, count badges, loading, exact panel geometry, pagination,
deduplication, native video navigation, empty/error states, sign-out during
an in-flight request, reconnection, and context actions. Screenshots use
`E2E_SHOTS` as usual. The sign-in prompt and its text are centered in the
full panel, including after cancellation or expired authentication.

## TV account storage and home refresh

`cargo test -p yt-core auth::tests` checks verified plaintext-to-keyring
migration, failed writes and reads, retry, corrupt credentials, offline
sign-out, and token renewal after restart. These use an in-memory store
and a local HTTP fixture. The ignored
`auth::tests::system_keyring_round_trip_isolated_by_profile` test additionally
checks the real OS keyring using disposable dummy credentials; run it with
`-- --ignored --exact` and an unlocked keyring.

Install `tests/e2e/auth` in a fresh profile to exercise the actual TV sign-in
UI with fake OAuth responses. Its nine checks cover the cached signed-out
home after login, a signed-in empty home, recommendations, sign-out while
home is hidden, and signing in again. It never stores a credential or sends
authenticated HTTP. Run with `YT_RS_DRY_RUN=1` and `G_DEBUG=fatal-criticals`
under the isolated compositor. Screenshots use `E2E_SHOTS`.

## Notification account and protocol

Install `tests/e2e/android-notifications` as
`plugins/notifications-auth-test` in a throwaway profile with no TV token.
It finds the notifications plugin (the bundled one, or a copy in the
plugins folder) through `require`. Run under an isolated headless
compositor with `YT_RS_DRY_RUN=1`, `G_DEBUG=fatal-criticals`, and
`YT_RS_LOG=warn,plugin=info`. This needs libsecret and libsoup 3. Fixtures
exercise exchange, renewal, restart, one retry after HTTP 401, sign-out
racing a keyring write, count tab selection, message/model parsing,
pagination, actions, and cancellation. The real keyring test stores a dummy
value under the test profile's own attributes and deletes it afterward.

With `E2E_LIVE_LOGIN=1`, WebKitGTK 6.0 loads the public Google Android setup
page in the actual app's sign-in dialog. The test checks the form and bridge,
and makes one anonymous YouTube request through the real HTTP transport.
An offline WebKit session verifies a path-scoped test handoff and cleanup.
It never enters real credentials, reads a browser profile, or sends a
signed-in inbox request. This mode reports 58 checks passed. Screenshots use
`E2E_SHOTS`. For an interactive account test, run
`bash tests/manual/android-notifications.sh`; see the
[plugin guide](../../plugins/notifications/README.md).

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

## Live chat

`live-chat` plays a live stream by its ID alone (`E2E_VIDEO`: one live now,
with a busy chat) and checks the chat: it shows and fills, new messages come in, Live chat, hide
and show, theater mode, nothing asked for off screen (the mini player plays)
and a fresh start back on the watch page, and the `chat.messages` filter.
Reads only.

Live streams end within hours, so no fixture lasts: `find-live` (run it
signed out, `SIGNED_OUT=1`, so its searches stay out of the account's
history) searches a few topics filtered to Live and prints the streams
with the fastest chats, in messages a minute. A chat of a few messages a
minute is too slow for "new messages come in".

## Memory

`memory` goes round the same pages (Home, a video, its channel, a search,
Playlists, Home) `E2E_ROUNDS` times (10 by default) and logs, after each
round, the RSS, the PSS (its share of shared pages: GPU drivers are mapped
by every GL program) and the private dirty memory (what only the app wrote:
heap, buffers, caches), with a breakdown by mapping at the start, after the
first round and after the last. Private dirty memory that keeps growing
round after round is a leak; caches stop growing at their limits. Run it
signed out (`SIGNED_OUT=1`): it searches "hello" every round.

## Startup

`startup` logs when the window is ready and when Home has its first videos,
counted from when plugins load (the app's own steps, from the start, log
with `YT_RS_LOG=startup=debug`).

## All subscriptions

`channels` opens "All subscriptions" from the sidebar's heading and checks
a row for every subscription, A to Z, unsubscribing (it asks; the row stays,
saying Subscribe) and a row opening its channel. It needs
`YT_RS_DRY_RUN=1`: unsubscribing sends nothing then.

`probe` plays one video (`E2E_VIDEO`) and logs the player every second;
`E2E_SEEKS=600,3000` seeks there, `E2E_START` starts there, and
`E2E_SHOT_AT=N` takes a screenshot of the watch page at N seconds.
