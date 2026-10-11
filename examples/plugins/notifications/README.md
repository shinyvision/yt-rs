# YouTube notifications

The real youtube.com inbox in yt-rs's top bar: likes, uploads, replies, mentions
and posts returned by YouTube, with an unseen badge, avatars, thumbnails,
read indicators, more pages and YouTube's own notification actions.

The app's TV sign-in cannot access this inbox. This plugin uses the YouTube
session in your **default web browser**. Browser cookies and web authentication
stay in the plugin's Python helper; the Rust app receives notification data.

## Install

Copy this directory into your yt-rs plugins directory (shown in Settings).
On Linux, from the repository root:

```sh
mkdir -p ~/.config/yt-rs/plugins
cp -r examples/plugins/notifications ~/.config/yt-rs/plugins/
python3 -m venv ~/.config/yt-rs/plugins/notifications/.venv
~/.config/yt-rs/plugins/notifications/.venv/bin/python -m pip install -r ~/.config/yt-rs/plugins/notifications/requirements.txt
```

Restart yt-rs. Sign in to the app to show the bell, and sign in to YouTube
in your default browser. If that browser has no YouTube sign-in, the bell
shows a notice with **Open YouTube** and **Try again**.

The helper also detects the default browser on macOS and Windows. Use the
platform's yt-rs plugins directory and create `.venv` inside this plugin;
on Windows its interpreter is `.venv\Scripts\python.exe`. The plugin finds
either layout automatically, then falls back to system Python. The app
itself currently targets Linux; the other platforms' browser detection has
unit coverage but has not been tested on those operating systems.

If system Python already provides the requirements, a virtual environment
is optional. To use another interpreter or browser profile, put these
optional settings in `plugin-data/notifications.json` under the app's config
directory:

```json
{"python": "/path/to/python", "profile": "/path/to/browser/profile"}
```

Leave `profile` unset for yt-dlp's most recently used profile of the default
browser. A profile override still uses the detected browser's cookie reader.
There is no hardcoded preferred browser or fallback to another browser.

## Browser detection and access

- Linux: the default HTTPS handler from Gio, with `xdg-mime` and
  `xdg-settings` fallbacks.
- macOS: Launch Services' default HTTPS handler.
- Windows: the user's HTTPS association and its registered executable.

The cookie reader supports Chrome, Chromium, Firefox, Edge, Brave, Vivaldi,
Opera, Whale and Safari, where yt-dlp supports that browser on the host OS.
Other browsers get an unsupported-browser notice. The default is checked
again for each request, so changing it does not require restarting yt-rs.

Browser encryption may prevent access. Locked keyrings, locked browser
databases, Safari permissions, and Windows app-bound encryption produce an
access notice rather than an empty inbox. There is no attempt to bypass
OS protections. On minimal Linux desktops such as Sway or Hyprland, the
helper can detect an already running Secret Service or KWallet when yt-dlp
cannot infer the keyring from the desktop name. Secret Service access needs
the `SecretStorage` Python dependency in `requirements.txt`.

## Behaviour

The inbox belongs to the browser's selected YouTube account, which can
differ from the account signed into yt-rs. While loading, a centered spinner
shows “Checking your default browser” and then “Fetching notifications from
<browser>”. The loaded panel has no browser subtitle. The helper fetches
YouTube's current web-client configuration and account context, then browses
`FEnotifications_inbox`, the same feed as the website's bell. The older
`notification/get_notification_menu` inbox can return a different feed.

The panel follows YouTube's 480×642 layout (clamped to smaller windows),
with its outlined bell, header, row spacing, six-line messages, 48-pixel
avatars and 86×48 thumbnails. Scrolling to the bottom loads the next page.
Roboto is bundled under the [SIL Open Font License](fonts/OFL.txt), from
[Google Fonts](https://github.com/google/fonts/tree/main/ofl/roboto), and
loaded into the panel's own Pango font map (Pango 1.56+). The icon paths and
layout measurements follow youtube.com's notification panel.

The unseen count is checked at startup and every three minutes while the
window is active, with a check on return after that interval. Opening the
bell fetches the menu and marks notifications seen, as on youtube.com.
Clicking records the interaction and uses the app's link router. Video
notifications, including comment likes and replies, open the video in yt-rs.
Comment links don't yet scroll to the particular thread. Destinations the
app doesn't support, such as community posts, open in the browser.
The gear opens YouTube's notification settings. YouTube's context actions
are sent only when selected. Sign-out cancels pending work and clears the
panel; failures have an explicit retry action.

Requests run asynchronously in a bounded child process, one at a time.
Only unexpired YouTube cookies enter its HTTP cookie jar. Credentials are
never returned to Lua, saved in plugin storage, or logged. yt-dlp makes a
temporary database copy while reading it and removes it afterwards. The
helper refuses HTTP redirects so authorization cannot follow another host.
`YT_RS_DRY_RUN=1` allows count reads, skips interactions and refuses menu
reads, since opening the real menu can change the account's seen state.

## Extension points

These are registered by the plugin, through the same registries as the app:

| Kind | Name | Data |
|---|---|---|
| Component | `notifications_button` | The top-bar bell and popover |
| Component | `notification` | One parsed notification |
| Command | `notifications.open` | Open the panel |
| Command | `notifications.refresh` | Refresh the visible panel or the count |
| Command | `notifications.more` | Fetch the next page |
| Command | `notifications.activate` | `{ item }` |
| Command | `notifications.action` | `{ action }`: a server-provided notification action |
| Filter | `notifications.page` | `{ items, continuation }`, before each page is displayed |
| Event | `notifications.loaded` | `{ items, continuation, first }`, all displayed items |
| Event | `notifications.unseen` | `{ count }` |
| Event | `notifications.read` | `{ id }` |
| Event | `notifications.failed` | `{ status, message }`, sanitized and safe to display |
| Event | `notifications.progress` | `{ message }`, browser detection or fetching progress |

Tests: `python3 -m unittest discover -s tests/plugins -v` and the
`tests/e2e/notifications` plugin; see the [test guide](../../../tests/e2e/README.md).
The protocol follows the live web responses, cross-checked against
[YouTube.js's notifications parser](https://github.com/LuanRT/YouTube.js/blob/main/src/parser/classes/Notification.ts)
and [yt-dlp's browser cookie reader](https://github.com/yt-dlp/yt-dlp/blob/master/yt_dlp/cookies.py).
