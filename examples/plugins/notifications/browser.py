"""The notifications plugin's browser session, isolated from the Rust app.

One JSON request on stdin, one JSON result on stdout. Cookies, page config,
authorization headers and upstream errors never cross this boundary.
"""

import contextlib
import ctypes
import hashlib
import http.cookiejar
import io
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

ORIGIN = "https://www.youtube.com"
USER_AGENT = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36"
MENU_TYPE = "NOTIFICATIONS_MENU_REQUEST_TYPE_INBOX"


class Notice(Exception):
    def __init__(self, status, message):
        self.status, self.message = status, message


def mac_browser():
    # Launch Services is authoritative even before a browser has been opened.
    cf = ctypes.CDLL("/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation")
    ls = ctypes.CDLL("/System/Library/Frameworks/CoreServices.framework/CoreServices")
    cf.CFStringCreateWithCString.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_uint32]
    cf.CFStringCreateWithCString.restype = ctypes.c_void_p
    cf.CFStringGetCString.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_long, ctypes.c_uint32]
    cf.CFStringGetCString.restype = ctypes.c_bool
    cf.CFRelease.argtypes = [ctypes.c_void_p]
    ls.LSCopyDefaultHandlerForURLScheme.argtypes = [ctypes.c_void_p]
    ls.LSCopyDefaultHandlerForURLScheme.restype = ctypes.c_void_p
    scheme = cf.CFStringCreateWithCString(None, b"https", 0x08000100)
    try:
        handler = ls.LSCopyDefaultHandlerForURLScheme(scheme)
        if not handler:
            return ""
        try:
            buf = ctypes.create_string_buffer(4096)
            return buf.value.decode() if cf.CFStringGetCString(handler, buf, len(buf), 0x08000100) else ""
        finally:
            cf.CFRelease(handler)
    finally:
        cf.CFRelease(scheme)


def windows_browser():
    # Let Windows resolve the effective protocol association, including its
    # system default when the user has not set an explicit override.
    from ctypes import wintypes

    query = ctypes.WinDLL("shlwapi").AssocQueryStringW
    query.argtypes = [wintypes.DWORD, ctypes.c_int, wintypes.LPCWSTR, wintypes.LPCWSTR,
                      wintypes.LPWSTR, ctypes.POINTER(wintypes.DWORD)]
    query.restype = ctypes.c_long
    length = wintypes.DWORD(32768)
    executable = ctypes.create_unicode_buffer(length.value)
    # ASSOCF_IS_PROTOCOL, ASSOCSTR_EXECUTABLE.
    if query(0x1000, 2, "https", "open", executable, ctypes.byref(length)) == 0 and executable.value:
        return executable.value
    return windows_browser_registration()


def windows_browser_registration():
    import winreg

    path = r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https\UserChoice"
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER, path) as key:
        prog_id = winreg.QueryValueEx(key, "ProgId")[0]
    # Chrome-derived browsers can register opaque ProgIds. Resolve the command
    # as well, but never run it: the executable name identifies the browser.
    try:
        with winreg.OpenKey(winreg.HKEY_CLASSES_ROOT, prog_id + r"\shell\open\command") as key:
            command = winreg.QueryValueEx(key, None)[0]
            executable = re.match(r'\s*(?:"([^"]+)"|(\S+))', command)
            if executable:
                return prog_id + " " + (executable[1] or executable[2])
    except OSError:
        pass
    return prog_id


def default_browser(desktop=""):
    if sys.platform == "darwin":
        return mac_browser()
    if sys.platform == "win32":
        return windows_browser()
    # Gio's desktop association is passed by the plugin on Unix. The fallback
    # also makes this helper useful on its own, without importing PyGObject.
    if desktop:
        return desktop
    for command in (["xdg-mime", "query", "default", "x-scheme-handler/https"],
                    ["xdg-settings", "get", "default-web-browser"]):
        try:
            result = subprocess.run(command, capture_output=True, text=True, timeout=5, check=False)
            if result.returncode == 0 and result.stdout.strip():
                return result.stdout.strip()
        except (OSError, subprocess.TimeoutExpired):
            pass
    raise Notice("browser_unavailable", "Set a default web browser to use YouTube notifications.")


def browser_name(identity):
    identity = identity.lower()
    for name, aliases in (
        ("vivaldi", ("vivaldi",)), ("brave", ("brave",)),
        ("edge", ("microsoft-edge", "microsoft.edgemac", "msedge", "microsoftedge")),
        ("opera", ("opera",)), ("whale", ("whale",)),
        ("chromium", ("chromium",)), ("chrome", ("google-chrome", "google.chrome", "chromehtml", "chrome.exe")),
        ("firefox", ("firefox",)), ("safari", ("safari",)),
    ):
        if any(alias in identity for alias in aliases):
            return name
    raise Notice("unsupported_browser", "Your default browser is not supported by the notifications plugin's cookie reader.")


class QuietLogger:
    def __init__(self):
        self.failed = False
        self.missing_key = False
        self.missing_dependency = False

    def debug(self, *args, **kwargs):
        pass

    info = debug

    def warning(self, *args, **kwargs):
        self.failed = True
        if args and "cannot decrypt v11 cookies: no key found" in str(args[0]):
            self.missing_key = True
        if args and "secretstorage not available" in str(args[0]):
            self.missing_dependency = True

    error = warning


def active_linux_keyring():
    # Minimal desktops (Sway/Hyprland) have no GNOME/KDE desktop identifier.
    # Look for an already running keyring; don't start or unlock one here.
    if not sys.platform.startswith("linux"):
        return None
    for service, keyring in (("org.freedesktop.secrets", "GNOMEKEYRING"),
                             ("org.kde.kwalletd6", "KWALLET6"), ("org.kde.kwalletd5", "KWALLET5")):
        try:
            result = subprocess.run([
                "gdbus", "call", "--session", "--dest", "org.freedesktop.DBus",
                "--object-path", "/org/freedesktop/DBus", "--method", "org.freedesktop.DBus.NameHasOwner", service,
            ], capture_output=True, text=True, timeout=3, check=False)
            if result.returncode == 0 and "true" in result.stdout:
                return keyring
        except (OSError, subprocess.TimeoutExpired):
            pass
    return None


def browser_cookies(browser, profile=None):
    try:
        from yt_dlp.cookies import extract_cookies_from_browser
    except ImportError:
        raise Notice("dependency_missing", "Install yt-dlp for Python to use browser notifications.") from None
    try:
        # yt-dlp handles each browser's database format and OS keyring. Never
        # log its output: diagnostics may contain private paths or cookies.
        logger = QuietLogger()
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            cookies = extract_cookies_from_browser(browser, profile=profile, logger=logger)
            if logger.missing_key and (keyring := active_linux_keyring()):
                logger = QuietLogger()
                cookies = extract_cookies_from_browser(browser, profile=profile, logger=logger, keyring=keyring)
    except FileNotFoundError:
        raise Notice("sign_in_required", "Sign in to YouTube in your default browser to see your notifications here.") from None
    except Exception:
        raise Notice("cookies_unavailable", "Couldn't read your browser's YouTube sign-in. Unlock its keyring or close the browser, then try again.") from None
    jar = http.cookiejar.CookieJar()
    for cookie in cookies:
        domain = cookie.domain.lstrip(".")
        if (domain == "youtube.com" or domain.endswith(".youtube.com")) and not cookie.is_expired():
            jar.set_cookie(cookie)
    if logger.failed and not authorization(cookie_values(jar), {}):
        if logger.missing_dependency:
            raise Notice("dependency_missing", "Install the notifications plugin's Python dependencies to read your browser sign-in.")
        raise Notice("cookies_unavailable", "Couldn't read your browser's YouTube sign-in. Unlock its keyring or close the browser, then try again.")
    return jar


def cookie_values(jar):
    # Ask CookieJar which cookies actually match the HTTPS origin and path.
    request = urllib.request.Request(ORIGIN + "/")
    jar.add_cookie_header(request)
    return dict(part.strip().split("=", 1) for part in request.get_header("Cookie", "").split(";") if "=" in part)


def authorization(cookies, config):
    sync = config.get("DATASYNC_ID", "").split("||")
    user = config.get("USER_SESSION_ID") or (sync[1] if len(sync) > 1 and sync[1] else sync[0])
    timestamp = str(int(time.time()))
    values = (
        ("SAPISIDHASH", cookies.get("SAPISID") or cookies.get("__Secure-3PAPISID")),
        ("SAPISID1PHASH", cookies.get("__Secure-1PAPISID")),
        ("SAPISID3PHASH", cookies.get("__Secure-3PAPISID")),
    )
    headers = []
    for scheme, value in values:
        if value:
            parts = ([str(user)] if user else []) + [timestamp, value, ORIGIN]
            digest = hashlib.sha1(" ".join(parts).encode()).hexdigest()
            headers.append(f"{scheme} {timestamp}_{digest}" + ("_u" if user else ""))
    return " ".join(headers)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    # urllib otherwise forwards Authorization to a redirected host.
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise Notice("sign_in_required", "Open YouTube in your default browser and sign in again, then retry.")


class YouTube:
    def __init__(self, jar):
        self.jar = jar
        self.opener = urllib.request.build_opener(NoRedirect(), urllib.request.HTTPCookieProcessor(jar))
        if not authorization(cookie_values(jar), {}):
            raise Notice("sign_in_required", "Sign in to YouTube in your default browser to see your notifications here.")
        page = self.fetch(ORIGIN + "/?hl=en", {"User-Agent": USER_AGENT}).decode()
        self.config = {}
        for match in re.finditer(r"ytcfg\.set\s*\(\s*", page):
            try:
                value, _ = json.JSONDecoder().raw_decode(page[match.end():])
                if isinstance(value, dict):
                    self.config.update(value)
            except ValueError:
                continue
        if self.config.get("LOGGED_IN") is False:
            raise Notice("sign_in_required", "Sign in to YouTube in your default browser to see your notifications here.")
        if not self.config.get("INNERTUBE_CONTEXT", {}).get("client", {}).get("clientVersion"):
            raise Notice("service_error", "YouTube couldn't load notifications. Open YouTube in your browser, then try again.")

    def fetch(self, url, headers, data=None):
        request = urllib.request.Request(url, headers=headers, data=data)
        try:
            with self.opener.open(request, timeout=20) as response:
                return response.read(8 * 1024 * 1024)
        except urllib.error.HTTPError as error:
            if error.code in (401, 403):
                raise Notice("sign_in_required", "Open YouTube in your default browser and sign in again, then retry.") from None
            raise Notice("service_error", "YouTube couldn't load notifications. Try again in a moment.") from None
        except (OSError, urllib.error.URLError):
            raise Notice("network_error", "Couldn't connect to YouTube. Check your connection and try again.") from None

    def call(self, endpoint, body):
        config = self.config
        context = json.loads(json.dumps(config["INNERTUBE_CONTEXT"]))
        context["client"]["hl"] = "en"
        headers = {
            "User-Agent": USER_AGENT, "Content-Type": "application/json",
            "Origin": ORIGIN, "X-Origin": ORIGIN, "Referer": ORIGIN + "/",
            "Authorization": authorization(cookie_values(self.jar), config),
            "X-Youtube-Client-Name": str(config.get("INNERTUBE_CONTEXT_CLIENT_NAME", 1)),
            "X-Youtube-Client-Version": context["client"]["clientVersion"],
            "X-Goog-AuthUser": str(config.get("SESSION_INDEX", 0)),
            "X-Youtube-Bootstrap-Logged-In": "true",
        }
        sync = config.get("DATASYNC_ID", "").split("||")
        delegated = config.get("DELEGATED_SESSION_ID") or (sync[0] if len(sync) > 1 and sync[1] else None)
        if delegated:
            headers["X-Goog-PageId"] = delegated
        if visitor := context["client"].get("visitorData"):
            headers["X-Goog-Visitor-Id"] = visitor
        payload = dict(body, context=context)
        data = self.fetch(ORIGIN + "/youtubei/v1/" + endpoint + "?prettyPrint=false", headers, json.dumps(payload).encode())
        response = json.loads(data)
        if response.get("responseContext", {}).get("mainAppWebResponseContext", {}).get("loggedOut"):
            raise Notice("sign_in_required", "Sign in to YouTube in your default browser to see your notifications here.")
        if "error" in response:
            raise Notice("service_error", "YouTube couldn't load notifications. Try again in a moment.")
        return response


def nodes(value, key):
    if isinstance(value, dict):
        if key in value:
            yield value[key]
        for item in value.values():
            yield from nodes(item, key)
    elif isinstance(value, list):
        for item in value:
            yield from nodes(item, key)


def text(value):
    return value.get("simpleText") or "".join(run.get("text", "") for run in value.get("runs", []))


def thumbnail(value):
    thumbnails = value.get("thumbnails", [])
    return thumbnails[-1].get("url") if thumbnails else None


def action(endpoint):
    """Only the notification endpoints returned by YouTube can be invoked."""
    api = endpoint.get("commandMetadata", {}).get("webCommandMetadata", {}).get("apiUrl", "")
    if "recordNotificationInteractionsEndpoint" in endpoint:
        return {"endpoint": "notification/record_interactions", "body": {
            "serializedInteractionsRequest": endpoint["recordNotificationInteractionsEndpoint"]["serializedInteractionsRequest"]}}
    if "continuationCommand" in endpoint:
        return {"endpoint": "notification/get_notification_menu", "body": {
            "ctoken": endpoint["continuationCommand"]["token"], "notificationsMenuRequestType": MENU_TYPE}}
    if "getNotificationMenuEndpoint" in endpoint:
        return {"endpoint": "notification/get_notification_menu", "body": endpoint["getNotificationMenuEndpoint"]}
    if api == "/youtubei/v1/notification/modify_channel_preference" and "modifyChannelNotificationPreferenceEndpoint" in endpoint:
        return {"endpoint": "notification/modify_channel_preference", "body": endpoint["modifyChannelNotificationPreferenceEndpoint"]}
    return None


def destination(endpoint):
    comment = endpoint.get("getCommentsFromInboxCommand", {})
    if comment.get("videoId") and comment.get("linkedCommentId"):
        return {"url": ORIGIN + "/watch?" + urllib.parse.urlencode({
            "v": comment["videoId"], "lc": comment["linkedCommentId"]})}
    url = endpoint.get("commandMetadata", {}).get("webCommandMetadata", {}).get("url")
    if not url:
        url = endpoint.get("urlEndpoint", {}).get("url")
    watch = endpoint.get("watchEndpoint", {})
    # Keep comment IDs and posts in their original URLs: the native watch page
    # doesn't yet scroll to a particular comment or display community posts.
    if watch.get("videoId") and not watch.get("params") and not (url and ("lc=" in url or "/post/" in url)):
        return {"link": {"kind": "video", "video_id": watch["videoId"], "start": watch.get("startTimeSeconds")}}
    if not url and watch.get("videoId"):
        url = "/watch?" + urllib.parse.urlencode({"v": watch["videoId"]})
    if url:
        url = urllib.parse.urljoin(ORIGIN, url)
        if urllib.parse.urlsplit(url).scheme in ("http", "https"):
            return {"url": url}
    return {}


def parse_menu(response):
    items, seen = [], set()
    for row in nodes(response, "notificationRenderer"):
        identifier = row.get("notificationId", "")
        if identifier and identifier in seen:
            continue
        seen.add(identifier)
        menu = []
        for entry in nodes(row.get("contextualMenu", {}), "menuServiceItemRenderer"):
            command = action(entry.get("serviceEndpoint", {}))
            if command:
                menu.append({"label": text(entry.get("text", {})), "action": command})
        items.append({
            "id": identifier, "message": text(row.get("shortMessage", {})),
            "sent": text(row.get("sentTimeText", {})), "avatar": thumbnail(row.get("thumbnail", {})),
            "thumbnail": thumbnail(row.get("videoThumbnail", {})), "read": row.get("read", False),
            "destination": destination(row.get("navigationEndpoint", {})),
            "record_click": action(row.get("recordClickEndpoint", {})), "menu": menu,
        })
    if not items and not any(any(nodes(response, key)) for key in (
            "multiPageMenuRenderer", "multiPageMenuNotificationSectionRenderer", "appendContinuationItemsAction",
            "reloadContinuationItemsCommand", "notificationSectionContinuation")):
        raise Notice("service_error", "YouTube didn't return a notification menu. Try again in a moment.")
    continuation = next((command for item in nodes(response, "continuationItemRenderer")
                         if (command := action(item.get("continuationEndpoint", {})))), None)
    # Older responses use a continuation list instead of a continuation item.
    if not continuation:
        token = next((item.get("continuation") for item in nodes(response, "nextContinuationData")), None)
        if token:
            continuation = {"endpoint": "notification/get_notification_menu", "body": {
                "ctoken": token, "notificationsMenuRequestType": MENU_TYPE}}
    return {"items": items, "continuation": continuation}


def run(request):
    operation = request.get("operation", "count")
    if operation not in ("count", "menu", "more", "interact"):
        raise Notice("invalid_request", "Unknown notification request.")
    command = request.get("action") or {}
    if operation == "more" and command.get("endpoint") != "notification/get_notification_menu":
        raise Notice("invalid_request", "Invalid notification continuation.")
    if operation == "interact" and command.get("endpoint") not in (
            "notification/record_interactions", "notification/modify_channel_preference"):
        raise Notice("invalid_request", "Invalid notification action.")
    # Even opening the menu can mark notifications seen. Dry-run must never
    # send that request; fixture tests supply a menu locally instead.
    if os.environ.get("YT_RS_DRY_RUN") == "1" and operation != "count":
        if operation == "interact":
            return {"status": "ok", "dry_run": True}
        return {"status": "dry_run", "message": "Notification menus are disabled in dry-run mode because opening them marks notifications seen."}
    browser = browser_name(default_browser(request.get("desktop", "")))
    youtube = YouTube(browser_cookies(browser, request.get("profile")))
    if operation == "count":
        response = youtube.call("notification/get_unseen_count", {})
        count = response.get("unseenCount")
        if count is None:
            count = next((item.get("unseenCount") for item in nodes(response, "updateNotificationsUnseenCountAction")), None)
        if count is None:
            raise Notice("service_error", "YouTube didn't return a notification count. Try again in a moment.")
        result = {"count": max(0, int(count))}
    elif operation == "menu":
        result = parse_menu(youtube.call("notification/get_notification_menu", {"notificationsMenuRequestType": MENU_TYPE}))
    elif operation == "more":
        result = parse_menu(youtube.call(command["endpoint"], command["body"]))
    else:
        youtube.call(command["endpoint"], command["body"])
        result = {}
    return dict(result, status="ok", browser=browser)


def main():
    try:
        result = run(json.load(sys.stdin))
    except Notice as notice:
        result = {"status": notice.status, "message": notice.message}
    except Exception:
        # No tracebacks here: urllib exceptions can contain authenticated URLs
        # and cookie-reader exceptions can include decrypted data.
        result = {"status": "service_error", "message": "Couldn't load notifications. Update the notifications plugin and yt-dlp, then try again."}
    print(json.dumps(result))


if __name__ == "__main__":
    main()
