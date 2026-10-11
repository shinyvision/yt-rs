# YouTube notifications

The real YouTube inbox in yt-rs's top-bar bell: likes, replies, uploads,
mentions and posts, with avatars, thumbnails, read indicators, pagination
and YouTube's notification actions. Video notifications open inside yt-rs.

Click the bell and **Sign in** to connect your YouTube account in an embedded
Google sign-in window. This sign-in is separate from yt-rs's existing TV
sign-in; use the same account for both. After sign-in, the bell immediately
loads the inbox. Future launches restore the account from your system
keyring and renew access automatically. Your browser can remain closed.

## Install

The plugin currently supports the Linux app and needs **WebKitGTK 6.0**,
**libsoup 3**, and **libsecret**, with their GObject introspection typelibs
and a working Secret Service keyring. On Arch Linux the packages are
`webkitgtk-6.0`, `libsoup3`, and `libsecret`. Python, yt-dlp, browser extensions
and browser-cookie access are not used.

Copy this directory into the plugins directory shown in yt-rs Settings:

```sh
mkdir -p ~/.config/yt-rs/plugins
cp -r examples/plugins/notifications ~/.config/yt-rs/plugins/
```

Restart yt-rs and click the bell. Existing installations of the old browser
helper can delete this plugin's `browser.py`, `requirements.txt` and `.venv`;
the plugin no longer loads them. Old `python` and `profile` plugin settings
are ignored. Use a build containing the GI hash-table and record ownership
fixes introduced with Android notifications.

The gear opens notification account settings, including **Sign out of
notifications** and a link to YouTube's notification preferences. Signing
out of yt-rs also disconnects notifications. A revoked or expired sign-in
returns to the prompt in the bell. Cancelling the Google form closes its
private session and returns to the prompt without leaving a loading state.

## Inbox and badge

Requests use the Android YouTube account protocol. The inbox browses
`FEnotifications_inbox` and reads `inboxNotificationModel` entries inside
`elementRenderer`. It preserves quoted comments and combines the separate
headings and bodies, matching the web inbox's English reply/like wording.
The older `notificationRenderer` format is also supported. Unknown responses
produce an error rather than a fabricated empty inbox.

Android rejects the web client's `notification/get_unseen_count` endpoint.
Its home response instead supplies `updateBrowseTabNewContentAction`, with
`tabIdentifier=FEnotifications_inbox` and `unseenItemCount`. The badge checks
this metadata at startup, every three minutes while the window is active,
and on returning after that interval. A `fields=onResponseReceivedActions`
filter keeps this request small. It does not browse the notification inbox
or mark notifications seen. Opening the bell fetches the inbox; YouTube may
then update its seen state. No browser-name status or subtitle is shown.
Sign-in prompts and loading indicators are centered in the panel's body.

The panel retains the web layout: 480×642 (clamped to smaller windows), the
outlined bell and gear, six-line messages, 48-pixel avatars, 86×48 thumbnails,
and Roboto loaded only into the panel's Pango font map. Scrolling to the
bottom loads the next page. The bundled font is under the
[SIL Open Font License](fonts/OFL.txt).

Clicking records YouTube's supplied interaction and opens the destination
through the app's link router. Android's comment-inbox commands use their
YouTube video thumbnail to identify the video. Comment notifications open
the video, but do not yet scroll to that comment thread. Supported explicit
YouTube URLs retain the app's existing routing; community posts can open
in the browser. Menu actions are sent only when selected. Unrecognized
actions are omitted. The account-settings link also opens in the browser.

## Credentials and lifecycle

Google handles password entry and challenges in an ephemeral, app-owned
WebKit session. Its one-time `oauth_token` handoff is exchanged for an
Android account credential. The session is cleared when the handoff arrives
or the form closes; no browser profile is read. The Android bridge exposes
only implemented callbacks, so Google does not wait for unsupported native
capabilities. Accounts requiring Android device attestation or passkeys may
not complete this sign-in flow.

The persistent account credential lives in the system keyring, keyed by the
yt-rs profile path. It can authorize Google services beyond YouTube, so it
is never written to plugin JSON, logs, command-line arguments or fixture
files. Short-lived YouTube access tokens remain in memory. A locked or
unavailable keyring is an error; there is no plaintext fallback. Local
sign-out removes the saved credential; Google Account settings control
remote session revocation.

Networking is asynchronous, allows only fixed Google/YouTube endpoints,
and refuses redirects. Token expiry triggers renewal; an HTTP 401 triggers
one renewal and retry before returning to sign-in. Generation checks reject
late callbacks, and sign-out waits for an in-flight keyring save before
clearing it. `YT_RS_DRY_RUN=1` permits the count metadata read but blocks
inbox reads and interactions.

Windows/macOS still need embedded-login and keyring adapters. This is an
unofficial protocol and may need updates when Google changes it.

## Extension points

| Kind | Name | Data |
|---|---|---|
| Component | `notifications_button` | Bell and popover |
| Component | `notification` | One parsed notification |
| Command | `notifications.open` | Open the panel |
| Command | `notifications.sign_in` | Open Google sign-in |
| Command | `notifications.sign_out` | Clear the notification account |
| Command | `notifications.settings` | Notification account settings |
| Command | `notifications.refresh` | Retry/refresh the inbox or badge |
| Command | `notifications.more` | Load the next page |
| Command | `notifications.activate` | `{ item }` |
| Command | `notifications.action` | `{ action }`, a supplied menu action |
| Filter | `notifications.page` | `{ items, continuation }` |
| Event | `notifications.loaded` | `{ items, continuation, first }` |
| Event | `notifications.unseen` | `{ count }` |
| Event | `notifications.read` | `{ id }` |
| Event | `notifications.failed` | `{ status, message }`, sanitized |
| Event | `notifications.progress` | `{ message }` |

## Verification and references

Live testing on 2026-10-11 loaded 21 real notifications in the actual bell,
with 21 avatars, thumbnails, menus and native video destinations. A fresh
process restored the account and renewed access without a login window;
forced renewal worked too. The filtered count request returned HTTP 200
with a 561-byte response. No continuation was present in that account's
response; pagination and destructive menu actions are covered by fixtures.

The [test guide](../../../tests/e2e/README.md) covers the UI and account
suites. `bash tests/manual/android-notifications.sh` opens the real app and
bell in an isolated profile. The separate account checker exercises restart
and renewal and logs only sanitized metadata and counts.

Protocol references (not runtime dependencies):

- [microG Android setup sign-in](https://github.com/microg/GmsCore/blob/master/play-services-core/src/main/java/org/microg/gms/auth/login/LoginActivity.java)
- [microG account token requests](https://github.com/microg/GmsCore/blob/master/play-services-base/core/src/main/java/org/microg/gms/auth/AuthRequest.java)
- [Desktop Google Play sign-in bridge](https://github.com/minecraft-linux/playdl-signin-ui-qt)
- [YouTube.js notification parser](https://github.com/LuanRT/YouTube.js/blob/main/src/parser/classes/Notification.ts)
