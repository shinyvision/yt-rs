# Android notifications experiment

An isolated feasibility probe for a browser-independent notification inbox.
Live account testing confirms access to the real likes/replies inbox with
Android credentials. This standalone prototype does **not** yet replace
the notifications plugin or its bell UI.

The plugin uses Lua, WebKitGTK 6.0, libsoup 3 and libsecret through the app's
existing GI bridge. Python and yt-dlp are not used. The Rust changes in this
experiment fix ownership of string hash-table entries for async keyring
calls and conversion of records returned in lists for the login handoff.

## Run

On Linux, with those libraries installed:

```sh
cargo build --release
bash tests/manual/android-notifications.sh
```

This opens a separate yt-rs instance with its own profile at
`$XDG_CONFIG_HOME/yt-rs-android-probe` (normally
`~/.config/yt-rs-android-probe`). It does not copy the normal app's TV token,
load the browser-cookie helper, or access the user's browser profile.

1. Complete Google's sign-in in the embedded window, which opens
   automatically for a fresh profile. **Sign in** restarts that window.
2. The plugin exchanges the temporary handoff credential for an Android
   account credential, saves it in the system keyring, requests a YouTube
   access token and tries the Android `FEnotifications_inbox` feed.
3. Check that the returned messages are the actual likes/replies inbox.
   **Web with Android token** separately tests the same browse endpoint with
   a WEB context and the Android-issued bearer token. Success there must not
   be mistaken for success with the ANDROID client.
4. **Renew access** discards the cached access token and obtains another.
5. Close this probe window, close the browser, and run the script again.
   The inbox loads automatically with the credential restored from the
   keyring. **Fetch inbox** repeats that request.
6. **Forget account** deletes the prototype's local keyring entry. It does
   not revoke the device session at Google; Google Account settings control
   remote sessions.

Opening an inbox may mark its notifications seen. No notification click
tracking, preferences, subscriptions or account contents are modified by
this probe. Clicking a video row opens it inside yt-rs.

## What is and isn't established

Fixture tests exercise exchange, access-token caching/renewal, restoring a
credential, pagination, exact message text, ordering, rejection of signed-out
or unrecognized responses, and cancellation. The Linux keyring round trip,
public Google sign-in page and anonymous HTTPS transport are tested live.
An offline WebKit test verifies a path-scoped handoff and cookie cleanup.
Live account tests on 2026-10-11 completed Google sign-in, exchanged the
handoff, saved the account credential in the keyring, and retrieved 21 real
notifications with the ANDROID client. A separate process restored the
credential and obtained fresh access without creating a login WebView or
reading a browser profile. Forced token renewal in that process retrieved
the same 21 notifications again, all with in-app video destinations.
Restarting the visible prototype also loaded the inbox without another
sign-in.

The Android response uses `inboxNotificationModel` inside `elementRenderer`.
Its headings and message bodies are joined in order, and comment
notifications use their YouTube thumbnails to identify the video to open
inside the app. Android wording can differ from the website (for example,
“Replied” versus “replied”). The first quoted comment matched the user's
website example exactly; its live like count had increased from 2 to 3.
The WEB client with the same Android-issued bearer token returned HTTP 400
`INVALID_ARGUMENT`; that combination is not supported by this prototype.
HTTP 200 with no recognized rows is reported as unverified rather than
claiming an empty working inbox. Pagination is fixture-tested; this account's
live response had no continuation.

`tests/manual/android-notifications-check.lua` can run as `init.lua` in a
separate profile containing the plugin. Set `ANDROID_PROBE_PROFILE` to the
interactive probe's profile path to select its keyring entry. This checks
restart, forced renewal and WEB compatibility, then exits. It logs only
status codes, response type names and counts, never account data or tokens.

The protocol follows microG's `Token` exchange and per-service token request.
It requests the YouTube scope using the Android YouTube package identity.
There is no Google developer project or app-owned OAuth client registration.
The initial embedded sign-in necessarily uses cookies in its **own ephemeral
session**; the one-time `oauth_token` handoff is consumed there, without
copying cookies from another browser. The login session is cleared when
the handoff is received. Subsequent requests use the saved account credential.

The master credential can authorize services beyond YouTube. It is kept in
the system keyring, keyed by the probe's profile path, never in plugin JSON,
logs or command-line arguments. Access tokens stay in memory. Authenticated
HTTP requests allow only the two fixed Google/YouTube destinations and do
not follow redirects. Errors shown in the UI are sanitized.

This is a **Linux prototype**, not a claim of Windows/macOS support. Those
platforms still need their own embedded-login and secure-storage adapters.
No Android emulator, check-in registration or DroidGuard runtime is bundled.
The bridge does not advertise DroidGuard support, so Google's page can use
its explicit unavailable-capability path instead of waiting forever for a
callback. Other unimplemented native capabilities are also absent; accounts
requiring device attestation or Android passkeys may not complete sign-in.
The protocol and client versions are unofficial and may need adjustment
when Google changes them.

## References

- [microG Android setup sign-in](https://github.com/microg/GmsCore/blob/master/play-services-core/src/main/java/org/microg/gms/auth/login/LoginActivity.java)
- [microG account token requests](https://github.com/microg/GmsCore/blob/master/play-services-base/core/src/main/java/org/microg/gms/auth/AuthRequest.java)
- [gpsoauth protocol reference](https://github.com/simon-weber/gpsoauth/blob/master/gpsoauth/__init__.py)
- [Desktop Google Play sign-in implementation](https://github.com/minecraft-linux/playdl-signin-ui-qt)
- [yt-dlp Android client context](https://github.com/yt-dlp/yt-dlp/blob/master/yt_dlp/extractor/youtube/_base.py)

The implementation was written for yt-rs; these are protocol references,
not runtime dependencies.
