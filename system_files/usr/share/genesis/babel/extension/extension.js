// Genesis in Babel: the maker as a sidebar, permission cards in the editor, ask about the selection.
// Plain JavaScript on purpose: no build step, ships as a built-in extension of Babel.
// Talks to genesis-agentd on 127.0.0.1 with the per-boot token (the daemon's front door), never to the network.
const vscode = require("vscode");
const http = require("http");
const fs = require("fs");
const path = require("path");

function token() {
  const rt = process.env.XDG_RUNTIME_DIR || "/tmp";
  try { return fs.readFileSync(path.join(rt, "genesis", "agentd.token"), "utf8").trim(); } catch { return ""; }
}

// every account has its own maker daemon, on the port it writes next to its token
function agentdPort() {
  const rt = process.env.XDG_RUNTIME_DIR || "/tmp";
  try { return parseInt(fs.readFileSync(path.join(rt, "genesis", "agentd.port"), "utf8").trim(), 10) || 11520; } catch { return 11520; }
}

function api(method, p, body) {
  const base = new URL(vscode.workspace.getConfiguration("genesis").get("agentd") || ("http://127.0.0.1:" + agentdPort()));
  return new Promise((resolve) => {
    const data = body ? JSON.stringify(body) : null;
    const req = http.request({ host: base.hostname, port: base.port || 80, path: p, method, headers: {
      "Content-Type": "application/json", "X-Genesis-Token": token(), ...(data ? { "Content-Length": Buffer.byteLength(data) } : {}) }, timeout: 15000 }, (res) => {
      let t = ""; res.on("data", (c) => t += c); res.on("end", () => { try { resolve({ status: res.statusCode, body: JSON.parse(t || "{}") }); } catch { resolve({ status: res.statusCode, body: {} }); } });
    });
    req.on("error", (e) => resolve({ status: 0, body: { error: "the maker is not running: " + e.message } }));
    req.on("timeout", () => { req.destroy(); resolve({ status: 0, body: { error: "the maker did not answer" } }); });
    if (data) req.write(data);
    req.end();
  });
}

// ---- Local completion: fill-in-the-middle on the model service (llama.cpp /infill through the router).
// The small hot model of every pack answers this; a pack with a dedicated "fim" model uses that instead.
// Nothing leaves the machine: the router only listens on 127.0.0.1.
const completion = { enabled: true, model: null, modelsAt: 0, downUntil: 0, last: { key: "", text: "" }, sb: null, out: null };
function clog(m) { try { if (!completion.out) completion.out = vscode.window.createOutputChannel("Genesis completion"); completion.out.appendLine(new Date().toISOString().slice(11, 19) + " " + m); } catch {} }
// the key the model service requires (/etc/genesis/router.key); a development router takes none
function routerAuth() { try { const k = require("fs").readFileSync("/etc/genesis/router.key", "utf8").trim(); return k ? { Authorization: "Bearer " + k } : {}; } catch { return {}; } }
function routerBase() { return new URL(vscode.workspace.getConfiguration("genesis").get("router") || "http://127.0.0.1:8080"); }
function routerJson(method, p, body, ms, signal) {
  const base = routerBase();
  return new Promise((resolve) => {
    const data = body ? JSON.stringify(body) : null;
    const req = http.request({ host: base.hostname, port: base.port || 80, path: p, method, headers: { "Content-Type": "application/json", ...routerAuth(), ...(data ? { "Content-Length": Buffer.byteLength(data) } : {}) }, timeout: ms }, (res) => {
      let t = ""; res.on("data", (c) => t += c); res.on("end", () => { try { resolve({ status: res.statusCode, body: JSON.parse(t || "{}") }); } catch { resolve({ status: res.statusCode, body: {} }); } });
    });
    req.on("error", () => resolve({ status: 0, body: {} }));
    req.on("timeout", () => { req.destroy(); resolve({ status: 0, body: {} }); });
    if (signal) signal.onCancellationRequested(() => { req.destroy(); resolve({ status: 0, body: {} }); });
    if (data) req.write(data);
    req.end();
  });
}
async function completionModel() {
  const now = Date.now();
  if (completion.model && now - completion.modelsAt < 60000) return completion.model;
  const r = await routerJson("GET", "/v1/models", null, 3000);
  const ids = ((r.body && r.body.data) || []).map((m) => m.id);
  completion.modelsAt = now;
  completion.model = ["fim", "fast", "code"].find((m) => ids.includes(m)) || null;
  return completion.model;
}
function updateCompletionBar() {
  const sb = completion.sb; if (!sb) return;
  if (!completion.enabled) { sb.text = "$(circle-slash) completion off"; sb.tooltip = "Local completion is off. Click to turn it on."; }
  else if (Date.now() < completion.downUntil || !completion.model) { sb.text = "$(debug-disconnect) no model"; sb.tooltip = "Local completion needs the model service. Set up models from the app menu, or wait for it to start."; }
  else { sb.text = `$(lightbulb) ${completion.model} completion`; sb.tooltip = `Inline suggestions from the local ${completion.model} model. Nothing leaves this machine. Click to turn off.`; }
  sb.show();
}
class LocalCompletion {
  async provideInlineCompletionItems(doc, pos, ctx, cancel) {
    if (!completion.enabled || Date.now() < completion.downUntil) return [];
    clog(`asked at ${pos.line}:${pos.character} (${ctx.triggerKind})`);
    const line = doc.lineAt(pos.line).text;
    if (line.slice(pos.character).trim()) return []; // only complete at the end of what is typed
    await new Promise((r) => setTimeout(r, 250)); if (cancel.isCancellationRequested) return [];
    const model = await completionModel(); if (!model) { updateCompletionBar(); return []; }
    const start = doc.positionAt(Math.max(0, doc.offsetAt(pos) - 2400));
    const end = doc.positionAt(Math.min(doc.getText().length, doc.offsetAt(pos) + 800));
    const prefix = doc.getText(new vscode.Range(start, pos));
    const suffix = doc.getText(new vscode.Range(pos, end));
    const key = doc.uri.toString() + " " + prefix + " " + suffix;
    if (completion.last.key === key && completion.last.text) return [new vscode.InlineCompletionItem(completion.last.text, new vscode.Range(pos, pos))];
    const t0 = Date.now();
    const r = await routerJson("POST", "/infill", { model, input_prefix: prefix, input_suffix: suffix, n_predict: 48, temperature: 0.2, top_p: 0.9, stop: ["\n\n\n"], cache_prompt: true }, 20000, cancel);
    clog(`infill ${model}: status ${r.status} in ${Date.now() - t0} ms, ${((r.body && r.body.content) || "").length} chars, cancelled=${cancel.isCancellationRequested}`);
    if (cancel.isCancellationRequested) return []; // the editor moved on (more typing, a popup): not a failure
    if (r.status !== 200) { if (r.status === 0) { completion.downUntil = Date.now() + 30000; completion.model = null; } updateCompletionBar(); return []; }
    updateCompletionBar();
    let text = ((r.body && r.body.content) || "").split("\r").join("");
    if (!text.trim()) return [];
    // never repeat what already follows the cursor
    const after = suffix.split("\n")[0].trimEnd();
    if (after && text.trimEnd().endsWith(after)) text = text.slice(0, text.lastIndexOf(after));
    if (!text.trim()) return [];
    completion.last = { key, text };
    return [new vscode.InlineCompletionItem(text, new vscode.Range(pos, pos))];
  }
}

function workspaceFolder() {
  const f = vscode.workspace.workspaceFolders;
  return f && f.length ? f[0].uri.fsPath : "";
}

/** The same plain verbs as the desktop's permission cards. */
function verb(tool, a) {
  a = a || {};
  if (tool === "shell") { const c = (a.command || "").trim(); if (/\b(pip3?|npm|dnf5?|flatpak|cargo|apt)\b.*\binstall\b/.test(c)) return ["install software", c]; if (a.needs_network) return ["use the network", c]; return ["run a command", c]; }
  if (tool.startsWith("mcp__")) { const m = tool.split("__"); return ["use your " + (m[1] || "") + " tool: " + (m.slice(2).join("__") || ""), JSON.stringify(a).slice(0, 200)]; }
  if (tool === "write_file" || tool === "edit_file") return ["change a file outside the project", a.path || ""];
  if (tool === "read_file" || tool === "list_dir") return ["read a file", a.path || ""];
  if (tool === "browser_open") return ["open a web page", a.url || ""];
  if (tool === "install_app") return ["add an app to your menu", a.display_name || ""];
  if (tool === "scaffold") return ["create the project " + (a.name || ""), "from the " + (a.template || "") + " template"];
  if (tool === "preview_start") return ["run the project", a.path || ""];
  return [tool.replace(/_/g, " "), JSON.stringify(a).slice(0, 120)];
}

class MakerView {
  constructor(ctx) { this.ctx = ctx; this.view = null; this.session = null; this.timer = null; this.seen = 0; this.answered = new Set(); }
  resolveWebviewView(view) {
    this.view = view;
    view.webview.options = { enableScripts: true };
    view.webview.html = this.html();
    view.webview.onDidReceiveMessage(async (m) => {
      if (m.type === "make") await this.make(m.text);
      if (m.type === "steer") await api("POST", `/api/sessions/${this.session}/prompt`, { text: m.text });
      if (m.type === "answer") { this.answered.add(m.id); await api("POST", `/api/prompts/${m.id}`, { allow: !!m.allow }); }
      if (m.type === "undo") await api("POST", `/api/sessions/${this.session}/undo`, {});
      if (m.type === "open") vscode.commands.executeCommand("genesis.openMaker");
      if (m.type === "openFolder" && m.path) vscode.commands.executeCommand("vscode.openFolder", vscode.Uri.file(m.path), { forceNewWindow: m.path !== workspaceFolder() });
      if (m.type === "ready") { this.health(); if (this.session) { this.seen = 0; this.post({ type: "started", id: this.session, text: this.pendingText || "" }); this.start(); } }
      if (m.type === "attach" && m.id) this.attach(m.id);
    });
    view.onDidDispose(() => { this.stop(); this.view = null; });
  }
  post(m) { if (this.view) this.view.webview.postMessage(m); }
  async health() {
    const h = await api("GET", "/api/health");
    this.post({ type: "health", ok: h.status === 200, router: !!(h.body && h.body.router_ok), model: h.body && h.body.model, error: h.body && h.body.error, folder: workspaceFolder() });
    const s = await api("GET", "/api/sessions");
    if (Array.isArray(s.body)) this.post({ type: "sessions", list: s.body.slice(-6).reverse() });
    const m = await api("GET", "/api/made");
    if (Array.isArray(m.body)) this.post({ type: "made", list: m.body.slice(0, 8) });
  }
  async make(text) {
    text = (text || "").trim(); if (!text) return;
    console.log("genesis: make", text.slice(0, 60));
    const project = workspaceFolder();
    let mode = "auto_edit";
    try { mode = JSON.parse(fs.readFileSync(path.join(process.env.XDG_CONFIG_HOME || path.join(process.env.HOME || "", ".config"), "genesis", "settings.json"), "utf8")).default_mode || mode; } catch {}
    const r = await api("POST", "/api/sessions", { mode, project });
    console.log("genesis: session", r.status, JSON.stringify(r.body).slice(0, 100));
    if (r.status < 200 || r.status >= 300 || !r.body.id) { this.post({ type: "error", text: (r.body && r.body.error) || "could not start" }); return; }
    this.session = r.body.id; this.seen = 0; this.answered = new Set(); this.pendingText = text;
    // the sidebar may still be opening (Ctrl+Alt+Space from anywhere): wait for it briefly
    for (let i = 0; i < 20 && !this.view; i++) await new Promise((res) => setTimeout(res, 150));
    this.post({ type: "started", id: this.session, text });
    const pr = await api("POST", `/api/sessions/${this.session}/prompt`, { text });
    console.log("genesis: prompt posted", pr.status, JSON.stringify(pr.body).slice(0, 120));
    if (pr.status >= 300 || pr.status === 0) this.post({ type: "error", text: (pr.body && pr.body.error) || ("could not send the request (" + pr.status + ")") });
    this.start();
  }
  attach(id) { this.session = id; this.seen = 0; this.answered = new Set(); this.post({ type: "started", id, text: "" }); this.start(); }
  start() { this.stop(); this.timer = setInterval(() => this.poll(), 1200); this.poll(); }
  stop() { if (this.timer) clearInterval(this.timer); this.timer = null; }
  async poll() {
    if (!this.session) return;
    const r = await api("GET", `/api/sessions/${this.session}`);
    if (r.status < 200 || r.status >= 300) return;
    const s = r.body;
    const events = (s.events || []).slice(this.seen); this.seen = (s.events || []).length;
    const pending = (s.pending || []).filter((p) => !this.answered.has(p.request_id)).map((p) => { const [v, d] = verb(p.tool, p.args); return { id: p.request_id, verb: v, detail: d, reason: (p.reason || "").replace(/^tier [A-Z0-9]+: /, "") }; });
    this.post({ type: "state", state: s.state, events, pending, preview: s.preview_url || null, project: s.active_project || s.project });
    if (pending.length && vscode.window.state.focused === false) {
      const p = pending[0];
      vscode.window.showInformationMessage(`Genesis wants to ${p.verb}${p.detail ? ": " + p.detail : ""}`, "Allow once", "Not now").then(async (c) => {
        if (!c) return; this.answered.add(p.id); await api("POST", `/api/prompts/${p.id}`, { allow: c === "Allow once" });
      });
    }
    if (s.state === "done" || s.state === "error") { this.stop(); if (s.state === "done") vscode.window.setStatusBarMessage("Genesis: done", 5000); }
  }
  html() {
    return `<!doctype html><html><head><meta charset="utf-8"><style>
    :root{--bg:var(--vscode-sideBar-background);--ink:var(--vscode-foreground);--ink2:var(--vscode-descriptionForeground);--acc:#5fb5bd;--line:var(--vscode-panel-border,#232c37);--panel:var(--vscode-editorWidget-background,#161d27);--warn:#e0a84a;--good:#6ccf94;--bad:#ef7b7b}
    body{margin:0;padding:10px 12px;font:13px/1.5 var(--vscode-font-family);color:var(--ink);background:var(--bg)}
    textarea{width:100%;box-sizing:border-box;min-height:64px;background:var(--vscode-input-background);color:var(--vscode-input-foreground);border:1px solid var(--vscode-input-border,var(--line));border-radius:6px;padding:8px;font:inherit;resize:vertical}
    button{background:var(--vscode-button-background);color:var(--vscode-button-foreground);border:0;border-radius:6px;padding:6px 12px;font:inherit;cursor:pointer;margin:4px 4px 0 0}
    button.sec{background:var(--vscode-button-secondaryBackground);color:var(--vscode-button-secondaryForeground)}
    .chip{display:inline-block;font-family:var(--vscode-editor-font-family);font-size:11px;padding:1px 8px;border-radius:999px;border:1px solid var(--line);color:var(--ink2);margin-right:6px}
    .chip.on{color:var(--good);border-color:var(--good)}.chip.off{color:var(--warn);border-color:var(--warn)}
    .card{background:var(--panel);border:1px solid var(--acc);border-radius:8px;padding:10px;margin:8px 0}.card b{display:block;margin-bottom:4px}.card .d{font-family:var(--vscode-editor-font-family);font-size:12px;color:var(--ink2);word-break:break-all}
    .steps{margin-top:8px;max-height:52vh;overflow:auto}.st{padding:4px 0;border-top:1px solid var(--line);font-size:12px;color:var(--ink2)}.st.a{color:var(--ink);font-size:13px}.st.u{color:var(--acc)}.st.e{color:var(--bad)}.st.d{color:var(--good)}.st code{font-family:var(--vscode-editor-font-family);font-size:11.5px}
    .muted{color:var(--ink2);font-size:12px}.recent button{display:block;width:100%;text-align:left;margin:4px 0;background:var(--panel);color:var(--ink);border:1px solid var(--line)}
    .hidden{display:none}
    </style></head><body>
    <div id="head"><span id="chip" class="chip">connecting…</span><span id="model" class="chip"></span></div>
    <p class="muted" id="folder"></p>
    <div id="start">
      <textarea id="q" placeholder="What should Genesis make or change here? e.g. add a --json flag to the CLI, write tests for parser.py, make a README"></textarea>
      <button id="go">Make it</button> <button class="sec" id="open">Open the maker window</button>
      <div class="recent" id="recent"></div>
      <div class="recent" id="made"></div>
    </div>
    <div id="job" class="hidden">
      <div><span id="state" class="chip"></span> <button class="sec" id="back">‹ New</button> <button class="sec" id="undo" title="Undo every change this job made outside the project">Undo</button> <a id="preview" class="hidden" href="#">preview</a></div>
      <div id="cards"></div>
      <div class="steps" id="steps"></div>
      <textarea id="steer" placeholder="Change something… (Enter to send)"></textarea>
    </div>
    <script>
    const vscode=acquireVsCodeApi();const $=s=>document.querySelector(s);
    function esc(x){return String(x==null?'':x).replace(/&/g,'&amp;').replace(/</g,'&lt;')}
    $('#go').onclick=()=>{const t=$('#q').value.trim();if(t)vscode.postMessage({type:'make',text:t})};
    $('#q').addEventListener('keydown',e=>{if(e.key==='Enter'&&!e.shiftKey){e.preventDefault();$('#go').click()}});
    $('#open').onclick=()=>vscode.postMessage({type:'open'});
    $('#back').onclick=()=>{$('#job').classList.add('hidden');$('#start').classList.remove('hidden');vscode.postMessage({type:'ready'})};
    $('#undo').onclick=()=>vscode.postMessage({type:'undo'});
    $('#steer').addEventListener('keydown',e=>{if(e.key==='Enter'&&!e.shiftKey){e.preventDefault();const t=$('#steer').value.trim();if(t){vscode.postMessage({type:'steer',text:t});$('#steer').value='';}}});
    window.addEventListener('message',ev=>{const m=ev.data;
      if(m.type==='health'){const c=$('#chip');c.textContent=m.ok?(m.router?'on this machine':'no models yet'):'maker not running';c.className='chip '+(m.ok&&m.router?'on':'off');$('#model').textContent=m.model?('model: '+m.model):'';$('#folder').textContent=m.folder?('Working in '+m.folder):'Open a folder to make things in it.';if(m.error)$('#folder').textContent=m.error}
      if(m.type==='made'){const r=$('#made');r.innerHTML=m.list.length?'<p class="muted">Made here</p>':'';m.list.forEach(x=>{const b=document.createElement('button');b.innerHTML='<b>'+esc(x.name)+'</b> <span class="muted">'+esc((x.prompts&&x.prompts[0])||x.template||'')+'</span>';b.title='Open in Babel';b.onclick=()=>vscode.postMessage({type:'openFolder',path:x.path});r.appendChild(b)})}
      if(m.type==='sessions'){const r=$('#recent');r.innerHTML=m.list.length?'<p class="muted">Recent</p>':'';m.list.forEach(s=>{const b=document.createElement('button');b.textContent=(s.state||'')+' · '+(s.project||'').split('/').pop();b.onclick=()=>vscode.postMessage({type:'attach',id:s.id});r.appendChild(b)})}
      if(m.type==='started'){$('#start').classList.add('hidden');$('#job').classList.remove('hidden');$('#steps').innerHTML=m.text?'<div class="st u">'+esc(m.text)+'</div>':'';$('#cards').innerHTML='';}
      if(m.type==='error'){alert(m.text)}
      if(m.type==='state'){const st=$('#state');st.textContent=m.state==='waiting'?'needs you':m.state;st.className='chip '+(m.state==='done'?'on':(m.state==='waiting'?'off':''));
        const steps=$('#steps');m.events.forEach(e=>{const d=document.createElement('div');let k=e.kind,t='';
          if(k==='assistant'){d.className='st a';t=esc(e.text)}else if(k==='user_prompt'){d.className='st u';t=esc(e.text)}else if(k==='tool_call'){d.className='st';t='› <code>'+esc(e.name)+'</code> '+esc(JSON.stringify(e.args||{}).slice(0,140))}else if(k==='tool_result'){d.className='st';t='<code>'+esc((e.summary||'').slice(0,240))+'</code>'}else if(k==='error'){d.className='st e';t='‼ '+esc(e.text)}else if(k==='done'){d.className='st d';t='done'}else if(k==='decision'){d.className='st';t='· '+esc(e.verdict)+' '+esc(e.reason||'')}else if(k==='snapshot'){d.className='st';t='· snapshot '+esc(e.detail||'')}else return;
          d.innerHTML=t;steps.appendChild(d)});if(m.events.length)steps.scrollTop=steps.scrollHeight;
        const cards=$('#cards');cards.innerHTML='';m.pending.forEach(p=>{const c=document.createElement('div');c.className='card';c.innerHTML='<b>Genesis wants to '+esc(p.verb)+'</b>'+(p.detail?'<div class="d">'+esc(p.detail)+'</div>':'')+(p.reason?'<div class="muted">'+esc(p.reason)+'</div>':'')+'<button data-a="1">Allow once</button><button class="sec" data-a="0">Not now</button>';c.querySelectorAll('button').forEach(b=>b.onclick=()=>{vscode.postMessage({type:'answer',id:p.id,allow:b.dataset.a==='1'});c.remove()});cards.appendChild(c)});
        const pv=$('#preview');if(m.preview){pv.classList.remove('hidden');pv.href=m.preview;pv.onclick=e=>{e.preventDefault();vscode.postMessage({type:'openFolder',path:''});}}}
    });
    vscode.postMessage({type:'ready'});
    </script></body></html>`;
  }
}

function activate(ctx) {
  const maker = new MakerView(ctx);
  ctx.subscriptions.push(vscode.window.registerWebviewViewProvider("genesis.maker", maker, { webviewOptions: { retainContextWhenHidden: true } }));
  const focus = () => vscode.commands.executeCommand("genesis.maker.focus");
  ctx.subscriptions.push(vscode.commands.registerCommand("genesis.make", async () => {
    const text = await vscode.window.showInputBox({ prompt: "What should Genesis make or change in this workspace?", placeHolder: "add a --json flag and a test" });
    if (text) { await focus(); await maker.make(text); }
  }));
  ctx.subscriptions.push(vscode.commands.registerCommand("genesis.changeFile", async () => {
    const ed = vscode.window.activeTextEditor; if (!ed) return;
    const rel = vscode.workspace.asRelativePath(ed.document.uri);
    const text = await vscode.window.showInputBox({ prompt: `What should change in ${rel}?`, placeHolder: "handle empty input, add docstrings" });
    if (text) { await focus(); await maker.make(`In ${rel}: ${text}`); }
  }));
  ctx.subscriptions.push(vscode.commands.registerCommand("genesis.askSelection", async () => {
    const ed = vscode.window.activeTextEditor; if (!ed) return;
    const sel = ed.document.getText(ed.selection); if (!sel.trim()) return;
    const q = await vscode.window.showInputBox({ prompt: "Ask about the selection", value: "What does this do?" });
    if (!q) return;
    const out = vscode.window.createOutputChannel("Genesis");
    out.show(true); out.appendLine(`> ${q}\n`);
    const sys = await api("GET", "/api/health");
    if (sys.status !== 200) { out.appendLine("The maker is not running."); return; }
    const project = workspaceFolder();
    const r = await api("POST", "/api/sessions", { mode: "assist", project });
    if (r.status < 200 || r.status >= 300) { out.appendLine((r.body && r.body.error) || "could not start"); return; }
    await api("POST", `/api/sessions/${r.body.id}/prompt`, { text: `Answer briefly, in plain text, without changing any file. ${q}\n\nSelection from ${vscode.workspace.asRelativePath(ed.document.uri)}:\n\n${sel.slice(0, 6000)}` });
    let seen = 0;
    const t = setInterval(async () => {
      const s = await api("GET", `/api/sessions/${r.body.id}`);
      const ev = (s.body.events || []).slice(seen); seen = (s.body.events || []).length;
      ev.forEach((e) => { if (e.kind === "assistant") out.appendLine(e.text); if (e.kind === "error") out.appendLine("‼ " + e.text); });
      (s.body.pending || []).forEach((p) => api("POST", `/api/prompts/${p.request_id}`, { allow: false }));
      if (s.body.state === "done" || s.body.state === "error") clearInterval(t);
    }, 1200);
  }));
  // Ctrl+I: select, describe, see it change; Undo (Ctrl+Z) drops it. Runs as a chat-kind session (no tools).
  ctx.subscriptions.push(vscode.commands.registerCommand("genesis.inline", async () => {
    const ed = vscode.window.activeTextEditor; if (!ed) return;
    const range = ed.selection.isEmpty ? ed.document.lineAt(ed.selection.active.line).range : ed.selection;
    const code = ed.document.getText(range);
    const ask = await vscode.window.showInputBox({ prompt: ed.selection.isEmpty ? "Change this line how?" : "Change the selection how?", placeHolder: "add error handling, rename to snake_case, make it async…" });
    if (!ask) return;
    const lang = ed.document.languageId;
    await vscode.window.withProgress({ location: vscode.ProgressLocation.Notification, title: "Genesis is rewriting…" }, async () => {
      const r = await api("POST", "/api/sessions", { mode: "assist", project: workspaceFolder(), kind: "chat" });
      if (r.status < 200 || r.status >= 300) { vscode.window.showErrorMessage((r.body && r.body.error) || "the maker is not running"); return; }
      const before = ed.document.getText(new vscode.Range(new vscode.Position(Math.max(0, range.start.line - 30), 0), range.start));
      const after = ed.document.getText(new vscode.Range(range.end, new vscode.Position(Math.min(ed.document.lineCount - 1, range.end.line + 30), 0)));
      await api("POST", `/api/sessions/${r.body.id}/prompt`, { text: `You are editing ${lang} code. Rewrite ONLY the code between the markers according to the instruction, keeping indentation and style. Reply with the replacement code only: no explanation, no markdown fences.\n\nInstruction: ${ask}\n\nContext before:\n${before}\n<<<START>>>\n${code}\n<<<END>>>\nContext after:\n${after}` });
      let text = null;
      for (let i = 0; i < 150; i++) {
        await new Promise((res) => setTimeout(res, 1000));
        const s = await api("GET", `/api/sessions/${r.body.id}`);
        (s.body.pending || []).forEach((p) => api("POST", `/api/prompts/${p.request_id}`, { allow: false }));
        if (s.body.state === "done" || s.body.state === "error") { const a = (s.body.events || []).filter((e) => e.kind === "assistant").pop(); text = a ? a.text : null; break; }
      }
      if (!text) { vscode.window.showWarningMessage("Genesis gave no answer"); return; }
      text = text.replace(/^```[a-zA-Z0-9_-]*\n?/m, "").replace(/\n?```\s*$/m, "").replace(/<<<START>>>|<<<END>>>/g, "").replace(/\s+$/, "");
      const ok = await ed.edit((b) => b.replace(range, text));
      if (ok) vscode.window.showInformationMessage("Changed. Ctrl+Z puts it back.", "Undo").then((c) => { if (c === "Undo") vscode.commands.executeCommand("undo"); });
    });
  }));
  // explain the diagnostic under the cursor, in the Genesis output channel
  ctx.subscriptions.push(vscode.commands.registerCommand("genesis.explainError", async () => {
    const ed = vscode.window.activeTextEditor; if (!ed) return;
    const diags = vscode.languages.getDiagnostics(ed.document.uri).filter((d) => d.range.contains(ed.selection.active) || d.range.start.line === ed.selection.active.line);
    if (!diags.length) { vscode.window.showInformationMessage("No problem reported on this line."); return; }
    const d = diags[0];
    const snippet = ed.document.getText(new vscode.Range(new vscode.Position(Math.max(0, d.range.start.line - 8), 0), new vscode.Position(Math.min(ed.document.lineCount - 1, d.range.end.line + 8), 0)));
    const out = vscode.window.createOutputChannel("Genesis"); out.show(true); out.appendLine(`> Explain this error: ${d.message}\n`);
    const r = await api("POST", "/api/sessions", { mode: "assist", project: workspaceFolder(), kind: "chat" });
    if (r.status < 200 || r.status >= 300) { out.appendLine("The maker is not running."); return; }
    await api("POST", `/api/sessions/${r.body.id}/prompt`, { text: `Explain this ${ed.document.languageId} error in two or three plain sentences and say the one change that fixes it. Error: ${d.message}\n\nCode around it:\n${snippet}` });
    let seen = 0; const t = setInterval(async () => { const s = await api("GET", `/api/sessions/${r.body.id}`); const ev = (s.body.events || []).slice(seen); seen = (s.body.events || []).length; ev.forEach((e) => { if (e.kind === "assistant") out.appendLine(e.text); }); if (s.body.state === "done" || s.body.state === "error") clearInterval(t); }, 1200);
  }));
  ctx.subscriptions.push(vscode.commands.registerCommand("genesis.openMaker", () => { const { spawn } = require("child_process"); spawn("genesis-window", ["http://127.0.0.1:11520/"], { detached: true, stdio: "ignore" }).unref(); }));
  ctx.subscriptions.push(vscode.commands.registerCommand("genesis.settings", () => { const { spawn } = require("child_process"); spawn("genesis-window", ["http://127.0.0.1:11520/settings"], { detached: true, stdio: "ignore" }).unref(); }));
  ctx.subscriptions.push(vscode.commands.registerCommand("genesis.welcome", () => {
    const lang = (vscode.env.language || "en").slice(0, 2);
    const localised = path.join(ctx.extensionPath, "media", `welcome.${lang}.md`);
    const uri = vscode.Uri.file(fs.existsSync(localised) ? localised : path.join(ctx.extensionPath, "media", "welcome.md"));
    vscode.commands.executeCommand("markdown.showPreview", uri);
  }));
  ctx.subscriptions.push(vscode.commands.registerCommand("genesis.openInBabel", async () => {
    const m = await api("GET", "/api/made");
    const items = (Array.isArray(m.body) ? m.body : []).map((x) => ({ label: x.name, description: (x.prompts && x.prompts[0]) || x.template || "", path: x.path }));
    if (!items.length) { vscode.window.showInformationMessage("Nothing made yet. Ask Genesis for something first."); return; }
    const pick = await vscode.window.showQuickPick(items, { placeHolder: "Open a project Genesis made" });
    if (pick) vscode.commands.executeCommand("vscode.openFolder", vscode.Uri.file(pick.path), { forceNewWindow: true });
  }));
  if (!ctx.globalState.get("welcomed")) {
    ctx.globalState.update("welcomed", true);
    setTimeout(() => { vscode.commands.executeCommand("genesis.welcome"); vscode.commands.executeCommand("genesis.maker.focus"); }, 1500);
  }
  const sb = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 100);
  sb.text = "$(sparkle) Genesis"; sb.tooltip = "Make something with Genesis (Ctrl+Alt+Space)"; sb.command = "genesis.make"; sb.show(); ctx.subscriptions.push(sb);
  // local completion: every language, the small model on this machine
  completion.enabled = vscode.workspace.getConfiguration("genesis").get("completion.enabled") !== false;
  completion.sb = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 50);
  completion.sb.command = "genesis.toggleCompletion"; ctx.subscriptions.push(completion.sb);
  ctx.subscriptions.push(vscode.languages.registerInlineCompletionItemProvider({ pattern: "**" }, new LocalCompletion()));
  ctx.subscriptions.push(vscode.commands.registerCommand("genesis.toggleCompletion", async () => {
    completion.enabled = !completion.enabled;
    await vscode.workspace.getConfiguration("genesis").update("completion.enabled", completion.enabled, vscode.ConfigurationTarget.Global);
    updateCompletionBar();
  }));
  completionModel().then(updateCompletionBar);
}
function deactivate() {}
module.exports = { activate, deactivate };
