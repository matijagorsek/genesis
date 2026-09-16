//! The time machine: what a job changed, file by file, as before-and-after lines, and "restore just
//! this file" from the pre-image genesis-txd kept. The snapshots exist since day one; this is the view.

use anyhow::{anyhow, Result};
use genesis_txd::{Entry, Store};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct FileChange {
    pub path: String,
    /// created | changed | deleted | unchanged | directory
    pub kind: String,
    /// Unified-style lines: " " context, "-" before, "+" after. Capped.
    pub diff: Vec<String>,
    pub restorable: bool,
}

#[derive(Debug, Serialize)]
pub struct Changes {
    pub id: String,
    pub status: String,
    pub files: Vec<FileChange>,
    pub notes: Vec<String>,
}

const MAX_LINES: usize = 1500;

/// A plain line diff (longest common subsequence, capped) between two texts.
pub fn line_diff(before: &str, after: &str) -> Vec<String> {
    let a: Vec<&str> = before.lines().take(MAX_LINES).collect();
    let b: Vec<&str> = after.lines().take(MAX_LINES).collect();
    let (n, m) = (a.len(), b.len());
    // dp[i][j] = LCS length of a[i..], b[j..]
    let mut dp = vec![vec![0u16; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] { dp[i + 1][j + 1] + 1 } else { dp[i + 1][j].max(dp[i][j + 1]) };
        }
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] { out.push(format!(" {}", a[i])); i += 1; j += 1; }
        else if dp[i + 1][j] >= dp[i][j + 1] { out.push(format!("-{}", a[i])); i += 1; }
        else { out.push(format!("+{}", b[j])); j += 1; }
    }
    while i < n { out.push(format!("-{}", a[i])); i += 1; }
    while j < m { out.push(format!("+{}", b[j])); j += 1; }
    // keep the interesting part: drop long runs of context, leave two lines around each change
    let changed: Vec<bool> = out.iter().map(|l| !l.starts_with(' ')).collect();
    let keep: Vec<bool> = (0..out.len()).map(|k| (k.saturating_sub(2)..(k + 3).min(out.len())).any(|x| changed[x])).collect();
    let mut trimmed = Vec::new();
    let mut skipping = false;
    for (k, l) in out.into_iter().enumerate() {
        if keep[k] { trimmed.push(l); skipping = false; }
        else if !skipping { trimmed.push("…".into()); skipping = true; }
    }
    if trimmed.len() > 400 { trimmed.truncate(400); trimmed.push("… (more)".into()); }
    trimmed
}

fn read_text(p: &Path) -> Option<String> {
    let b = std::fs::read(p).ok()?;
    if b.len() > 2 << 20 || b.iter().take(4096).any(|&c| c == 0) { return None; }
    Some(String::from_utf8_lossy(&b).to_string())
}

pub fn changes(store: &Store, id: &str) -> Result<Changes> {
    let tx = store.load(id)?;
    let mut files = Vec::new();
    let mut notes = Vec::new();
    for e in &tx.entries {
        match e {
            Entry::File { path, pre, was_dir } => {
                let now = Path::new(path);
                if *was_dir { files.push(FileChange { path: path.clone(), kind: "directory".into(), diff: vec![], restorable: pre.is_some() }); continue; }
                let (kind, diff) = match (pre.as_deref().and_then(|p| read_text(Path::new(p))), read_text(now)) {
                    (None, _) if pre.is_none() && now.exists() => ("created".to_string(), read_text(now).map(|t| t.lines().take(200).map(|l| format!("+{}", l)).collect()).unwrap_or_default()),
                    (None, _) if pre.is_none() => ("deleted".to_string(), vec![]),
                    (Some(b), Some(a)) if b == a => ("unchanged".to_string(), vec![]),
                    (Some(b), Some(a)) => ("changed".to_string(), line_diff(&b, &a)),
                    (Some(b), None) => ("deleted".to_string(), b.lines().take(200).map(|l| format!("-{}", l)).collect()),
                    (None, Some(_)) => ("changed".to_string(), vec!["(binary or large file)".into()]),
                    (None, None) => ("deleted".to_string(), vec!["(binary or large file)".into()]),
                };
                files.push(FileChange { path: path.clone(), kind, diff, restorable: pre.is_some() || !now.exists() || true });
            }
            Entry::Btrfs { source, .. } => notes.push(format!("whole folder snapshotted: {}", source)),
            Entry::FlatpakUser { r#ref } => notes.push(format!("installed app: {}", r#ref)),
            Entry::Shell { command, tier } => notes.push(format!("ran ({}): {}", tier, command)),
            Entry::Note { text } => notes.push(text.clone()),
        }
    }
    Ok(Changes { id: tx.id.clone(), status: format!("{:?}", tx.status).to_lowercase(), files, notes })
}

/// Put one file back the way it was before the job (its pre-image), leaving everything else alone.
pub fn restore_file(store: &Store, id: &str, path: &str) -> Result<String> {
    let tx = store.load(id)?;
    for e in &tx.entries {
        if let Entry::File { path: p, pre, was_dir } = e {
            if p != path { continue; }
            let target = Path::new(p);
            return match pre {
                Some(src) => {
                    if *was_dir { return Err(anyhow!("{} is a folder; undo the whole job to restore it", p)); }
                    if let Some(d) = target.parent() { std::fs::create_dir_all(d)?; }
                    std::fs::copy(src, target)?;
                    Ok(format!("restored {}", p))
                }
                None => {
                    if target.exists() { std::fs::remove_file(target)?; Ok(format!("removed {} (it did not exist before the job)", p)) } else { Ok(format!("{} is already gone", p)) }
                }
            };
        }
    }
    Err(anyhow!("this job did not touch {}", path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_marks_changed_lines_and_trims_context() {
        let before = (1..=40).map(|i| format!("line {}", i)).collect::<Vec<_>>().join("\n");
        let after = before.replace("line 20", "line twenty") + "\nline 41";
        let d = line_diff(&before, &after);
        assert!(d.iter().any(|l| l == "-line 20"));
        assert!(d.iter().any(|l| l == "+line twenty"));
        assert!(d.iter().any(|l| l == "+line 41"));
        assert!(d.iter().any(|l| l == "…"), "long unchanged runs are folded");
        assert!(d.len() < 20);
    }
}
