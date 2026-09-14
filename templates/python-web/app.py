#!/usr/bin/env python3
"""{{name}}: a small web app with a JSON API, served by Python's standard library.

  python3 app.py --port 5400      then open http://127.0.0.1:5400/
The API keeps a list of items in data.json next to this file. Extend `api_get` / `api_post` and static/index.html.
"""
import argparse
import json
import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HERE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.join(HERE, "data.json")
STATIC = os.path.join(HERE, "static")


def load():
    try:
        with open(DATA, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, json.JSONDecodeError):
        return {"items": []}


def save(state):
    tmp = DATA + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(state, f, indent=2)
    os.replace(tmp, DATA)


def api_get(path, state):
    if path == "/api/items":
        return 200, state["items"]
    return 404, {"error": "not found"}


def api_post(path, body, state):
    if path == "/api/items":
        text = str(body.get("text", "")).strip()
        if not text:
            return 400, {"error": "text is required"}
        item = {"id": (max([i["id"] for i in state["items"]] + [0]) + 1), "text": text, "done": False}
        state["items"].append(item)
        save(state)
        return 201, item
    if path.startswith("/api/items/") and path.endswith("/toggle"):
        try:
            item_id = int(path.split("/")[3])
        except ValueError:
            return 400, {"error": "bad id"}
        for it in state["items"]:
            if it["id"] == item_id:
                it["done"] = not it["done"]
                save(state)
                return 200, it
        return 404, {"error": "no such item"}
    return 404, {"error": "not found"}


class Handler(BaseHTTPRequestHandler):
    def _json(self, status, payload):
        data = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        if self.path.startswith("/api/"):
            status, payload = api_get(self.path, load())
            return self._json(status, payload)
        name = "index.html" if self.path in ("/", "") else self.path.lstrip("/")
        full = os.path.normpath(os.path.join(STATIC, name))
        if not full.startswith(STATIC) or not os.path.isfile(full):
            return self._json(404, {"error": "not found"})
        ctype = {"html": "text/html; charset=utf-8", "js": "text/javascript", "css": "text/css"}.get(full.rsplit(".", 1)[-1], "application/octet-stream")
        with open(full, "rb") as f:
            data = f.read()
        self.send_response(200)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        length = int(self.headers.get("Content-Length") or 0)
        try:
            body = json.loads(self.rfile.read(length) or b"{}")
        except json.JSONDecodeError:
            return self._json(400, {"error": "bad json"})
        status, payload = api_post(self.path, body, load())
        self._json(status, payload)

    def log_message(self, fmt, *args):  # quieter dev server
        pass


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--port", type=int, default=5400)
    args = p.parse_args()
    srv = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    print(f"{{name}} on http://127.0.0.1:{args.port}/", flush=True)
    srv.serve_forever()


if __name__ == "__main__":
    main()
