//! genesis-firstrun: the first-run wizard service.
//!
//! Serves a local web UI and a JSON API on 127.0.0.1:11510:
//!   GET  /                      the wizard
//!   GET  /api/state             profile, packs, progress, settings
//!   POST /api/plan   {pack}     download plan with sizes (no download)
//!   POST /api/choose {pack}     start downloading the pack; on completion renders the router config
//!   POST /api/privacy {cloud}   record the privacy choice
//!   POST /api/finish            write the done marker; the service exits
//!
//! Runs as root from genesis-firstrun.service until /var/lib/genesis/first-run-done exists.

mod download;
mod packs;

use anyhow::{Context, Result};
use clap::Parser;
use download::{Progress, Shared};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tiny_http::{Header, Method, Request, Response, Server};

const UI_HTML: &str = include_str!("../ui/index.html");

#[derive(Parser)]
#[command(name = "genesis-firstrun", version, about = "Genesis first-run wizard")]
struct Cli {
    #[arg(long, default_value = "127.0.0.1:11510")]
    listen: String,
    #[arg(long, default_value = "/etc/genesis/profile.json")]
    profile: PathBuf,
    #[arg(long, default_value = "/usr/share/genesis/packs")]
    packs: PathBuf,
    #[arg(long, default_value = "/var/lib/genesis/models")]
    models_dir: PathBuf,
    #[arg(long, default_value = "/etc/genesis/router.yaml")]
    router_out: PathBuf,
    #[arg(long, default_value = "/etc/genesis/settings.json")]
    settings_out: PathBuf,
    #[arg(long, default_value = "/var/lib/genesis/first-run-done")]
    done_marker: PathBuf,
    /// Exit after /api/finish (the systemd unit relies on this).
    #[arg(long, default_value_t = true)]
    exit_on_finish: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Settings {
    cloud_enabled: bool,
    pack: Option<String>,
    completed_at: Option<String>,
}

struct App {
    cli: Cli,
    progress: Shared,
    settings: Mutex<Settings>,
}

fn json_response<T: Serialize>(v: &T, status: u16) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = serde_json::to_vec(v).unwrap_or_default();
    Response::from_data(body).with_status_code(status).with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
}

fn read_body(req: &mut Request) -> String {
    let mut s = String::new();
    let _ = req.as_reader().read_to_string(&mut s);
    s
}

fn now() -> String {
    time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339).unwrap_or_default()
}

fn state(app: &App) -> serde_json::Value {
    let profile: serde_json::Value = std::fs::read_to_string(&app.cli.profile).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(serde_json::Value::Null);
    let packs = packs::list_packs(&app.cli.packs).unwrap_or_default();
    let progress = app.progress.lock().unwrap().clone();
    let settings = app.settings.lock().unwrap().clone();
    serde_json::json!({
        "profile": profile,
        "packs": packs,
        "progress": progress,
        "settings": settings,
        "done": app.cli.done_marker.exists(),
        "version": env!("CARGO_PKG_VERSION"),
    })
}

#[derive(Deserialize)]
struct PackReq {
    pack: String,
}
#[derive(Deserialize)]
struct PrivacyReq {
    cloud: bool,
}

fn handle(app: &Arc<App>, mut req: Request) -> Result<bool> {
    let url = req.url().to_string();
    let path = url.split('?').next().unwrap_or("/").to_string();
    let method = req.method().clone();
    let mut finish = false;
    let resp = match (method, path.as_str()) {
        (Method::Get, "/") | (Method::Get, "/index.html") => Response::from_string(UI_HTML).with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap()),
        (Method::Get, "/api/state") => json_response(&state(app), 200),
        (Method::Get, "/api/phone") => json_response(&phone(&["status"]), 200),
        (Method::Post, "/api/phone/action") => {
            let body = read_body(&mut req);
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
        (Method::Post, "/api/plan") => {
            let body = read_body(&mut req);
            match serde_json::from_str::<PackReq>(&body).ok().and_then(|r| packs::load_pack(&app.cli.packs, &r.pack).ok()) {
                Some(pack) => match packs::plan(&pack, &app.cli.models_dir, &packs::hf_files) {
                    Ok(files) => json_response(&serde_json::json!({ "pack": pack.id, "files": files, "bytes_total": files.iter().map(|f| f.size).sum::<u64>() }), 200),
                    Err(e) => json_response(&serde_json::json!({ "error": e.to_string() }), 502),
                },
                None => json_response(&serde_json::json!({ "error": "unknown pack" }), 400),
            }
        }
        (Method::Post, "/api/choose") => {
            let body = read_body(&mut req);
            // a pack that does not fit this machine is refused unless the caller insists ({force: true})
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                let force = v.get("force").and_then(|f| f.as_bool()).unwrap_or(false);
                if let Some(pack) = v.get("pack").and_then(|p| p.as_str()) {
                    let prof: serde_json::Value = std::fs::read_to_string(&app.cli.profile).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(serde_json::Value::Null);
                    let unfit = prof.get("packs").and_then(|a| a.as_array()).and_then(|a| a.iter().find(|f| f.get("id").and_then(|i| i.as_str()) == Some(pack))).and_then(|f| f.get("fits").and_then(|b| b.as_bool())).map(|fits| !fits).unwrap_or(false);
                    if unfit && !force {
                        let reason = prof.get("packs").and_then(|a| a.as_array()).and_then(|a| a.iter().find(|f| f.get("id").and_then(|i| i.as_str()) == Some(pack))).and_then(|f| f.get("reason").and_then(|r| r.as_str())).unwrap_or("").to_string();
                        let resp = json_response(&serde_json::json!({ "error": format!("pack {} does not fit this machine: {}", pack, reason) }), 409);
                        let _ = req.respond(resp);
                        return Ok(false);
                    }
                }
            }
            let busy = app.progress.lock().unwrap().state == "downloading";
            match (busy, serde_json::from_str::<PackReq>(&body).ok().and_then(|r| packs::load_pack(&app.cli.packs, &r.pack).ok())) {
                (true, _) => json_response(&serde_json::json!({ "error": "a download is already running" }), 409),
                (false, None) => json_response(&serde_json::json!({ "error": "unknown pack" }), 400),
                (false, Some(pack)) => {
                    app.progress.lock().unwrap().state = "planning".into();
                    match packs::plan(&pack, &app.cli.models_dir, &packs::hf_files) {
                        Ok(files) => {
                            app.settings.lock().unwrap().pack = Some(pack.id.clone());
                            let models_dir = app.cli.models_dir.clone();
                            let router_out = app.cli.router_out.clone();
                            let profile_path = app.cli.profile.clone();
                            let on_done: Box<dyn FnOnce() -> Result<()> + Send> = Box::new(move || {
                                let prof: serde_json::Value = std::fs::read_to_string(&profile_path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(serde_json::Value::Null);
                                let cores = prof.pointer("/cpu/cores").and_then(|c| c.as_u64()).unwrap_or(4) as u32;
                                let gpus: Vec<String> = prof.get("gpus").and_then(|g| g.as_array()).map(|a| a.iter().filter_map(|g| g.get("name").and_then(|n| n.as_str()).map(|s| s.to_string())).collect()).unwrap_or_default();
                                let compute = prof.get("gpus").and_then(|g| g.as_array()).map(|a| a.iter().any(|g| g.get("compute_ready").and_then(|c| c.as_bool()).unwrap_or(false))).unwrap_or(false);
                                let tuning = packs::tuning_for(cores, &gpus, compute);
                                let yaml = packs::render_router_tuned(&models_dir, 10001, tuning)?;
                                if let Some(d) = router_out.parent() {
                                    std::fs::create_dir_all(d)?;
                                }
                                std::fs::write(&router_out, yaml).with_context(|| format!("writing {}", router_out.display()))?;
                                tracing::info!(path = %router_out.display(), "router config rendered");
                                // (re)start the local model router so the maker works right away
                                // the service has ConditionPathExists on the config we just wrote; (re)start it now
                                let _ = std::process::Command::new("systemctl").args(["restart", "genesis-router.service"]).status();
                                Ok(())
                            });
                            download::start(app.progress.clone(), pack.id.clone(), files.clone(), on_done);
                            json_response(&serde_json::json!({ "started": true, "files": files.len(), "bytes_total": files.iter().map(|f| f.size).sum::<u64>() }), 202)
                        }
                        Err(e) => {
                            let mut p = app.progress.lock().unwrap();
                            p.state = "error".into();
                            p.error = Some(e.to_string());
                            json_response(&serde_json::json!({ "error": e.to_string() }), 502)
                        }
                    }
                }
            }
        }
        (Method::Post, "/api/privacy") => {
            let body = read_body(&mut req);
            match serde_json::from_str::<PrivacyReq>(&body) {
                Ok(r) => {
                    app.settings.lock().unwrap().cloud_enabled = r.cloud;
                    json_response(&serde_json::json!({ "cloud_enabled": r.cloud }), 200)
                }
                Err(_) => json_response(&serde_json::json!({ "error": "expected {cloud: bool}" }), 400),
            }
        }
        (Method::Post, "/api/finish") => {
            // an optional first thing to make: genesis-window opens the maker with it after this wizard closes
            let body = read_body(&mut req);
            {
                let state = app.progress.lock().unwrap().state.clone();
                let force = serde_json::from_str::<serde_json::Value>(&body).ok().and_then(|v| v.get("force").and_then(|f| f.as_bool())).unwrap_or(false);
                if (state == "downloading" || state == "error") && !force {
                    let resp = json_response(&serde_json::json!({ "error": format!("a download is {}; finish it, retry it, or pass force", state) }), 409);
                    let _ = req.respond(resp);
                    return Ok(false);
                }
            }
            let starter = serde_json::from_str::<serde_json::Value>(&body).ok().and_then(|v| v.get("starter").and_then(|s| s.as_str()).map(|s| s.trim().to_string())).unwrap_or_default();
            if !starter.is_empty() {
                if let Some(d) = app.cli.done_marker.parent() { let _ = std::fs::write(d.join("first-run-starter"), format!("{}\n", starter.chars().take(300).collect::<String>())); }
            }
            let mut s = app.settings.lock().unwrap();
            s.completed_at = Some(now());
            if let Some(d) = app.cli.settings_out.parent() {
                std::fs::create_dir_all(d)?;
            }
            std::fs::write(&app.cli.settings_out, serde_json::to_string_pretty(&*s)?)?;
            if let Some(d) = app.cli.done_marker.parent() {
                std::fs::create_dir_all(d)?;
            }
            std::fs::write(&app.cli.done_marker, format!("{}\n", now()))?;
            tracing::info!(marker = %app.cli.done_marker.display(), "first run complete");
            finish = true;
            json_response(&serde_json::json!({ "done": true }), 200)
        }
        _ => json_response(&serde_json::json!({ "error": "not found" }), 404),
    };
    let _ = req.respond(resp);
    Ok(finish)
}

fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive("info".parse()?)).with_writer(std::io::stderr).init();
    let cli = Cli::parse();
    if cli.done_marker.exists() {
        tracing::info!("first run already completed ({}); nothing to do", cli.done_marker.display());
        return Ok(());
    }
    let server = Server::http(&cli.listen).map_err(|e| anyhow::anyhow!("listen {}: {}", cli.listen, e))?;
    tracing::info!(listen = %cli.listen, "genesis-firstrun serving the wizard");
    let exit_on_finish = cli.exit_on_finish;
    let app = Arc::new(App { cli, progress: Arc::new(Mutex::new(Progress { state: "idle".into(), ..Default::default() })), settings: Mutex::new(Settings::default()) });
    for req in server.incoming_requests() {
        match handle(&app, req) {
            Ok(true) if exit_on_finish => {
                // give the browser a moment to receive the response
                std::thread::sleep(std::time::Duration::from_millis(300));
                break;
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "request failed"),
        }
    }
    Ok(())
}

/// The phone side lives in `genesis-phone` (KDE Connect over D-Bus); the wizard only shows and clicks.
fn phone(args: &[&str]) -> serde_json::Value {
    match std::process::Command::new("/usr/bin/genesis-phone").args(args).output() {
        Ok(o) if o.status.success() => serde_json::from_slice(&o.stdout).unwrap_or(serde_json::json!({ "available": false, "reason": "unexpected output", "devices": [] })),
        Ok(o) => serde_json::json!({ "available": false, "reason": String::from_utf8_lossy(&o.stderr).trim(), "devices": [] }),
        Err(e) => serde_json::json!({ "available": false, "reason": e.to_string(), "devices": [] }),
    }
}
