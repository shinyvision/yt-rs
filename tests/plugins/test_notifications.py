"""Notification protocol and browser selection, without real credentials."""
import hashlib
import http.cookiejar
import importlib.util
import io
import json
from pathlib import Path
import sys
import types
import unittest
from unittest.mock import MagicMock, patch

SPEC = importlib.util.spec_from_file_location("browser", Path(__file__).parents[2] / "examples/plugins/notifications/browser.py")
browser = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(browser)


def cookie(name="SAPISID", domain=".youtube.com", value="test-secret", expires=None):
    return http.cookiejar.Cookie(0, name, value, None, False, domain, True, domain.startswith("."),
                                 "/", True, True, expires, False, None, None, {})


def row(identifier="notice-1"):
    return {"notificationRenderer": {
        "notificationId": identifier, "shortMessage": {"runs": [{"text": "A channel "}, {"text": "uploaded a video"}]},
        "sentTimeText": {"simpleText": "2 hours ago"}, "read": False,
        "thumbnail": {"thumbnails": [{"url": "https://example.org/avatar"}]},
        "navigationEndpoint": {"watchEndpoint": {"videoId": "abcdefghijk"}},
        "recordClickEndpoint": {"recordNotificationInteractionsEndpoint": {"serializedInteractionsRequest": "click-token"}},
        "contextualMenu": {"menuRenderer": {"items": [{"menuServiceItemRenderer": {
            "text": {"runs": [{"text": "Hide this notification"}]},
            "serviceEndpoint": {"recordNotificationInteractionsEndpoint": {"serializedInteractionsRequest": "hide-token"}},
        }}]}},
    }}


class BrowserTests(unittest.TestCase):
    def test_browser_associations(self):
        identities = {
            "vivaldi-stable.desktop": "vivaldi", "com.vivaldi.Vivaldi": "vivaldi",
            "org.mozilla.firefox.desktop": "firefox", "FirefoxURL-1234": "firefox",
            "com.google.Chrome": "chrome", "ChromeHTML": "chrome",
            "com.brave.Browser": "brave", "BraveHTML ChromeHTML": "brave",
            "MSEdgeHTM": "edge", "com.microsoft.edgemac": "edge",
            "chromium-browser.desktop": "chromium", "com.apple.Safari": "safari",
            "com.operasoftware.Opera": "opera", "WhaleHTML": "whale",
        }
        for identity, expected in identities.items():
            with self.subTest(identity=identity):
                self.assertEqual(browser.browser_name(identity), expected)
        with self.assertRaises(browser.Notice) as error:
            browser.browser_name("org.example.Unknown")
        self.assertEqual(error.exception.status, "unsupported_browser")

    def test_platform_dispatch_and_linux_association(self):
        with patch.object(browser.sys, "platform", "darwin"), patch.object(browser, "mac_browser", return_value="com.apple.Safari"):
            self.assertEqual(browser.default_browser("ignored"), "com.apple.Safari")
        with patch.object(browser.sys, "platform", "win32"), patch.object(browser, "windows_browser", return_value="MSEdgeHTM"):
            self.assertEqual(browser.default_browser("ignored"), "MSEdgeHTM")
        with patch.object(browser.sys, "platform", "linux"), patch.object(browser.subprocess, "run") as run:
            self.assertEqual(browser.default_browser("firefox.desktop"), "firefox.desktop")
            run.assert_not_called()
            run.return_value = types.SimpleNamespace(returncode=0, stdout="google-chrome.desktop\n")
            self.assertEqual(browser.default_browser(), "google-chrome.desktop")
            self.assertIn("x-scheme-handler/https", run.call_args.args[0])

    def test_windows_resolves_opaque_registration(self):
        registry = MagicMock()
        registry.QueryValueEx.side_effect = [("OpaqueId", 1), ('"C:\\Program Files\\Brave\\brave.exe" -- "%1"', 1)]
        with patch.dict(sys.modules, winreg=registry):
            self.assertEqual(browser.browser_name(browser.windows_browser_registration()), "brave")
        self.assertIn("https", registry.OpenKey.call_args_list[0].args[1])

    def test_windows_queries_effective_https_handler(self):
        def query(flags, kind, scheme, verb, output, length):
            self.assertEqual((flags, kind, scheme, verb), (0x1000, 2, "https", "open"))
            output.value = r"C:\Program Files\Mozilla Firefox\firefox.exe"
            return 0
        library = types.SimpleNamespace(AssocQueryStringW=MagicMock(side_effect=query))
        with patch.object(browser.ctypes, "WinDLL", return_value=library, create=True):
            self.assertEqual(browser.browser_name(browser.windows_browser()), "firefox")

    def test_only_live_youtube_cookies_are_retained(self):
        cookies = [cookie(), cookie(domain=".google.com"), cookie(domain="evil-youtube.com"),
                   cookie(domain=".youtube.com.evil.test"), cookie(name="expired", expires=1)]
        with patch("yt_dlp.cookies.extract_cookies_from_browser", return_value=cookies):
            jar = browser.browser_cookies("firefox")
        self.assertEqual([(c.name, c.domain) for c in jar], [("SAPISID", ".youtube.com")])

    def test_decryption_failure_is_not_missing_sign_in(self):
        def extract(*args, logger, **kwargs):
            logger.warning("Failed to decrypt")
            return []
        with patch("yt_dlp.cookies.extract_cookies_from_browser", side_effect=extract):
            with self.assertRaises(browser.Notice) as error:
                browser.browser_cookies("chrome")
        self.assertEqual(error.exception.status, "cookies_unavailable")

    def test_missing_key_uses_running_keyring(self):
        def extract(*args, logger, keyring=None, **kwargs):
            if keyring:
                return [cookie()]
            logger.warning("cannot decrypt v11 cookies: no key found")
            return []
        with patch("yt_dlp.cookies.extract_cookies_from_browser", side_effect=extract) as reader, \
                patch.object(browser, "active_linux_keyring", return_value="GNOMEKEYRING"):
            self.assertEqual(len(browser.browser_cookies("vivaldi")), 1)
            self.assertEqual(reader.call_args.kwargs["keyring"], "GNOMEKEYRING")

    def test_signed_out_never_requests_youtube(self):
        with patch.object(browser.YouTube, "fetch") as fetch:
            with self.assertRaises(browser.Notice) as error:
                browser.YouTube(http.cookiejar.CookieJar())
            self.assertEqual(error.exception.status, "sign_in_required")
            fetch.assert_not_called()

    def test_cookie_auth_with_secondary_account(self):
        with patch.object(browser.time, "time", return_value=100):
            auth = browser.authorization({"__Secure-3PAPISID": "secret"}, {"DATASYNC_ID": "channel||user"})
        digest = hashlib.sha1(b"user 100 secret https://www.youtube.com").hexdigest()
        self.assertEqual(auth, f"SAPISIDHASH 100_{digest}_u SAPISID3PHASH 100_{digest}_u")

    def test_redirects_never_forward_credentials(self):
        with self.assertRaises(browser.Notice):
            browser.NoRedirect().redirect_request(None, None, 302, "", {}, "https://example.org")


class ProtocolTests(unittest.TestCase):
    def test_menu_uses_current_bell_feed_and_reports_progress(self):
        events = []
        with patch.dict(browser.os.environ, YT_RS_DRY_RUN="0"), \
                patch.object(browser, "default_browser", return_value="firefox.desktop"), \
                patch.object(browser, "browser_cookies"), patch.object(browser, "YouTube") as youtube:
            youtube.return_value.call.return_value = {"multiPageMenuRenderer": {"sections": []}}
            result = browser.run({"operation": "menu"}, events.append)
            self.assertEqual(result["status"], "ok")
            youtube.return_value.call.assert_called_once_with("browse", {"browseId": "FEnotifications_inbox"})
        self.assertEqual([event["stage"] for event in events], ["checking_browser", "fetching"])
        self.assertEqual(events[-1]["browser"], "firefox")

    def test_current_inbox_continuation_stays_on_browse(self):
        endpoint = {"commandMetadata": {"webCommandMetadata": {"apiUrl": "/youtubei/v1/browse"}},
                    "continuationCommand": {"token": "new-feed-token", "request": "CONTINUATION_REQUEST_TYPE_BROWSE"}}
        continuation = browser.action(endpoint)
        self.assertEqual(continuation, {"endpoint": "browse", "body": {"continuation": "new-feed-token"}})
        with patch.dict(browser.os.environ, YT_RS_DRY_RUN="0"), \
                patch.object(browser, "default_browser", return_value="firefox.desktop"), \
                patch.object(browser, "browser_cookies"), patch.object(browser, "YouTube") as youtube:
            youtube.return_value.call.return_value = {"appendContinuationItemsAction": {"continuationItems": []}}
            browser.run({"operation": "more", "action": continuation})
            youtube.return_value.call.assert_called_once_with("browse", {"continuation": "new-feed-token"})

    def test_empty_menu_differs_from_unrecognized_response(self):
        self.assertEqual(browser.parse_menu({"multiPageMenuRenderer": {"sections": []}})["items"], [])
        with self.assertRaises(browser.Notice):
            browser.parse_menu({"unexpected": "response"})

    def test_menu_parses_notifications_actions_and_continuation(self):
        response = {"actions": [{"openPopupAction": {"popup": {"items": [row(), row(),
            {"continuationItemRenderer": {"continuationEndpoint": {"continuationCommand": {"token": "next"}}}}]}}}]}
        menu = browser.parse_menu(response)
        self.assertEqual(len(menu["items"]), 1)
        item = menu["items"][0]
        self.assertEqual(item["message"], "A channel uploaded a video")
        self.assertEqual(item["menu"][0]["action"]["body"]["serializedInteractionsRequest"], "hide-token")
        self.assertEqual(item["record_click"]["endpoint"], "notification/record_interactions")
        self.assertEqual(item["destination"]["link"]["video_id"], "abcdefghijk")
        self.assertFalse(item["read"])
        self.assertEqual(menu["continuation"]["body"]["ctoken"], "next")

    def test_comment_and_post_urls_are_preserved_for_link_router(self):
        for url in ("/watch?v=abcdefghijk&lc=comment-id", "/post/post-id"):
            target = browser.destination({"watchEndpoint": {"videoId": "abcdefghijk"},
                                          "commandMetadata": {"webCommandMetadata": {"url": url}}})
            self.assertEqual(target, {"url": browser.ORIGIN + url})
        self.assertEqual(browser.destination({"urlEndpoint": {"url": "javascript:bad()"}}), {})
        self.assertEqual(browser.destination({"getCommentsFromInboxCommand": {
            "videoId": "abcdefghijk", "linkedCommentId": "comment-id"}}),
            {"url": browser.ORIGIN + "/watch?v=abcdefghijk&lc=comment-id"})

    def test_unknown_actions_are_not_executed(self):
        self.assertIsNone(browser.action({"commandMetadata": {"webCommandMetadata": {"apiUrl": "https://evil.test"}}}))
        with self.assertRaises(browser.Notice):
            browser.run({"operation": "interact", "action": {"endpoint": "https://evil.test"}})

    def test_dry_run_sends_no_menu_or_writes(self):
        with patch.dict(browser.os.environ, YT_RS_DRY_RUN="1"), patch.object(browser, "YouTube") as youtube:
            self.assertEqual(browser.run({"operation": "menu"})["status"], "dry_run")
            self.assertTrue(browser.run({"operation": "interact", "action": {
                "endpoint": "notification/record_interactions"}})["dry_run"])
            youtube.assert_not_called()

    def test_count_reads_action_and_does_not_open_menu(self):
        with patch.object(browser, "default_browser", return_value="firefox.desktop"), \
                patch.object(browser, "browser_cookies"), patch.object(browser, "YouTube") as youtube:
            youtube.return_value.call.return_value = {"actions": [{"updateNotificationsUnseenCountAction": {"unseenCount": 12}}]}
            self.assertEqual(browser.run({"operation": "count"})["count"], 12)
            youtube.return_value.call.assert_called_once_with("notification/get_unseen_count", {})

    def test_process_errors_do_not_leak_credentials(self):
        output = io.StringIO()
        with patch.object(browser.sys, "stdin", io.StringIO('{}')), patch.object(browser.sys, "stdout", output), \
                patch.object(browser, "run", side_effect=RuntimeError("secret-cookie")):
            browser.main()
        self.assertNotIn("secret-cookie", output.getvalue())
        self.assertEqual(json.loads(output.getvalue())["status"], "service_error")


if __name__ == "__main__":
    unittest.main()
