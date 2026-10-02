#!/usr/bin/env python3
"""Local stand-in for Cloudflare's Web Search endpoint, used by demo.sh --mock.

It speaks the documented request/response shapes, so the demo exercises the
real `ws` binary end to end without credentials or billing. A gateway named
"unfunded" answers with the HTTP 402 body observed from the live API.
"""
import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

SNIPPETS = {
    "ceramic": "Ceramic returns long excerpts from its own index of the web.",
    "exa": "Exa returns highlights chosen by its 'auto' search type.",
    "linkup": "Linkup returns raw results from its 'fast' search depth.",
}


class Handler(BaseHTTPRequestHandler):
    def do_POST(self):  # noqa: N802 (http.server API)
        body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))))
        gateway = body["options"]["gateway"]["id"]
        if gateway == "unfunded":
            return self.reply(402, {"ok": False, "error": {
                "category": "gateway", "code": "web_search_payment_required",
                "status": 402, "retryable": False,
                "gatewayRequestId": "00000000-0000-4000-8000-000000000000"}})
        provider = body.get("provider", "ceramic")
        items = [{
            "url": f"https://example.com/{provider}/{n}",
            "title": f"{body['query']} - result {n} from {provider}",
            "description": SNIPPETS[provider],
        } for n in range(1, body.get("limit", 10) + 1)]
        self.reply(200, {"items": items, "metadata": {
            "query": body["query"], "requestId": f"mock-{provider}-{gateway}", "latencyMs": 12.5}})

    def reply(self, status, payload):
        data = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *_):
        pass


if __name__ == "__main__":
    HTTPServer(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
