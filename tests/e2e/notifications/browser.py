"""A subprocess fixture. Never opens a browser database or the network."""
import json
import sys
import time

request = json.load(sys.stdin)
mode = request.get("fixture_mode")
operation = request["operation"]
print(json.dumps({"status": "progress", "stage": "checking_browser"}), flush=True)
time.sleep(0.1)
print(json.dumps({"status": "progress", "stage": "fetching", "browser": "firefox"}), flush=True)
time.sleep(0.1)
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
        {"id": "upload", "message": 'You got 2 likes: "All fun and games until you get arrested with "one honest caveat" for doing a mission called "two things worth mentioning"."', "sent": "10 hours ago", "read": False,
         "destination": {"link": {"kind": "video", "video_id": "abcdefghijk"}}, "record_click": action,
         "menu": [{"label": "Hide this notification", "action": action}]},
        {"id": "reply", "message": "Someone replied to your comment: Thanks for watching!", "sent": "3 hours ago", "read": True,
         "destination": {"url": "https://www.youtube.com/watch?v=abcdefghijk&lc=comment"}, "record_click": action},
    ]
    for item in items:
        item["avatar"] = "fixture://avatar"
        item["thumbnail"] = "fixture://thumbnail"
    if mode == "geometry" and operation == "menu":
        items.extend(dict(items[0], id=f"geometry-{n}") for n in range(6))
    if operation == "more":
        items = [items[0], {"id": "post", "message": "A channel shared a community post", "sent": "Yesterday", "read": False,
                           "destination": {"url": "https://www.youtube.com/post/example"}}]
    result = {"status": "ok", "browser": "firefox", "items": [] if mode == "empty" else items}
    if operation == "menu" and mode != "empty":
        result["continuation"] = {"endpoint": "browse", "body": {"continuation": "next"}}
print(json.dumps(result))
