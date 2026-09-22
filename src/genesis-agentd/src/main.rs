//! genesis-agentd: the Genesis agent daemon.
//!
//!   genesis-agentd run --project DIR [--mode auto_edit] "task"   headless run, events on stderr, answer on stdout
//!   genesis-agentd serve                                        local API on 127.0.0.1:11520
//!
//! API:
//!   POST /api/sessions                {mode, project}            -> {id}
//!   POST /api/sessions/{id}/prompt    {text}                     -> {accepted} (runs in a thread)
//!   GET  /api/sessions/{id}                                      -> SessionInfo (state, events, pending prompts)
//!   POST /api/prompts/{request_id}    {allow: bool}              -> resolves a pending permission prompt
//!   POST /api/sessions/{id}/undo                                 -> rolls back the session's transaction
//!   GET  /api/health
//!   GET  /                          the workspace UI

mod agent;
mod browser;
mod voice;
mod llm;
mod maker;
mod mcp;
mod chat;
mod ocr;
mod timeline;
mod queue;
mod sandbox;

use agent::{Agent, Event, SessionInfo, Shared};
use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use genesis_permd::{default_audit_path, default_policy_sources, Broker, Mode, Policy};
use genesis_txd::{default_store, Store};
use llm::Client;
use serde::Deserialize;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use tiny_http::{Header, Method, Request, Response, Server};

// Each page is markup plus its own .js file, so the script can be diffed, checked and linted on its own;
// the daemon puts them back together when it serves the page.
const WORKSPACE_HTML: &str = include_str!("../ui/workspace.html");
const SETTINGS_HTML: &str = include_str!("../ui/settings.html");
const PALETTE_HTML: &str = include_str!("../ui/palette.html");
const CHAT_HTML: &str = include_str!("../ui/chat.html");
const WORKSPACE_JS: &str = include_str!("../ui/workspace.js");
const SETTINGS_JS: &str = include_str!("../ui/settings.js");
const PALETTE_JS: &str = include_str!("../ui/palette.js");
const CHAT_JS: &str = include_str!("../ui/chat.js");

fn page(html: &str, js: &str, token: &str) -> String {
    html.replace("__GENESIS_JS__", js).replace("__GENESIS_TOKEN__", token)
}

#[derive(Parser)]
#[command(name = "genesis-agentd", version, about = "Genesis agent daemon")]
struct Cli {
    /// OpenAI-compatible endpoint (the Genesis router).
    #[arg(long, global = true, default_value = "http://127.0.0.1:8080/v1", env = "GENESIS_ENDPOINT")]
    endpoint: String,
    #[arg(long, global = true, default_value = "code", env = "GENESIS_MODEL")]
    model: String,
    #[arg(long, global = true, env = "GENESIS_AUDIT")]
    audit: Option<PathBuf>,
    #[arg(long, global = true)]
    policy: Vec<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run one task headlessly.
    Run {
        #[arg(long)]
        project: PathBuf,
        #[arg(long, default_value = "auto_edit")]
        mode: String,
        /// Auto-answer permission prompts (allow|deny); default: deny after asking on stderr is impossible headlessly.
        #[arg(long, default_value = "deny")]
        prompts: String,
        #[arg(long)]
        json: bool,
        text: String,
    },
    /// Serve the local API.
    Serve {
        #[arg(long, default_value = "127.0.0.1:11520")]
        listen: String,
    },
}

struct Daemon {
    endpoint: String,
    model: String,
    broker: Arc<Mutex<Broker>>,
    sessions: Mutex<HashMap<String, (Arc<Shared>, Arc<Mutex<Option<Agent>>>)>>,
    /// Where we listen (Host header must match) and the per-boot token state-changing calls must carry.
    listen: String,
    token: String,
    /// chat id -> the agent session that carries it (recreated, with the history replayed, after a restart)
    chat_sessions: Mutex<HashMap<String, String>>,
}

fn new_shared(id: &str, mode: Mode, project: &str, model: &str) -> Arc<Shared> {
    Arc::new(Shared { stop: std::sync::atomic::AtomicBool::new(false), info: Mutex::new(SessionInfo { seq: SESSION_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst), id: id.into(), mode, project: project.into(), model: model.into(), state: "idle".into(), events: vec![], pending: vec![], transaction: None, preview_url: None, active_project: None, network_uses: vec![] }), answers: Mutex::new(VecDeque::new()), cv: Condvar::new() })
}

fn parse_mode(s: &str) -> Result<Mode> {
    serde_json::from_value(serde_json::Value::String(s.into())).map_err(|_| anyhow!("mode must be assist, auto_edit or autonomous"))
}

fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive("info".parse()?)).with_writer(std::io::stderr).init();
    let cli = Cli::parse();
    let mut sources = default_policy_sources();
    sources.extend(cli.policy.iter().cloned());
    let policy = match Policy::load(&sources) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(%e, "the policy did not load with the user's overlay; starting with the image's policy only");
            let system_only: Vec<std::path::PathBuf> = sources.iter().filter(|p| !genesis_permd::policy::home_dir().map(|h| p.starts_with(&h)).unwrap_or(false)).cloned().collect();
            Policy::load(&system_only)?
        }
    };
    let audit = cli.audit.clone().unwrap_or_else(default_audit_path);
    let broker = Arc::new(Mutex::new(Broker::new(policy, &audit)?));

    match cli.cmd {
        Cmd::Run { project, mode, prompts, json, text } => {
            let mode = parse_mode(&mode)?;
            let project = std::fs::canonicalize(&project)?;
            let id = uuid::Uuid::new_v4().to_string();
            broker.lock().unwrap().open_session(&id, mode, vec![project.display().to_string()], "cli")?;
            let shared = new_shared(&id, mode, &project.display().to_string(), &cli.model);
            let mut agent = Agent::new(Client { endpoint: cli.endpoint.clone(), model: cli.model.clone(), api_key: "local".into() }, broker.clone(), id.clone(), project.clone(), shared.clone());
            agent.prompt_timeout = std::time::Duration::from_secs(1);
            agent.tx_store = Store::open(default_store()).ok();
            // headless: auto-resolve prompts per --prompts and print events as they happen
            let auto_allow = prompts == "allow";
            let printer = shared.clone();
            let handle = std::thread::spawn(move || {
                let mut seen = 0usize;
                loop {
                    let (events, pending, state) = {
                        let i = printer.info.lock().unwrap();
                        (i.events[seen..].to_vec(), i.pending.clone(), i.state.clone())
                    };
                    for e in &events {
                        if json { eprintln!("{}", serde_json::to_string(e).unwrap()); } else { eprintln!("{}", fmt_event(e)); }
                    }
                    seen += events.len();
                    for p in pending {
                        eprintln!("  ↳ permission {} for {} {}: auto-{}", p.tier, p.tool, p.reason, if auto_allow { "allow" } else { "deny" });
                        printer.resolve(&p.request_id, auto_allow);
                    }
                    if state == "done" || state == "error" {
                        let i = printer.info.lock().unwrap();
                        for e in &i.events[seen..] { if json { eprintln!("{}", serde_json::to_string(e).unwrap()); } else { eprintln!("{}", fmt_event(e)); } }
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(150));
                }
            });
            let result = agent.run(&text);
            let _ = handle.join();
            match result {
                Ok(answer) => { println!("{}", answer); Ok(()) }
                Err(e) => Err(e),
            }
        }
        Cmd::Serve { listen } => {
            let server = Server::http(&listen).map_err(|e| anyhow!("listen {}: {}", listen, e))?;
            tracing::info!(listen = %listen, endpoint = %cli.endpoint, model = %cli.model, "genesis-agentd serving");
            let d = Arc::new(Daemon { endpoint: cli.endpoint.clone(), model: cli.model.clone(), broker, sessions: Mutex::new(HashMap::new()), listen: listen.to_string(), token: load_or_create_token("agentd"), chat_sessions: Mutex::new(HashMap::new()) });
            // make it while I sleep: the queue runner, once a minute
            {
                let d = d.clone();
                std::thread::spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_secs(60));
                    let q = queue::load();
                    if !queue::due(&q) || !queue::on_mains() { continue; }
                    let busy = d.sessions.lock().unwrap().values().any(|(s, _)| { let st = s.info.lock().unwrap().state.clone(); st == "running" || st == "waiting" });
                    if busy { continue; }
                    let Some(item) = q.items.first().cloned() else { continue };
                    if let Err(e) = run_queued(&d, &item) { tracing::warn!(%e, "queued make failed to start"); }
                });
            }
            // A second door on a Unix socket in the runtime directory, mode 0600: the local programs and
            // scripts use it instead of the loopback port (curl --unix-socket), and a sandboxed command
            // cannot reach it at all, because the sandbox puts a tmpfs over /run. The port stays for the
            // pages, because a browser cannot speak a Unix socket.
            if let Some(sock) = socket_path() {
                let _ = std::fs::remove_file(&sock);
                if let Some(dir) = sock.parent() { let _ = std::fs::create_dir_all(dir); }
                match Server::http_unix(&sock) {
                    Ok(unix_server) => {
                        #[cfg(unix)]
                        { use std::os::unix::fs::PermissionsExt; let _ = std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o600)); }
                        tracing::info!(socket = %sock.display(), "genesis-agentd also serving on a unix socket");
                        let d = d.clone();
                        std::thread::spawn(move || {
                            for req in unix_server.incoming_requests() {
                                let d = d.clone();
                                std::thread::spawn(move || { let _ = handle(&d, req); });
                            }
                        });
                    }
                    Err(e) => tracing::warn!(%e, "no unix socket; the loopback port is the only door"),
                }
            }
            for req in server.incoming_requests() {
                let d = d.clone();
                std::thread::spawn(move || { let _ = handle(&d, req); });
            }
            Ok(())
        }
    }
}

fn fmt_event(e: &Event) -> String {
    match e {
        Event::UserPrompt { text } => format!("▶ task: {}", text),
        Event::Assistant { text } => format!("💬 {}", text.trim()),
        Event::ToolCall { name, args, .. } => format!("🔧 {} {}", name, serde_json::to_string(args).unwrap_or_default().chars().take(160).collect::<String>()),
        Event::Decision { tier, verdict, reason, .. } => format!("   permd: {} {} — {}", tier, verdict, reason),
        Event::ToolResult { ok, summary, .. } => format!("   {} {}", if *ok { "✓" } else { "✗" }, summary.replace('\n', " ").chars().take(160).collect::<String>()),
        Event::Waiting { name, reason, .. } => format!("⏸ waiting for permission: {} ({})", name, reason),
        Event::Resolved { allowed, .. } => format!("   resolved: {}", if *allowed { "allowed" } else { "denied" }),
        Event::Error { text } => format!("‼ {}", text),
        Event::Done { turns } => format!("■ done in {} turns", turns),
        Event::Snapshot { tx, detail } => format!("⎘ {} {}", tx, detail),
    }
}

/// Where new sessions default to: ~/Projects if it exists, else the home directory.
fn default_project() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let p = format!("{}/Projects", home);
    if std::path::Path::new(&p).is_dir() { p } else { home }
}

fn json_response<T: serde::Serialize>(v: &T, status: u16) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_data(serde_json::to_vec(v).unwrap_or_default()).with_status_code(status).with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
}

#[derive(Deserialize)]
struct NewSession { mode: String, project: String, #[serde(default)] kind: String }
#[derive(Deserialize)]
struct ChatSay { text: String, #[serde(default)] attachments: Vec<String> }
#[derive(Deserialize)]
struct ChatNew { #[serde(default)] title: String }
#[derive(Deserialize)]
struct PromptReq { text: String }
#[derive(Deserialize)]
struct ResolveReq { allow: bool }

fn handle(d: &Arc<Daemon>, mut req: Request) -> Result<()> {
    let url = req.url().to_string();
    let path: Vec<&str> = url.split('?').next().unwrap_or("/").trim_matches('/').split('/').collect();
    let query: HashMap<String, String> = url.splitn(2, '?').nth(1).unwrap_or("").split('&').filter(|kv| !kv.is_empty()).map(|kv| { let (k, v) = kv.split_once('=').unwrap_or((kv, "")); (k.to_string(), url_decode(v)) }).collect();
    let method = req.method().clone();
    // The front door: only our own pages (same origin) and local programs holding the token may talk to
    // this daemon. A web page open in a browser cannot: cross-site requests carry Origin / Sec-Fetch-Site,
    // and a rebound DNS name fails the Host check.
    // Requests over the Unix socket come from a program on this machine that could open a 0600 file in the
    // user's runtime directory; no browser can reach it, so the Host and Origin checks (which exist for
    // browsers) do not apply. The token is still required, exactly as over the port.
    let over_socket = req.remote_addr().is_none();
    if !over_socket && !same_origin(&req, &d.listen) {
        return req.respond(json_response(&serde_json::json!({"error": "forbidden: not a Genesis origin"}), 403)).map_err(|e| anyhow!(e));
    }
    // GETs that hand out a secret (the phone pairing payload carries the companion's bearer token) need
    // the token like every state-changing call; the status widget's reads (health, sessions) stay open
    if (method != Method::Get || !open_get(&path)) && header(&req, "X-Genesis-Token") != Some(d.token.as_str()) {
        return req.respond(json_response(&serde_json::json!({"error": "unauthorized: missing Genesis token"}), 401)).map_err(|e| anyhow!(e));
    }
    let limit: usize = match path.as_slice() { ["api", "vision"] => 32 << 20, ["api", "transcribe"] => 16 << 20, _ => 1 << 20 };
    if req.body_length().unwrap_or(0) > limit {
        return req.respond(json_response(&serde_json::json!({"error": "request body too large"}), 413)).map_err(|e| anyhow!(e));
    }
    let mut raw: Vec<u8> = Vec::new();
    { use std::io::Read; let _ = req.as_reader().take(limit as u64 + 1).read_to_end(&mut raw); }
    if raw.len() > limit {
        return req.respond(json_response(&serde_json::json!({"error": "request body too large"}), 413)).map_err(|e| anyhow!(e));
    }
    if method == Method::Post && path.as_slice() == ["api", "transcribe"] {
        let resp = match voice::transcribe(&raw) {
            Ok(text) => json_response(&serde_json::json!({"text": text}), 200),
            Err(e) => json_response(&serde_json::json!({"error": e.to_string()}), 503),
        };
        return req.respond(resp).map_err(|e| anyhow!(e));
    }
    let body = String::from_utf8_lossy(&raw).to_string();
    if method == Method::Post && path.as_slice() == ["api", "speak"] {
        let text = serde_json::from_str::<serde_json::Value>(&body).ok().and_then(|v| v.get("text").and_then(|t| t.as_str()).map(|s| s.to_string())).unwrap_or_default();
        let resp = match voice::speak(&text) {
            Ok(wav) => Response::from_data(wav).with_header(Header::from_bytes("Content-Type", "audio/wav").unwrap()),
            Err(e) => json_response(&serde_json::json!({"error": e.to_string()}), 503),
        };
        return req.respond(resp).map_err(|e| anyhow!(e));
    }
    let resp = match (method, path.as_slice()) {
        (Method::Get, [""]) | (Method::Get, ["index.html"]) | (Method::Get, ["workspace"]) => Response::from_string(page(WORKSPACE_HTML, WORKSPACE_JS, page_token(d, &req))).with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap()),
        (Method::Get, ["api", "health"]) => json_response(&serde_json::json!({"ok": true, "endpoint": d.endpoint, "router_ok": router_ok(&d.endpoint), "model": served_model(&d.endpoint, &d.model), "sandbox": sandbox::bwrap_available(), "voice": voice::available(), "speech": voice::speech_available(), "default_project": default_project(), "default_mode": read_user_settings().get("default_mode").and_then(|m| m.as_str()).unwrap_or("auto_edit")}), 200),
        (Method::Get, ["api", "templates"]) => json_response(&maker::list_templates(), 200),
        (Method::Get, ["palette"]) => Response::from_string(page(PALETTE_HTML, PALETTE_JS, page_token(d, &req))).with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap()),
        (Method::Get, ["chat"]) => Response::from_string(page(CHAT_HTML, CHAT_JS, page_token(d, &req))).with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap()),
        (Method::Get, ["api", "chat-templates"]) => json_response(&chat::templates(), 200),
        (Method::Post, ["api", "chat-templates"]) => match serde_json::from_str::<serde_json::Value>(&body) {
            Ok(v) => match chat::save_template(v.get("title").and_then(|t| t.as_str()).unwrap_or(""), v.get("text").and_then(|t| t.as_str()).unwrap_or("")) {
                Ok(t) if !t.text.is_empty() => json_response(&t, 201),
                Ok(_) => json_response(&serde_json::json!({"error": "a template needs text"}), 400),
                Err(e) => json_response(&serde_json::json!({"error": e.to_string()}), 500),
            },
            Err(_) => json_response(&serde_json::json!({"error": "expected {title, text}"}), 400),
        },
        (Method::Post, ["api", "chat-templates", id, "delete"]) => json_response(&serde_json::json!({"deleted": chat::delete_template(id)}), 200),
        (Method::Get, ["api", "chats"]) => json_response(&chat::list(&query.get("q").cloned().unwrap_or_default()), 200),
        (Method::Post, ["api", "chats"]) => match chat::create(&serde_json::from_str::<ChatNew>(&body).map(|c| c.title).unwrap_or_default()) {
            Ok(c) => json_response(&c, 201),
            Err(e) => json_response(&serde_json::json!({"error": e.to_string()}), 500),
        },
        (Method::Get, ["api", "chats", id]) => match chat::load(id) {
            Some(c) => {
                let session = d.chat_sessions.lock().unwrap().get(*id).cloned();
                json_response(&serde_json::json!({"chat": c, "session": session}), 200)
            }
            None => json_response(&serde_json::json!({"error": "no such chat"}), 404),
        },
        (Method::Post, ["api", "chats", id, "delete"]) => {
            if let Some(sid) = d.chat_sessions.lock().unwrap().remove(*id) { d.sessions.lock().unwrap().remove(&sid); }
            json_response(&serde_json::json!({"deleted": chat::delete(id)}), 200)
        }
        (Method::Post, ["api", "chats", id, "say"]) => match (chat::load(id), serde_json::from_str::<ChatSay>(&body)) {
            (Some(existing), Ok(say)) => {
                let text = say.text.trim().to_string();
                if text.is_empty() { return Ok(req.respond(json_response(&serde_json::json!({"error": "say something"}), 400))?); }
                // an image attached: the vision model answers directly, the exchange is kept like any other
                let images: Vec<String> = say.attachments.iter().filter(|a| { let l = a.to_lowercase(); l.ends_with(".png") || l.ends_with(".jpg") || l.ends_with(".jpeg") || l.ends_with(".webp") }).cloned().collect();
                let sid = chat_session_for(d, &existing)?;
                let (shared, slot) = d.sessions.lock().unwrap().get(&sid).cloned().ok_or_else(|| anyhow!("chat session vanished"))?;
                let state = shared.info.lock().unwrap().state.clone();
                if state == "running" || state == "waiting" { return Ok(req.respond(json_response(&serde_json::json!({"error": "still answering"}), 409))?); }
                chat::append(id, "user", &text, say.attachments.clone())?;
                let mut prompt = text.clone();
                for a in say.attachments.iter().filter(|a| !images.contains(a)) { prompt.push_str(&format!("\n\n(The user attached the file {}: read it with read_document before answering.)", a)); }
                let chat_id = id.to_string();
                let endpoint = d.endpoint.clone();
                let d2 = Arc::clone(d);
                std::thread::spawn(move || {
                    let answer = if let Some(img) = images.first() {
                        match std::fs::read(img).map(|b| base64_encode(&b)).map_err(|e| anyhow!("{}: {}", img, e)).and_then(|b64| vision_answer(&endpoint, &prompt, &b64)) {
                            Ok(a) => { shared.push(Event::UserPrompt { text: prompt.clone() }); shared.push(Event::Assistant { text: a.clone() }); shared.push(Event::Done { turns: 1 }); shared.set_state("done"); a }
                            Err(e) => { shared.push(Event::Error { text: e.to_string() }); shared.set_state("error"); format!("I could not look at that image: {}", e) }
                        }
                    } else {
                        let mut guard = slot.lock().unwrap();
                        match guard.as_mut() { Some(agent) => agent.run(&prompt).unwrap_or_else(|e| format!("I could not finish: {}", e)), None => "the chat session is gone; open the chat again".into() }
                    };
                    let _ = chat::append(&chat_id, "assistant", &answer, vec![]);
                    let _ = d2; // keeps the daemon alive for the thread's lifetime
                });
                json_response(&serde_json::json!({"accepted": true, "session": sid}), 202)
            }
            (None, _) => json_response(&serde_json::json!({"error": "no such chat"}), 404),
            (_, Err(_)) => json_response(&serde_json::json!({"error": "expected {text, attachments?}"}), 400),
        },
        (Method::Get, ["settings"]) => Response::from_string(page(SETTINGS_HTML, SETTINGS_JS, page_token(d, &req))).with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap()),
        (Method::Get, ["api", "system"]) => json_response(&system_overview(d), 200),
        (Method::Post, ["api", "system", "theme"]) => {
            let v = serde_json::from_str::<serde_json::Value>(&body).unwrap_or(serde_json::Value::Null);
            let want = v.get("theme").and_then(|m| m.as_str()).map(|s| s.to_string()).unwrap_or_else(|| "toggle".into());
            let look = v.get("look").and_then(|m| m.as_str()).unwrap_or("").trim().chars().take(120).collect::<String>();
            if !["dark", "light", "toggle", "undo", "make"].contains(&want.as_str()) || (want == "make" && look.is_empty()) { json_response(&serde_json::json!({"error": "expected {theme: dark|light|toggle|undo} or {theme: make, look: \"...\"}"}), 400) }
            else { let mut cmd = std::process::Command::new("genesis-theme"); cmd.arg(&want); if want == "make" { cmd.arg(&look); }
              match cmd.output() {
                Ok(o) if o.status.success() => json_response(&serde_json::json!({"scheme": String::from_utf8_lossy(&o.stdout).trim()}), 200),
                Ok(o) => json_response(&serde_json::json!({"error": String::from_utf8_lossy(&o.stderr).trim()}), 500),
                Err(e) => json_response(&serde_json::json!({"error": format!("genesis-theme: {}", e)}), 503),
            } }
        }
        (Method::Post, ["api", "vision"]) => {
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
            let q = v.get("question").and_then(|q| q.as_str()).unwrap_or("What is this? Answer briefly.").to_string();
            let img = v.get("image_png_b64").and_then(|i| i.as_str()).unwrap_or("").to_string();
            if img.is_empty() { json_response(&serde_json::json!({"error": "image_png_b64 required"}), 400) }
            else { match vision_answer(&d.endpoint, &q, &img) { Ok(a) => json_response(&serde_json::json!({"answer": a}), 200), Err(e) => json_response(&serde_json::json!({"error": e.to_string()}), 502) } }
        }
        (Method::Get, ["api", "index"]) => json_response(&run_json("/usr/bin/genesis-index", &["status"]), 200),
        (Method::Post, ["api", "index"]) => {
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
            let action = v.get("action").and_then(|a| a.as_str()).unwrap_or("").to_string();
            let folder = v.get("folder").and_then(|a| a.as_str()).unwrap_or("").to_string();
            match action.as_str() {
                "add" | "remove" if !folder.is_empty() => json_response(&run_json("/usr/bin/genesis-index", &[&action, &folder]), 200),
                "update" => json_response(&run_json("/usr/bin/genesis-index", &["update"]), 200),
                _ => json_response(&serde_json::json!({"error": "expected {action: add|remove|update, folder}"}), 400),
            }
        }
        (Method::Get, ["api", "phone", "notifications"]) => json_response(&phone(&["notifications"]), 200),
        (Method::Post, ["api", "phone", "reply"]) => {
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
            let g = |k: &str| v.get(k).and_then(|a| a.as_str()).unwrap_or("").to_string();
            let (dev, id, text) = (g("device"), g("id"), g("text"));
            if dev.is_empty() || id.is_empty() || text.is_empty() { json_response(&serde_json::json!({"error": "device, id, text required"}), 400) }
            else { json_response(&phone(&["reply", &dev, &id, &text]), 200) }
        }
        (Method::Get, ["api", "history", id, "changes"]) => match Store::open(default_store()).and_then(|st| timeline::changes(&st, id)) {
            Ok(c) => json_response(&c, 200),
            Err(e) => json_response(&serde_json::json!({"error": e.to_string()}), 404),
        },
        (Method::Post, ["api", "history", id, "restore-file"]) => {
            let path = serde_json::from_str::<serde_json::Value>(&body).ok().and_then(|v| v.get("path").and_then(|p| p.as_str()).map(|s| s.to_string())).unwrap_or_default();
            match Store::open(default_store()).and_then(|st| timeline::restore_file(&st, id, &path)) {
                Ok(m) => json_response(&serde_json::json!({"ok": true, "message": m}), 200),
                Err(e) => json_response(&serde_json::json!({"error": e.to_string()}), 400),
            }
        }
        (Method::Post, ["api", "history", id, "undo"]) => {
            // undo from Settings, for jobs whose session is gone (after a re-login): straight through the store
            match genesis_txd::Store::open(genesis_txd::default_store()).and_then(|st| st.load(id).and_then(|mut tx| st.rollback(&mut tx))) {
                Ok(lines) => json_response(&serde_json::json!({"undone": true, "log": lines}), 200),
                Err(e) => json_response(&serde_json::json!({"error": e.to_string()}), 500),
            }
        }
        (Method::Get, ["api", "companion"]) => json_response(&companion_pairing(), 200),
        (Method::Post, ["api", "companion", "reset"]) => {
            let _ = std::process::Command::new("/usr/bin/genesis-companiond").arg("--reset").output();
            let _ = std::process::Command::new("systemctl").args(["--user", "restart", "genesis-companiond.service"]).status();
            json_response(&companion_pairing(), 200)
        }
        (Method::Post, ["api", "babel", "open"]) => {
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
            let p = v.get("path").and_then(|p| p.as_str()).unwrap_or("").to_string();
            match openable(&p) {
                Ok(canon) if canon.is_dir() => match std::process::Command::new("/usr/bin/babel").arg(&canon).spawn() {
                    Ok(_) => json_response(&serde_json::json!({"opened": true}), 200),
                    Err(e) => json_response(&serde_json::json!({"error": format!("Babel could not start: {}", e)}), 503),
                },
                Ok(_) => json_response(&serde_json::json!({"error": "not a project folder"}), 400),
                Err(e) => json_response(&serde_json::json!({"error": e.to_string()}), 400),
            }
        }
        (Method::Post, ["api", "backup", "export"]) => json_response(&run_json("/usr/bin/genesis-backup", &["export"]), 200),
        (Method::Post, ["api", "backup", "restore"]) => {
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
            let f = v.get("file").and_then(|p| p.as_str()).unwrap_or("").to_string();
            let home = std::env::var("HOME").unwrap_or_default();
            match std::fs::canonicalize(&f) {
                Ok(c) if c.starts_with(&home) && c.file_name().map(|n| n.to_string_lossy().starts_with("genesis-backup-")).unwrap_or(false) => json_response(&run_json("/usr/bin/genesis-backup", &["restore", &c.display().to_string()]), 200),
                _ => json_response(&serde_json::json!({"error": "pick a genesis-backup-*.tar.gz under your home folder"}), 400),
            }
        }
        (Method::Get, ["api", "policy", "rules"]) => json_response(&user_rules(), 200),
        (Method::Post, ["api", "policy", "rules"]) => match serde_json::from_str::<serde_json::Value>(&body) {
            Ok(v) => {
                let pattern = v.get("pattern").and_then(|p| p.as_str()).unwrap_or("").trim().to_string();
                let kind = v.get("kind").and_then(|k| k.as_str()).unwrap_or("always").to_string();
                if pattern.is_empty() || pattern.len() > 200 || !(kind == "always" || kind == "never") { return Ok(req.respond(json_response(&serde_json::json!({"error": "expected {pattern, kind: always|never}"}), 400))?); }
                let mut rules = user_rules();
                let id = format!("user.{}", uuid::Uuid::new_v4().to_string().split('-').next().unwrap_or("x"));
                rules.push(serde_json::json!({"id": id, "pattern": pattern, "kind": kind}));
                match save_user_rules(d, &rules) { Ok(()) => json_response(&rules, 201), Err(e) => json_response(&serde_json::json!({"error": e.to_string()}), 500) }
            }
            Err(_) => json_response(&serde_json::json!({"error": "expected {pattern, kind}"}), 400),
        },
        (Method::Post, ["api", "policy", "rules", id, "delete"]) => {
            let rules: Vec<serde_json::Value> = user_rules().into_iter().filter(|r| r.get("id").and_then(|i| i.as_str()) != Some(*id)).collect();
            match save_user_rules(d, &rules) { Ok(()) => json_response(&rules, 200), Err(e) => json_response(&serde_json::json!({"error": e.to_string()}), 500) }
        }
        (Method::Post, ["api", "phone", "send"]) => {
            let text = serde_json::from_str::<serde_json::Value>(&body).ok().and_then(|v| v.get("text").and_then(|t| t.as_str()).map(|s| s.to_string())).unwrap_or_default();
            if text.trim().is_empty() { return Ok(req.respond(json_response(&serde_json::json!({"error": "nothing to send"}), 400))?); }
            match std::process::Command::new("/usr/bin/genesis-phone").args(["send", &text]).output() {
                Ok(o) => { let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap_or(serde_json::json!({"ok": false, "error": "genesis-phone gave no answer"})); json_response(&v, 200) }
                Err(e) => json_response(&serde_json::json!({"ok": false, "error": e.to_string()}), 500),
            }
        }
        (Method::Get, ["api", "queue"]) => json_response(&queue::load(), 200),
        (Method::Post, ["api", "queue"]) => match serde_json::from_str::<serde_json::Value>(&body) {
            Ok(v) => {
                let text = v.get("text").and_then(|t| t.as_str()).unwrap_or("").trim().to_string();
                if text.is_empty() { return Ok(req.respond(json_response(&serde_json::json!({"error": "say what to make"}), 400))?); }
                let mut q = queue::load();
                q.items.push(queue::Item { id: uuid::Uuid::new_v4().to_string(), text, project: v.get("project").and_then(|p| p.as_str()).unwrap_or("").to_string(), added: queue::now() });
                queue::save(&q); json_response(&q, 201)
            }
            Err(_) => json_response(&serde_json::json!({"error": "expected {text, project?}"}), 400),
        },
        (Method::Post, ["api", "queue", "start"]) => { let mut q = queue::load(); q.start_now = true; queue::save(&q); json_response(&q, 200) }
        (Method::Post, ["api", "queue", "settings"]) => {
            let run_at = serde_json::from_str::<serde_json::Value>(&body).ok().and_then(|v| v.get("run_at").and_then(|t| t.as_str()).map(|s| s.to_string())).unwrap_or_default();
            if run_at.len() != 5 || run_at.as_bytes()[2] != b':' { return Ok(req.respond(json_response(&serde_json::json!({"error": "run_at must be HH:MM"}), 400))?); }
            let mut q = queue::load(); q.run_at = run_at; queue::save(&q); json_response(&q, 200)
        }
        (Method::Post, ["api", "queue", "clear-done"]) => { let mut q = queue::load(); q.done.clear(); queue::save(&q); json_response(&q, 200) }
        (Method::Post, ["api", "queue", id, "delete"]) => { let mut q = queue::load(); q.items.retain(|i| i.id != *id); queue::save(&q); json_response(&q, 200) }
        (Method::Post, ["api", "pick"]) => {
            let kind = serde_json::from_str::<serde_json::Value>(&body).ok().and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(|s| s.to_string())).unwrap_or_else(|| "file".into());
            let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
            let arg = if kind == "folder" { "--getexistingdirectory" } else { "--getopenfilename" };
            match std::process::Command::new("kdialog").args([arg, &home]).output() {
                Ok(o) if o.status.success() => json_response(&serde_json::json!({"path": String::from_utf8_lossy(&o.stdout).trim()}), 200),
                Ok(_) => json_response(&serde_json::json!({"path": ""}), 200),
                Err(e) => json_response(&serde_json::json!({"error": format!("no file dialog: {}", e)}), 500),
            }
        }
        // What Genesis keeps about you, with its size, and one way to delete each kind.
        // The switches for everything that can speak without being asked.
        (Method::Post, ["api", "settings"]) => match serde_json::from_str::<serde_json::Value>(&body) {
            Ok(v) => { write_user_settings(&v); json_response(&read_user_settings(), 200) }
            Err(_) => json_response(&serde_json::json!({"error": "expected a settings object"}), 400),
        },
        // What changed in this version and in the one waiting: the notes ship in the image.
        (Method::Get, ["api", "release-notes"]) => {
            let notes = std::fs::read_to_string("/usr/share/genesis/release-notes.md").unwrap_or_default();
            json_response(&serde_json::json!({"notes": notes.chars().take(20_000).collect::<String>()}), 200)
        }
        // "Choose a different pack": reopens the model wizard (it asks for the password itself).
        (Method::Post, ["api", "models", "setup"]) => match std::process::Command::new("/usr/bin/genesis-setup-models").spawn() {
            Ok(mut c) => { std::thread::spawn(move || { let _ = c.wait(); }); json_response(&serde_json::json!({"started": true}), 202) }
            Err(e) => json_response(&serde_json::json!({"error": e.to_string()}), 500),
        },
        // Clipboard transforms: one short model call on the selected text, no tools, nothing kept.
        (Method::Post, ["api", "transform"]) => {
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
            let (instr, text) = (v.get("instruction").and_then(|i| i.as_str()).unwrap_or(""), v.get("text").and_then(|t| t.as_str()).unwrap_or(""));
            if text.trim().is_empty() { return Ok(req.respond(json_response(&serde_json::json!({"error": "nothing selected"}), 400))?); }
            let client = Client { endpoint: d.endpoint.clone(), model: served_model_for(&d.endpoint, &d.model, "chat"), api_key: "local".into() };
            let msgs = vec![llm::Message::system(instr.to_string()), llm::Message::user(agent::cut_at_char(text, 8000).to_string())];
            match client.chat(&msgs, &serde_json::json!([]), 0.2) {
                Ok(r) => json_response(&serde_json::json!({"text": r.message.content.unwrap_or_default().trim()}), 200),
                Err(e) => json_response(&serde_json::json!({"error": format!("the model did not answer: {}", e)}), 502),
            }
        }
        // A whole recording or video: the text with its times, and subtitles written next to the file.
        (Method::Post, ["api", "transcribe-file"]) => {
            let path = serde_json::from_str::<serde_json::Value>(&body).ok().and_then(|v| v.get("path").and_then(|p| p.as_str()).map(|s| s.to_string())).unwrap_or_default();
            let p = std::path::PathBuf::from(shellexpand(&path));
            match voice::transcribe_file(&p) {
                Ok((text, srt)) => json_response(&serde_json::json!({"text": text, "subtitles": srt.display().to_string()}), 200),
                Err(e) => json_response(&serde_json::json!({"error": e.to_string()}), 400),
            }
        }
        (Method::Get, ["api", "data"]) => json_response(&stored_data(), 200),
        (Method::Post, ["api", "data", kind, "delete"]) => json_response(&delete_stored(kind), 200),
        (Method::Get, ["api", "mcp"]) => json_response(&mcp::status(), 200),
        (Method::Post, ["api", "mcp", "reload"]) => { mcp::reload(); json_response(&mcp::status(), 200) }
        (Method::Get, ["api", "recipes"]) => json_response(&recipes(), 200),
        (Method::Post, ["api", "recipes"]) => {
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
            match save_recipe(&v) { Ok(id) => json_response(&serde_json::json!({"saved": id}), 200), Err(e) => json_response(&serde_json::json!({"error": e.to_string()}), 400) }
        }
        (Method::Post, ["api", "recipes", id, "delete"]) => {
            let p = my_recipes_dir().join(format!("{}.json", id.replace(['/', '.'], "")));
            match std::fs::remove_file(&p) { Ok(_) => json_response(&serde_json::json!({"deleted": true}), 200), Err(e) => json_response(&serde_json::json!({"error": e.to_string()}), 404) }
        }
        (Method::Get, ["api", "phone"]) => json_response(&phone(&["status"]), 200),
        (Method::Post, ["api", "phone", "action"]) => {
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
            let action = v.get("action").and_then(|a| a.as_str()).unwrap_or("").to_string();
            let arg = v.get("id").or(v.get("address")).and_then(|a| a.as_str()).unwrap_or("").to_string();
            match action.as_str() {
                "refresh" | "pair" | "accept" | "cancel" | "unpair" | "add" | "commands" => {
                    let mut args = vec![action.as_str()];
                    if !arg.is_empty() { args.push(arg.as_str()); }
                    json_response(&phone(&args), 200)
                }
                _ => json_response(&serde_json::json!({ "error": "unknown phone action" }), 400),
            }
        }
        (Method::Post, ["api", "system", "update"]) => match std::process::Command::new("systemctl").args(["start", "--no-block", "bootc-fetch-apply-updates.service"]).output() {
            Ok(o) if o.status.success() => json_response(&serde_json::json!({"started": true}), 202),
            Ok(o) => json_response(&serde_json::json!({"error": String::from_utf8_lossy(&o.stderr).trim()}), 500),
            Err(e) => json_response(&serde_json::json!({"error": e.to_string()}), 503),
        },
        (Method::Post, ["api", "system", "mode"]) => match serde_json::from_str::<serde_json::Value>(&body).ok().and_then(|v| v.get("mode").and_then(|m| m.as_str()).map(|s| s.to_string())) {
            Some(mode) if parse_mode(&mode).is_ok() => { write_user_settings(&serde_json::json!({"default_mode": mode})); json_response(&serde_json::json!({"ok": true, "default_mode": mode}), 200) }
            _ => json_response(&serde_json::json!({"error": "expected {mode: assist|auto_edit|autonomous}"}), 400),
        },
        (Method::Post, ["api", "made", "export"]) => match serde_json::from_str::<serde_json::Value>(&body).ok().and_then(|v| v.get("path").and_then(|p| p.as_str()).map(|s| s.to_string())) {
            Some(p) => match maker::export_bundle(std::path::Path::new(&p)) {
                Ok(out) => json_response(&serde_json::json!({"bundle": out.display().to_string()}), 200),
                Err(e) => json_response(&serde_json::json!({"error": e.to_string()}), 400),
            },
            None => json_response(&serde_json::json!({"error": "expected {path}"}), 400),
        },
        (Method::Post, ["api", "open"]) => {
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
            let p = v.get("path").and_then(|p| p.as_str()).unwrap_or("").to_string();
            match openable(&p) {
                Ok(canon) => match std::process::Command::new("xdg-open").arg(&canon).spawn() {
                    Ok(_) => json_response(&serde_json::json!({"opened": true, "path": canon.display().to_string()}), 200),
                    Err(e) => json_response(&serde_json::json!({"error": e.to_string()}), 503),
                },
                Err(e) => json_response(&serde_json::json!({"error": e.to_string()}), 400),
            }
        }
        (Method::Get, ["api", "activity"]) => json_response(&recent_activity(40), 200),
        (Method::Post, ["api", "notices", "morning", "dismiss"]) => { maker::dismiss_morning(); json_response(&serde_json::json!({"ok": true}), 200) }
        (Method::Get, ["api", "notices"]) => json_response(&maker::notices(std::path::Path::new(&default_project())), 200),
        (Method::Get, ["api", "claude"]) => json_response(&claude_status(), 200),
        (Method::Post, ["api", "claude", "open"]) => {
            let project = serde_json::from_str::<serde_json::Value>(&body).ok().and_then(|v| v.get("project").and_then(|p| p.as_str()).map(|s| s.to_string())).unwrap_or_else(|| default_project());
            match std::process::Command::new("konsole").args(["--hold", "--workdir", &project, "-e", "/usr/bin/genesis-claude", &project]).spawn() {
                Ok(_) => json_response(&serde_json::json!({"opened": true, "project": project}), 200),
                Err(e) => json_response(&serde_json::json!({"error": format!("could not open a terminal: {}", e)}), 503),
            }
        }
        (Method::Get, ["api", "made"]) => json_response(&maker::made_here(std::path::Path::new(&default_project())), 200),
        // A thing you made can be renamed, taken out of the app menu, shown in the file manager, or deleted.
        (Method::Post, ["api", "made", "shortcut"]) => {
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
            let (path, key) = (v.get("path").and_then(|p| p.as_str()).unwrap_or(""), v.get("key").and_then(|k| k.as_str()).unwrap_or(""));
            match maker::set_shortcut(std::path::Path::new(path), key) {
                Ok(m) => json_response(&serde_json::json!({"ok": true, "message": m}), 200),
                Err(e) => json_response(&serde_json::json!({"error": e.to_string()}), 400),
            }
        }
        (Method::Post, ["api", "made", "manage"]) => {
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
            let (action, path) = (v.get("action").and_then(|a| a.as_str()).unwrap_or(""), v.get("path").and_then(|p| p.as_str()).unwrap_or(""));
            match maker::manage_made(action, path, v.get("name").and_then(|n| n.as_str()).unwrap_or("")) {
                Ok(m) => json_response(&serde_json::json!({"ok": true, "message": m}), 200),
                Err(e) => json_response(&serde_json::json!({"error": e.to_string()}), 400),
            }
        }
        (Method::Get, ["api", "sessions"]) => {
            let list: Vec<serde_json::Value> = d.sessions.lock().unwrap().values().map(|(s, _)| { let i = s.info.lock().unwrap(); serde_json::json!({"id": i.id, "mode": i.mode, "project": i.project, "state": i.state, "transaction": i.transaction}) }).collect();
            json_response(&list, 200)
        }
        (Method::Post, ["api", "sessions"]) => { release_old_sessions(d); match serde_json::from_str::<NewSession>(&body) {
            Ok(n) => match (parse_mode(&n.mode), std::fs::canonicalize(if n.project.trim().is_empty() { default_project() } else { n.project.clone() })) {
                (Ok(mode), Ok(project)) => {
                    let id = uuid::Uuid::new_v4().to_string();
                    d.broker.lock().unwrap().open_session(&id, mode, vec![project.display().to_string()], "api")?;
                    let model = served_model_for(&d.endpoint, &d.model, if n.kind == "chat" { "chat" } else { "make" });
                    let shared = new_shared(&id, mode, &project.display().to_string(), &model);
                    let mut agent = Agent::new(Client { endpoint: d.endpoint.clone(), model, api_key: "local".into() }, d.broker.clone(), id.clone(), project, shared.clone());
                    agent.tx_store = Store::open(default_store()).ok();
                    if n.kind == "chat" { agent.kind = "chat".into(); }
                    d.sessions.lock().unwrap().insert(id.clone(), (shared, Arc::new(Mutex::new(Some(agent)))));
                    json_response(&serde_json::json!({"id": id}), 201)
                }
                (Err(e), _) => json_response(&serde_json::json!({"error": e.to_string()}), 400),
                (_, Err(e)) => json_response(&serde_json::json!({"error": format!("project: {}", e)}), 400),
            },
            Err(_) => json_response(&serde_json::json!({"error": "expected {mode, project}"}), 400),
        } },
        (Method::Get, ["api", "sessions", id]) => match d.sessions.lock().unwrap().get(*id) {
            Some((shared, _)) => json_response(&*shared.info.lock().unwrap(), 200),
            None => json_response(&serde_json::json!({"error": "no such session"}), 404),
        },
        (Method::Post, ["api", "sessions", id, "stop"]) => {
            // the Stop button: the loop ends at its next checkpoint (at most a quarter of a second while it
            // waits for the model, at once for a permission card or a running command)
            match d.sessions.lock().unwrap().get(*id).cloned() {
                Some((shared, _)) => { shared.stop.store(true, std::sync::atomic::Ordering::SeqCst); shared.cv.notify_all(); json_response(&serde_json::json!({"stopping": true}), 202) }
                None => json_response(&serde_json::json!({"error": "no such session"}), 404),
            }
        }
        (Method::Post, ["api", "sessions", id, "prompt"]) => {
            let entry = d.sessions.lock().unwrap().get(*id).cloned();
            match (entry, serde_json::from_str::<PromptReq>(&body)) {
                (Some((shared, agent_slot)), Ok(p)) => {
                    let state = shared.info.lock().unwrap().state.clone();
                    if state == "running" || state == "waiting" {
                        json_response(&serde_json::json!({"error": "session busy"}), 409)
                    } else {
                        let slot = agent_slot.clone();
                        std::thread::spawn(move || {
                            let mut guard = slot.lock().unwrap();
                            if let Some(agent) = guard.as_mut() {
                                let _ = agent.run(&p.text);
                            }
                        });
                        json_response(&serde_json::json!({"accepted": true}), 202)
                    }
                }
                (None, _) => json_response(&serde_json::json!({"error": "no such session"}), 404),
                (_, Err(_)) => json_response(&serde_json::json!({"error": "expected {text}"}), 400),
            }
        }
        (Method::Post, ["api", "sessions", id, "plan"]) => {
            // the plan card: synchronous, one short model call; the client shows it and then sends /prompt
            let entry = d.sessions.lock().unwrap().get(*id).cloned();
            match (entry, serde_json::from_str::<PromptReq>(&body)) {
                (Some((shared, slot)), Ok(p)) => {
                    let state = shared.info.lock().unwrap().state.clone();
                    if state == "running" || state == "waiting" { json_response(&serde_json::json!({"error": "session busy"}), 409) }
                    else {
                        let mut guard = slot.lock().unwrap();
                        match guard.as_mut() { Some(agent) => json_response(&agent.plan(&p.text), 200), None => json_response(&serde_json::json!({"error": "no agent"}), 500) }
                    }
                }
                (None, _) => json_response(&serde_json::json!({"error": "no such session"}), 404),
                (_, Err(_)) => json_response(&serde_json::json!({"error": "expected {text}"}), 400),
            }
        }
        (Method::Post, ["api", "sessions", id, "undo"]) => {
            let entry = d.sessions.lock().unwrap().get(*id).cloned();
            match entry {
                Some((shared, slot)) => {
                    let state = shared.info.lock().unwrap().state.clone();
                    if state == "running" || state == "waiting" {
                        json_response(&serde_json::json!({"error": "session busy"}), 409)
                    } else {
                        let mut guard = slot.lock().unwrap();
                        match guard.as_mut().map(|a| a.undo()) {
                            Some(Ok(lines)) => json_response(&serde_json::json!({"undone": true, "log": lines}), 200),
                            Some(Err(e)) => json_response(&serde_json::json!({"error": e.to_string()}), 500),
                            None => json_response(&serde_json::json!({"error": "no agent"}), 500),
                        }
                    }
                }
                None => json_response(&serde_json::json!({"error": "no such session"}), 404),
            }
        }
        (Method::Post, ["api", "prompts", request_id]) => match serde_json::from_str::<ResolveReq>(&body) {
            Ok(r) => {
                let sessions = d.sessions.lock().unwrap();
                let found = sessions.values().find(|(s, _)| s.info.lock().unwrap().pending.iter().any(|p| p.request_id == *request_id)).cloned();
                match found {
                    Some((shared, _)) => { shared.resolve(request_id, r.allow); json_response(&serde_json::json!({"resolved": true, "allow": r.allow}), 200) }
                    None => json_response(&serde_json::json!({"error": "no pending prompt with that id"}), 404),
                }
            }
            Err(_) => json_response(&serde_json::json!({"error": "expected {allow: bool}"}), 400),
        },
        _ => json_response(&serde_json::json!({"error": "not found"}), 404),
    };
    let _ = req.respond(resp);
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_chat_never_takes_the_coder_just_because_it_is_the_default() {
        let ids: Vec<String> = ["code", "fast"].iter().map(|s| s.to_string()).collect();
        // the first real laptop: a code pack, no chat model, the command-line default "code"
        assert_eq!(super::pick_model(&ids, "code", "chat", false), "fast", "a crash, a failed service or an update is answered by the small model, not a 9B cold load");
        assert_eq!(super::pick_model(&ids, "code", "vision", false), "fast");
        assert_eq!(super::pick_model(&ids, "code", "make", false), "code", "making things still takes the coder");
        assert_eq!(super::pick_model(&ids, "code", "make", true), "fast", "on battery, not even that");
        let with_chat: Vec<String> = ["code", "fast", "chat"].iter().map(|s| s.to_string()).collect();
        assert_eq!(super::pick_model(&with_chat, "code", "chat", false), "chat");
        assert_eq!(super::pick_model(&ids, "claude-sonnet-5", "make", false), "code", "a name that is not served: the preference order");
        assert_eq!(super::pick_model(&["tiny".to_string()], "code", "chat", false), "code", "nothing preferred is served: the name as given, as before");
    }

    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// A canned OpenAI-compatible endpoint: first call returns a tool call, second returns text.
    fn fake_llm(script: Vec<serde_json::Value>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for (i, mut stream) in listener.incoming().filter_map(|s| s.ok()).enumerate() {
                // read the whole request: headers, then Content-Length bytes of body
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                let (mut header_end, mut content_len) = (None, 0usize);
                loop {
                    let n = stream.read(&mut chunk).unwrap_or(0);
                    if n == 0 { break; }
                    buf.extend_from_slice(&chunk[..n]);
                    if header_end.is_none() {
                        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            header_end = Some(pos + 4);
                            let head = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
                            content_len = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
                        }
                    }
                    if let Some(h) = header_end { if buf.len() >= h + content_len { break; } }
                }
                let msg = script.get(i).cloned().unwrap_or(serde_json::json!({"role":"assistant","content":"done"}));
                let body = serde_json::json!({"choices":[{"message": msg, "finish_reason":"stop"}]}).to_string();
                let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
            }
        });
        format!("http://{}/v1", addr)
    }

    fn setup(project: &std::path::Path, mode: Mode, endpoint: String) -> (Agent, Arc<Shared>) {
        let dir = tempfile::tempdir().unwrap();
        let audit = dir.path().join("audit.jsonl");
        std::mem::forget(dir);
        let broker = Arc::new(Mutex::new(Broker::new(Policy::default_policy(), audit).unwrap()));
        broker.lock().unwrap().open_session("t", mode, vec![project.display().to_string()], "test").unwrap();
        let shared = new_shared("t", mode, &project.display().to_string(), "fake");
        let mut agent = Agent::new(Client { endpoint, model: "fake".into(), api_key: "x".into() }, broker, "t".into(), project.to_path_buf(), shared.clone());
        agent.prompt_timeout = std::time::Duration::from_millis(300);
        let txdir = tempfile::tempdir().unwrap();
        agent.tx_store = Some(Store::open(txdir.path()).unwrap());
        std::mem::forget(txdir);
        (agent, shared)
    }

    fn tool_call(name: &str, args: serde_json::Value) -> serde_json::Value {
        serde_json::json!({"role":"assistant","content":null,"tool_calls":[{"id":"c1","type":"function","function":{"name":name,"arguments":args.to_string()}}]})
    }

    #[test]
    fn project_write_runs_in_auto_edit_and_file_exists() {
        let proj = tempfile::tempdir().unwrap();
        let ep = fake_llm(vec![tool_call("write_file", serde_json::json!({"path":"hello.txt","content":"hi"})), serde_json::json!({"role":"assistant","content":"wrote hello.txt"})]);
        let (mut agent, shared) = setup(proj.path(), Mode::AutoEdit, ep);
        let out = agent.run("make hello").unwrap();
        assert_eq!(out, "wrote hello.txt");
        assert_eq!(std::fs::read_to_string(proj.path().join("hello.txt")).unwrap(), "hi");
        let ev = shared.info.lock().unwrap().events.clone();
        assert!(ev.iter().any(|e| matches!(e, Event::Decision { tier, verdict, .. } if tier == "W1" && verdict == "allow")));
    }

    #[test]
    fn project_write_prompts_in_assist_and_is_denied_without_answer() {
        let proj = tempfile::tempdir().unwrap();
        let ep = fake_llm(vec![tool_call("write_file", serde_json::json!({"path":"x.txt","content":"1"})), serde_json::json!({"role":"assistant","content":"could not"})]);
        let (mut agent, shared) = setup(proj.path(), Mode::Assist, ep);
        let _ = agent.run("write x").unwrap();
        assert!(!proj.path().join("x.txt").exists());
        let ev = shared.info.lock().unwrap().events.clone();
        assert!(ev.iter().any(|e| matches!(e, Event::Waiting { .. })));
        assert!(ev.iter().any(|e| matches!(e, Event::ToolResult { ok: false, .. })));
    }

    #[test]
    fn prompt_can_be_allowed_by_a_client() {
        let proj = tempfile::tempdir().unwrap();
        let ep = fake_llm(vec![tool_call("write_file", serde_json::json!({"path":"y.txt","content":"2"})), serde_json::json!({"role":"assistant","content":"ok"})]);
        let (mut agent, shared) = setup(proj.path(), Mode::Assist, ep);
        agent.prompt_timeout = std::time::Duration::from_secs(5);
        let s2 = shared.clone();
        std::thread::spawn(move || {
            loop {
                let pending = s2.info.lock().unwrap().pending.clone();
                if let Some(p) = pending.first() { s2.resolve(&p.request_id, true); break; }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        });
        agent.run("write y").unwrap();
        assert_eq!(std::fs::read_to_string(proj.path().join("y.txt")).unwrap(), "2");
    }

    #[test]
    fn phase0_escalation_is_gated_even_in_auto_edit() {
        let proj = tempfile::tempdir().unwrap();
        let ep = fake_llm(vec![tool_call("shell", serde_json::json!({"command":"pip3 install --user --break-system-packages pytest"})), serde_json::json!({"role":"assistant","content":"needs permission"})]);
        let (mut agent, shared) = setup(proj.path(), Mode::AutoEdit, ep);
        let _ = agent.run("install pytest").unwrap();
        let ev = shared.info.lock().unwrap().events.clone();
        assert!(ev.iter().any(|e| matches!(e, Event::Decision { tier, verdict, .. } if tier == "S1" && verdict == "prompt")));
        assert!(ev.iter().any(|e| matches!(e, Event::ToolResult { ok: false, .. })));
    }

    #[test]
    fn user_scope_write_opens_transaction_and_undo_restores() {
        let proj = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let cfg = home.path().join("notes.md");
        std::fs::write(&cfg, "original").unwrap();
        let ep = fake_llm(vec![tool_call("write_file", serde_json::json!({"path": cfg.display().to_string(), "content":"changed"})), serde_json::json!({"role":"assistant","content":"done"})]);
        let (mut agent, shared) = setup(proj.path(), Mode::Autonomous, ep);
        agent.run("edit notes").unwrap();
        assert_eq!(std::fs::read_to_string(&cfg).unwrap(), "changed");
        let ev = shared.info.lock().unwrap().events.clone();
        assert!(ev.iter().any(|e| matches!(e, Event::Decision { tier, verdict, .. } if tier == "W2" && verdict == "allowwithsnapshot")));
        assert!(ev.iter().any(|e| matches!(e, Event::Snapshot { .. })));
        assert!(shared.info.lock().unwrap().transaction.is_some());
        let log = agent.undo().unwrap();
        assert!(log.iter().any(|l| l.starts_with("restored")));
        assert_eq!(std::fs::read_to_string(&cfg).unwrap(), "original");
    }

    #[test]
    fn secrets_are_denied_outright() {
        let proj = tempfile::tempdir().unwrap();
        let ep = fake_llm(vec![tool_call("read_file", serde_json::json!({"path":"~/.ssh/id_ed25519"})), serde_json::json!({"role":"assistant","content":"cannot"})]);
        let (mut agent, shared) = setup(proj.path(), Mode::Autonomous, ep);
        let _ = agent.run("read key").unwrap();
        let ev = shared.info.lock().unwrap().events.clone();
        assert!(ev.iter().any(|e| matches!(e, Event::Decision { tier, verdict, .. } if tier == "X" && verdict == "deny")));
    }

    #[test]
    fn readonly_shell_runs_and_returns_output() {
        let proj = tempfile::tempdir().unwrap();
        std::fs::write(proj.path().join("a.txt"), "x").unwrap();
        let ep = fake_llm(vec![tool_call("shell", serde_json::json!({"command":"ls"})), serde_json::json!({"role":"assistant","content":"listed"})]);
        let (mut agent, shared) = setup(proj.path(), Mode::Assist, ep);
        agent.run("list").unwrap();
        let ev = shared.info.lock().unwrap().events.clone();
        assert!(ev.iter().any(|e| matches!(e, Event::ToolResult { ok: true, summary, .. } if summary.contains("a.txt"))));
    }

    /// A project with a genesis.json whose dev command just runs the script (no server), so previews return output.
    fn script_project() -> tempfile::TempDir {
        let proj = tempfile::tempdir().unwrap();
        std::fs::write(proj.path().join("genesis.json"), r#"{"id":"python-script","name":"s","dev":{"cmd":"python3 app.py","port":0},"entry":"app.py"}"#).unwrap();
        std::fs::write(proj.path().join("app.py"), "print('v0')\n").unwrap();
        proj
    }

    #[test]
    fn third_rewrite_without_a_run_makes_genesis_run_it_and_finish() {
        let proj = script_project();
        // the model rewrites app.py forever; Genesis runs it after the third rewrite, sees a clean run, and ends the job
        let mut script = Vec::new();
        for i in 0..8 { script.push(tool_call("write_file", serde_json::json!({"path":"app.py","content":format!("print('v{}')\n", i + 1)}))); }
        let ep = fake_llm(script);
        let (mut agent, shared) = setup(proj.path(), Mode::AutoEdit, ep);
        agent.edited_after_scaffold = true;
        let out = agent.run("a script").unwrap();
        assert!(out.contains("It is made and it runs"), "got: {}", out);
        let calls = shared.info.lock().unwrap().events.iter().filter(|e| matches!(e, Event::ToolCall { name, .. } if name == "write_file")).count();
        assert_eq!(calls, 3, "three rewrites, then Genesis took over");
        assert_eq!(shared.info.lock().unwrap().state, "done");
    }

    #[test]
    fn a_repeated_clean_run_ends_the_job() {
        let proj = script_project();
        let mut script = vec![tool_call("write_file", serde_json::json!({"path":"app.py","content":"print('ok')\n"}))];
        for _ in 0..8 { script.push(tool_call("preview_start", serde_json::json!({}))); }
        let ep = fake_llm(script);
        let (mut agent, shared) = setup(proj.path(), Mode::AutoEdit, ep);
        let out = agent.run("a script").unwrap();
        assert!(out.contains("It is made and it runs"), "got: {}", out);
        // the second identical run after a clean one is enough: the thing works and the model is circling
        let runs = shared.info.lock().unwrap().events.iter().filter(|e| matches!(e, Event::ToolCall { name, .. } if name == "preview_start")).count();
        assert_eq!(runs, 2);
    }

    #[test]
    fn a_run_that_keeps_failing_ends_the_job_instead_of_running_to_the_timeout() {
        let proj = script_project();
        // The program is broken, so every run reports the same failure. The tool call itself SUCCEEDS —
        // it ran the thing and reported a failing test — so the failure guard never sees it. In the
        // evaluation this was eleven previews of a failing test, and a shell loop deleting its own work,
        // both sitting there until the 25-minute timeout. Four runs with nothing changed in between ends it.
        let mut script = vec![tool_call("write_file", serde_json::json!({"path":"app.py","content":"raise SystemExit('Traceback: boom')\n"}))];
        for _ in 0..10 { script.push(tool_call("shell", serde_json::json!({"command":"python3 app.py"}))); }
        let ep = fake_llm(script);
        let (mut agent, shared) = setup(proj.path(), Mode::AutoEdit, ep);
        let err = agent.run("a script").unwrap_err().to_string();
        assert!(err.contains("without anything changing in between"), "got: {}", err);
        let runs = shared.info.lock().unwrap().events.iter().filter(|e| matches!(e, Event::ToolCall { name, .. } if name == "shell")).count();
        assert_eq!(runs, 4, "it stopped at the fourth run, not the tenth");
    }

    #[test]
    fn changing_and_running_in_turns_does_not_escape_every_guard() {
        let proj = script_project();
        // A write resets the run counter and a run resets the write counters, so alternating the two slipped
        // past all of them: 25 edits to a word counter that already worked. Once it has run clean and a dozen
        // changes have been made, the thing is made.
        let mut script = vec![];
        for i in 0..20 {
            script.push(tool_call("write_file", serde_json::json!({"path":"app.py","content":format!("print({})\n", i)})));
            script.push(tool_call("preview_start", serde_json::json!({})));
        }
        let ep = fake_llm(script);
        let (mut agent, shared) = setup(proj.path(), Mode::AutoEdit, ep);
        let out = agent.run("a script").expect("a made program, not the turn limit");
        assert!(out.contains("It is made and it runs"), "got: {}", out);
        let writes = shared.info.lock().unwrap().events.iter().filter(|e| matches!(e, Event::ToolCall { name, .. } if name == "write_file")).count();
        assert!(writes <= 13, "it stopped near the twelfth change, not after twenty: {}", writes);
    }

    #[test]
    fn a_manifest_that_is_already_right_is_not_a_failure() {
        let proj = script_project();
        // A small model writes genesis.json as an array, four times. The manifest on disk is fine, so there
        // is nothing to do and nothing that failed: refusing it four times killed a make whose program was
        // already finished (the dice roller). Genesis says it is set up and the job carries on.
        let bad = serde_json::json!({"path":"genesis.json","content":"[\"dice\", \"a dice roller\"]"});
        let mut script = vec![tool_call("write_file", serde_json::json!({"path":"app.py","content":"print('roll')\n"}))];
        for _ in 0..2 { script.push(tool_call("write_file", bad.clone())); }
        script.push(serde_json::json!({"role":"assistant","content":"done"}));
        let ep = fake_llm(script);
        let (mut agent, shared) = setup(proj.path(), Mode::AutoEdit, ep);
        assert_eq!(agent.run("a dice roller").unwrap(), "done", "the job finishes instead of being killed");
        let results: Vec<String> = shared.info.lock().unwrap().events.iter().filter_map(|e| if let Event::ToolResult { summary, .. } = e { Some(summary.clone()) } else { None }).collect();
        assert!(results[1].contains("already set up"), "got: {:?}", results);
        // and the manifest Genesis needs is still there, untouched
        let m: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(proj.path().join("genesis.json")).unwrap()).unwrap();
        assert!(m.get("dev").and_then(|d| d.get("cmd")).is_some());
    }

    #[test]
    fn many_changes_and_never_a_clean_run_stops_the_job() {
        let proj = script_project();
        // Twenty-six writes, previews in between, and the program never once ran without an error: the job
        // ran to the turn limit with nothing to show. The finish-line guard needs a clean run to arm, so
        // this one must not; it stops at eighteen changes and says why.
        let mut script = vec![];
        for i in 0..30 {
            script.push(tool_call("write_file", serde_json::json!({"path":"app.py","content":format!("raise SystemExit('Traceback {}')\n", i)})));
            if i % 2 == 1 { script.push(tool_call("shell", serde_json::json!({"command":"python3 app.py"}))); }
        }
        let ep = fake_llm(script);
        let (mut agent, shared) = setup(proj.path(), Mode::AutoEdit, ep);
        let err = agent.run("a script").unwrap_err().to_string();
        assert!(err.contains("eighteen changes"), "got: {}", err);
        let writes = shared.info.lock().unwrap().events.iter().filter(|e| matches!(e, Event::ToolCall { name, .. } if name == "write_file")).count();
        assert!(writes <= 19, "it stopped at eighteen, not thirty: {}", writes);
    }

    #[test]
    fn no_op_edits_with_a_run_between_them_do_not_reach_the_turn_limit() {
        let proj = script_project();
        // The word counter's actual shape, twice in a row: an edit whose old_text equals its new_text, then
        // a preview, then the same again. Each no-op is counted as a failure, but the run between them
        // resets the streak, so it ran to the turn limit with 25 edits. Counted for the whole job instead,
        // five of them end it — and since the program ran clean, it ends as a made program.
        let noop = serde_json::json!({"path":"app.py","old_text":"print('v0')","new_text":"print('v0')"});
        let mut script = vec![tool_call("write_file", serde_json::json!({"path":"app.py","content":"print('v0')\n"}))];
        for _ in 0..10 {
            script.push(tool_call("edit_file", noop.clone()));
            script.push(tool_call("preview_start", serde_json::json!({})));
        }
        let ep = fake_llm(script);
        let (mut agent, shared) = setup(proj.path(), Mode::AutoEdit, ep);
        let out = agent.run("count the words").expect("a made program, not the turn limit");
        assert!(out.contains("It is made and it runs"), "got: {}", out);
        let edits = shared.info.lock().unwrap().events.iter().filter(|e| matches!(e, Event::ToolCall { name, .. } if name == "edit_file")).count();
        assert!(edits <= 6, "it stopped at the fifth no-op, not the tenth: {}", edits);
    }

    #[test]
    fn consecutive_no_op_edits_end_with_a_sentence_about_the_program() {
        let proj = script_project();
        // Four no-op edits in a row tripped the generic "that same call has failed four times in a row",
        // and what the person read was a tool error cut off mid-word. The no-op ending is more specific and
        // now comes first, so they are told about their program instead.
        let noop = serde_json::json!({"path":"app.py","old_text":"print('v0')","new_text":"print('v0')"});
        let script: Vec<_> = (0..6).map(|_| tool_call("edit_file", noop.clone())).collect();
        let ep = fake_llm(script);
        let (mut agent, _s) = setup(proj.path(), Mode::AutoEdit, ep);
        let err = agent.run("count the words").unwrap_err().to_string();
        assert!(err.contains("wrote nothing new"), "got: {}", err);
        assert!(!err.contains("failed four times"), "the generic ending must not win: {}", err);
    }

    #[test]
    fn stop_ends_the_loop_cleanly() {
        let proj = tempfile::tempdir().unwrap();
        // a long-running command, then more work that must never happen: Stop kills the command and ends the loop
        let mut script = vec![tool_call("shell", serde_json::json!({"command":"sleep 30"}))];
        for i in 0..5 { script.push(tool_call("write_file", serde_json::json!({"path":format!("f{}.txt", i),"content":"x"}))); }
        let ep = fake_llm(script);
        let (mut agent, shared) = setup(proj.path(), Mode::AutoEdit, ep);
        let s2 = shared.clone();
        std::thread::spawn(move || { std::thread::sleep(std::time::Duration::from_millis(700)); s2.stop.store(true, std::sync::atomic::Ordering::SeqCst); s2.cv.notify_all(); });
        let t0 = std::time::Instant::now();
        let out = agent.run("wait then write").unwrap();
        assert_eq!(out, "stopped");
        assert!(t0.elapsed() < std::time::Duration::from_secs(10), "the sleep was killed, not waited out");
        assert_eq!(shared.info.lock().unwrap().state, "done");
        assert!(!proj.path().join("f0.txt").exists(), "nothing after the stop ran");
    }

    #[test]
    fn a_huge_tool_result_is_cut_when_it_goes_in_and_older_messages_are_never_rewritten() {
        let proj = tempfile::tempdir().unwrap();
        std::fs::write(proj.path().join("big.txt"), "x".repeat(200_000)).unwrap();
        let ep = fake_llm(vec![tool_call("read_file", serde_json::json!({"path":"big.txt"})),
                               tool_call("read_file", serde_json::json!({"path":"big.txt"})),
                               serde_json::json!({"role":"assistant","content":"read it"})]);
        let (mut agent, _shared) = setup(proj.path(), Mode::AutoEdit, ep);
        let first_after_one = std::cell::RefCell::new(String::new());
        assert_eq!(agent.run("read the file").unwrap(), "read it");
        let tools: Vec<&crate::llm::Message> = agent.messages.iter().filter(|m| m.role == "tool").collect();
        assert_eq!(tools.len(), 2);
        for m in &tools {
            let c = m.content.as_deref().unwrap_or("");
            assert!(c.len() < 30_000, "a 200 kB file is cut before it enters the conversation: {}", c.len());
            assert!(c.contains("characters in all"), "and says so");
        }
        // the first tool message must be byte-for-byte what it was when it was sent: the model service
        // reuses its work by matching the start of the conversation
        first_after_one.replace(tools[0].content.clone().unwrap_or_default());
        assert_eq!(tools[0].content.as_deref().unwrap(), first_after_one.borrow().as_str());
    }

    #[test]
    fn a_small_model_is_shown_one_worked_example_and_the_scaffold_hands_back_the_file() {
        let proj = tempfile::tempdir().unwrap();
        let ep = fake_llm(vec![serde_json::json!({"role":"assistant","content":"ok"})]);
        let (mut agent, _s) = setup(proj.path(), Mode::AutoEdit, ep);
        agent.client.model = "qwen3-2b".into(); // a small model: the compact script and the example
        let _ = agent.run("a timer");
        let roles: Vec<&str> = agent.messages.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles[0], "system");
        assert!(agent.messages.iter().any(|m| m.content.as_deref().map(|c| c.contains("random quote")).unwrap_or(false)), "the example is in front of it");
        assert!(agent.messages.iter().any(|m| m.tool_calls.as_ref().map(|c| c[0].function.name == "write_file").unwrap_or(false)), "and it shows a whole file being written");
    }

    #[test]
    fn a_write_that_changes_nothing_still_ends_in_a_finished_job() {
        let proj = script_project();
        // A small model writes the same finished file over and over. The file is correct and on disk, so
        // this must end as a made program, not as "failed four times in a row": Genesis takes over on the
        // third write, runs it, and stops on a clean run. Discounting these writes held the forced run off
        // while the failure guard fired, and six of ten evaluation makes died with a working program.
        let same = serde_json::json!({"path":"app.py","content":"print('v0')\n"});
        let script: Vec<_> = (0..8).map(|_| tool_call("write_file", same.clone())).collect();
        let ep = fake_llm(script);
        let (mut agent, shared) = setup(proj.path(), Mode::AutoEdit, ep);
        let out = agent.run("a script").expect("a finished program, not an error");
        assert!(out.contains("It is made"), "got: {}", out);
        let ev = shared.info.lock().unwrap().events.clone();
        assert!(ev.iter().any(|e| matches!(e, Event::Done { .. })), "and the job is done");
        assert!(ev.iter().filter(|e| matches!(e, Event::ToolCall { .. })).count() <= 5, "it stopped early, not after the whole script");
    }

    #[test]
    fn a_write_to_the_project_manifest_keeps_how_the_program_runs() {
        let proj = script_project();
        // what a small model actually writes: a package-manifest shape with no idea how Genesis runs things
        let theirs = serde_json::json!({"path":"genesis.json","content":"{\"name\":\"checklist\",\"version\":\"1.0\",\"description\":\"a checklist\"}"});
        let broken = serde_json::json!({"path":"genesis.json","content":"{ not json"});
        let ep = fake_llm(vec![tool_call("write_file", theirs), tool_call("write_file", broken), serde_json::json!({"role":"assistant","content":"done"})]);
        let (mut agent, shared) = setup(proj.path(), Mode::AutoEdit, ep);
        assert_eq!(agent.run("change how it runs").unwrap(), "done");
        let ev = shared.info.lock().unwrap().events.clone();
        let results: Vec<String> = ev.iter().filter_map(|e| if let Event::ToolResult { summary, .. } = e { Some(summary.clone()) } else { None }).collect();
        assert!(results[0].starts_with("wrote"), "their fields are taken, not refused: {:?}", results);
        assert!(results[1].contains("already set up"), "a manifest Genesis can already run is left alone, not refused: {:?}", results);
        let m: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(proj.path().join("genesis.json")).unwrap()).unwrap();
        assert_eq!(m["name"], "checklist", "what the model wrote is kept");
        assert_eq!(m["dev"]["cmd"], "python3 app.py", "how the program runs is kept");
        assert_eq!(m["entry"], "app.py");
    }

    #[test]
    fn a_queued_make_whose_folder_is_gone_is_dropped_with_a_reason() {
        let state = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_STATE_HOME", state.path());
        let item = queue::Item { id: "q1".into(), text: "a timer".into(), project: "/nonexistent/folder".into(), added: queue::now() };
        let mut q = queue::load(); q.items.push(item.clone()); q.start_now = true; queue::save(&q);
        super::drop_queued(&item, "could not start: no such folder");
        let after = queue::load();
        assert!(after.items.is_empty(), "the item is gone, not retried");
        assert!(!after.start_now, "an empty queue stops asking to start");
        assert_eq!(after.done.len(), 1);
        assert!(after.done[0].state.contains("could not start"), "the morning card says why");
    }

    #[test]
    fn ocr_removes_its_temp_directory_when_a_pdf_cannot_be_rendered() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("broken.pdf");
        std::fs::write(&bad, b"not a pdf at all").unwrap();
        let base = std::env::var("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
        let count = || std::fs::read_dir(&base).map(|rd| rd.filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().starts_with("genesis-ocr-")).count()).unwrap_or(0);
        let before = count();
        let r = crate::ocr::read_text("http://127.0.0.1:1", &bad);
        assert!(r.is_err(), "a broken PDF is an error, not an empty answer");
        assert_eq!(count(), before, "no temp directory is left behind");
    }

    #[test]
    fn an_mcp_call_may_last_longer_than_the_slowest_tool() {
        // the system server allows 1800 s for a Flatpak install; a shorter call timeout killed the server
        let src = include_str!("mcp.rs");
        let call = src.split("pub fn call(").nth(1).unwrap();
        let secs: u64 = call.split("Duration::from_secs(").nth(1).unwrap().split(')').next().unwrap().parse().unwrap();
        assert!(secs > 1800, "an MCP call times out after {} s, before an install can finish", secs);
    }
}


// ---- settings: everything a person needs to see about "their" Genesis, from local sources only --------

fn user_settings_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    std::path::Path::new(&home).join(".config/genesis/settings.json")
}

fn read_user_settings() -> serde_json::Value {
    std::fs::read_to_string(user_settings_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(serde_json::json!({}))
}

fn write_user_settings(patch: &serde_json::Value) {
    let mut cur = read_user_settings();
    if let (Some(c), Some(p)) = (cur.as_object_mut(), patch.as_object()) { for (k, v) in p { c.insert(k.clone(), v.clone()); } }
    let p = user_settings_path();
    if let Some(d) = p.parent() { let _ = std::fs::create_dir_all(d); }
    let _ = std::fs::write(&p, serde_json::to_string_pretty(&cur).unwrap_or_default());
}

fn system_overview(d: &Arc<Daemon>) -> serde_json::Value {
    let read_json = |p: &str| std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
    let profile = read_json("/etc/genesis/profile.json");
    let setup = read_json("/etc/genesis/settings.json");
    let models_dir = std::env::var("GENESIS_MODELS_DIR").unwrap_or_else(|_| "/var/lib/genesis/models".into());
    let mut on_disk = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&models_dir) {
        for role in rd.flatten().filter(|e| e.path().is_dir()) {
            if let Ok(files) = std::fs::read_dir(role.path()) {
                for f in files.flatten() {
                    let size = f.metadata().map(|m| m.len()).unwrap_or(0);
                    on_disk.push(serde_json::json!({"role": role.file_name().to_string_lossy(), "file": f.file_name().to_string_lossy(), "size_gb": (size as f64 / 1e9 * 10.0).round() / 10.0}));
                }
            }
        }
    }
    let router = d.endpoint.trim_end_matches("/v1").to_string();
    let running = ureq::get(&format!("{}/running", router)).timeout(std::time::Duration::from_secs(2)).call().ok().and_then(|r| r.into_json::<serde_json::Value>().ok());
    let history: Vec<serde_json::Value> = Store::open(default_store()).ok().and_then(|s| s.list().ok()).unwrap_or_default().into_iter().rev().take(30)
        .map(|t| {
            // what the person asked for, when the job left a note of it, so the list is readable
            let asked = t.entries.iter().find_map(|e| if let genesis_txd::Entry::Note { text } = e { text.strip_prefix("asked: ").map(|s| s.to_string()) } else { None }).unwrap_or_default();
            // apps installed in this job, so the panel can say "installed GIMP · Undo" rather than "a job"
            let apps: Vec<String> = t.entries.iter().filter_map(|e| if let genesis_txd::Entry::FlatpakUser { r#ref } = e { r#ref.rsplit('/').next().or(Some(r#ref.as_str())).map(|s| s.split('/').next().unwrap_or(s).to_string()) } else { None }).collect();
            serde_json::json!({"id": t.id, "asked": asked, "apps": apps, "started_at": t.started_at, "finished_at": t.finished_at, "status": format!("{:?}", t.status).to_lowercase(), "changes": t.entries.len()})
        }).collect();
    serde_json::json!({
        "profile": profile, "setup": setup, "models_on_disk": on_disk, "router": {"endpoint": d.endpoint, "running": running},
        "voice": {"input": voice::available(), "output": voice::speech_available()},
        "os": os_status(),
        "last_generation": agent::LAST_GENERATION.lock().unwrap().clone().map(|(tok, secs, model)| serde_json::json!({"tokens": tok, "seconds": (secs * 10.0).round() / 10.0, "tokens_per_second": (tok as f64 / secs * 10.0).round() / 10.0, "model": model})),
        "sandbox": sandbox::bwrap_available(), "user": read_user_settings(), "history": history,
        "power": if agent::on_battery_saving() { "battery" } else { "ac" },
    })
}

/// The model to use: the configured one if the router serves it, otherwise the best one the router does
/// serve (small packs have no "code" model; the tiny pack has only "fast"). Falls back to the configured name.
/// Routing by task: the maker wants the coder, a chat wants the chat model, and either falls back to
/// the small always-loaded one. A model named explicitly on the command line (anything but "auto")
/// is used as given when the router serves it.
fn served_model(endpoint: &str, wanted: &str) -> String { served_model_for(endpoint, wanted, "make") }

pub(crate) fn served_model_for(endpoint: &str, wanted: &str, kind: &str) -> String {
    let list = ureq::get(&format!("{}/models", endpoint.trim_end_matches('/'))).timeout(std::time::Duration::from_secs(8)).call().ok()
        .and_then(|r| r.into_json::<serde_json::Value>().ok())
        .and_then(|v| v.get("data").and_then(|d| d.as_array()).map(|a| a.iter().filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(|s| s.to_string())).collect::<Vec<_>>()));
    // On battery, with saving on, everything goes to the small always-loaded model -- including a model
    // named on the command line, because "code" is the default there and a laptop unplugged should not
    // start a 9B model for a question a 4B answers. genesis-power puts the big ones away at the same moment.
    let battery = agent::on_battery_saving();
    // The name on the command line wins only for making things. Its default is "code", so for a chat --
    // a crash, a failed service, an update, a question -- it sent every machine to the coder: on a CPU-only
    // laptop that is a five-minute cold load of a 9B model to answer in plain words, and the first real
    // crash handed to the assistant timed out on exactly that. A chat takes the chat model, or the small one.
    match list {
        Some(ids) if !ids.is_empty() => pick_model(&ids, wanted, kind, battery),
        _ => wanted.to_string(),
    }
}

/// The choice itself, apart from the network, so it can be pinned.
pub(crate) fn pick_model(ids: &[String], wanted: &str, kind: &str, battery: bool) -> String {
    let explicit = kind == "make" && !battery && wanted != "auto";
    if explicit && ids.iter().any(|i| i == wanted) { return wanted.to_string(); }
    let prefs: &[&str] = if battery { &["fast", "chat", "code", "auto"] } else { match kind { "chat" => &["chat", "fast", "code", "auto"], "vision" => &["fast", "chat", "code", "auto"], _ => &["code", "fast", "chat", "auto"] } };
    for pref in prefs { if ids.iter().any(|i| i == pref) { return pref.to_string(); } }
    wanted.to_string()
}

/// One queued make: a Trusted session on the project (or a folder named after the request), every
/// permission prompt answered "not now" so it stays inside the project, the outcome recorded for the
/// morning card. Runs on the runner thread; the agent itself runs on its own thread so prompts can be
/// answered while it waits.
/// Take an item off the queue with a reason the morning card can read. A folder the user deleted must be
/// reported once, not retried every minute for ever.
fn drop_queued(item: &queue::Item, state: &str) {
    let mut q = queue::load();
    q.items.retain(|i| i.id != item.id);
    q.done.push(queue::Done { id: item.id.clone(), text: item.text.clone(), state: state.into(), at: queue::now(), summary: String::new(), project: item.project.clone() });
    if q.items.is_empty() { q.start_now = false; }
    queue::save(&q);
}

fn run_queued(d: &Arc<Daemon>, item: &queue::Item) -> Result<()> {
    let project = match if item.project.trim().is_empty() { std::fs::canonicalize(default_project()) } else { std::fs::canonicalize(&item.project) } {
        Ok(p) => p,
        Err(e) => { drop_queued(item, &format!("could not start: {}", e)); return Ok(()); }
    };
    let id = uuid::Uuid::new_v4().to_string();
    let mode = Mode::AutoEdit;
    d.broker.lock().unwrap().open_session(&id, mode, vec![project.display().to_string()], "queue")?;
    let model = served_model_for(&d.endpoint, &d.model, "make");
    let shared = new_shared(&id, mode, &project.display().to_string(), &model);
    let mut agent = Agent::new(Client { endpoint: d.endpoint.clone(), model, api_key: "local".into() }, d.broker.clone(), id.clone(), project, shared.clone());
    agent.tx_store = Store::open(default_store()).ok();
    let slot = Arc::new(Mutex::new(Some(agent)));
    d.sessions.lock().unwrap().insert(id.clone(), (shared.clone(), slot.clone()));
    let text = item.text.clone();
    let worker = std::thread::spawn(move || { let mut g = slot.lock().unwrap(); g.as_mut().map(|a| a.run(&text)) });
    let started = std::time::Instant::now();
    loop {
        std::thread::sleep(std::time::Duration::from_secs(5));
        let (state, pending): (String, Vec<String>) = { let i = shared.info.lock().unwrap(); (i.state.clone(), i.pending.iter().map(|p| p.request_id.clone()).collect()) };
        for p in pending { shared.resolve(&p, false); }
        if state == "done" || state == "error" { break; }
        if started.elapsed() > std::time::Duration::from_secs(1800) { shared.stop.store(true, std::sync::atomic::Ordering::SeqCst); shared.cv.notify_all(); } // half an hour is enough; Stop ends it
        if started.elapsed() > std::time::Duration::from_secs(1900) { break; }
    }
    let _ = worker.join();
    let (state, summary) = { let i = shared.info.lock().unwrap(); (i.state.clone(), i.events.iter().rev().find_map(|e| if let Event::Assistant { text } = e { Some(text.chars().take(200).collect::<String>()) } else { None }).unwrap_or_default()) };
    let mut q = queue::load();
    q.items.retain(|i| i.id != item.id);
    q.done.push(queue::Done { id: item.id.clone(), text: item.text.clone(), state: if state == "done" { "done".into() } else { "did not finish".into() }, at: queue::now(), summary, project: item.project.clone() });
    if q.items.is_empty() { q.start_now = false; }
    queue::save(&q);
    Ok(())
}

/// Finished sessions keep their steps (the list and the page still show them) but give up their agent:
/// the preview servers and the headless browser behind it. Only the three most recent finished ones keep
/// a live preview (the newest one only: ten makes in a row otherwise pile up dev servers until the model
/// service is killed for memory, which the evaluation hit twice)
/// for memory, which is what the evaluation of 17 Sep ran into.
static SESSION_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn release_old_sessions(d: &Arc<Daemon>) {
    let sessions = d.sessions.lock().unwrap();
    let mut finished: Vec<(u64, &String)> = sessions.iter().filter_map(|(id, (shared, _))| {
        let i = shared.info.lock().unwrap();
        if i.state == "done" || i.state == "error" { Some((i.seq, id)) } else { None }
    }).collect();
    if finished.len() <= 1 { return; }
    finished.sort_by_key(|(seq, _)| *seq); // oldest first
    let n = finished.len() - 1;
    for (_, id) in finished.into_iter().take(n) {
        if let Some((shared, slot)) = sessions.get(id) {
            if let Ok(mut g) = slot.try_lock() { if g.take().is_some() { shared.info.lock().unwrap().preview_url = None; } }
        }
    }
}

/// Every place Genesis keeps something of yours, what is in it and how big it is. The paths are real and
/// shown, so nothing about this has to be taken on trust.
fn stored_data() -> serde_json::Value {
    fn size(p: &std::path::Path) -> u64 {
        if p.is_file() { return std::fs::metadata(p).map(|m| m.len()).unwrap_or(0); }
        std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| { let q = e.path(); if q.is_dir() { size(&q) } else { std::fs::metadata(&q).map(|m| m.len()).unwrap_or(0) } }).sum()).unwrap_or(0)
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let data = std::path::PathBuf::from(format!("{}/.local/share/genesis", home));
    let state = std::path::PathBuf::from(format!("{}/.local/state/genesis", home));
    let chats = chat::dir();
    let index = data.join("index.sqlite");
    let audit = default_audit_path();
    let tx = default_store();
    let n_chats = std::fs::read_dir(&chats).map(|rd| rd.flatten().filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false)).count()).unwrap_or(0);
    let idx: serde_json::Value = run_json("/usr/bin/genesis-index", &["status"]);
    serde_json::json!({
        "kinds": [
            {"id": "chats", "what": "Your conversations with Genesis", "detail": format!("{} chats", n_chats), "path": chats.display().to_string(), "bytes": size(&chats)},
            {"id": "index", "what": "The search index of the folders you opted in", "detail": format!("{} files, {} passages", idx.get("files").and_then(|v| v.as_u64()).unwrap_or(0), idx.get("chunks").and_then(|v| v.as_u64()).unwrap_or(0)), "path": index.display().to_string(), "bytes": size(&index)},
            {"id": "activity", "what": "The record of what Genesis did and what you allowed", "detail": "one line per action", "path": audit.display().to_string(), "bytes": size(&audit)},
            {"id": "snapshots", "what": "Snapshots kept so jobs can be undone", "detail": "older than a week can go", "path": tx.display().to_string(), "bytes": size(&tx)},
            {"id": "queue", "what": "Makes queued for the night and their results", "detail": "", "path": state.join("queue.json").display().to_string(), "bytes": size(&state.join("queue.json"))}
        ],
        "note": "Nothing here has ever left this computer. Voice is not kept: what you say is turned into text and the audio is discarded."
    })
}

/// Delete one kind of stored data. Snapshots older than a week only, so today's Undo still works.
fn delete_stored(kind: &str) -> serde_json::Value {
    let home = std::env::var("HOME").unwrap_or_default();
    let done = |what: &str| serde_json::json!({"ok": true, "message": what});
    match kind {
        "chats" => { let _ = std::fs::remove_dir_all(chat::dir()); done("your chats are gone") }
        "index" => { let _ = std::process::Command::new("/usr/bin/genesis-index").arg("reset").status(); let _ = std::fs::remove_file(format!("{}/.local/share/genesis/index.sqlite", home)); done("the search index is gone; the folders you opted in are still listed and will be indexed again") }
        "activity" => { let _ = std::fs::remove_file(default_audit_path()); done("the activity record is gone") }
        "snapshots" => {
            let mut n = 0;
            if let Ok(st) = Store::open(default_store()) {
                let week = time::OffsetDateTime::now_utc() - time::Duration::days(7);
                let cutoff = week.format(&time::format_description::well_known::Rfc3339).unwrap_or_default();
                for t in st.list().unwrap_or_default() {
                    if t.started_at < cutoff { let _ = std::fs::remove_dir_all(default_store().join(&t.id)); n += 1; }
                }
            }
            done(&format!("{} snapshots older than a week removed; newer jobs can still be undone", n))
        }
        "queue" => { let _ = std::fs::remove_file(format!("{}/.local/state/genesis/queue.json", home)); done("the queue and its results are gone") }
        _ => serde_json::json!({"error": "unknown kind"}),
    }
}

/// `~` the way a person writes it.
fn shellexpand(p: &str) -> String {
    match p.strip_prefix('~') { Some(rest) => format!("{}{}", std::env::var("HOME").unwrap_or_default(), rest), None => p.to_string() }
}

/// The user's always/never command rules (Settings > Always and never): kept as JSON, rendered into
/// ~/.config/genesis/policy.toml (a source permd reads), and loaded into the running broker at once.
fn user_rules_path() -> std::path::PathBuf { user_settings_path().with_file_name("rules.json") }

fn user_rules() -> Vec<serde_json::Value> {
    std::fs::read_to_string(user_rules_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save_user_rules(d: &Arc<Daemon>, rules: &[serde_json::Value]) -> Result<()> {
    let dir = user_rules_path().parent().map(|p| p.to_path_buf()).unwrap_or_default();
    std::fs::create_dir_all(&dir)?;
    // a pattern is plain printable text with * as the only special character: anything else could write
    // a policy file that no longer parses, and a daemon that cannot load its policy does not start
    for r in rules {
        let pat = r.get("pattern").and_then(|x| x.as_str()).unwrap_or("");
        if pat.is_empty() || pat.chars().any(|c| c.is_control() || matches!(c, '[' | ']' | '{' | '}' | '\\' | '"')) || !pat.is_ascii() {
            return Err(anyhow!("a pattern is plain text with * as the wildcard; brackets, braces, quotes and backslashes are not allowed"));
        }
    }
    let (old_json, old_toml) = (std::fs::read(user_rules_path()).ok(), std::fs::read(dir.join("policy.toml")).ok());
    std::fs::write(user_rules_path(), serde_json::to_string_pretty(rules)?)?;
    let mut toml = String::from("# Written by Genesis Settings (Always and never). Edit there, not here.\n\n");
    for r in rules {
        let (id, pattern, kind) = (r.get("id").and_then(|x| x.as_str()).unwrap_or(""), r.get("pattern").and_then(|x| x.as_str()).unwrap_or(""), r.get("kind").and_then(|x| x.as_str()).unwrap_or("always"));
        let tier = if kind == "never" { "never" } else { "read_local" };
        let reason = if kind == "never" { "you marked this never" } else { "you marked this always" };
        toml.push_str(&format!("[[commands]]\nid = {:?}\npatterns = [{:?}]\ntier = {:?}\nreason = {:?}\n\n", id, pattern, tier, reason));
    }
    std::fs::write(dir.join("policy.toml"), toml)?;
    let loaded = genesis_permd::Policy::load(&genesis_permd::default_policy_sources()).and_then(|p| d.broker.lock().unwrap().reload_policy(p));
    if let Err(e) = loaded {
        // put back what was there: a bad rule must never survive to the next start
        match old_json { Some(b) => { let _ = std::fs::write(user_rules_path(), b); } None => { let _ = std::fs::remove_file(user_rules_path()); } }
        match old_toml { Some(b) => { let _ = std::fs::write(dir.join("policy.toml"), b); } None => { let _ = std::fs::remove_file(dir.join("policy.toml")); } }
        return Err(anyhow!("that rule does not load, nothing was changed: {}", e));
    }
    Ok(())
}

/// The agent session behind a chat: created on first use (or after a restart) with the stored
/// conversation replayed, so the model sees what was said before.
fn chat_session_for(d: &Arc<Daemon>, c: &chat::Chat) -> Result<String> {
    if let Some(sid) = d.chat_sessions.lock().unwrap().get(&c.id).cloned() {
        if d.sessions.lock().unwrap().contains_key(&sid) { return Ok(sid); }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let project = chat::scratch_dir();
    let mode = Mode::AutoEdit;
    d.broker.lock().unwrap().open_session(&id, mode, vec![project.display().to_string()], "chat")?;
    let model = served_model_for(&d.endpoint, &d.model, "chat");
    let shared = new_shared(&id, mode, &project.display().to_string(), &model);
    let mut agent = Agent::new(Client { endpoint: d.endpoint.clone(), model, api_key: "local".into() }, d.broker.clone(), id.clone(), project, shared.clone());
    agent.tx_store = Store::open(default_store()).ok();
    agent.kind = "chat".into();
    agent.messages[0] = crate::llm::Message::system(agent::SYSTEM_PROMPT_CHAT);
    for m in c.messages.iter().rev().take(40).collect::<Vec<_>>().into_iter().rev() {
        agent.messages.push(if m.role == "assistant" { crate::llm::Message::assistant(m.text.clone()) } else { crate::llm::Message::user(m.text.clone()) });
    }
    d.sessions.lock().unwrap().insert(id.clone(), (shared, Arc::new(Mutex::new(Some(agent)))));
    d.chat_sessions.lock().unwrap().insert(c.id.clone(), id.clone());
    Ok(id)
}

/// Which Genesis is running and whether an update is staged, from os-release and the published bootc status.
fn os_status() -> serde_json::Value {
    let osr = std::fs::read_to_string("/usr/lib/os-release").unwrap_or_default();
    let get = |k: &str| osr.lines().find(|l| l.starts_with(&format!("{}=", k))).map(|l| l[k.len() + 1..].trim_matches('"').to_string()).unwrap_or_default();
    let bootc: serde_json::Value = std::fs::read_to_string("/run/genesis/bootc-status.json").ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(serde_json::Value::Null);
    let booted = bootc.pointer("/status/booted/image/image/image").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let booted_ts = bootc.pointer("/status/booted/image/timestamp").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let staged = bootc.pointer("/status/staged/image/image/image").and_then(|v| v.as_str()).map(|s| s.to_string());
    let staged_ts = bootc.pointer("/status/staged/image/timestamp").and_then(|v| v.as_str()).map(|s| s.to_string());
    serde_json::json!({"name": get("NAME"), "version": get("IMAGE_VERSION"), "pretty": get("PRETTY_NAME"), "image": booted, "built": booted_ts, "staged": staged, "staged_built": staged_ts, "unit_active": std::process::Command::new("systemctl").args(["is-active", "bootc-fetch-apply-updates.service"]).output().map(|o| String::from_utf8_lossy(&o.stdout).trim() == "active").unwrap_or(false)})
}

/// Claude Code (Anthropic's terminal agent, signs in with a Claude account): installed for this user?
fn claude_status() -> serde_json::Value {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let local = std::path::Path::new(&home).join(".local/bin/claude");
    let on_path = std::env::var("PATH").unwrap_or_default().split(':').map(|d| std::path::Path::new(d).join("claude")).find(|p| p.is_file());
    let path = if local.is_file() { Some(local) } else { on_path };
    serde_json::json!({"installed": path.is_some(), "path": path.map(|p| p.display().to_string()), "launcher": "/usr/bin/genesis-claude"})
}

/// The permission broker's audit, newest first: what Genesis was asked to do and what was decided.
fn recent_activity(n: usize) -> serde_json::Value {
    let path = default_audit_path();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut rows: Vec<serde_json::Value> = text.lines().rev().filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|e| e.get("kind").and_then(|k| k.as_str()) == Some("decision") || e.get("kind").and_then(|k| k.as_str()) == Some("resolve"))
        .take(n).map(|e| serde_json::json!({
            "ts": e.get("ts").cloned().unwrap_or(serde_json::Value::Null),
            "kind": e.get("kind").cloned().unwrap_or(serde_json::Value::Null),
            "tool": e.get("tool").cloned().unwrap_or(serde_json::Value::Null),
            "tier": e.get("tier").cloned().unwrap_or(serde_json::Value::Null),
            "verdict": e.get("verdict").cloned().unwrap_or(serde_json::Value::Null),
            "reason": e.get("reason").cloned().unwrap_or(serde_json::Value::Null),
            "command": e.pointer("/detail/command").cloned().unwrap_or(serde_json::Value::Null),
        })).collect();
    rows.reverse(); rows.reverse();
    serde_json::json!({"path": path.display().to_string(), "entries": rows})
}

/// Phones paired through KDE Connect; the work is in `genesis-phone`.
fn phone(args: &[&str]) -> serde_json::Value {
    match std::process::Command::new("/usr/bin/genesis-phone").args(args).output() {
        Ok(o) if o.status.success() => serde_json::from_slice(&o.stdout).unwrap_or(serde_json::json!({ "available": false, "reason": "unexpected output", "devices": [] })),
        Ok(o) => serde_json::json!({ "available": false, "reason": String::from_utf8_lossy(&o.stderr).trim(), "devices": [] }),
        Err(e) => serde_json::json!({ "available": false, "reason": e.to_string(), "devices": [] }),
    }
}

/// Run a Genesis helper that prints JSON and hand its output through.
fn run_json(bin: &str, args: &[&str]) -> serde_json::Value {
    match std::process::Command::new(bin).args(args).output() {
        Ok(o) if o.status.success() => serde_json::from_slice(&o.stdout).unwrap_or(serde_json::json!({"error": "unexpected output"})),
        Ok(o) => serde_json::json!({"error": String::from_utf8_lossy(&o.stderr).trim()}),
        Err(e) => serde_json::json!({"error": e.to_string()}),
    }
}

/// Ask the local vision-capable model about a PNG (base64). The router's "fast" model carries the
/// projector in every pack from 0.1.103; older packs get a clear message instead of an answer.
fn vision_answer(endpoint: &str, question: &str, png_b64: &str) -> anyhow::Result<String> {
    let model = served_model_for(endpoint, "auto", "vision");
    let body = serde_json::json!({
        "model": model, "temperature": 0.2, "max_tokens": 400,
        "messages": [
            {"role": "system", "content": "You are Genesis, answering about a region of the user's screen, on their machine. Be concrete and brief: plain text, no markdown, at most five sentences. If text is visible, read it exactly."},
            {"role": "user", "content": [
                {"type": "text", "text": question},
                {"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{}", png_b64)}}
            ]}
        ]
    });
    let resp = ureq::post(&format!("{}/chat/completions", endpoint.trim_end_matches('/'))).timeout(std::time::Duration::from_secs(240)).send_json(body);
    match resp {
        Ok(r) => {
            let v: serde_json::Value = r.into_json()?;
            Ok(v.pointer("/choices/0/message/content").and_then(|c| c.as_str()).unwrap_or("").trim().to_string())
        }
        Err(ureq::Error::Status(code, r)) => {
            let text = r.into_string().unwrap_or_default();
            if text.contains("image input is not supported") || text.contains("mmproj") || text.contains("multimodal") {
                Err(anyhow::anyhow!("this model pack has no vision model yet; packs from 0.1.103 include one (Settings > Models)"))
            } else { Err(anyhow::anyhow!("model router {}: {}", code, text.chars().take(200).collect::<String>())) }
        }
        Err(e) => Err(anyhow::anyhow!("model router: {}", e)),
    }
}

/// Per-boot secret shared with our own pages and local helpers (0600 under XDG_RUNTIME_DIR).
fn token_path(name: &str) -> std::path::PathBuf {
    let rt = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    std::path::PathBuf::from(rt).join("genesis").join(format!("{}.token", name))
}
fn load_or_create_token(name: &str) -> String {
    let p = token_path(name);
    if let Ok(t) = std::fs::read_to_string(&p) { if t.trim().len() >= 32 { return t.trim().to_string(); } }
    let t = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    if let Some(d) = p.parent() { let _ = std::fs::create_dir_all(d); }
    use std::os::unix::fs::OpenOptionsExt;
    if let Ok(mut f) = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&p) {
        use std::io::Write;
        let _ = f.write_all(t.as_bytes());
    }
    t
}
fn header<'a>(req: &'a Request, name: &'static str) -> Option<&'a str> {
    req.headers().iter().find(|h| h.field.equiv(name)).map(|h| h.value.as_str())
}
/// Same origin only: Host is ours; Origin, when a browser sends one, is ours; Sec-Fetch-Site is not cross-site.
/// The token goes into a page only when the request already proves it knows the token (the Genesis
/// window adds the header from the runtime directory). A bare GET, which a sandboxed shell with network
/// access can make over the shared loopback, gets a page that cannot change anything.
fn page_token<'a>(d: &'a Daemon, req: &Request) -> &'a str {
    if header(req, "X-Genesis-Token").map(|t| t == d.token).unwrap_or(false) { &d.token } else { "" }
}

/// Deny by default: a GET needs the token unless it is one of these. The pages themselves (they carry no
/// token on a bare GET), and the three reads the dock widget makes without one. Every new route is closed
/// until someone adds it here on purpose.
fn open_get(path: &[&str]) -> bool {
    matches!(path, [""] | ["index.html"] | ["workspace"] | ["palette"] | ["chat"] | ["settings"] | ["favicon.ico"]
        | ["api", "health"] | ["api", "system"] | ["api", "sessions"] | ["api", "sessions", _])
}

/// $XDG_RUNTIME_DIR/genesis/agentd.sock: the door for local programs, which the sandbox cannot see.
fn socket_path() -> Option<std::path::PathBuf> {
    std::env::var("XDG_RUNTIME_DIR").ok().map(|r| std::path::PathBuf::from(r).join("genesis").join("agentd.sock"))
}

fn same_origin(req: &Request, listen: &str) -> bool {
    let host_ok = header(req, "Host").map(|h| h == listen).unwrap_or(false);
    let origin_ok = match header(req, "Origin") { None => true, Some(o) => o == format!("http://{}", listen) };
    let sfs_ok = matches!(header(req, "Sec-Fetch-Site"), None | Some("same-origin") | Some("none"));
    host_ok && origin_ok && sfs_ok
}
/// What /api/open may hand to xdg-open: an existing file or folder under the projects or exports
/// directories, resolved through symlinks, and never a launcher or executable.
fn openable(p: &str) -> anyhow::Result<std::path::PathBuf> {
    let canon = std::fs::canonicalize(p).map_err(|_| anyhow!("no such file"))?;
    let home = std::env::var("HOME").unwrap_or_else(|_| "/nonexistent".into());
    let mut roots = vec![format!("{}/Projects", home), format!("{}/Genesis", home), format!("{}/Documents/Genesis", home), format!("{}/Downloads", home)];
    // and every project the maker made, wherever it lives (they sit under the home folder by default)
    roots.extend(maker::made_here(std::path::Path::new(&default_project())).into_iter().map(|m| m.path));
    if !roots.iter().any(|r| std::fs::canonicalize(r).map(|c| canon.starts_with(&c)).unwrap_or(false)) { return Err(anyhow!("only things Genesis made or exported can be opened from here")); }
    let name = canon.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    if name.ends_with(".desktop") || name.ends_with(".sh") || name.ends_with(".run") || name.ends_with(".appimage") { return Err(anyhow!("launchers are not opened from here")); }
    Ok(canon)
}

/// Is the model router answering at all (quick, 1.5 s)? The badge and the start page say so before a job is started.
fn router_ok(endpoint: &str) -> bool {
    ureq::get(&format!("{}/models", endpoint.trim_end_matches('/'))).timeout(std::time::Duration::from_millis(1500)).call().is_ok()
}

/// The pairing code for the phone app: what genesis-companiond encodes, plus the QR as a PNG (qrencode).
fn companion_pairing() -> serde_json::Value {
    let out = match std::process::Command::new("/usr/bin/genesis-companiond").arg("--pairing").output() {
        Ok(o) if o.status.success() => o.stdout,
        _ => return serde_json::json!({"available": false, "reason": "the companion service is not installed"}),
    };
    let payload = String::from_utf8_lossy(&out).trim().to_string();
    let v: serde_json::Value = serde_json::from_str(&payload).unwrap_or(serde_json::Value::Null);
    let qr = std::process::Command::new("qrencode").args(["-o", "-", "-t", "PNG", "-s", "6", "-m", "2"]).arg(format!("genesis-pair:{}", payload)).output().ok()
        .filter(|o| o.status.success()).map(|o| base64_encode(&o.stdout));
    let running = std::process::Command::new("systemctl").args(["--user", "is-active", "genesis-companiond.service"]).output().map(|o| o.status.success()).unwrap_or(false);
    serde_json::json!({"available": true, "running": running, "hosts": v.get("hosts").cloned().unwrap_or(serde_json::json!([])), "port": v.get("port").cloned().unwrap_or(serde_json::json!(11530)),
        "fingerprint": v.get("fp").cloned().unwrap_or(serde_json::Value::Null), "qr_png_b64": qr, "payload": payload})
}

pub(crate) fn base64_encode(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char); out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Recipes: the ones that ship with Genesis (/usr/share/genesis/recipes) and the user's own (~/.local/share/genesis/recipes).
fn my_recipes_dir() -> std::path::PathBuf {
    let base = std::env::var("XDG_DATA_HOME").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| format!("{}/.local/share", std::env::var("HOME").unwrap_or_default()));
    std::path::PathBuf::from(base).join("genesis").join("recipes")
}
fn recipes() -> serde_json::Value {
    let mut out = Vec::new();
    for (dir, mine) in [(std::path::PathBuf::from(std::env::var("GENESIS_RECIPES").unwrap_or_else(|_| "/usr/share/genesis/recipes".into())), false), (my_recipes_dir(), true)] {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            let mut files: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().map(|x| x == "json").unwrap_or(false)).collect();
            files.sort();
            for f in files {
                if let Ok(mut v) = std::fs::read_to_string(&f).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()).ok_or(()) {
                    if let Some(o) = v.as_object_mut() { o.insert("mine".into(), serde_json::json!(mine)); if !o.contains_key("id") { o.insert("id".into(), serde_json::json!(f.file_stem().unwrap().to_string_lossy())); } }
                    out.push(v);
                }
            }
        }
    }
    serde_json::Value::Array(out)
}
fn save_recipe(v: &serde_json::Value) -> anyhow::Result<String> {
    let name = v.get("name").and_then(|n| n.as_str()).map(|s| s.trim()).filter(|s| !s.is_empty()).ok_or_else(|| anyhow!("a name is needed"))?;
    let steps: Vec<String> = v.get("steps").and_then(|s| s.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.trim().to_string())).filter(|s| !s.is_empty()).collect()).unwrap_or_default();
    if steps.is_empty() { return Err(anyhow!("at least one step is needed")); }
    let id: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect::<String>().trim_matches('-').chars().take(48).collect();
    let id = if id.is_empty() { format!("recipe-{}", uuid::Uuid::new_v4().simple()) } else { id };
    let dir = my_recipes_dir(); std::fs::create_dir_all(&dir)?;
    let doc = serde_json::json!({"id": id, "name": name, "template": v.get("template").and_then(|t| t.as_str()).unwrap_or(""), "steps": steps, "description": v.get("description").and_then(|d| d.as_str()).unwrap_or(""), "source": "mine"});
    std::fs::write(dir.join(format!("{}.json", id)), serde_json::to_string_pretty(&doc)?)?;
    Ok(id)
}

/// Percent-decoding for query strings (+ as space), enough for a search box.
fn url_decode(s: &str) -> String {
    let b = s.as_bytes(); let mut out = Vec::with_capacity(b.len()); let mut i = 0;
    while i < b.len() {
        if b[i] == b'+' { out.push(b' '); i += 1; continue; }
        if b[i] == b'%' && i + 2 < b.len() + 0 + 1 && i + 2 <= b.len() - 1 { // two hex digits follow
            if let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16) { out.push(v); i += 3; continue; }
        }
        out.push(b[i]); i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod query_tests {
    #[test]
    fn gets_are_closed_unless_listed() {
        for open in [vec![""], vec!["palette"], vec!["api", "health"], vec!["api", "sessions"], vec!["api", "sessions", "abc"]] { assert!(super::open_get(&open), "{:?} should be open", open); }
        for closed in [vec!["api", "companion"], vec!["api", "backup"], vec!["api", "policy", "rules"], vec!["api", "chats"], vec!["api", "chats", "x"], vec!["api", "queue"], vec!["api", "history", "t", "changes"], vec!["api", "mcp"], vec!["api", "activity"], vec!["api", "index"], vec!["api", "made"], vec!["api", "notices"], vec!["api", "phone"], vec!["api", "anything-new"]] {
            assert!(!super::open_get(&closed), "{:?} must need the token", closed);
        }
    }

    #[test]
    fn decodes() { assert_eq!(super::url_decode("trip+to%20Lisbon%C3%A9%"), "trip to Lisboné%"); }
}
