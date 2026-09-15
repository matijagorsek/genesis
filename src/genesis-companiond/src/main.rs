//! genesis-companiond: Genesis on your phone.
//!
//! A small TLS server on the local network (default 0.0.0.0:11530) that the companion app talks to
//! after pairing. Pairing is a QR code shown in Settings: it carries this machine's address, the
//! certificate fingerprint and a long secret. The app pins the fingerprint and sends the secret as a
//! bearer token; anything else gets 401. Everything is proxied to genesis-agentd on 127.0.0.1 with the
//! daemon's own token, so the permission broker and its cards apply exactly as at the keyboard.
//!
//!   GET  /v1/health                    Genesis version, model, whether the router answers
//!   GET  /v1/sessions                  jobs, newest first
//!   GET  /v1/sessions/{id}             one job: state, steps, pending permission prompts
//!   POST /v1/sessions        {text}    start a job from the phone (default project, default mode)
//!   POST /v1/sessions/{id}/prompt {text}  steer a job
//!   POST /v1/prompts/{id}    {allow}   answer a permission card from the phone
//!   GET  /v1/notices                   the cards the maker shows on its start page
//!   GET  /v1/made                      the "Made here" gallery: what Genesis built on this machine
//!   GET  /v1/screenshot                the whole screen, PNG, on demand (never continuous)
//!
//! Same network only, no relay. The pairing secret and the certificate live in
//! ~/.config/genesis/companion.json (0600). Delete the file to unpair every phone.

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use sha2::Digest;
use std::io::Read;
use std::path::PathBuf;
use tiny_http::{Header, Method, Request, Response, Server, SslConfig};

#[derive(Parser)]
#[command(name = "genesis-companiond", version, about = "Genesis on your phone (TLS API on the local network)")]
struct Cli {
    #[arg(long, default_value = "0.0.0.0:11530", env = "GENESIS_COMPANION_LISTEN")]
    listen: String,
    /// The maker daemon this proxies to.
    #[arg(long, default_value = "http://127.0.0.1:11520", env = "GENESIS_AGENTD")]
    agentd: String,
    /// Print the pairing payload (what the QR encodes) and exit.
    #[arg(long)]
    pairing: bool,
    /// Forget the pairing secret and certificate (every phone must pair again), then exit.
    #[arg(long)]
    reset: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct Pairing {
    token: String,
    cert_pem: String,
    key_pem: String,
    fingerprint: String,
    created: String,
}

fn config_path() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| format!("{}/.config", std::env::var("HOME").unwrap_or_default()));
    PathBuf::from(base).join("genesis").join("companion.json")
}

fn load_or_create_pairing() -> Result<Pairing> {
    let p = config_path();
    if let Ok(t) = std::fs::read_to_string(&p) {
        if let Ok(c) = serde_json::from_str::<Pairing>(&t) { return Ok(c); }
    }
    // a self-signed certificate for this machine; the phone pins its fingerprint from the QR
    let mut names = vec!["genesis".to_string(), "localhost".to_string()];
    names.extend(lan_addresses());
    let ck = rcgen::generate_simple_self_signed(names).context("generating the certificate")?;
    let cert_pem = ck.cert.pem();
    let key_pem = ck.key_pair.serialize_pem();
    let fingerprint = hex::encode(sha2::Sha256::digest(ck.cert.der()));
    let token = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    let c = Pairing { token, cert_pem, key_pem, fingerprint, created: now() };
    if let Some(d) = p.parent() { std::fs::create_dir_all(d)?; }
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&p)?;
    use std::io::Write;
    f.write_all(serde_json::to_string_pretty(&c)?.as_bytes())?;
    Ok(c)
}

fn now() -> String {
    std::process::Command::new("date").arg("-u").arg("+%Y-%m-%dT%H:%M:%SZ").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
}

/// IPv4 addresses of this machine on the local network (no loopback, no link-local).
fn lan_addresses() -> Vec<String> {
    let out = std::process::Command::new("hostname").arg("-I").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    out.split_whitespace().filter(|a| a.contains('.') && !a.starts_with("127.") && !a.starts_with("169.254.")).map(|s| s.to_string()).collect()
}

/// What the QR in Settings encodes: enough for the app to find, verify and authenticate this machine.
fn pairing_payload(c: &Pairing, port: u16) -> serde_json::Value {
    serde_json::json!({
        "v": 1, "name": std::fs::read_to_string("/etc/hostname").map(|s| s.trim().to_string()).unwrap_or_else(|_| "genesis".into()),
        "hosts": lan_addresses(), "port": port, "fp": c.fingerprint, "token": c.token,
    })
}

fn agentd_token() -> String {
    let rt = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    std::fs::read_to_string(format!("{}/genesis/agentd.token", rt)).map(|s| s.trim().to_string()).unwrap_or_default()
}

fn json(v: &serde_json::Value, status: u16) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(v.to_string()).with_status_code(status).with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
}

fn header<'a>(req: &'a Request, name: &'static str) -> Option<&'a str> {
    req.headers().iter().find(|h| h.field.equiv(name)).map(|h| h.value.as_str())
}

/// Forward to agentd; the reply body and status come back as they are.
fn proxy(agentd: &str, method: &str, path: &str, body: Option<&str>) -> (serde_json::Value, u16) {
    let url = format!("{}{}", agentd.trim_end_matches('/'), path);
    let tok = agentd_token();
    let r = match method {
        "POST" => ureq::post(&url).set("X-Genesis-Token", &tok).set("Content-Type", "application/json").timeout(std::time::Duration::from_secs(30)).send_string(body.unwrap_or("{}")),
        _ => ureq::get(&url).set("X-Genesis-Token", &tok).timeout(std::time::Duration::from_secs(30)).call(),
    };
    match r {
        Ok(resp) => { let st = resp.status(); (resp.into_json().unwrap_or(serde_json::json!({"ok": true})), st) }
        Err(ureq::Error::Status(code, resp)) => (serde_json::from_str(&resp.into_string().unwrap_or_default()).unwrap_or(serde_json::json!({"error": "maker error"})), code),
        Err(e) => (serde_json::json!({"error": format!("the maker is not reachable: {}", e)}), 503),
    }
}

/// The whole screen as PNG, through Spectacle in the user's session. On demand only.
fn screenshot() -> Result<Vec<u8>> {
    let path = std::env::temp_dir().join(format!("genesis-companion-{}.png", uuid::Uuid::new_v4().simple()));
    let st = std::process::Command::new("spectacle").args(["-b", "-n", "-f", "-o"]).arg(&path).status().context("running spectacle")?;
    if !st.success() { return Err(anyhow!("spectacle failed")); }
    let bytes = std::fs::read(&path)?;
    let _ = std::fs::remove_file(&path);
    Ok(bytes)
}

fn handle(req: &mut Request, pairing: &Pairing, agentd: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let url = req.url().to_string();
    let path: Vec<&str> = url.split('?').next().unwrap_or("/").trim_matches('/').split('/').collect();
    let method = req.method().clone();
    let auth = header(req, "Authorization").unwrap_or("");
    if auth.strip_prefix("Bearer ").map(|t| t.trim()) != Some(pairing.token.as_str()) {
        return json(&serde_json::json!({"error": "not paired: scan the code in Genesis Settings"}), 401);
    }
    let mut body = String::new();
    if req.body_length().unwrap_or(0) > (1 << 20) { return json(&serde_json::json!({"error": "request body too large"}), 413); }
    let _ = req.as_reader().take((1 << 20) + 1).read_to_string(&mut body);
    match (method, path.as_slice()) {
        (Method::Get, ["v1", "health"]) => {
            let (mut v, st) = proxy(agentd, "GET", "/api/health", None);
            let (sys, _) = proxy(agentd, "GET", "/api/system", None);
            if let Some(o) = v.as_object_mut() { o.insert("os".into(), sys.get("os").cloned().unwrap_or(serde_json::Value::Null)); }
            json(&v, st)
        }
        (Method::Get, ["v1", "sessions"]) => { let (v, st) = proxy(agentd, "GET", "/api/sessions", None); json(&v, st) }
        (Method::Get, ["v1", "sessions", id]) => { let (v, st) = proxy(agentd, "GET", &format!("/api/sessions/{}", id), None); json(&v, st) }
        (Method::Get, ["v1", "notices"]) => { let (v, st) = proxy(agentd, "GET", "/api/notices", None); json(&v, st) }
        (Method::Get, ["v1", "made"]) => { let (v, st) = proxy(agentd, "GET", "/api/made", None); json(&v, st) }
        (Method::Post, ["v1", "sessions"]) => {
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
            let text = v.get("text").and_then(|t| t.as_str()).unwrap_or("").trim().to_string();
            if text.is_empty() { return json(&serde_json::json!({"error": "text required"}), 400); }
            let (sys, _) = proxy(agentd, "GET", "/api/system", None);
            let mode = sys.pointer("/settings/mode").and_then(|m| m.as_str()).unwrap_or("auto_edit").to_string();
            let (created, st) = proxy(agentd, "POST", "/api/sessions", Some(&serde_json::json!({"mode": mode, "project": ""}).to_string()));
            if st >= 300 { return json(&created, st); }
            let id = created.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string();
            let (_, st2) = proxy(agentd, "POST", &format!("/api/sessions/{}/prompt", id), Some(&serde_json::json!({"text": text}).to_string()));
            json(&serde_json::json!({"id": id, "started": st2 < 300}), if st2 < 300 { 200 } else { st2 })
        }
        (Method::Post, ["v1", "sessions", id, "prompt"]) => { let (v, st) = proxy(agentd, "POST", &format!("/api/sessions/{}/prompt", id), Some(&body)); json(&v, st) }
        (Method::Post, ["v1", "prompts", id]) => { let (v, st) = proxy(agentd, "POST", &format!("/api/prompts/{}", id), Some(&body)); json(&v, st) }
        (Method::Get, ["v1", "screenshot"]) => match screenshot() {
            Ok(png) => Response::from_data(png).with_header(Header::from_bytes("Content-Type", "image/png").unwrap()),
            Err(e) => json(&serde_json::json!({"error": e.to_string()}), 503),
        },
        _ => json(&serde_json::json!({"error": "no such route"}), 404),
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive("info".parse()?)).init();
    let cli = Cli::parse();
    if cli.reset { let _ = std::fs::remove_file(config_path()); println!("pairing reset: every phone must scan the code again"); return Ok(()); }
    let pairing = load_or_create_pairing()?;
    let port: u16 = cli.listen.rsplit(':').next().and_then(|p| p.parse().ok()).unwrap_or(11530);
    if cli.pairing { println!("{}", pairing_payload(&pairing, port)); return Ok(()); }
    let server = Server::https(&cli.listen, SslConfig { certificate: pairing.cert_pem.clone().into_bytes(), private_key: pairing.key_pem.clone().into_bytes() })
        .map_err(|e| anyhow!("listen {}: {}", cli.listen, e))?;
    tracing::info!(listen = %cli.listen, fingerprint = %pairing.fingerprint, "genesis-companiond serving (TLS, bearer token)");
    for mut req in server.incoming_requests() {
        let resp = handle(&mut req, &pairing, &cli.agentd);
        let _ = req.respond(resp);
    }
    Ok(())
}
