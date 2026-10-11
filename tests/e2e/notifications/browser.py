"""A subprocess fixture. Never opens a browser database or the network."""
import json
import sys
import time

request = json.load(sys.stdin)
mode = request.get("fixture_mode")
operation = request["operation"]
if mode == "slow":
    time.sleep(1)
if mode == "missing":
    result = {"status": "sign_in_required", "message": "Sign in to YouTube in your default browser to see your notifications here."}
elif mode == "error":
    result = {"status": "network_error", "message": "Couldn't connect to YouTube. Check your connection and try again."}
elif operation == "count":
    result = {"status": "ok", "count": 12, "browser": "firefox"}
elif operation == "interact":
    result = {"status": "ok"}
else:
    action = {"endpoint": "notification/record_interactions", "body": {"serializedInteractionsRequest": "test-only"}}
    items = [
        {"id": "upload", "message": "A favorite channel uploaded: Exploring the night sky", "sent": "2 hours ago", "read": False,
         "destination": {"link": {"kind": "video", "video_id": "abcdefghijk"}}, "record_click": action,
         "menu": [{"label": "Hide this notification", "action": action}]},
        {"id": "reply", "message": "Someone replied to your comment: Thanks for watching!", "sent": "3 hours ago", "read": True,
         "destination": {"url": "https://www.youtube.com/watch?v=abcdefghijk&lc=comment"}, "record_click": action},
    ]
    if operation == "more":
        items = [items[0], {"id": "post", "message": "A channel shared a community post", "sent": "Yesterday", "read": False,
                           "destination": {"url": "https://www.youtube.com/post/example"}}]
    result = {"status": "ok", "browser": "firefox", "items": [] if mode == "empty" else items}
    if operation == "menu" and mode != "empty":
        result["continuation"] = {"endpoint": "notification/get_notification_menu", "body": {"ctoken": "next"}}
print(json.dumps(result))
