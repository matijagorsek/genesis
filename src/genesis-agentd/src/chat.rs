//! Chats: ordinary conversations with Genesis that are kept, searchable, and can look at documents.
//!
//! A chat is a JSON file under ~/.local/share/genesis/chats/<id>.json with its messages. Each chat
//! has one agent session behind it (kind "chat": the assistant prompt, the read-only tools, the user's
//! MCP tools, no scaffolding); when agentd restarts, the stored messages are replayed into the new
//! session so the conversation continues where it was.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String, // user | assistant
    pub text: String,
    pub at: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chat {
    pub id: String,
    pub title: String,
    pub created: String,
    pub updated: String,
    pub messages: Vec<ChatMessage>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatSummary {
    pub id: String,
    pub title: String,
    pub updated: String,
    pub messages: usize,
    pub snippet: String,
}

pub fn dir() -> PathBuf {
    let data = std::env::var("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into())).join(".local/share"));
    data.join("genesis").join("chats")
}

/// Where a chat session's file tools resolve relative paths: a scratch folder of its own, never a project.
pub fn scratch_dir() -> PathBuf {
    let p = dir().join(".scratch");
    let _ = std::fs::create_dir_all(&p);
    p
}

fn now() -> String { time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339).unwrap_or_default() }

fn clean_id(id: &str) -> Option<String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') { None } else { Some(id.to_string()) }
}

pub fn load(id: &str) -> Option<Chat> {
    let id = clean_id(id)?;
    let s = std::fs::read_to_string(dir().join(format!("{}.json", id))).ok()?;
    serde_json::from_str(&s).ok()
}

pub fn save(chat: &Chat) -> Result<()> {
    std::fs::create_dir_all(dir())?;
    let p = dir().join(format!("{}.json", chat.id));
    let tmp = dir().join(format!(".{}.json.tmp", chat.id));
    std::fs::write(&tmp, serde_json::to_string_pretty(chat)?)?;
    std::fs::rename(tmp, p)?;
    Ok(())
}

pub fn create(title: &str) -> Result<Chat> {
    let id = uuid::Uuid::new_v4().to_string();
    let t = now();
    let chat = Chat { id, title: title.trim().to_string(), created: t.clone(), updated: t, messages: vec![] };
    save(&chat)?;
    Ok(chat)
}

pub fn delete(id: &str) -> bool {
    match clean_id(id) { Some(id) => std::fs::remove_file(dir().join(format!("{}.json", id))).is_ok(), None => false }
}

/// Append a message; the first user message becomes the title when there is none.
pub fn append(id: &str, role: &str, text: &str, attachments: Vec<String>) -> Result<Option<Chat>> {
    let Some(mut chat) = load(id) else { return Ok(None) };
    if chat.title.is_empty() && role == "user" {
        chat.title = text.lines().next().unwrap_or("").chars().take(60).collect::<String>().trim().to_string();
    }
    chat.messages.push(ChatMessage { role: role.into(), text: text.into(), at: now(), attachments });
    chat.updated = now();
    save(&chat)?;
    Ok(Some(chat))
}

/// Every chat, newest first; with `q`, only those whose title or messages contain every word of it.
pub fn list(q: &str) -> Vec<ChatSummary> {
    let words: Vec<String> = q.split_whitespace().map(|w| w.to_lowercase()).collect();
    let mut out: Vec<ChatSummary> = std::fs::read_dir(dir()).map(|rd| rd.filter_map(|e| e.ok()).filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false) && !e.file_name().to_string_lossy().starts_with('.')).filter_map(|e| {
        let chat: Chat = serde_json::from_str(&std::fs::read_to_string(e.path()).ok()?).ok()?;
        let hay = format!("{}\n{}", chat.title, chat.messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>().join("\n")).to_lowercase();
        if !words.iter().all(|w| hay.contains(w)) { return None; }
        let snippet = if words.is_empty() {
            chat.messages.last().map(|m| m.text.chars().take(120).collect()).unwrap_or_default()
        } else {
            // the first line that holds the first word, so the search result shows why it matched
            let w = &words[0];
            chat.messages.iter().flat_map(|m| m.text.lines()).find(|l| l.to_lowercase().contains(w)).map(|l| l.chars().take(140).collect()).unwrap_or_default()
        };
        Some(ChatSummary { id: chat.id.clone(), title: if chat.title.is_empty() { "New chat".into() } else { chat.title.clone() }, updated: chat.updated.clone(), messages: chat.messages.len(), snippet })
    }).collect()).unwrap_or_default();
    out.sort_by(|a, b| b.updated.cmp(&a.updated));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_append_list_search() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_DATA_HOME", dir.path());
        let c = create("").unwrap();
        append(&c.id, "user", "What did I write about the trip to Lisbon?", vec![]).unwrap();
        append(&c.id, "assistant", "Your notes mention Belém and a tram.", vec![]).unwrap();
        let d = create("").unwrap();
        append(&d.id, "user", "Convert 3 miles to km", vec![]).unwrap();
        let all = list("");
        assert_eq!(all.len(), 2);
        assert_eq!(load(&c.id).unwrap().title, "What did I write about the trip to Lisbon?");
        let hits = list("lisbon tram");
        assert_eq!(hits.len(), 1);
        assert!(hits[0].snippet.to_lowercase().contains("lisbon"));
        assert!(list("nothing-here").is_empty());
        assert!(delete(&d.id));
        assert!(!delete("../etc/passwd"));
        assert_eq!(list("").len(), 1);
    }
}
