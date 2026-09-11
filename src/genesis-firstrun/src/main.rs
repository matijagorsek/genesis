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
                            let on_done: Box<dyn FnOnce() -> Result<()> + Send> = Box::new(move || {
                                let yaml = packs::render_router(&models_dir, 10001)?;
                                if let Some(d) = router_out.parent() {
                                    std::fs::create_dir_all(d)?;
                                }
                                std::fs::write(&router_out, yaml).with_context(|| format!("writing {}", router_out.display()))?;
                                tracing::info!(path = %router_out.display(), "router config rendered");
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
