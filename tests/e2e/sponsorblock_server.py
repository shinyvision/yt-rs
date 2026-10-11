"""A stand-in for the SponsorBlock server, for the sponsorblock e2e test:
known segments for one video (one of each kind), and every POST (skips
counted, votes, submissions) written to a log, one JSON object a line.

Usage: python3 -I sponsorblock_server.py PORT LOG
"""

import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

VIDEO = "dQw4w9WgXcQ"
PREFIX = "5f6b"  # sha256(VIDEO)[:4]


def segment(category, action, start, end, uuid):
    return {
        "category": category,
        "actionType": action,
        "segment": [start, end],
        "UUID": uuid,
        "videoDuration": 213.0,
        "locked": 0,
        "votes": 3,
        "description": "",
    }


SEGMENTS = [
    {
        "videoID": VIDEO,
        "segments": [
            segment("exclusive_access", "full", 0, 0, "full-1"),
            segment("intro", "skip", 5, 8, "intro-1"),
            segment("sponsor", "skip", 40, 45, "sponsor-1"),
            segment("selfpromo", "skip", 60, 66, "selfpromo-1"),
            segment("sponsor", "mute", 75, 80, "mute-1"),
            segment("poi_highlight", "poi", 100, 100, "poi-1"),
            # Shorter than the test's minimum duration (3 s): left out.
            segment("filler", "skip", 150, 152, "filler-1"),
        ],
    },
    # Another video under the same prefix, which must be ignored.
    {"videoID": "zzzzzzzzzzz", "segments": [segment("sponsor", "skip", 1, 200, "other-1")]},
]


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def answer(self, status, body):
        data = body.encode() if isinstance(body, str) else json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        url = urlparse(self.path)
        if url.path == "/api/skipSegments/" + PREFIX:
            query = parse_qs(url.query)
            wanted = set(json.loads(query.get("categories", ["[]"])[0]))
            result = [
                {"videoID": v["videoID"], "segments": [s for s in v["segments"] if s["category"] in wanted]}
                for v in SEGMENTS
            ]
            self.answer(200, result)
        else:
            self.answer(404, "Not Found")

    def do_POST(self):
        url = urlparse(self.path)
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length).decode() if length else ""
        entry = {"path": url.path, "query": {k: v[0] for k, v in parse_qs(url.query).items()}}
        if body:
            entry["body"] = json.loads(body)
        with open(LOG, "a") as log:
            log.write(json.dumps(entry) + "\n")
        self.answer(200, "OK")


if __name__ == "__main__":
    PORT, LOG = int(sys.argv[1]), sys.argv[2]
    ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
