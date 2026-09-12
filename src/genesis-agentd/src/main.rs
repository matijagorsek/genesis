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
            let d = Arc::new(Daemon { endpoint: cli.endpoint.clone(), model: cli.model.clone(), broker, sessions: Mutex::new(HashMap::new()) });
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
    let mut raw: Vec<u8> = Vec::new();
    let _ = req.as_reader().read_to_end(&mut raw);
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
        (Method::Get, [""]) | (Method::Get, ["index.html"]) | (Method::Get, ["workspace"]) => Response::from_string(WORKSPACE_HTML).with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap()),
        (Method::Get, ["api", "health"]) => json_response(&serde_json::json!({"ok": true, "endpoint": d.endpoint, "model": d.model, "sandbox": sandbox::bwrap_available(), "voice": voice::available(), "speech": voice::speech_available(), "default_project": default_project()}), 200),
        (Method::Get, ["api", "templates"]) => json_response(&maker::list_templates(), 200),
        (Method::Get, ["api", "made"]) => json_response(&maker::made_here(std::path::Path::new(&default_project())), 200),
        (Method::Get, ["api", "sessions"]) => {
            let list: Vec<serde_json::Value> = d.sessions.lock().unwrap().values().map(|(s, _)| { let i = s.info.lock().unwrap(); serde_json::json!({"id": i.id, "mode": i.mode, "project": i.project, "state": i.state, "transaction": i.transaction}) }).collect();
            json_response(&list, 200)
        }
        (Method::Post, ["api", "sessions"]) => match serde_json::from_str::<NewSession>(&body) {
            Ok(n) => match (parse_mode(&n.mode), std::fs::canonicalize(&n.project)) {
                (Ok(mode), Ok(project)) => {
                    let id = uuid::Uuid::new_v4().to_string();
                    d.broker.lock().unwrap().open_session(&id, mode, vec![project.display().to_string()], "api")?;
                    let shared = new_shared(&id, mode, &project.display().to_string(), &d.model);
                    let mut agent = Agent::new(Client { endpoint: d.endpoint.clone(), model: d.model.clone(), api_key: "local".into() }, d.broker.clone(), id.clone(), project, shared.clone());
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
