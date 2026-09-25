//! Minimal OpenAI-compatible chat client with tool calling (llama-swap / llama-server behind it).

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type", default = "function_type")]
    pub kind: String,
    pub function: FunctionCall,
}
fn function_type() -> String {
    "function".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    /// JSON-encoded arguments, as the API delivers them.
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Message {
    pub fn system(s: impl Into<String>) -> Self {
        Message { role: "system".into(), content: Some(s.into()), tool_calls: None, tool_call_id: None, name: None }
    }
    pub fn user(s: impl Into<String>) -> Self {
        Message { role: "user".into(), content: Some(s.into()), tool_calls: None, tool_call_id: None, name: None }
    }
    pub fn assistant(s: impl Into<String>) -> Self {
        Message { role: "assistant".into(), content: Some(s.into()), tool_calls: None, tool_call_id: None, name: None }
    }
    pub fn tool(id: &str, name: &str, content: impl Into<String>) -> Self {
        Message { role: "tool".into(), content: Some(content.into()), tool_calls: None, tool_call_id: Some(id.into()), name: Some(name.into()) }
    }
}

thread_local! {
    /// a seed for one call only: the retry after an unreadable tool call must not repeat the same draw
    static SEED: std::cell::Cell<Option<i64>> = const { std::cell::Cell::new(None) };
}

pub struct Client {
    pub endpoint: String,
    pub model: String,
    pub api_key: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<Value>,
}
#[derive(Debug, Deserialize)]
struct Choice {
    message: Message,
    #[serde(default)]
    finish_reason: Option<String>,
}

pub struct Reply {
    pub message: Message,
    pub finish_reason: String,
    pub usage: Option<Value>,
}

impl Client {
    /// One retry when the model service is not there for a moment: it swaps models, and can be restarted
    /// under us (the evaluation lost two makes to a restart). A second refusal is reported as before.
    pub fn chat(&self, messages: &[Message], tools: &Value, temperature: f64) -> Result<Reply> {
        match self.chat_once(messages, tools, temperature) {
            Err(e) if e.to_string().contains("Connection Failed") || e.to_string().contains("Unexpected EOF") || e.to_string().contains("connection refused") => {
                std::thread::sleep(std::time::Duration::from_secs(5));
                self.chat_once(messages, tools, temperature)
            }
            other => other,
        }
    }

    fn chat_once(&self, messages: &[Message], tools: &Value, temperature: f64) -> Result<Reply> {
        self.chat_once_with(messages, tools, temperature, "auto")
    }

    /// `tool_choice` is "required" while the job has produced nothing: a small model otherwise answers with
    /// a description of what it would do, and the loop spends a turn telling it off.
    pub fn chat_with(&self, messages: &[Message], tools: &Value, temperature: f64, tool_choice: &str) -> Result<Reply> {
        match self.chat_once_with(messages, tools, temperature, tool_choice) {
            Err(e) if e.to_string().contains("Connection Failed") || e.to_string().contains("Unexpected EOF") || e.to_string().contains("connection refused") => {
                std::thread::sleep(std::time::Duration::from_secs(5));
                self.chat_once_with(messages, tools, temperature, tool_choice)
            }
            // A tool call the server could not read as JSON ended the whole job: the 2B's pomodoro, one
            // broken reply after two good writes. It is one bad draw, not a broken job; ask again, with
            // another seed so it is not the same draw, twice at most.
            Err(e) if e.to_string().contains("Failed to parse tool call") => {
                let mut last = e;
                for attempt in 1..=2 {
                    tracing::warn!(attempt, "the model's tool call could not be read; asking again");
                    match self.chat_once_seeded(messages, tools, temperature, tool_choice, 7 + attempt) {
                        Err(e) if e.to_string().contains("Failed to parse tool call") => last = e,
                        other => return other,
                    }
                }
                Err(last)
            }
            other => other,
        }
    }

    fn chat_once_seeded(&self, messages: &[Message], tools: &Value, temperature: f64, tool_choice: &str, seed: i64) -> Result<Reply> {
        SEED.with(|s| s.set(Some(seed)));
        let r = self.chat_once_with(messages, tools, temperature, tool_choice);
        SEED.with(|s| s.set(None));
        r
    }

    /// Have the model read these messages and tools and say one token: what it read stays in its cache.
    /// Patient, because this is the call that loads a model that was not loaded.
    ///
    /// It ends with a user message of its own because of where the model server keeps its place. On the
    /// hybrid Qwen3.5 layers it can only resume from a checkpoint, and it makes one where the last user
    /// message starts and a few tokens before the end. With the opening alone, the real request parts
    /// from it before the last of those, and 537 of 924 tokens were read again; with a user message the
    /// checkpoint is exactly where the real one begins, and 28 were (tools/prefill-bench.py, decision
    /// 221). tool_choice "none": one token of a tool call cannot be parsed, and the 2B answered it with 500.
    pub fn warm(&self, messages: &[Message], tools: &Value) -> Result<()> {
        let mut messages = messages.to_vec();
        messages.push(Message::user("(getting ready)".to_string()));
        let body = serde_json::json!({"model": self.model, "messages": messages, "tools": tools, "tool_choice": "none", "max_tokens": 1, "temperature": 0.0, "stream": false});
        ureq::post(&format!("{}/chat/completions", self.endpoint.trim_end_matches('/')))
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .timeout(std::time::Duration::from_secs(900))
            .send_json(body).map_err(|e| anyhow!("warm-up: {}", e))?;
        Ok(())
    }

    /// A question whose answer must be JSON of this schema (the server holds the reply to it with a
    /// grammar). The tools go along, unused, because they are part of the conversation the model already
    /// read: leave them out and the prompt no longer matches the cache, and all of it is read again.
    pub fn check_json(&self, messages: &[Message], tools: &Value, schema: &Value, max_tokens: u32) -> Result<Value> {
        let body = serde_json::json!({"model": self.model, "messages": messages, "tools": tools, "tool_choice": "none",
            "response_format": {"type": "json_schema", "json_schema": {"name": "check", "schema": schema}},
            "max_tokens": max_tokens, "temperature": 0.0, "seed": 7, "stream": false});
        let resp = ureq::post(&format!("{}/chat/completions", self.endpoint.trim_end_matches('/')))
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .timeout(std::time::Duration::from_secs(300))
            .send_json(body).map_err(|e| anyhow!("check: {}", e))?;
        let parsed: ChatResponse = resp.into_json().context("parsing the check")?;
        let text = parsed.choices.into_iter().next().and_then(|c| c.message.content).unwrap_or_default();
        serde_json::from_str(text.trim()).context("the check's answer is not JSON")
    }

    /// One model call, streamed. A fixed limit on the whole call could not tell a model that has stopped
    /// from one that is slowly writing a long file: the 2B writing a pomodoro app on a four-core CPU ran
    /// past five minutes and was cut off mid-file, in make after make (decision 223). So the answer
    /// streams, and the limit is on silence: the server sends a progress note after each batch of the
    /// prompt it reads and then every token, and a call that sends nothing for two minutes has stalled.
    /// Before the first byte the wait is long, because that is where a model that was not loaded loads.
    /// A server that answers in one piece instead (the tests' fake model) is read as before.
    fn chat_once_with(&self, messages: &[Message], tools: &Value, temperature: f64, tool_choice: &str) -> Result<Reply> {
        let secs = |var: &str, default: u64| std::env::var(var).ok().and_then(|v| v.parse().ok()).unwrap_or(default);
        let body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "tools": tools,
            "tool_choice": tool_choice,
            "temperature": temperature,
            // a fixed seed unless the machine asks for otherwise: two runs of the same tree should be
            // comparable, or a change cannot be told apart from the sampler's mood
            "seed": SEED.with(|s| s.get()).or_else(|| std::env::var("GENESIS_SEED").ok().and_then(|v| v.parse::<i64>().ok())).unwrap_or(7),
            "stream": true,
            "return_progress": true,
            "stream_options": {"include_usage": true},
        });
        let url = format!("{}/chat/completions", self.endpoint.trim_end_matches('/'));
        let first_byte = std::time::Duration::from_secs(secs("GENESIS_LLM_LOAD_TIMEOUT", 900));
        let silence = std::time::Duration::from_secs(secs("GENESIS_LLM_SILENCE", 120));
        let whole = std::time::Duration::from_secs(secs("GENESIS_LLM_TIMEOUT", 3600));
        let agent = ureq::AgentBuilder::new().timeout_connect(std::time::Duration::from_secs(10)).timeout_read(first_byte).build();
        let resp = match agent.post(&url).set("Authorization", &format!("Bearer {}", self.api_key)).send_json(body) {
            Ok(r) => r,
            Err(ureq::Error::Status(code, r)) => {
                let text = r.into_string().unwrap_or_default();
                return Err(anyhow!("model endpoint returned {}: {}", code, text.chars().take(300).collect::<String>()));
            }
            Err(e) => return Err(anyhow!("model endpoint unreachable: {}", e)),
        };
        if !resp.content_type().contains("event-stream") {
            let parsed: ChatResponse = resp.into_json().context("parsing chat completion")?;
            let choice = parsed.choices.into_iter().next().ok_or_else(|| anyhow!("no choices in response"))?;
            return Ok(Reply { message: choice.message, finish_reason: choice.finish_reason.unwrap_or_default(), usage: parsed.usage });
        }
        // the lines are read on their own thread, so silence can be timed here
        let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
        let reader = resp.into_reader();
        std::thread::spawn(move || {
            use std::io::BufRead;
            for line in std::io::BufReader::new(reader).lines() {
                match line {
                    Ok(l) => if let Some(data) = l.strip_prefix("data:") { if tx.send(Some(data.trim().to_string())).is_err() { return; } },
                    Err(_) => break,
                }
            }
            let _ = tx.send(None);
        });
        let started = std::time::Instant::now();
        let mut acc = Streamed::default();
        loop {
            if started.elapsed() > whole {
                return Err(anyhow!("the model took more than {} minutes over one answer", whole.as_secs() / 60));
            }
            match rx.recv_timeout(silence) {
                Ok(Some(data)) if data == "[DONE]" => break,
                Ok(Some(data)) => acc.add(&data)?,
                Ok(None) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return Err(anyhow!("the model went silent for {} seconds: nothing read, nothing written", silence.as_secs())),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        acc.reply()
    }
}

/// An answer put together from its stream: text and tool calls arrive in pieces, a tool call's arguments
/// in many, keyed by the call's index.
#[derive(Default)]
struct Streamed {
    content: String,
    calls: Vec<ToolCall>,
    finish_reason: String,
    usage: Option<Value>,
    any: bool,
}

impl Streamed {
    fn add(&mut self, data: &str) -> Result<()> {
        let v: Value = match serde_json::from_str(data) { Ok(v) => v, Err(_) => return Ok(()) };
        if let Some(e) = v.get("error") {
            return Err(anyhow!("model endpoint returned an error: {}", e.to_string().chars().take(300).collect::<String>()));
        }
        if let Some(u) = v.get("usage").filter(|u| !u.is_null()) { self.usage = Some(u.clone()); }
        let Some(choice) = v.get("choices").and_then(|c| c.as_array()).and_then(|a| a.first()) else { return Ok(()) };
        if let Some(f) = choice.get("finish_reason").and_then(|f| f.as_str()) { self.finish_reason = f.to_string(); }
        let Some(delta) = choice.get("delta") else { return Ok(()) };
        if let Some(t) = delta.get("content").and_then(|c| c.as_str()) { self.content.push_str(t); self.any = true; }
        for tc in delta.get("tool_calls").and_then(|t| t.as_array()).into_iter().flatten() {
            self.any = true;
            let i = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
            while self.calls.len() <= i {
                self.calls.push(ToolCall { id: String::new(), kind: "function".into(), function: FunctionCall { name: String::new(), arguments: String::new() } });
            }
            let c = &mut self.calls[i];
            if let Some(id) = tc.get("id").and_then(|x| x.as_str()) { if !id.is_empty() { c.id = id.to_string(); } }
            if let Some(f) = tc.get("function") {
                if let Some(n) = f.get("name").and_then(|x| x.as_str()) { c.function.name.push_str(n); }
                if let Some(a) = f.get("arguments").and_then(|x| x.as_str()) { c.function.arguments.push_str(a); }
            }
        }
        Ok(())
    }

    fn reply(self) -> Result<Reply> {
        if !self.any && self.finish_reason.is_empty() {
            return Err(anyhow!("no choices in response"));
        }
        let calls: Vec<ToolCall> = self.calls.into_iter().filter(|c| !c.function.name.is_empty()).enumerate()
            .map(|(i, mut c)| { if c.id.is_empty() { c.id = format!("call_{}", i); } c }).collect();
        let message = Message { role: "assistant".into(), content: if self.content.is_empty() && !calls.is_empty() { None } else { Some(self.content) },
            tool_calls: if calls.is_empty() { None } else { Some(calls) }, tool_call_id: None, name: None };
        Ok(Reply { message, finish_reason: self.finish_reason, usage: self.usage })
    }
}

/// The key the model service requires (see genesis-firstrun, packs::with_api_key). A machine without one --
/// a development router -- takes any key, so "local" is sent then.
pub fn router_key() -> String {
    std::fs::read_to_string(std::env::var("GENESIS_ROUTER_KEY_FILE").unwrap_or_else(|_| "/etc/genesis/router.key".into()))
        .ok().map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).unwrap_or_else(|| "local".into())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_real_stream_becomes_the_tool_call_it_was() {
        // captured from llama-server b10901: progress notes while it reads, then a write_file call in pieces
        let sse = include_str!("../testdata/tool-call.sse");
        let mut acc = super::Streamed::default();
        for line in sse.lines() {
            if let Some(d) = line.strip_prefix("data:") { if d.trim() == "[DONE]" { break; } acc.add(d.trim()).unwrap(); }
        }
        let r = acc.reply().unwrap();
        let calls = r.message.tool_calls.expect("a tool call");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "write_file");
        let args: serde_json::Value = serde_json::from_str(&calls[0].function.arguments).expect("the arguments, whole");
        assert_eq!(args["path"], "hello.txt");
        assert!(!calls[0].id.is_empty());
        assert_eq!(r.finish_reason, "tool_calls");
        assert!(r.usage.and_then(|u| u.get("completion_tokens").cloned()).is_some(), "usage, for the tokens-per-second figure");
    }

    #[test]
    fn silence_is_a_stall_and_slow_is_not() {
        use std::io::{Read, Write};
        // a server that streams a token a second for three seconds, then says nothing more
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut buf = [0u8; 65536]; let _ = s.read(&mut buf);
            let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n");
            for _ in 0..3 {
                let _ = write!(s, "data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"a\"}}}}]}}\n\n");
                let _ = s.flush();
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            std::thread::sleep(std::time::Duration::from_secs(30));
        });
        std::env::set_var("GENESIS_LLM_SILENCE", "2");
        let c = super::Client { endpoint: format!("http://{}/v1", addr), model: "m".into(), api_key: "k".into() };
        let t = std::time::Instant::now();
        let e = c.chat_once_with(&[super::Message::user("hi")], &serde_json::json!([]), 0.2, "auto").err().expect("a stall");
        std::env::remove_var("GENESIS_LLM_SILENCE");
        assert!(e.to_string().contains("went silent"), "{}", e);
        // three seconds of tokens a second apart are not silence; the stall is caught two seconds after
        let took = t.elapsed().as_secs_f64();
        assert!(took > 4.0 && took < 10.0, "took {}", took);
    }

    #[test]
    fn a_server_error_in_the_stream_is_an_error() {
        let mut acc = super::Streamed::default();
        assert!(acc.add(r#"{"error":{"code":500,"message":"failed to parse"}}"#).is_err());
    }
}
