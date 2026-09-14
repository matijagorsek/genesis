#!/usr/bin/env node
// {{name}}: a small web app. Static files from ./public, a JSON API, data in data.json. No dependencies.
//   node server.js --port 5500
const http = require("http"); const fs = require("fs"); const path = require("path");
const port = Number(process.argv[process.argv.indexOf("--port") + 1] || 5500);
const DATA = path.join(__dirname, "data.json"); const PUB = path.join(__dirname, "public");
const load = () => { try { return JSON.parse(fs.readFileSync(DATA, "utf8")); } catch { return { items: [] }; } };
const save = (s) => { fs.writeFileSync(DATA + ".tmp", JSON.stringify(s, null, 2)); fs.renameSync(DATA + ".tmp", DATA); };
const json = (res, status, body) => { const b = JSON.stringify(body); res.writeHead(status, { "Content-Type": "application/json", "Content-Length": Buffer.byteLength(b) }); res.end(b); };
const types = { ".html": "text/html; charset=utf-8", ".js": "text/javascript", ".css": "text/css" };
http.createServer((req, res) => {
  const url = new URL(req.url, "http://x");
  if (req.method === "GET" && url.pathname === "/api/items") return json(res, 200, load().items);
  if (req.method === "POST" && url.pathname === "/api/items") {
    let body = ""; req.on("data", (c) => (body += c)); req.on("end", () => {
      let text = ""; try { text = String(JSON.parse(body || "{}").text || "").trim(); } catch { return json(res, 400, { error: "bad json" }); }
      if (!text) return json(res, 400, { error: "text is required" });
      const s = load(); const item = { id: Math.max(0, ...s.items.map((i) => i.id)) + 1, text, done: false }; s.items.push(item); save(s); json(res, 201, item);
    }); return;
  }
  const m = url.pathname.match(/^\/api\/items\/(\d+)\/toggle$/);
  if (req.method === "POST" && m) { const s = load(); const it = s.items.find((i) => i.id === Number(m[1])); if (!it) return json(res, 404, { error: "no such item" }); it.done = !it.done; save(s); return json(res, 200, it); }
  const file = path.normalize(path.join(PUB, url.pathname === "/" ? "index.html" : url.pathname));
  if (!file.startsWith(PUB) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) return json(res, 404, { error: "not found" });
  res.writeHead(200, { "Content-Type": types[path.extname(file)] || "application/octet-stream" }); fs.createReadStream(file).pipe(res);
}).listen(port, "127.0.0.1", () => console.log(`{{name}} on http://127.0.0.1:${port}/`));
