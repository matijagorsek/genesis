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
    /// Write the router config for this machine from the models already on it, and exit.
    ///
    /// The config is written once, at first run, and the wizard never starts again afterwards — so an
    /// upgrade that changes how models should be run reaches /usr and nothing else. Every fix made on
    /// 19 Sep (the GPU devices taken away, the small models unloading on a CPU pack, what may stay
    /// resident, reusing a processed prompt, starting models through the wrapper that survives a GPU
    /// failure) would have reached a new install and no existing one, including the laptop they were
    /// found on. This is how an upgraded machine gets them.
    #[arg(long)]
    render_router: bool,
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
    /// Per-boot token our page carries on every state-changing call (/run/genesis/firstrun.token).
    token: String,
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
        // which packs have a signed definition on disk (genesis-packs refresh), and when it was resolved
        "signed": packs.iter().filter_map(|p| packs::signed_pack(&p.id).map(|sp| (p.id.clone(), sp.resolved_at))).collect::<std::collections::BTreeMap<_, _>>(),
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
    // same front door as genesis-agentd: our own pages and token holders only (this daemon runs as root)
    if !same_origin(&req, &app.cli.listen) {
        let _ = req.respond(json_response(&serde_json::json!({"error": "forbidden: not a Genesis origin"}), 403));
        return Ok(false);
    }
    if method != Method::Get && header(&req, "X-Genesis-Token") != Some(app.token.as_str()) {
        let _ = req.respond(json_response(&serde_json::json!({"error": "unauthorized: missing Genesis token"}), 401));
        return Ok(false);
    }
    if req.body_length().unwrap_or(0) > (1 << 20) {
        let _ = req.respond(json_response(&serde_json::json!({"error": "request body too large"}), 413));
        return Ok(false);
    }
    let resp = match (method, path.as_str()) {
        (Method::Get, "/") | (Method::Get, "/index.html") => Response::from_string(UI_HTML.replace("__GENESIS_TOKEN__", &app.token)).with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap()),
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
                Some(pack) => match plan_for(&pack, &app.cli.models_dir) {
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
                    match plan_for(&pack, &app.cli.models_dir) {
                        Ok(files) => {
                            let need: u64 = files.iter().filter(|f| !std::path::Path::new(&f.dest).exists()).map(|f| f.size).sum();
                            let free = free_bytes(&app.cli.models_dir);
                            if free > 0 && need > free {
                                app.progress.lock().unwrap().state = "idle".into();
                                let resp = json_response(&serde_json::json!({ "error": format!("this pack needs {:.1} GB more and the models volume has {:.1} GB free; free some space or pick a smaller pack", need as f64 / 1e9, free as f64 / 1e9) }), 409);
                                let _ = req.respond(resp);
                                return Ok(false);
                            }
                            app.settings.lock().unwrap().pack = Some(pack.id.clone());
                            let models_dir = app.cli.models_dir.clone();
                            let router_out = app.cli.router_out.clone();
                            let profile_path = app.cli.profile.clone();
                            // the small model first: the maker can start the moment it is on disk, while the rest
                            // of the pack (embedder, voices, the big models) keeps downloading behind it
                            let order = |r: &str| match r { "fast" => 0, "embed" => 1, "stt" => 2, "tts" => 3, "fim" => 4, "code" => 5, "chat" => 6, _ => 7 };
                            let mut files = files;
                            files.sort_by_key(|f| order(&f.role));
                            let render = std::sync::Arc::new(move || -> Result<()> {
                                let prof: serde_json::Value = std::fs::read_to_string(&profile_path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(serde_json::Value::Null);
                                let cores = prof.pointer("/cpu/cores").and_then(|c| c.as_u64()).unwrap_or(4) as u32;
                                let gpus: Vec<String> = prof.get("gpus").and_then(|g| g.as_array()).map(|a| a.iter().filter_map(|g| g.get("name").and_then(|n| n.as_str()).map(|s| s.to_string())).collect()).unwrap_or_default();
                                let compute = prof.get("gpus").and_then(|g| g.as_array()).map(|a| a.iter().any(|g| g.get("compute_ready").and_then(|c| c.as_bool()).unwrap_or(false))).unwrap_or(false);
                                // the most memory any one GPU has to itself, and whether it shares the machine's
                                let vram_mb = prof.get("gpus").and_then(|g| g.as_array()).map(|a| a.iter().filter(|g| g.get("compute_ready").and_then(|c| c.as_bool()).unwrap_or(false)).filter_map(|g| g.get("vram_mb").and_then(|v| v.as_u64())).max().unwrap_or(0)).unwrap_or(0);
                                let unified = prof.get("unified_memory").and_then(|u| u.as_bool()).unwrap_or(false);
                                // what this machine can hold at once, as the probe worked it out
                                let budget_mb = prof.get("budget_mb").and_then(|b| b.as_u64()).unwrap_or(0);
                                let mut tuning = packs::tuning_for(cores, &gpus, compute, vram_mb, unified);
                                tuning.budget_mb = budget_mb;
                                // If this machine has been measured, that answer wins over anything guessed
                                // from device names. genesis-pick-device writes it after the models land.
                                tuning.device = std::fs::read_to_string("/var/lib/genesis/device.json").ok()
                                    .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                                    .and_then(|v| v.get("device").and_then(|d| d.as_str()).map(|s| s.to_string()));
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
                            // the model file itself is enough to start; the vision projector (mmproj) joins at the
                            // final render. Waiting for both meant a dropped projector download left no model at all.
                            files.sort_by_key(|f| (order(&f.role), f.filename.to_lowercase().contains("mmproj")));
                            let render_early = render.clone();
                            let on_file: Box<dyn Fn(&packs::PlannedFile) + Send> = Box::new(move |f| {
                                if f.role == "fast" && !f.filename.to_lowercase().contains("mmproj") {
                                    match render_early() { Ok(()) => tracing::info!("small model on disk: router up before the rest of the pack"), Err(e) => tracing::warn!(%e, "early router start failed") }
                                }
                            });
                            let render_done = render.clone();
                            let render_done_again = render.clone();
                            let done_marker = app.cli.done_marker.clone();
                            let on_done: Box<dyn FnOnce() -> Result<()> + Send> = Box::new(move || {
                                render_done()?;
                                // The models are here and the config is written: this machine is set up,
                                // whether or not anybody reached the last page of the wizard. Waiting for
                                // that page is why a machine that was finished downloading kept offering
                                // to set itself up — checked on a fresh VM, where the wizard was still
                                // running over a working assistant. Marking it here rather than only at
                                // the next start means the offer stops when it stops being true.
                                if let Some(d) = done_marker.parent() { let _ = std::fs::create_dir_all(d); }
                                if !done_marker.exists() {
                                    let _ = std::fs::write(&done_marker, format!("{}\n", now()));
                                    tracing::info!(marker = %done_marker.display(), "models and config are in place; first run is complete");
                                }
                                // Measure this machine, now that there is something on it to measure with, and
                                // render once more with the answer. It takes a couple of minutes and nobody
                                // waits for it: the assistant works from the render above and gets faster when
                                // the measurement lands. This used to be spawned when the pack was chosen,
                                // which is before a single model has been downloaded — so it found nothing to
                                // measure, said so into a log, and no machine was ever measured at all.
                                // Deciding from device names instead cost the first real laptop 21x its speed.
                                let render_again = render_done_again.clone();
                                std::thread::spawn(move || {
                                    if std::path::Path::new("/var/lib/genesis/device.json").exists() {
                                        return;  // already measured; genesis-pick-device --json redoes it by hand
                                    }
                                    // exit 3 means it ran and could not measure anything; the file it writes
                                    // then says measured: false, and taking it as an answer would record a
                                    // decision nobody made
                                    match std::process::Command::new("/usr/bin/genesis-pick-device").arg("--json").output() {
                                        Ok(o) if o.status.success() => {
                                            let _ = std::fs::create_dir_all("/var/lib/genesis");
                                            if std::fs::write("/var/lib/genesis/device.json", &o.stdout).is_ok() {
                                                tracing::info!("measured this machine; rendering the router again");
                                                let _ = render_again();
                                            }
                                        }
                                        Ok(o) => tracing::warn!(status = ?o.status, stderr = %String::from_utf8_lossy(&o.stderr).chars().take(200).collect::<String>(), "could not measure this machine; keeping what the hardware said"),
                                        Err(e) => tracing::warn!(error = %e, "could not run genesis-pick-device"),
                                    }
                                });
                                Ok(())
                            });
                            download::start_with(app.progress.clone(), pack.id.clone(), files.clone(), on_file, on_done);
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

/// Write the router config from what is on this machine now, without the wizard.
/// The small always-loaded model is on the disk: any .gguf under models/fast that is not a vision projector.
fn has_small_model(models_dir: &std::path::Path) -> bool {
    std::fs::read_dir(models_dir.join("fast")).map(|rd| rd.flatten().any(|e| {
        let n = e.file_name().to_string_lossy().to_string();
        n.ends_with(".gguf") && !n.starts_with("mmproj") && e.metadata().map(|m| m.len() > 1_000_000).unwrap_or(false)
    })).unwrap_or(false)
}

fn render_router_now(cli: &Cli) -> Result<()> {
    if !cli.models_dir.is_dir() {
        tracing::info!("no models on this machine yet; nothing to write");
        return Ok(());
    }
    let prof: serde_json::Value = std::fs::read_to_string(&cli.profile).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(serde_json::Value::Null);
    let cores = prof.pointer("/cpu/cores").and_then(|c| c.as_u64()).unwrap_or(4) as u32;
    let gpus: Vec<String> = prof.get("gpus").and_then(|g| g.as_array()).map(|a| a.iter().filter_map(|g| g.get("name").and_then(|n| n.as_str()).map(|s| s.to_string())).collect()).unwrap_or_default();
    let compute = prof.get("gpus").and_then(|g| g.as_array()).map(|a| a.iter().any(|g| g.get("compute_ready").and_then(|c| c.as_bool()).unwrap_or(false))).unwrap_or(false);
    let vram_mb = prof.get("gpus").and_then(|g| g.as_array()).map(|a| a.iter().filter(|g| g.get("compute_ready").and_then(|c| c.as_bool()).unwrap_or(false)).filter_map(|g| g.get("vram_mb").and_then(|v| v.as_u64())).max().unwrap_or(0)).unwrap_or(0);
    let unified = prof.get("unified_memory").and_then(|u| u.as_bool()).unwrap_or(false);
    let mut tuning = packs::tuning_for(cores, &gpus, compute, vram_mb, unified);
    tuning.budget_mb = prof.get("budget_mb").and_then(|b| b.as_u64()).unwrap_or(0);
    // what this machine measured about itself beats anything read off device names
    tuning.device = std::fs::read_to_string("/var/lib/genesis/device.json").ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("device").and_then(|d| d.as_str()).map(|s| s.to_string()));
    let yaml = packs::render_router_tuned(&cli.models_dir, 10001, tuning)?;
    let same = std::fs::read_to_string(&cli.router_out).map(|old| old == yaml).unwrap_or(false);
    if same {
        tracing::info!("the router config already says what it should; leaving it alone");
        return Ok(());
    }
    if let Some(d) = cli.router_out.parent() { std::fs::create_dir_all(d)?; }
    std::fs::write(&cli.router_out, &yaml).with_context(|| format!("writing {}", cli.router_out.display()))?;
    tracing::info!(path = %cli.router_out.display(), "the router config was rewritten for this machine");
    // --no-block, and it matters. This also runs from genesis-router-refresh, which is ordered *before*
    // the model service so that the service starts with the config this just wrote. Asking systemd to
    // restart that service and then waiting for it deadlocks: systemd will not start the router until
    // this unit finishes, and this unit will not finish until the restart it is waiting on completes.
    // It hung for as long as the timeout allowed with the model service down — a worse outcome than the
    // stale config it was rewriting. Queuing the restart is right in both cases: from the refresh the
    // router starts once this returns, and by hand it restarts a moment later.
    let _ = std::process::Command::new("systemctl").args(["restart", "--no-block", "genesis-router.service"]).status();
    Ok(())
}

fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive("info".parse()?)).with_writer(std::io::stderr).init();
    let cli = Cli::parse();
    if cli.render_router {
        return render_router_now(&cli);
    }
    if cli.done_marker.exists() {
        tracing::info!("first run already completed ({}); nothing to do", cli.done_marker.display());
        return Ok(());
    }
    // A machine can be completely set up and never marked as such: the marker is written when the last
    // page of the wizard is reached, and a person who closes the window once their models have downloaded
    // never reaches it. That machine has its models and its router config and is ready to use — and the
    // wizard would offer to set it up again at every boot for the rest of its life. Setting up is what
    // finishing means, so a machine that is set up is finished.
    // A machine installed from the ISO arrives with the smallest pack already on its disk (the kickstart
    // copies it in), and no router config, because that is written when a download finishes and there was
    // no download. Without this it would show the pack wizard and have no assistant until somebody chose a
    // pack it already had. Models on the disk and no config means: write the config for them now and start
    // the model service, so the first boot answers, offline. The machine is then set up, which the block
    // below recognises. Measuring the devices happens at the next config refresh, as it would after an update.
    if !cli.router_out.exists() && has_small_model(&cli.models_dir) {
        tracing::info!("the small model is on the disk and there is no router config: writing it for the models this machine already has");
        render_router_now(&cli)?;
    }
    if cli.router_out.exists() && cli.models_dir.join("fast").is_dir() {
        if let Some(d) = cli.done_marker.parent() { std::fs::create_dir_all(d)?; }
        std::fs::write(&cli.done_marker, format!("{}\n", now()))?;
        tracing::info!("this machine has its models and its router config already; marking first run complete rather than offering to do it again");
        return Ok(());
    }
    let server = Server::http(&cli.listen).map_err(|e| anyhow::anyhow!("listen {}: {}", cli.listen, e))?;
    tracing::info!(listen = %cli.listen, "genesis-firstrun serving the wizard");
    let exit_on_finish = cli.exit_on_finish;
    let app = Arc::new(App { cli, progress: Arc::new(Mutex::new(Progress { state: "idle".into(), ..Default::default() })), settings: Mutex::new(Settings::default()), token: load_or_create_token() });
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

/// Files to download for a pack: from the signed definition when `genesis-packs refresh` has pulled one
/// (exact files, checksums, no registry call), else resolved live from the Hugging Face API.
fn plan_for(pack: &packs::Pack, models_dir: &std::path::Path) -> Result<Vec<packs::PlannedFile>> {
    if let Some(sp) = packs::signed_pack(&pack.id) {
        tracing::info!(pack = %pack.id, resolved_at = %sp.resolved_at, "using the signed pack definition");
        return Ok(packs::plan_signed(&sp, models_dir));
    }
    packs::plan(pack, models_dir, &packs::hf_files)
}

fn token_path() -> std::path::PathBuf {
    if std::path::Path::new("/run/genesis").is_dir() { std::path::PathBuf::from("/run/genesis/firstrun.token") }
    else { std::path::PathBuf::from(std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into())).join("genesis-firstrun.token") }
}
/// Readable by local users (the page is served with it anyway); what matters is that browsers cannot read files.
fn load_or_create_token() -> String {
    let p = token_path();
    if let Ok(t) = std::fs::read_to_string(&p) { if t.trim().len() >= 32 { return t.trim().to_string(); } }
    let mut buf = [0u8; 32];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") { use std::io::Read; let _ = f.read_exact(&mut buf); }
    let t: String = buf.iter().map(|b| format!("{:02x}", b)).collect();
    use std::os::unix::fs::OpenOptionsExt;
    if let Ok(mut f) = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o644).open(&p) { use std::io::Write; let _ = f.write_all(t.as_bytes()); }
    t
}
fn header<'a>(req: &'a Request, name: &'static str) -> Option<&'a str> {
    req.headers().iter().find(|h| h.field.equiv(name)).map(|h| h.value.as_str())
}
fn same_origin(req: &Request, listen: &str) -> bool {
    let host_ok = header(req, "Host").map(|h| h == listen).unwrap_or(false);
    let origin_ok = match header(req, "Origin") { None => true, Some(o) => o == format!("http://{}", listen) };
    let sfs_ok = matches!(header(req, "Sec-Fetch-Site"), None | Some("same-origin") | Some("none"));
    host_ok && origin_ok && sfs_ok
}

/// Free bytes on the filesystem holding a directory (0 when unknown).
fn free_bytes(dir: &std::path::Path) -> u64 {
    std::process::Command::new("df").args(["--output=avail", "-B1"]).arg(dir).output().ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).lines().nth(1).and_then(|l| l.trim().parse().ok())).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_small_model_on_the_disk_is_recognised_and_a_projector_alone_is_not() {
        let d = tempfile::tempdir().unwrap();
        assert!(!super::has_small_model(d.path()), "no fast folder");
        std::fs::create_dir_all(d.path().join("fast")).unwrap();
        std::fs::write(d.path().join("fast/mmproj-F16.gguf"), vec![0u8; 2_000_000]).unwrap();
        assert!(!super::has_small_model(d.path()), "a vision projector is not a model");
        std::fs::write(d.path().join("fast/partial.gguf"), b"x").unwrap();
        assert!(!super::has_small_model(d.path()), "a stub is not a model");
        std::fs::write(d.path().join("fast/Qwen3.5-2B-Q4_K_M.gguf"), vec![0u8; 2_000_000]).unwrap();
        assert!(super::has_small_model(d.path()));
    }
}
