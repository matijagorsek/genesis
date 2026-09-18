"""Tests for the Python tools under system_files/usr/bin. They import the scripts by path and run them
against fake services on localhost; nothing touches the real machine. Run: python3 -m pytest -q tests/python"""
import importlib.machinery, importlib.util, json, os, subprocess, sys, threading, time, urllib.request
from http.server import BaseHTTPRequestHandler, HTTPServer

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
BIN = os.path.join(ROOT, "system_files", "usr", "bin")


def load(name):
    """Import a tool that has no .py extension (importlib needs the loader named explicitly)."""
    path = os.path.join(BIN, name)
    loader = importlib.machinery.SourceFileLoader(name.replace("-", "_"), path)
    spec = importlib.util.spec_from_loader(loader.name, loader)
    mod = importlib.util.module_from_spec(spec)
    loader.exec_module(mod)
    return mod


def serve(handler):
    srv = HTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv, f"http://127.0.0.1:{srv.server_port}"


# ---- genesis-index: words, meaning, rerank, removal -------------------------------------------

class FakeModels(BaseHTTPRequestHandler):
    """Embeddings as a bag of a tiny vocabulary with a synonym, and a reranker that prefers 'milk'."""
    VOCAB = ["lisbon", "tram", "milk", "eggs", "rust"]
    SYN = {"trip": ["lisbon", "tram"], "groceries": ["milk", "eggs"]}

    def log_message(self, *a): pass

    def vec(self, t):
        v = [0.0] * len(self.VOCAB)
        for w in t.lower().split():
            for s in [w.strip(",.")] + self.SYN.get(w.strip(",."), []):
                if s in self.VOCAB: v[self.VOCAB.index(s)] += 1
        return v

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        if self.path == "/v1/embeddings":
            inp = body["input"]; inp = [inp] if isinstance(inp, str) else inp
            out = {"data": [{"index": i, "embedding": self.vec(t)} for i, t in enumerate(inp)]}
        else:
            out = {"results": [{"index": i, "relevance_score": 0.9 if "milk" in d else 0.1} for i, d in enumerate(body["documents"])]}
        b = json.dumps(out).encode(); self.send_response(200); self.send_header("Content-Type", "application/json"); self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)


def test_index_words_meaning_rerank_and_remove(tmp_path, monkeypatch):
    srv, url = serve(FakeModels)
    monkeypatch.setenv("XDG_CONFIG_HOME", str(tmp_path / "conf")); monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "data")); monkeypatch.setenv("GENESIS_ROUTER", url)
    idx = load("genesis-index")
    notes = tmp_path / "notes"; notes.mkdir()
    (notes / "holiday.md").write_text("Our week in Lisbon\n\nWe took the tram.\n")
    (notes / "list.md").write_text("Shopping\n\nmilk, eggs\n")
    (notes / "rust.md").write_text("Rust notes\n\nownership\n")
    idx.save_conf({"folders": [str(notes)]}); r = idx.update()
    assert r["files"] == 3 and r["embedded"] == 3
    hits = idx.search("trip", n=2)
    assert hits and hits[0]["path"].endswith("holiday.md") or any(h["path"].endswith("holiday.md") for h in hits)
    assert any("meaning" in h["by"] for h in hits), "the synonym only works through embeddings"
    assert all("rerank" in h["by"] for h in idx.search("lisbon tram", n=3))
    idx.save_conf({"folders": []}); idx.update()
    c = idx.db()
    assert c.execute("select count(*) from chunks").fetchone()[0] == 0
    assert c.execute("select count(*) from vectors").fetchone()[0] == 0, "removing a folder removes its vectors too"
    srv.shutdown()


# ---- genesis-ollama: tags, streaming, tool-call fragments, embeddings, HTTP/1.1 ------------------

class FakeRouter(BaseHTTPRequestHandler):
    def log_message(self, *a): pass

    def do_GET(self):
        b = json.dumps({"data": [{"id": "fast"}, {"id": "embed"}]}).encode(); self.send_response(200); self.send_header("Content-Type", "application/json"); self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        if self.path == "/v1/embeddings":
            b = json.dumps({"data": [{"index": 0, "embedding": [0.5, 0.5]}]}).encode(); self.send_response(200); self.send_header("Content-Type", "application/json"); self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b); return
        self.send_response(200); self.send_header("Content-Type", "text/event-stream"); self.end_headers()
        if body.get("tools"):
            # one tool call, its arguments streamed in three fragments, as OpenAI-style servers do
            for frag in ['{"cit', 'y":"Lju', 'bljana"}']:
                self.wfile.write(("data: " + json.dumps({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "call1", "function": {"name": "weather", "arguments": frag}}]}}]}) + "\n\n").encode())
        else:
            for w in ["Hello", " there"]:
                self.wfile.write(("data: " + json.dumps({"choices": [{"delta": {"content": w}}]}) + "\n\n").encode())
        self.wfile.write(b"data: [DONE]\n\n"); self.wfile.flush()


def test_ollama_shim(tmp_path, monkeypatch):
    router, url = serve(FakeRouter)
    port = 20000 + os.getpid() % 10000
    env = dict(os.environ, GENESIS_ROUTER=url, GENESIS_OLLAMA_PORT=str(port))
    p = subprocess.Popen([sys.executable, os.path.join(BIN, "genesis-ollama")], env=env)
    try:
        for _ in range(50):
            try: urllib.request.urlopen(f"http://127.0.0.1:{port}/", timeout=1); break
            except Exception: time.sleep(0.1)
        base = f"http://127.0.0.1:{port}"
        tags = json.load(urllib.request.urlopen(base + "/api/tags"))
        assert [m["name"] for m in tags["models"]] == ["fast:latest", "embed:latest"]
        r = urllib.request.urlopen(urllib.request.Request(base + "/api/generate", data=json.dumps({"model": "fast", "prompt": "hi"}).encode()))
        assert r.version == 11, "chunked responses need HTTP/1.1 on the status line"
        lines = [json.loads(l) for l in r.read().decode().strip().splitlines()]
        assert "".join(l.get("response", "") for l in lines) == "Hello there" and lines[-1]["done"] is True
        r = urllib.request.urlopen(urllib.request.Request(base + "/api/chat", data=json.dumps({"model": "fast", "messages": [{"role": "user", "content": "x"}], "stream": False, "tools": [{"type": "function", "function": {"name": "weather"}}]}).encode()))
        msg = json.load(r)["message"]
        assert msg["tool_calls"] == [{"function": {"name": "weather", "arguments": {"city": "Ljubljana"}}}], msg
        e = json.load(urllib.request.urlopen(urllib.request.Request(base + "/api/embeddings", data=json.dumps({"model": "embed", "prompt": "x"}).encode())))
        assert e["embedding"] == [0.5, 0.5]
        # a request from a web page (cross-site) is refused
        req = urllib.request.Request(base + "/api/generate", data=b'{"model":"fast","prompt":"hi"}', headers={"Origin": "https://evil.example"})
        try:
            urllib.request.urlopen(req); assert False, "should be refused"
        except urllib.error.HTTPError as err:
            assert err.code == 403
    finally:
        p.terminate(); router.shutdown()


# ---- genesis-mcp-system: handshake, tools, id validation -----------------------------------------

def test_mcp_system_validates_ids_and_lists_tools():
    p = subprocess.Popen([sys.executable, os.path.join(BIN, "genesis-mcp-system")], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
    def rq(i, m, params=None):
        p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": i, "method": m, "params": params or {}}) + "\n"); p.stdin.flush(); return json.loads(p.stdout.readline())
    try:
        assert "result" in rq(1, "initialize")
        tools = rq(2, "tools/list")["result"]["tools"]
        assert len(tools) >= 16 and all("name" in t and "inputSchema" in t for t in tools)
        for bad in ["--all", "a b", "", "x"]:
            t = rq(3, "tools/call", {"name": "remove_app", "arguments": {"app_id": bad}})["result"]["content"][0]["text"]
            assert "give the Flatpak application id" in t, bad
    finally:
        p.stdin.close(); p.terminate()
