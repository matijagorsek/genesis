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

function api(method, p, body) {
  const base = new URL(vscode.workspace.getConfiguration("genesis").get("agentd") || "http://127.0.0.1:11520");
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

function workspaceFolder() {
  const f = vscode.workspace.workspaceFolders;
  return f && f.length ? f[0].uri.fsPath : "";
}

/** The same plain verbs as the desktop's permission cards. */
function verb(tool, a) {
  a = a || {};
  if (tool === "shell") { const c = (a.command || "").trim(); if (/\b(pip3?|npm|dnf5?|flatpak|cargo|apt)\b.*\binstall\b/.test(c)) return ["install software", c]; if (a.needs_network) return ["use the network", c]; return ["run a command", c]; }
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
      if (m.type === "openFolder" && m.path) vscode.commands.executeCommand("vscode.openFolder", vscode.Uri.file(m.path), { forceNewWindow: false });
      if (m.type === "ready") this.health();
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
  }
  async make(text) {
    text = (text || "").trim(); if (!text) return;
    const project = workspaceFolder();
    let mode = "auto_edit";
    try { mode = JSON.parse(fs.readFileSync(path.join(process.env.XDG_CONFIG_HOME || path.join(process.env.HOME || "", ".config"), "genesis", "settings.json"), "utf8")).default_mode || mode; } catch {}
    const r = await api("POST", "/api/sessions", { mode, project });
    if (r.status !== 200 || !r.body.id) { this.post({ type: "error", text: (r.body && r.body.error) || "could not start" }); return; }
    this.session = r.body.id; this.seen = 0; this.answered = new Set();
    this.post({ type: "started", id: this.session, text });
    await api("POST", `/api/sessions/${this.session}/prompt`, { text });
    this.start();
  }
  attach(id) { this.session = id; this.seen = 0; this.answered = new Set(); this.post({ type: "started", id, text: "" }); this.start(); }
  start() { this.stop(); this.timer = setInterval(() => this.poll(), 1200); this.poll(); }
  stop() { if (this.timer) clearInterval(this.timer); this.timer = null; }
  async poll() {
    if (!this.session) return;
    const r = await api("GET", `/api/sessions/${this.session}`);
    if (r.status !== 200) return;
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
    if (r.status !== 200) { out.appendLine((r.body && r.body.error) || "could not start"); return; }
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
  ctx.subscriptions.push(vscode.commands.registerCommand("genesis.openMaker", () => { const { spawn } = require("child_process"); spawn("genesis-window", ["http://127.0.0.1:11520/"], { detached: true, stdio: "ignore" }).unref(); }));
  ctx.subscriptions.push(vscode.commands.registerCommand("genesis.settings", () => { const { spawn } = require("child_process"); spawn("genesis-window", ["http://127.0.0.1:11520/settings"], { detached: true, stdio: "ignore" }).unref(); }));
  const sb = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 100);
  sb.text = "$(sparkle) Genesis"; sb.tooltip = "Make something with Genesis (Ctrl+Alt+Space)"; sb.command = "genesis.make"; sb.show(); ctx.subscriptions.push(sb);
}
function deactivate() {}
module.exports = { activate, deactivate };
