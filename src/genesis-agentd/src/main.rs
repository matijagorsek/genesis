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

const WORKSPACE_HTML: &str = include_str!("../ui/workspace.html");
const SETTINGS_HTML: &str = include_str!("../ui/settings.html");
const PALETTE_HTML: &str = include_str!("../ui/palette.html");

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
}

fn new_shared(id: &str, mode: Mode, project: &str, model: &str) -> Arc<Shared> {
    Arc::new(Shared { info: Mutex::new(SessionInfo { id: id.into(), mode, project: project.into(), model: model.into(), state: "idle".into(), events: vec![], pending: vec![], transaction: None, preview_url: None, active_project: None, network_uses: vec![] }), answers: Mutex::new(VecDeque::new()), cv: Condvar::new() })
}

fn parse_mode(s: &str) -> Result<Mode> {
    serde_json::from_value(serde_json::Value::String(s.into())).map_err(|_| anyhow!("mode must be assist, auto_edit or autonomous"))
}

fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive("info".parse()?)).with_writer(std::io::stderr).init();
    let cli = Cli::parse();
    let mut sources = default_policy_sources();
    sources.extend(cli.policy.iter().cloned());
    let policy = Policy::load(&sources)?;
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
            let d = Arc::new(Daemon { endpoint: cli.endpoint.clone(), model: cli.model.clone(), broker, sessions: Mutex::new(HashMap::new()), listen: listen.to_string(), token: load_or_create_token("agentd") });
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
struct NewSession { mode: String, project: String }
#[derive(Deserialize)]
struct PromptReq { text: String }
#[derive(Deserialize)]
struct ResolveReq { allow: bool }

fn handle(d: &Arc<Daemon>, mut req: Request) -> Result<()> {
    let url = req.url().to_string();
    let path: Vec<&str> = url.split('?').next().unwrap_or("/").trim_matches('/').split('/').collect();
    let method = req.method().clone();
    // The front door: only our own pages (same origin) and local programs holding the token may talk to
    // this daemon. A web page open in a browser cannot: cross-site requests carry Origin / Sec-Fetch-Site,
    // and a rebound DNS name fails the Host check.
    if !same_origin(&req, &d.listen) {
        return req.respond(json_response(&serde_json::json!({"error": "forbidden: not a Genesis origin"}), 403)).map_err(|e| anyhow!(e));
    }
    if method != Method::Get && header(&req, "X-Genesis-Token") != Some(d.token.as_str()) {
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
        (Method::Get, [""]) | (Method::Get, ["index.html"]) | (Method::Get, ["workspace"]) => Response::from_string(WORKSPACE_HTML.replace("__GENESIS_TOKEN__", &d.token)).with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap()),
        (Method::Get, ["api", "health"]) => json_response(&serde_json::json!({"ok": true, "endpoint": d.endpoint, "router_ok": router_ok(&d.endpoint), "model": served_model(&d.endpoint, &d.model), "sandbox": sandbox::bwrap_available(), "voice": voice::available(), "speech": voice::speech_available(), "default_project": default_project()}), 200),
        (Method::Get, ["api", "templates"]) => json_response(&maker::list_templates(), 200),
        (Method::Get, ["palette"]) => Response::from_string(PALETTE_HTML.replace("__GENESIS_TOKEN__", &d.token)).with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap()),
        (Method::Get, ["settings"]) => Response::from_string(SETTINGS_HTML.replace("__GENESIS_TOKEN__", &d.token)).with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap()),
        (Method::Get, ["api", "system"]) => json_response(&system_overview(d), 200),
        (Method::Post, ["api", "system", "theme"]) => {
            let want = serde_json::from_str::<serde_json::Value>(&body).ok().and_then(|v| v.get("theme").and_then(|m| m.as_str()).map(|s| s.to_string())).unwrap_or_else(|| "toggle".into());
            if !["dark", "light", "toggle"].contains(&want.as_str()) { json_response(&serde_json::json!({"error": "expected {theme: dark|light|toggle}"}), 400) }
            else { match std::process::Command::new("genesis-theme").arg(&want).output() {
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
        (Method::Get, ["api", "sessions"]) => {
            let list: Vec<serde_json::Value> = d.sessions.lock().unwrap().values().map(|(s, _)| { let i = s.info.lock().unwrap(); serde_json::json!({"id": i.id, "mode": i.mode, "project": i.project, "state": i.state, "transaction": i.transaction}) }).collect();
            json_response(&list, 200)
        }
        (Method::Post, ["api", "sessions"]) => match serde_json::from_str::<NewSession>(&body) {
            Ok(n) => match (parse_mode(&n.mode), std::fs::canonicalize(if n.project.trim().is_empty() { default_project() } else { n.project.clone() })) {
                (Ok(mode), Ok(project)) => {
                    let id = uuid::Uuid::new_v4().to_string();
                    d.broker.lock().unwrap().open_session(&id, mode, vec![project.display().to_string()], "api")?;
                    let model = served_model(&d.endpoint, &d.model);
                    let shared = new_shared(&id, mode, &project.display().to_string(), &model);
                    let mut agent = Agent::new(Client { endpoint: d.endpoint.clone(), model, api_key: "local".into() }, d.broker.clone(), id.clone(), project, shared.clone());
                    agent.tx_store = Store::open(default_store()).ok();
                    d.sessions.lock().unwrap().insert(id.clone(), (shared, Arc::new(Mutex::new(Some(agent)))));
                    json_response(&serde_json::json!({"id": id}), 201)
                }
                (Err(e), _) => json_response(&serde_json::json!({"error": e.to_string()}), 400),
                (_, Err(e)) => json_response(&serde_json::json!({"error": format!("project: {}", e)}), 400),
            },
            Err(_) => json_response(&serde_json::json!({"error": "expected {mode, project}"}), 400),
        },
        (Method::Get, ["api", "sessions", id]) => match d.sessions.lock().unwrap().get(*id) {
            Some((shared, _)) => json_response(&*shared.info.lock().unwrap(), 200),
            None => json_response(&serde_json::json!({"error": "no such session"}), 404),
        },
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
        .map(|t| serde_json::json!({"id": t.id, "started_at": t.started_at, "finished_at": t.finished_at, "status": format!("{:?}", t.status).to_lowercase(), "changes": t.entries.len()})).collect();
    serde_json::json!({
        "profile": profile, "setup": setup, "models_on_disk": on_disk, "router": {"endpoint": d.endpoint, "running": running},
        "voice": {"input": voice::available(), "output": voice::speech_available()},
        "os": os_status(),
        "last_generation": agent::LAST_GENERATION.lock().unwrap().clone().map(|(tok, secs, model)| serde_json::json!({"tokens": tok, "seconds": (secs * 10.0).round() / 10.0, "tokens_per_second": (tok as f64 / secs * 10.0).round() / 10.0, "model": model})),
        "sandbox": sandbox::bwrap_available(), "user": read_user_settings(), "history": history,
    })
}

/// The model to use: the configured one if the router serves it, otherwise the best one the router does
/// serve (small packs have no "code" model; the tiny pack has only "fast"). Falls back to the configured name.
fn served_model(endpoint: &str, wanted: &str) -> String {
    let list = ureq::get(&format!("{}/models", endpoint.trim_end_matches('/'))).timeout(std::time::Duration::from_secs(8)).call().ok()
        .and_then(|r| r.into_json::<serde_json::Value>().ok())
        .and_then(|v| v.get("data").and_then(|d| d.as_array()).map(|a| a.iter().filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(|s| s.to_string())).collect::<Vec<_>>()));
    match list {
        Some(ids) if !ids.is_empty() => {
            if ids.iter().any(|i| i == wanted) { return wanted.to_string(); }
            for pref in ["code", "chat", "fast", "auto"] { if ids.iter().any(|i| i == pref) { return pref.to_string(); } }
            wanted.to_string()
        }
        _ => wanted.to_string(),
    }
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
    let model = served_model(endpoint, "fast");
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

fn base64_encode(bytes: &[u8]) -> String {
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
