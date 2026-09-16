#!/usr/bin/python3
"""{{name}}: a small JSON API on the Python standard library.

Routes (extend `ROUTES` below):
  GET  /api/health          -> {"ok": true}
  GET  /api/items           -> the list
  POST /api/items {"text"}  -> adds one, returns it
  DELETE /api/items/<id>    -> removes one
Data lives in items.json next to this file. Run: python3 app.py --port 5500
"""
import argparse, json, os, re
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

DATA = os.path.join(os.path.dirname(os.path.abspath(__file__)), "items.json")


def load():
    try:
        return json.load(open(DATA))
    except (OSError, ValueError):
        return []


def save(items):
    tmp = DATA + ".tmp"
    with open(tmp, "w") as f:
        json.dump(items, f, indent=1)
    os.replace(tmp, DATA)


# ---- the API: (method, path pattern) -> handler(match, body) returning (status, object) ----

def health(_m, _b):
    return 200, {"ok": True}


def list_items(_m, _b):
    return 200, load()


def add_item(_m, body):
    text = (body or {}).get("text", "").strip()
    if not text:
        return 400, {"error": "text is required"}
    items = load()
    item = {"id": (max([i["id"] for i in items] or [0]) + 1), "text": text}
    items.append(item)
    save(items)
    return 201, item


def delete_item(m, _b):
    wanted = int(m.group(1))
    items = load()
    kept = [i for i in items if i["id"] != wanted]
    if len(kept) == len(items):
        return 404, {"error": "no such item"}
    save(kept)
    return 200, {"deleted": wanted}


ROUTES = [
    ("GET", re.compile(r"^/api/health$"), health),
    ("GET", re.compile(r"^/api/items$"), list_items),
    ("POST", re.compile(r"^/api/items$"), add_item),
    ("DELETE", re.compile(r"^/api/items/(\d+)$"), delete_item),
]


def dispatch(method, path, body):
    for m, pat, fn in ROUTES:
        match = pat.match(path)
        if m == method and match:
            return fn(match, body)
    return 404, {"error": "not found"}


class Handler(BaseHTTPRequestHandler):
    def _send(self, status, obj):
        data = json.dumps(obj).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def _body(self):
        n = int(self.headers.get("Content-Length", 0) or 0)
        try:
            return json.loads(self.rfile.read(n) or b"{}")
        except ValueError:
            return {}

    def do_GET(self):
        self._send(*dispatch("GET", self.path.split("?")[0], None))

    def do_POST(self):
        self._send(*dispatch("POST", self.path.split("?")[0], self._body()))

    def do_DELETE(self):
        self._send(*dispatch("DELETE", self.path.split("?")[0], None))

    def do_OPTIONS(self):
        self.send_response(204)
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "GET, POST, DELETE, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "Content-Type")
        self.end_headers()

    def log_message(self, fmt, *args):
        print("%s %s" % (self.command, self.path))


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--port", type=int, default=5500)
    a = ap.parse_args()
    print(f"{{name}} API on http://127.0.0.1:{a.port}/api/items")
    ThreadingHTTPServer(("127.0.0.1", a.port), Handler).serve_forever()


if __name__ == "__main__":
    main()
