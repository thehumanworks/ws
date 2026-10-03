#!/usr/bin/env python3
"""Local stand-in for Cloudflare's Web Search endpoint, used by demo.sh --mock.
It also serves one small HTML page (any GET) for the `ws fetch` part of the tour,
and answers Browser Run's content endpoint with that page plus a line "added by
JavaScript", for `ws fetch --render`.

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


PAGE = b"""<!doctype html>
<html><head><title>Rust async runtimes, compared</title>
<style>body { font-family: sans-serif }</style>
<script>console.log("never shown")</script></head>
<body>
<header><a href="/">Example Blog</a></header>
<nav><a href="/posts">Posts</a> <a href="/about">About</a></nav>
<main>
<h1>Rust async runtimes</h1>
<p>An <em>async runtime</em> polls futures. The common choices:</p>
<ul><li><a href="https://tokio.rs">Tokio</a>: multi-threaded, the default pick</li>
<li><a href="https://github.com/smol-rs/smol">smol</a>: small and modular</li></ul>
<div id="comments"></div>
</main>
<aside class="sidebar">Subscribe to the newsletter</aside>
<footer>Copyright Example Blog</footer>
</body></html>
"""


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):  # noqa: N802 (http.server API)
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(PAGE)))
        self.end_headers()
        self.wfile.write(PAGE)

    def do_POST(self):  # noqa: N802 (http.server API)
        body = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))))
        if self.path.endswith("/browser-run/content"):
            html = PAGE.decode().replace(
                '<div id="comments"></div>', "<p>2 comments, loaded by JavaScript.</p>")
            return self.reply(200, {"success": True, "result": html})
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
