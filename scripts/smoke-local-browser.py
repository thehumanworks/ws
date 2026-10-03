#!/usr/bin/env python3
"""Smoke a release executable against a local JS page, without internet/billing."""
import http.server
import json
import pathlib
import subprocess
import sys
import tempfile
import threading

binary = pathlib.Path(sys.argv[1]).resolve()
html = b"<html><head><title>Release smoke</title></head><body><main>Initial</main><script src='/script.js'></script></body></html>"

class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        script = self.path == "/script.js"
        body = b"document.querySelector('main').textContent='Rendered by bundled Lightpanda';" if script else html
        self.send_response(200)
        self.send_header("Content-Type", "application/javascript" if script else "text/html")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass

server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
thread = threading.Thread(target=server.serve_forever, daemon=True)
thread.start()
try:
    with tempfile.TemporaryDirectory(prefix="ws-release-smoke-") as temporary:
        env = {"WS_CACHE_DIR": temporary, "WS_CONFIG": str(pathlib.Path(temporary) / "config.toml"), "CLOUDFLARE_API_TOKEN": "malformed unused token", "WS_PROVIDER": "invalid-unused-provider"}
        url = f"http://127.0.0.1:{server.server_port}/"
        def run(*args):
            result = subprocess.run([str(binary), "fetch", url, "--backend", "lightpanda", "--allow-private", "--timeout", "10", *args], env=env, capture_output=True, timeout=15)
            if result.returncode:
                raise SystemExit(result.stderr.decode())
            return result.stdout
        page = json.loads(run("--format", "json"))
        assert page["rendered"] and "Rendered by bundled Lightpanda" in page["markdown"]
        png = run("--format", "png", "--render", "--raw")
        assert png.startswith(b"\x89PNG\r\n\x1a\n") and png[12:16] == b"IHDR"
        print(f"PASS bundled release JS and binary PNG ({len(png)} bytes)")
finally:
    server.shutdown()
    server.server_close()
    thread.join()
