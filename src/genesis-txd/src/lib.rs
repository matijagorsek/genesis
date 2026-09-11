//! Transactions: snapshot before a change, remember what changed, undo on request.
//!
//! Store layout (default /var/lib/genesis/tx, or ~/.local/state/genesis/tx for a user):
//!   <store>/<tx-id>.json          the transaction record
//!   <store>/<tx-id>/pre/<n>       file backend pre-images (content of a path before it was written)
//!   <store>/<tx-id>/btrfs/<name>  btrfs read-only snapshots (Linux, root, btrfs paths)

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Open,
    Committed,
    RolledBack,
}

/// One recorded change and how to undo it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Entry {
    /// A file (or directory) whose pre-image was saved before a write. `pre` is None when it did not exist.
    File { path: String, pre: Option<String>, was_dir: bool },
    /// A btrfs subvolume snapshot of `source` at `snapshot`.
    Btrfs { source: String, snapshot: String },
    /// A user-scope Flatpak ref installed during the transaction; undone by uninstalling it.
    FlatpakUser { r#ref: String },
    /// A shell command at user or system scope. Not undoable by itself; listed so the user knows.
    Shell { command: String, tier: String },
    /// Anything else the caller wants remembered.
    Note { text: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub id: String,
    pub session_id: String,
    pub origin: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub status: Status,
    pub entries: Vec<Entry>,
    /// Flatpak user refs before the transaction, to detect additions at commit time.
    #[serde(default)]
    pub flatpak_user_before: Vec<String>,
}

fn now() -> String {
    time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339).unwrap_or_default()
}

pub struct Store {
    pub root: PathBuf,
}

pub fn default_store() -> PathBuf {
    if let Ok(p) = std::env::var("GENESIS_TX_STORE") {
        return PathBuf::from(p);
    }
    let system = Path::new("/var/lib/genesis/tx");
    if is_root() || system.is_dir() && std::fs::metadata(system).map(|m| !m.permissions().readonly()).unwrap_or(false) {
        return system.to_path_buf();
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(format!("{}/.local/state/genesis/tx", home))
}

fn is_root() -> bool {
    std::env::var("USER").map(|u| u == "root").unwrap_or(false) || std::env::var("UID").map(|u| u == "0").unwrap_or(false)
}

impl Store {
    pub fn open(root: impl AsRef<Path>) -> Result<Store> {
        std::fs::create_dir_all(root.as_ref()).with_context(|| format!("creating tx store {}", root.as_ref().display()))?;
        Ok(Store { root: root.as_ref().to_path_buf() })
    }

    fn record_path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{}.json", id))
    }

    pub fn save(&self, tx: &Transaction) -> Result<()> {
        let tmp = self.record_path(&tx.id).with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(tx)?)?;
        std::fs::rename(&tmp, self.record_path(&tx.id))?;
        Ok(())
    }

    pub fn load(&self, id: &str) -> Result<Transaction> {
        let p = self.record_path(id);
        let text = std::fs::read_to_string(&p).with_context(|| format!("no transaction {}", id))?;
        Ok(serde_json::from_str(&text)?)
    }

    pub fn list(&self) -> Result<Vec<Transaction>> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(&self.root)? {
            let p = e?.path();
            if p.extension().map(|x| x == "json").unwrap_or(false) {
                if let Ok(t) = serde_json::from_str::<Transaction>(&std::fs::read_to_string(&p)?) {
                    out.push(t);
                }
            }
        }
        out.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        Ok(out)
    }

    /// Open a transaction for a session. Records the current user Flatpak refs for later diffing.
    pub fn begin(&self, session_id: &str, origin: &str) -> Result<Transaction> {
        let tx = Transaction {
            id: format!("tx-{}", &uuid::Uuid::new_v4().to_string()[..8]),
            session_id: session_id.into(),
            origin: origin.into(),
            started_at: now(),
            finished_at: None,
            status: Status::Open,
            entries: vec![],
            flatpak_user_before: flatpak_user_refs(),
        };
        std::fs::create_dir_all(self.root.join(&tx.id).join("pre"))?;
        self.save(&tx)?;
        Ok(tx)
    }

    /// Call before writing `path`: saves its pre-image (or records that it did not exist).
    /// If the path is on btrfs and we can snapshot the subvolume, a btrfs snapshot is taken once instead.
    pub fn pre_write(&self, tx: &mut Transaction, path: &Path) -> Result<()> {
        if tx.status != Status::Open {
            return Err(anyhow!("transaction {} is {:?}", tx.id, tx.status));
        }
        let path_s = path.display().to_string();
        // already covered?
        if tx.entries.iter().any(|e| matches!(e, Entry::File { path: p, .. } if p == &path_s)) {
            return Ok(());
        }
        if let Some(subvol) = btrfs_subvolume_for(path) {
            let already = tx.entries.iter().any(|e| matches!(e, Entry::Btrfs { source, .. } if source == &subvol));
            if !already {
                if let Ok(snap) = btrfs_snapshot(&self.root.join(&tx.id).join("btrfs"), &subvol) {
                    tx.entries.push(Entry::Btrfs { source: subvol, snapshot: snap });
                    self.save(tx)?;
                    return Ok(());
                }
            } else {
                return Ok(());
            }
        }
        let n = tx.entries.len();
        let pre_dir = self.root.join(&tx.id).join("pre");
        let entry = if path.exists() {
            let dest = pre_dir.join(format!("{}", n));
            copy_path(path, &dest)?;
            Entry::File { path: path_s, pre: Some(dest.display().to_string()), was_dir: path.is_dir() }
        } else {
            Entry::File { path: path_s, pre: None, was_dir: false }
        };
        tx.entries.push(entry);
        self.save(tx)
    }

    pub fn note_shell(&self, tx: &mut Transaction, command: &str, tier: &str) -> Result<()> {
        tx.entries.push(Entry::Shell { command: command.into(), tier: tier.into() });
        self.save(tx)
    }

    pub fn note(&self, tx: &mut Transaction, text: &str) -> Result<()> {
        tx.entries.push(Entry::Note { text: text.into() });
        self.save(tx)
    }

    /// Close the transaction, keeping the snapshots so it can still be rolled back for a while.
    pub fn commit(&self, tx: &mut Transaction) -> Result<()> {
        let after = flatpak_user_refs();
        for r in after.iter().filter(|r| !tx.flatpak_user_before.contains(r)) {
            tx.entries.push(Entry::FlatpakUser { r#ref: r.clone() });
        }
        tx.status = Status::Committed;
        tx.finished_at = Some(now());
        self.save(tx)
    }

    /// Undo everything undoable, newest first. Returns human-readable lines of what happened.
    pub fn rollback(&self, tx: &mut Transaction) -> Result<Vec<String>> {
        if tx.status == Status::RolledBack {
            return Err(anyhow!("transaction {} was already rolled back", tx.id));
        }
        let mut log = Vec::new();
        for e in tx.entries.iter().rev() {
            match e {
                Entry::File { path, pre, was_dir } => {
                    let p = Path::new(path);
                    match pre {
                        Some(src) => {
                            if p.exists() {
                                remove_path(p)?;
                            }
                            copy_path(Path::new(src), p)?;
                            log.push(format!("restored {}{}", path, if *was_dir { "/" } else { "" }));
                        }
                        None => {
                            if p.exists() {
                                remove_path(p)?;
                                log.push(format!("removed {} (did not exist before)", path));
                            }
                        }
                    }
                }
                Entry::Btrfs { source, snapshot } => match btrfs_restore(source, snapshot) {
                    Ok(()) => log.push(format!("restored btrfs subvolume {} from snapshot", source)),
                    Err(err) => log.push(format!("could not restore {}: {}", source, err)),
                },
                Entry::FlatpakUser { r#ref } => {
                    let ok = Command::new("flatpak").args(["uninstall", "--user", "-y", "--noninteractive", r#ref]).status().map(|s| s.success()).unwrap_or(false);
                    log.push(format!("{} flatpak {}", if ok { "uninstalled" } else { "could not uninstall" }, r#ref));
                }
                Entry::Shell { command, tier } => log.push(format!("not undoable: shell ({}) {}", tier, command)),
                Entry::Note { text } => log.push(format!("note: {}", text)),
            }
        }
        tx.status = Status::RolledBack;
        tx.finished_at = Some(now());
        self.save(tx)?;
        Ok(log)
    }

    /// Human summary for the session view.
    pub fn summary(&self, tx: &Transaction) -> Vec<String> {
        let mut out = vec![format!("{} · session {} · {:?} · started {}", tx.id, tx.session_id, tx.status, tx.started_at)];
        for e in &tx.entries {
            out.push(match e {
                Entry::File { path, pre, was_dir } => format!("file      {}{}  ({})", path, if *was_dir { "/" } else { "" }, if pre.is_some() { "pre-image saved" } else { "created" }),
                Entry::Btrfs { source, .. } => format!("btrfs     {}  (snapshot)", source),
                Entry::FlatpakUser { r#ref } => format!("flatpak   {}  (user install, undo = uninstall)", r#ref),
                Entry::Shell { command, tier } => format!("shell     [{}] {}  (not undoable)", tier, command),
                Entry::Note { text } => format!("note      {}", text),
            });
        }
        out
    }
}

// ---------- helpers ----------

fn copy_path(src: &Path, dest: &Path) -> Result<()> {
    if let Some(d) = dest.parent() {
        std::fs::create_dir_all(d)?;
    }
    if src.is_dir() {
        std::fs::create_dir_all(dest)?;
        for e in std::fs::read_dir(src)? {
            let e = e?;
            copy_path(&e.path(), &dest.join(e.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(src, dest).with_context(|| format!("copying {} to {}", src.display(), dest.display()))?;
        Ok(())
    }
}

fn remove_path(p: &Path) -> Result<()> {
    if p.is_dir() {
        std::fs::remove_dir_all(p)?;
    } else {
        std::fs::remove_file(p)?;
    }
    Ok(())
}

fn flatpak_user_refs() -> Vec<String> {
    Command::new("flatpak")
        .args(["list", "--user", "--app", "--columns=ref"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default()
}

/// If `path` lives on a btrfs subvolume we may snapshot, return the subvolume mount point.
fn btrfs_subvolume_for(path: &Path) -> Option<String> {
    if !cfg!(target_os = "linux") || !is_root() {
        return None;
    }
    // walk up to the nearest existing ancestor, then ask btrfs
    let mut p = path.to_path_buf();
    while !p.exists() {
        if !p.pop() {
            return None;
        }
    }
    let out = Command::new("stat").args(["-f", "-c", "%T", &p.display().to_string()]).output().ok()?;
    if String::from_utf8_lossy(&out.stdout).trim() != "btrfs" {
        return None;
    }
    // find the mount point of this path
    let mp = Command::new("findmnt").args(["-n", "-o", "TARGET", "-T", &p.display().to_string()]).output().ok()?;
    let target = String::from_utf8_lossy(&mp.stdout).trim().to_string();
    if target.is_empty() { None } else { Some(target) }
}

fn btrfs_snapshot(dir: &Path, source: &str) -> Result<String> {
    std::fs::create_dir_all(dir)?;
    let name = source.trim_matches('/').replace('/', "_");
    let dest = dir.join(if name.is_empty() { "root".into() } else { name });
    let st = Command::new("btrfs").args(["subvolume", "snapshot", "-r", source, &dest.display().to_string()]).status()?;
    if !st.success() {
        return Err(anyhow!("btrfs snapshot of {} failed", source));
    }
    Ok(dest.display().to_string())
}

/// Restore by rsync-ing the read-only snapshot back over the live subvolume (keeps the subvolume in place,
/// which is what a running system needs; a full swap-and-reboot restore is Phase 2).
fn btrfs_restore(source: &str, snapshot: &str) -> Result<()> {
    let st = Command::new("rsync").args(["-aHAX", "--delete", &format!("{}/", snapshot), &format!("{}/", source)]).status()?;
    if !st.success() {
        return Err(anyhow!("rsync from snapshot failed"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_backend_roundtrip_modify_create_delete() {
        let store_dir = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        let existing = work.path().join("config.toml");
        let created = work.path().join("new.txt");
        let dir = work.path().join("settings");
        std::fs::write(&existing, "old").unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a"), "1").unwrap();

        let mut tx = store.begin("s1", "test").unwrap();
        store.pre_write(&mut tx, &existing).unwrap();
        store.pre_write(&mut tx, &created).unwrap();
        store.pre_write(&mut tx, &dir).unwrap();
        // the "agent" now changes things
        std::fs::write(&existing, "new").unwrap();
        std::fs::write(&created, "made").unwrap();
        std::fs::write(dir.join("a"), "2").unwrap();
        std::fs::write(dir.join("b"), "3").unwrap();
        store.note_shell(&mut tx, "flatpak install --user org.x", "S1").unwrap();
        store.commit(&mut tx).unwrap();
        assert_eq!(store.load(&tx.id).unwrap().status, Status::Committed);

        let mut tx2 = store.load(&tx.id).unwrap();
        let log = store.rollback(&mut tx2).unwrap();
        assert_eq!(std::fs::read_to_string(&existing).unwrap(), "old");
        assert!(!created.exists());
        assert_eq!(std::fs::read_to_string(dir.join("a")).unwrap(), "1");
        assert!(!dir.join("b").exists());
        assert!(log.iter().any(|l| l.starts_with("not undoable: shell")));
        assert_eq!(store.load(&tx.id).unwrap().status, Status::RolledBack);
        assert!(store.rollback(&mut tx2).is_err());
    }

    #[test]
    fn pre_write_is_idempotent_and_list_orders_newest_first() {
        let store_dir = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();
        let f = work.path().join("f");
        std::fs::write(&f, "x").unwrap();
        let mut tx = store.begin("s", "t").unwrap();
        store.pre_write(&mut tx, &f).unwrap();
        store.pre_write(&mut tx, &f).unwrap();
        assert_eq!(tx.entries.len(), 1);
        std::thread::sleep(std::time::Duration::from_millis(5));
        let _tx2 = store.begin("s", "t").unwrap();
        let all = store.list().unwrap();
        assert_eq!(all.len(), 2);
        assert!(all[0].started_at >= all[1].started_at);
        let s = store.summary(&tx);
        assert!(s[1].contains("pre-image saved"));
    }
}
