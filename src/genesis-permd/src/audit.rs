//! Append-only, hash-chained audit log. One JSON object per line; each line carries the SHA-256 of the
//! previous line, so tampering with history breaks the chain. Verified with `genesis-permd audit verify`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub ts: String,
    pub kind: String,
    pub session_id: String,
    pub request_id: String,
    #[serde(default)]
    pub tool: String,
    #[serde(default)]
    pub tier: String,
    #[serde(default)]
    pub verdict: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub detail: serde_json::Value,
    pub prev: String,
}

pub struct AuditLog {
    path: PathBuf,
    last_hash: String,
    key: Vec<u8>,
}

/// The chain is keyed (HMAC-SHA256) with a per-install secret kept next to the log, mode 0600: someone
/// who can edit the log but not read the key cannot recompute the chain. The key is created on first use.
fn chain_key(log: &Path) -> Vec<u8> {
    let p = log.with_extension("key");
    if let Ok(k) = std::fs::read(&p) { if k.len() >= 32 { return k; } }
    let mut k = vec![0u8; 32];
    if let Ok(mut f) = File::open("/dev/urandom") { use std::io::Read; let _ = f.read_exact(&mut k); }
    use std::os::unix::fs::OpenOptionsExt;
    if let Ok(mut f) = OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&p) { let _ = f.write_all(&k); }
    k
}

fn hmac(key: &[u8], msg: &[u8]) -> String {
    let mut k = [0u8; 64];
    if key.len() > 64 { let d = Sha256::digest(key); k[..32].copy_from_slice(&d); } else { k[..key.len()].copy_from_slice(key); }
    let ipad: Vec<u8> = k.iter().map(|b| b ^ 0x36).collect();
    let opad: Vec<u8> = k.iter().map(|b| b ^ 0x5c).collect();
    let inner = { let mut h = Sha256::new(); h.update(&ipad); h.update(msg); h.finalize() };
    let mut h = Sha256::new(); h.update(&opad); h.update(inner);
    hex::encode(h.finalize())
}

fn hash_line_keyed(key: &[u8], line: &str) -> String {
    hmac(key, line.as_bytes())
}

impl AuditLog {
    pub fn open(path: impl AsRef<Path>) -> Result<AuditLog> {
        let path = path.as_ref().to_path_buf();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let key = chain_key(&path);
        let last_hash = match File::open(&path) {
            Ok(f) => {
                let mut last = String::from("genesis");
                for line in BufReader::new(f).lines() {
                    let line = line?;
                    if !line.trim().is_empty() {
                        last = hash_line_keyed(&key, &line);
                    }
                }
                last
            }
            Err(_) => String::from("genesis"),
        };
        Ok(AuditLog { path, last_hash, key })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&mut self, mut entry: AuditEntry) -> Result<()> {
        entry.prev = self.last_hash.clone();
        let line = serde_json::to_string(&entry)?;
        let mut f = OpenOptions::new().create(true).append(true).open(&self.path)?;
        f.write_all(line.as_bytes())?;
        f.write_all(b"\n")?;
        f.flush()?;
        self.last_hash = hash_line_keyed(&self.key, &line);
        Ok(())
    }

    /// Walk the file and check every `prev` field matches the hash of the previous line.
    pub fn verify(path: impl AsRef<Path>) -> Result<usize> {
        let f = File::open(path.as_ref()).with_context(|| format!("opening {}", path.as_ref().display()))?;
        let key = chain_key(path.as_ref());
        let mut expected = String::from("genesis");
        let mut n = 0usize;
        for (i, line) in BufReader::new(f).lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let entry: AuditEntry = serde_json::from_str(&line).with_context(|| format!("line {} is not an audit entry", i + 1))?;
            if entry.prev != expected {
                anyhow::bail!("chain broken at line {}: expected prev {} got {}", i + 1, expected, entry.prev);
            }
            expected = hash_line_keyed(&key, &line);
            n += 1;
        }
        Ok(n)
    }
}

pub fn now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown".into())
}
