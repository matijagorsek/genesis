//! The agent loop: prompt -> model -> tool calls -> permission broker -> execute -> repeat.
//! Every tool call becomes a permd Intent. Prompts block the loop until a client resolves them.

use crate::llm::{Client, Message};
use crate::sandbox;
use anyhow::{anyhow, Result};
use genesis_permd::{Broker, Intent, Mode, Verdict};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub const SYSTEM_PROMPT: &str = "You are Genesis, the local assistant built into this computer. You work inside one project directory. \
Use tools to inspect and change files and to run commands; never guess file contents. \
Before changing anything, read what is there. Keep changes small and verify them (run tests or the program). \
Some actions need the user's permission; if a tool result says the action was denied or is waiting, do not retry it, explain instead. \
When the task is complete, reply with a short summary of what you did and how to run or verify it.";

pub fn tool_schemas() -> Value {
    json!([
        {"type":"function","function":{"name":"shell","description":"Run a shell command in the project directory. Returns exit code, stdout and stderr.","parameters":{"type":"object","properties":{"command":{"type":"string"},"needs_network":{"type":"boolean","description":"true if the command must reach the network (installs, fetches)"}},"required":["command"]}}},
        {"type":"function","function":{"name":"read_file","description":"Read a text file. Path relative to the project or absolute.","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}},
        {"type":"function","function":{"name":"write_file","description":"Create or overwrite a text file with the given content.","parameters":{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}}},
        {"type":"function","function":{"name":"edit_file","description":"Replace one exact occurrence of old_text with new_text in a file. Fails if old_text is not found exactly once.","parameters":{"type":"object","properties":{"path":{"type":"string"},"old_text":{"type":"string"},"new_text":{"type":"string"}},"required":["path","old_text","new_text"]}}},
        {"type":"function","function":{"name":"list_dir","description":"List files and directories under a path (non-recursive).","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}}
    ])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    UserPrompt { text: String },
    Assistant { text: String },
    ToolCall { id: String, name: String, args: Value },
    Decision { id: String, tier: String, verdict: String, reason: String, request_id: String },
    ToolResult { id: String, ok: bool, summary: String },
    Waiting { request_id: String, name: String, args: Value, reason: String },
    Resolved { request_id: String, allowed: bool },
    Error { text: String },
    Done { turns: usize },
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionInfo {
    pub id: String,
    pub mode: Mode,
    pub project: String,
    pub model: String,
    pub state: String, // idle | running | waiting | done | error
    pub events: Vec<Event>,
    pub pending: Vec<PendingPrompt>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PendingPrompt {
    pub request_id: String,
    pub tool: String,
    pub args: Value,
    pub tier: String,
    pub reason: String,
}

/// Shared between the running loop and the API: events, pending prompts and their answers.
pub struct Shared {
    pub info: Mutex<SessionInfo>,
    pub answers: Mutex<VecDeque<(String, bool)>>,
    pub cv: Condvar,
}

impl Shared {
    pub fn push(&self, e: Event) {
        self.info.lock().unwrap().events.push(e);
    }
    pub fn set_state(&self, s: &str) {
        self.info.lock().unwrap().state = s.into();
    }
    /// Block until a client resolves `request_id` (or the timeout passes). Returns Some(allowed).
    pub fn wait_answer(&self, request_id: &str, timeout: Duration) -> Option<bool> {
        let deadline = std::time::Instant::now() + timeout;
        let mut q = self.answers.lock().unwrap();
        loop {
            if let Some(pos) = q.iter().position(|(id, _)| id == request_id) {
                let (_, allowed) = q.remove(pos).unwrap();
                return Some(allowed);
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                return None;
            }
            let (guard, _) = self.cv.wait_timeout(q, deadline - now).unwrap();
            q = guard;
        }
    }
    pub fn resolve(&self, request_id: &str, allowed: bool) {
        self.answers.lock().unwrap().push_back((request_id.to_string(), allowed));
        self.info.lock().unwrap().pending.retain(|p| p.request_id != request_id);
        self.cv.notify_all();
        self.push(Event::Resolved { request_id: request_id.into(), allowed });
    }
}

pub struct Agent {
    pub client: Client,
    pub broker: Arc<Mutex<Broker>>,
    pub session_id: String,
    pub project: PathBuf,
    pub shared: Arc<Shared>,
    pub max_turns: usize,
    pub prompt_timeout: Duration,
    pub messages: Vec<Message>,
}

fn resolve_path(project: &Path, p: &str) -> PathBuf {
    let path = Path::new(p);
    if path.is_absolute() { path.to_path_buf() } else { project.join(path) }
}

impl Agent {
    pub fn new(client: Client, broker: Arc<Mutex<Broker>>, session_id: String, project: PathBuf, shared: Arc<Shared>) -> Self {
        Agent { client, broker, session_id, project, shared, max_turns: 40, prompt_timeout: Duration::from_secs(600), messages: vec![Message::system(SYSTEM_PROMPT)] }
    }

    /// Run one user prompt to completion (or until a tool call is denied and the model gives up).
    pub fn run(&mut self, text: &str) -> Result<String> {
        self.shared.push(Event::UserPrompt { text: text.into() });
        self.shared.set_state("running");
        self.messages.push(Message::user(format!("Project directory: {}\n\nTask: {}", self.project.display(), text)));
        let tools = tool_schemas();
        let mut final_text = String::new();
        for turn in 0..self.max_turns {
            let reply = match self.client.chat(&self.messages, &tools, 0.2) {
                Ok(r) => r,
                Err(e) => {
                    self.shared.push(Event::Error { text: e.to_string() });
                    self.shared.set_state("error");
                    return Err(e);
                }
            };
            tracing::debug!(finish = %reply.finish_reason, usage = ?reply.usage, "model reply");
            let msg = reply.message.clone();
            self.messages.push(msg.clone());
            if let Some(t) = msg.content.as_deref().filter(|t| !t.trim().is_empty()) {
                self.shared.push(Event::Assistant { text: t.into() });
                final_text = t.into();
            }
            let calls = msg.tool_calls.clone().unwrap_or_default();
            if calls.is_empty() {
                self.shared.push(Event::Done { turns: turn + 1 });
                self.shared.set_state("done");
                return Ok(final_text);
            }
            for call in calls {
                let args: Value = serde_json::from_str(&call.function.arguments).unwrap_or(json!({}));
                self.shared.push(Event::ToolCall { id: call.id.clone(), name: call.function.name.clone(), args: args.clone() });
                let result = self.execute(&call.id, &call.function.name, &args);
                let (ok, text) = match result {
                    Ok(t) => (true, t),
                    Err(e) => (false, format!("ERROR: {}", e)),
                };
                self.shared.push(Event::ToolResult { id: call.id.clone(), ok, summary: text.chars().take(200).collect() });
                self.messages.push(Message::tool(&call.id, &call.function.name, text));
            }
        }
        self.shared.push(Event::Error { text: "turn limit reached".into() });
        self.shared.set_state("error");
        Err(anyhow!("turn limit reached"))
    }

    fn intent_for(&self, name: &str, args: &Value) -> Intent {
        let s = |k: &str| args.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let path = |k: &str| resolve_path(&self.project, &s(k)).display().to_string();
        let mut i = Intent { session_id: self.session_id.clone(), origin: "agentd".into(), tool: match name {
            "shell" => "shell",
            "read_file" | "list_dir" => "fs.read",
            "write_file" | "edit_file" => "fs.write",
            other => other,
        }.into(), ..Default::default() };
        match name {
            "shell" => {
                i.command = Some(s("command"));
                if args.get("needs_network").and_then(|v| v.as_bool()).unwrap_or(false) {
                    i.network.domains = vec!["*".into()];
                }
            }
            "read_file" | "list_dir" => i.reads = vec![path("path")],
            "write_file" | "edit_file" => i.writes = vec![path("path")],
            _ => {}
        }
        i
    }

    /// Gate through the broker, wait for a human if needed, then execute.
    fn execute(&mut self, call_id: &str, name: &str, args: &Value) -> Result<String> {
        let intent = self.intent_for(name, args);
        let decision = self.broker.lock().unwrap().evaluate(&intent)?;
        self.shared.push(Event::Decision { id: call_id.into(), tier: decision.tier.code().into(), verdict: format!("{:?}", decision.verdict).to_lowercase(), reason: decision.reason.clone(), request_id: decision.request_id.clone() });
        let allowed = match decision.verdict {
            Verdict::Allow | Verdict::AllowWithSnapshot => true,
            Verdict::Deny => return Err(anyhow!("denied by policy ({}): {}", decision.tier.code(), decision.reason)),
            Verdict::Prompt => {
                {
                    let mut info = self.shared.info.lock().unwrap();
                    info.pending.push(PendingPrompt { request_id: decision.request_id.clone(), tool: name.into(), args: args.clone(), tier: decision.tier.code().into(), reason: decision.reason.clone() });
                    info.state = "waiting".into();
                }
                self.shared.push(Event::Waiting { request_id: decision.request_id.clone(), name: name.into(), args: args.clone(), reason: decision.reason.clone() });
                let answer = self.shared.wait_answer(&decision.request_id, self.prompt_timeout);
                self.shared.set_state("running");
                let allowed = answer.unwrap_or(false);
                let _ = self.broker.lock().unwrap().resolve(&decision.request_id, allowed, if answer.is_some() { "client" } else { "timeout" });
                allowed
            }
        };
        if !allowed {
            return Err(anyhow!("the user did not allow this action ({}): {}", decision.tier.code(), decision.reason));
        }
        self.perform(name, args)
    }

    fn perform(&self, name: &str, args: &Value) -> Result<String> {
        let s = |k: &str| args.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        match name {
            "shell" => {
                let net = args.get("needs_network").and_then(|v| v.as_bool()).unwrap_or(false);
                let r = sandbox::run_shell(&self.project, &s("command"), net, Duration::from_secs(300))?;
                let mut out = format!("exit={}{}{}\n", r.exit_code, if r.timed_out { " (timed out)" } else { "" }, if r.sandboxed { "" } else { " [unsandboxed dev mode]" });
                if !r.stdout.is_empty() { out.push_str("stdout:\n"); out.push_str(&r.stdout); out.push('\n'); }
                if !r.stderr.is_empty() { out.push_str("stderr:\n"); out.push_str(&r.stderr); out.push('\n'); }
                Ok(out)
            }
            "read_file" => {
                let p = resolve_path(&self.project, &s("path"));
                let text = std::fs::read_to_string(&p).map_err(|e| anyhow!("{}: {}", p.display(), e))?;
                Ok(if text.len() > 60_000 { format!("{}\n…[truncated, {} bytes total]", &text[..60_000], text.len()) } else { text })
            }
            "write_file" => {
                let p = resolve_path(&self.project, &s("path"));
                if let Some(d) = p.parent() { std::fs::create_dir_all(d)?; }
                let content = s("content");
                std::fs::write(&p, &content).map_err(|e| anyhow!("{}: {}", p.display(), e))?;
                Ok(format!("wrote {} ({} bytes)", p.display(), content.len()))
            }
            "edit_file" => {
                let p = resolve_path(&self.project, &s("path"));
                let text = std::fs::read_to_string(&p).map_err(|e| anyhow!("{}: {}", p.display(), e))?;
                let old = s("old_text");
                let n = text.matches(&old).count();
                if n != 1 {
                    return Err(anyhow!("old_text found {} times in {}; it must match exactly once", n, p.display()));
                }
                std::fs::write(&p, text.replacen(&old, &s("new_text"), 1))?;
                Ok(format!("edited {}", p.display()))
            }
            "list_dir" => {
                let p = resolve_path(&self.project, &s("path"));
                let mut names: Vec<String> = std::fs::read_dir(&p).map_err(|e| anyhow!("{}: {}", p.display(), e))?.filter_map(|e| e.ok()).map(|e| {
                    let t = if e.path().is_dir() { "/" } else { "" };
                    format!("{}{}", e.file_name().to_string_lossy(), t)
                }).collect();
                names.sort();
                Ok(names.join("\n"))
            }
            other => Err(anyhow!("unknown tool {}", other)),
        }
    }
}
