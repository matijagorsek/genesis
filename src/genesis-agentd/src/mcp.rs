//! MCP servers as the extension point: anyone can give the maker a new tool without touching Rust.
//!
//! A server is a program speaking the Model Context Protocol over stdio (JSON-RPC 2.0, one message per
//! line). Genesis starts it on first use, lists its tools, and offers them to the model as
//! `mcp__<server>__<tool>`. Every call still goes through the permission broker: the server's config
//! says which tier its tools are (read, write, network, system, never), per tool if needed, and the
//! broker answers per session mode like for any other tool. The server runs as the user, unsandboxed:
//! it is the user's own choice of tool, the same as a program they run in a terminal.
//!
//! Where servers are declared:
//!   /usr/share/genesis/mcp/*.json      shipped with the image
//!   ~/.config/genesis/mcp.json         the user's own; an entry with the same name overrides a shipped one
//! Shape: {"servers": {"name": {"command": "...", "args": [...], "env": {...}, "tier": "write",
//!         "tiers": {"tool": "system"}, "description": "...", "disabled": false}}}

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Default tier for every tool of this server: read | write | network | system | never.
    #[serde(default = "default_tier")]
    pub tier: String,
    /// Per-tool overrides of the tier.
    #[serde(default)]
    pub tiers: BTreeMap<String, String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub disabled: bool,
}

fn default_tier() -> String { "write".into() }

#[derive(Debug, Clone, Serialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub schema: Value,
    pub tier: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServerStatus {
    pub name: String,
    pub description: String,
    pub command: String,
    pub source: String,
    pub running: bool,
    pub error: Option<String>,
    pub tools: Vec<ToolInfo>,
}

/// One running server: its process and a reader thread that hands every JSON line over a channel.
struct Live {
    child: Child,
    stdin: std::process::ChildStdin,
    rx: Receiver<Value>,
    next_id: u64,
    pending: Vec<Value>,
    tools: Vec<ToolInfo>,
}

impl Drop for Live {
    fn drop(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); }
}

impl Live {
    fn start(name: &str, cfg: &ServerConfig) -> Result<Live> {
        let mut cmd = Command::new(&cfg.command);
        cmd.args(&cfg.args).envs(&cfg.env).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        let mut child = cmd.spawn().with_context(|| format!("starting MCP server {} ({})", name, cfg.command))?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = channel::<Value>();
        std::thread::Builder::new().name(format!("mcp-{}", name)).spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if let Ok(v) = serde_json::from_str::<Value>(&line) { if tx.send(v).is_err() { break; } }
            }
        })?;
        let mut live = Live { child, stdin, rx, next_id: 1, pending: Vec::new(), tools: Vec::new() };
        let init = live.request("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "genesis-agentd", "version": env!("CARGO_PKG_VERSION")}}), Duration::from_secs(30))?;
        tracing::info!(server = name, info = ?init.get("serverInfo"), "MCP server up");
        live.notify("notifications/initialized", json!({}))?;
        let listed = live.request("tools/list", json!({}), Duration::from_secs(30))?;
        let tools = listed.get("tools").and_then(|t| t.as_array()).cloned().unwrap_or_default();
        live.tools = tools.iter().filter_map(|t| {
            let tool = t.get("name")?.as_str()?.to_string();
            let tier = cfg.tiers.get(&tool).cloned().unwrap_or_else(|| cfg.tier.clone());
            Some(ToolInfo { name: tool, description: t.get("description").and_then(|d| d.as_str()).unwrap_or("").to_string(), schema: t.get("inputSchema").cloned().unwrap_or(json!({"type":"object","properties":{}})), tier })
        }).collect();
        Ok(live)
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        let msg = json!({"jsonrpc": "2.0", "method": method, "params": params});
        self.stdin.write_all(format!("{}\n", msg).as_bytes())?;
        self.stdin.flush()?;
        Ok(())
    }

    fn request(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let id = self.next_id; self.next_id += 1;
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.stdin.write_all(format!("{}\n", msg).as_bytes())?;
        self.stdin.flush()?;
        if let Some(pos) = self.pending.iter().position(|v| v.get("id").and_then(|i| i.as_u64()) == Some(id)) {
            return Self::unwrap(self.pending.remove(pos));
        }
        let deadline = Instant::now() + timeout;
        loop {
            let now = Instant::now();
            if now >= deadline { return Err(anyhow!("MCP server did not answer {} within {:?}", method, timeout)); }
            match self.rx.recv_timeout(deadline - now) {
                Ok(v) => {
                    if v.get("id").and_then(|i| i.as_u64()) == Some(id) { return Self::unwrap(v); }
                    if v.get("id").is_some() && v.get("method").is_none() { self.pending.push(v); } // an answer to something else
                    // requests from the server (sampling, roots) and notifications are ignored
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => return Err(anyhow!("MCP server exited")),
            }
        }
    }

    fn unwrap(v: Value) -> Result<Value> {
        if let Some(e) = v.get("error") {
            return Err(anyhow!("{}", e.get("message").and_then(|m| m.as_str()).unwrap_or("MCP error")));
        }
        Ok(v.get("result").cloned().unwrap_or(Value::Null))
    }
}

struct Registry {
    configs: BTreeMap<String, (ServerConfig, String)>,
    live: HashMap<String, Live>,
    errors: HashMap<String, String>,
}

static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();

fn registry() -> &'static Mutex<Registry> {
    REGISTRY.get_or_init(|| Mutex::new(Registry { configs: load_configs(), live: HashMap::new(), errors: HashMap::new() }))
}

pub fn shipped_dir() -> PathBuf { PathBuf::from(std::env::var("GENESIS_MCP_DIR").unwrap_or_else(|_| "/usr/share/genesis/mcp".into())) }
pub fn user_file() -> PathBuf {
    let conf = std::env::var("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into())).join(".config"));
    conf.join("genesis").join("mcp.json")
}

fn load_from(v: Value, source: &str, into: &mut BTreeMap<String, (ServerConfig, String)>) {
    if let Some(servers) = v.get("servers").and_then(|s| s.as_object()) {
        for (name, cfg) in servers {
            let ok_name = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if !ok_name { tracing::warn!(name, "MCP server name must be letters, digits, - or _"); continue; }
            match serde_json::from_value::<ServerConfig>(cfg.clone()) {
                Ok(c) => { into.insert(name.clone(), (c, source.to_string())); }
                Err(e) => tracing::warn!(name, %e, "bad MCP server entry"),
            }
        }
    }
}

pub fn load_configs() -> BTreeMap<String, (ServerConfig, String)> {
    let mut out = BTreeMap::new();
    if let Ok(rd) = std::fs::read_dir(shipped_dir()) {
        let mut files: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().map(|e| e == "json").unwrap_or(false)).collect();
        files.sort();
        for f in files {
            if let Ok(v) = std::fs::read_to_string(&f).and_then(|s| serde_json::from_str::<Value>(&s).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))) { load_from(v, "shipped", &mut out); }
        }
    }
    if let Ok(s) = std::fs::read_to_string(user_file()) {
        match serde_json::from_str::<Value>(&s) { Ok(v) => load_from(v, "yours", &mut out), Err(e) => tracing::warn!(%e, "~/.config/genesis/mcp.json is not valid JSON") }
    }
    out.retain(|_, (c, _)| !c.disabled);
    out
}

/// Forget every running server and re-read the configuration (Settings > Tools > Reload).
pub fn reload() {
    let mut r = registry().lock().unwrap();
    r.live.clear(); r.errors.clear();
    r.configs = load_configs();
}

fn ensure_started(r: &mut Registry, name: &str) -> Result<()> {
    if r.live.contains_key(name) { return Ok(()); }
    let (cfg, _) = r.configs.get(name).cloned().ok_or_else(|| anyhow!("no MCP server named {}", name))?;
    match Live::start(name, &cfg) {
        Ok(l) => { r.live.insert(name.to_string(), l); r.errors.remove(name); Ok(()) }
        Err(e) => { r.errors.insert(name.to_string(), e.to_string()); Err(e) }
    }
}

/// Start every configured server (once) and return the tools as OpenAI-style function schemas.
pub fn tool_schemas() -> Vec<Value> {
    let mut r = registry().lock().unwrap();
    let names: Vec<String> = r.configs.keys().cloned().collect();
    let mut out = Vec::new();
    for name in names {
        if ensure_started(&mut r, &name).is_err() { continue; }
        for t in &r.live[&name].tools {
            let desc = if t.description.is_empty() { format!("Tool {} from the {} server.", t.name, name) } else { t.description.clone() };
            out.push(json!({"type": "function", "function": {"name": tool_id(&name, &t.name), "description": format!("{} (from your {} tools; {} tier)", desc.chars().take(400).collect::<String>(), name, t.tier), "parameters": t.schema}}));
        }
    }
    out
}

pub fn tool_id(server: &str, tool: &str) -> String {
    let clean: String = tool.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
    format!("mcp__{}__{}", server, clean)
}

/// Split `mcp__server__tool` back into (server, tool) using the live tool list (tool names may carry
/// characters the model-facing id replaced).
pub fn split_id(id: &str) -> Option<(String, String)> {
    let rest = id.strip_prefix("mcp__")?;
    let (server, tool) = rest.split_once("__")?;
    let r = registry().lock().unwrap();
    let live = r.live.get(server)?;
    let real = live.tools.iter().find(|t| tool_id(server, &t.name) == id).map(|t| t.name.clone()).unwrap_or_else(|| tool.to_string());
    Some((server.to_string(), real))
}

/// The permission tier for a tool: "read" | "write" | "network" | "system" | "never".
pub fn tier_of(server: &str, tool: &str) -> String {
    let r = registry().lock().unwrap();
    r.live.get(server).and_then(|l| l.tools.iter().find(|t| t.name == tool)).map(|t| t.tier.clone())
        .or_else(|| r.configs.get(server).map(|(c, _)| c.tiers.get(tool).cloned().unwrap_or_else(|| c.tier.clone())))
        .unwrap_or_else(|| "write".into())
}

/// Call a tool; the text content of the result, joined.
pub fn call(server: &str, tool: &str, args: &Value) -> Result<String> {
    let mut r = registry().lock().unwrap();
    ensure_started(&mut r, server)?;
    let live = r.live.get_mut(server).unwrap();
    let res = match live.request("tools/call", json!({"name": tool, "arguments": args}), Duration::from_secs(300)) {
        Ok(v) => v,
        Err(e) => { r.live.remove(server); return Err(anyhow!("{} ({} restarts next time)", e, server)); }
    };
    let mut text = res.get("content").and_then(|c| c.as_array()).map(|items| items.iter().filter_map(|i| {
        match i.get("type").and_then(|t| t.as_str()) {
            Some("text") => i.get("text").and_then(|t| t.as_str()).map(|s| s.to_string()),
            Some(other) => Some(format!("[{} content omitted]", other)),
            None => None,
        }
    }).collect::<Vec<_>>().join("\n")).unwrap_or_default();
    if text.is_empty() { text = res.get("structuredContent").map(|s| s.to_string()).unwrap_or_else(|| "(no output)".into()); }
    if res.get("isError").and_then(|e| e.as_bool()).unwrap_or(false) { return Err(anyhow!("{}", text)); }
    if text.len() > 60_000 { text.truncate(60_000); text.push_str("\n…[truncated]"); }
    Ok(text)
}

/// For Settings: every configured server, whether it runs, its tools and the last error.
pub fn status() -> Vec<ServerStatus> {
    let mut r = registry().lock().unwrap();
    let names: Vec<String> = r.configs.keys().cloned().collect();
    let mut out = Vec::new();
    for name in names {
        let _ = ensure_started(&mut r, &name);
        let (cfg, source) = r.configs[&name].clone();
        out.push(ServerStatus { name: name.clone(), description: cfg.description.clone(), command: std::iter::once(cfg.command.clone()).chain(cfg.args.iter().cloned()).collect::<Vec<_>>().join(" "), source, running: r.live.contains_key(&name), error: r.errors.get(&name).cloned(), tools: r.live.get(&name).map(|l| l.tools.clone()).unwrap_or_default() });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A five-line MCP server in Python: enough to prove the handshake, the listing and a call.
    const FAKE: &str = r#"
import sys, json
for line in sys.stdin:
    m = json.loads(line)
    if 'id' not in m: continue
    if m['method'] == 'initialize': r = {'protocolVersion': '2025-06-18', 'capabilities': {'tools': {}}, 'serverInfo': {'name': 'fake', 'version': '0'}}
    elif m['method'] == 'tools/list': r = {'tools': [{'name': 'echo', 'description': 'Echo text', 'inputSchema': {'type': 'object', 'properties': {'text': {'type': 'string'}}, 'required': ['text']}}, {'name': 'boom', 'inputSchema': {'type': 'object'}}]}
    elif m['method'] == 'tools/call':
        if m['params']['name'] == 'boom': r = {'content': [{'type': 'text', 'text': 'it broke'}], 'isError': True}
        else: r = {'content': [{'type': 'text', 'text': 'echo: ' + m['params']['arguments']['text']}]}
    else: r = {}
    sys.stdout.write(json.dumps({'jsonrpc': '2.0', 'id': m['id'], 'result': r}) + '\n'); sys.stdout.flush()
"#;

    #[test]
    fn fake_server_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake.py");
        std::fs::write(&script, FAKE).unwrap();
        let cfg = ServerConfig { command: "python3".into(), args: vec![script.display().to_string()], tier: "read".into(), tiers: [("boom".to_string(), "system".to_string())].into_iter().collect(), ..Default::default() };
        let mut live = Live::start("fake", &cfg).unwrap();
        assert_eq!(live.tools.len(), 2);
        assert_eq!(live.tools[0].tier, "read");
        assert_eq!(live.tools[1].tier, "system");
        let r = live.request("tools/call", json!({"name": "echo", "arguments": {"text": "hi"}}), Duration::from_secs(10)).unwrap();
        assert_eq!(r["content"][0]["text"], "echo: hi");
        assert_eq!(tool_id("fake", "weird name!"), "mcp__fake__weird_name_");
    }

    #[test]
    fn configs_user_overrides_shipped_and_disabled_drops() {
        let dir = tempfile::tempdir().unwrap();
        let shipped = dir.path().join("shipped"); std::fs::create_dir_all(&shipped).unwrap();
        std::fs::write(shipped.join("a.json"), r#"{"servers": {"one": {"command": "x"}, "two": {"command": "y", "tier": "system"}}}"#).unwrap();
        let conf = dir.path().join("conf"); std::fs::create_dir_all(conf.join("genesis")).unwrap();
        std::fs::write(conf.join("genesis/mcp.json"), r#"{"servers": {"two": {"command": "z", "disabled": true}, "bad name": {"command": "q"}, "three": {"command": "w", "tier": "network"}}}"#).unwrap();
        std::env::set_var("GENESIS_MCP_DIR", &shipped); std::env::set_var("XDG_CONFIG_HOME", &conf);
        let c = load_configs();
        assert_eq!(c.keys().cloned().collect::<Vec<_>>(), vec!["one".to_string(), "three".to_string()]);
        assert_eq!(c["one"].0.tier, "write");
        assert_eq!(c["three"].1, "yours");
    }
}
