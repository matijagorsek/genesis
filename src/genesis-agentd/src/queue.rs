//! Make it while I sleep: a queue of makes that runs when the machine is idle for the night, one by one,
//! on the small model, with permission prompts answered "not now" (the job stays inside its project),
//! everything snapshotted like any other job. The morning card says what came of it.
//!
//! State: ~/.local/state/genesis/queue.json  {run_at: "23:00", items: [...], done: [...]}

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item { pub id: String, pub text: String, #[serde(default)] pub project: String, pub added: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Done { pub id: String, pub text: String, pub state: String, pub at: String, #[serde(default)] pub summary: String, #[serde(default)] pub project: String }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Queue {
    #[serde(default = "default_run_at")]
    pub run_at: String,
    #[serde(default)]
    pub items: Vec<Item>,
    #[serde(default)]
    pub done: Vec<Done>,
    /// Set by "Start now"; cleared when the queue empties.
    #[serde(default)]
    pub start_now: bool,
}

fn default_run_at() -> String { "23:00".into() }

pub fn path() -> PathBuf {
    let state = std::env::var("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into())).join(".local/state"));
    state.join("genesis").join("queue.json")
}

pub fn load() -> Queue {
    std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(|| Queue { run_at: default_run_at(), ..Default::default() })
}

pub fn save(q: &Queue) {
    if let Some(d) = path().parent() { let _ = std::fs::create_dir_all(d); }
    let _ = std::fs::write(path(), serde_json::to_string_pretty(q).unwrap_or_default());
}

pub fn now() -> String { time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339).unwrap_or_default() }

/// On mains, or no battery at all. A queue that empties a laptop overnight is not a feature.
pub fn on_mains() -> bool {
    let Ok(rd) = std::fs::read_dir("/sys/class/power_supply") else { return true };
    let mut has_battery = false;
    for e in rd.flatten() {
        let p = e.path();
        if std::fs::read_to_string(p.join("type")).map(|t| t.trim() == "Battery").unwrap_or(false) {
            has_battery = true;
            if std::fs::read_to_string(p.join("status")).map(|s| { let s = s.trim(); s == "Charging" || s == "Full" || s == "Not charging" }).unwrap_or(false) { return true; }
        }
    }
    !has_battery
}

/// Is it time: "Start now" was pressed, or the local clock passed run_at (HH:MM) and it is still night.
pub fn due(q: &Queue) -> bool {
    if q.items.is_empty() { return false; }
    if q.start_now { return true; }
    let (h, m) = q.run_at.split_once(':').and_then(|(h, m)| Some((h.parse::<u8>().ok()?, m.parse::<u8>().ok()?))).unwrap_or((23, 0));
    // local time via the TZ-aware `date`, so a laptop in Ljubljana runs at 23:00 in Ljubljana
    let out = std::process::Command::new("date").arg("+%H:%M").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let (ch, cm) = out.split_once(':').and_then(|(a, b)| Some((a.parse::<u8>().ok()?, b.parse::<u8>().ok()?))).unwrap_or((0, 0));
    let mins = |h: u8, m: u8| h as u32 * 60 + m as u32;
    let start = mins(h, m);
    let cur = mins(ch, cm);
    // a window of six hours after run_at, wrapping past midnight
    let diff = (cur + 1440 - start) % 1440;
    diff < 6 * 60
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_queue_is_never_due_and_start_now_wins() {
        let mut q = Queue { run_at: "23:00".into(), ..Default::default() };
        assert!(!due(&q));
        q.items.push(Item { id: "a".into(), text: "x".into(), project: String::new(), added: now() });
        q.start_now = true;
        assert!(due(&q));
    }
}
