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
    pub fn tool(id: &str, name: &str, content: impl Into<String>) -> Self {
        Message { role: "tool".into(), content: Some(content.into()), tool_calls: None, tool_call_id: Some(id.into()), name: Some(name.into()) }
    }
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
    pub fn chat(&self, messages: &[Message], tools: &Value, temperature: f64) -> Result<Reply> {
        let body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "tools": tools,
            "tool_choice": "auto",
            "temperature": temperature,
            "stream": false,
        });
        let url = format!("{}/chat/completions", self.endpoint.trim_end_matches('/'));
        let resp = ureq::post(&url)
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .timeout(std::time::Duration::from_secs(600))
            .send_json(body);
        let resp = match resp {
            Ok(r) => r,
            Err(ureq::Error::Status(code, r)) => {
                let text = r.into_string().unwrap_or_default();
                return Err(anyhow!("model endpoint returned {}: {}", code, text.chars().take(300).collect::<String>()));
            }
            Err(e) => return Err(anyhow!("model endpoint unreachable: {}", e)),
        };
        let parsed: ChatResponse = resp.into_json().context("parsing chat completion")?;
        let choice = parsed.choices.into_iter().next().ok_or_else(|| anyhow!("no choices in response"))?;
        Ok(Reply { message: choice.message, finish_reason: choice.finish_reason.unwrap_or_default(), usage: parsed.usage })
    }
}
