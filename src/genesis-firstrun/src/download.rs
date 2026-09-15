//! Resumable downloads with curl, run on a background thread, with progress readable by the API.

use crate::packs::PlannedFile;
use anyhow::{anyhow, Result};
use serde::Serialize;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Debug, Clone, Serialize, Default)]
pub struct Progress {
    pub state: String, // idle | planning | downloading | done | error
    pub pack: Option<String>,
    pub files_total: usize,
    pub files_done: usize,
    pub bytes_total: u64,
    pub bytes_done: u64,
    pub current: Option<String>,
    pub error: Option<String>,
    pub started_at: Option<String>,
}

pub type Shared = Arc<Mutex<Progress>>;

fn now() -> String {
    time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339).unwrap_or_default()
}

/// Kick off the download of `files` on a thread. `on_done` runs after every file has finished.
pub fn start(shared: Shared, pack_id: String, files: Vec<PlannedFile>, on_done: Box<dyn FnOnce() -> Result<()> + Send>) {
    {
        let mut p = shared.lock().unwrap();
        *p = Progress { state: "downloading".into(), pack: Some(pack_id), files_total: files.len(), bytes_total: files.iter().map(|f| f.size).sum(), started_at: Some(now()), ..Default::default() };
    }
    thread::spawn(move || {
        let mut done_bytes = 0u64;
        for (i, f) in files.iter().enumerate() {
            {
                let mut p = shared.lock().unwrap();
                p.current = Some(format!("{} ({})", f.filename, f.role));
                p.files_done = i;
            }
            let dest = Path::new(&f.dest);
            if let Some(d) = dest.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            // already complete?
            if let Ok(md) = std::fs::metadata(dest) {
                if f.size > 0 && md.len() == f.size {
                    done_bytes += f.size;
                    shared.lock().unwrap().bytes_done = done_bytes;
                    continue;
                }
            }
            let res = fetch(&f.url, dest, &shared, done_bytes);
            if let Err(e) = res {
                let mut p = shared.lock().unwrap();
                p.state = "error".into();
                p.error = Some(format!("{}: {}", f.filename, e));
                return;
            }
            // a signed pack definition names the checksum: a mismatch is a corrupt or substituted file
            if let Some(want) = f.sha256.as_deref() {
                shared.lock().unwrap().current = Some(format!("verifying {}", f.filename));
                match sha256_of(dest) {
                    Ok(got) if got.eq_ignore_ascii_case(want) => {}
                    Ok(got) => {
                        let _ = std::fs::remove_file(dest);
                        let mut p = shared.lock().unwrap();
                        p.state = "error".into();
                        p.error = Some(format!("{}: checksum mismatch (expected {}, got {}); the file was removed, retry the download", f.filename, &want[..12], &got[..got.len().min(12)]));
                        return;
                    }
                    Err(e) => {
                        let mut p = shared.lock().unwrap();
                        p.state = "error".into();
                        p.error = Some(format!("{}: could not verify: {}", f.filename, e));
                        return;
                    }
                }
            }
            done_bytes += f.size;
            shared.lock().unwrap().bytes_done = done_bytes;
        }
        {
            let mut p = shared.lock().unwrap();
            p.files_done = files.len();
            p.current = None;
        }
        match on_done() {
            Ok(()) => shared.lock().unwrap().state = "done".into(),
            Err(e) => {
                let mut p = shared.lock().unwrap();
                p.state = "error".into();
                p.error = Some(format!("finishing: {}", e));
            }
        }
    });
}

/// curl with resume; progress is sampled from the growing file size.
fn fetch(url: &str, dest: &Path, shared: &Shared, base_done: u64) -> Result<()> {
    let part = dest.with_extension(format!("{}.part", dest.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default()));
    let mut child = Command::new("curl")
        .args(["-fL", "--retry", "5", "--retry-delay", "5", "-C", "-", "--silent", "--show-error", "-o"])
        .arg(&part)
        .arg(url)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    loop {
        if let Ok(md) = std::fs::metadata(&part) {
            shared.lock().unwrap().bytes_done = base_done + md.len();
        }
        match child.try_wait()? {
            Some(status) => {
                if !status.success() {
                    let mut err = String::new();
                    if let Some(mut e) = child.stderr.take() {
                        use std::io::Read;
                        let _ = e.read_to_string(&mut err);
                    }
                    return Err(anyhow!("curl exit {}: {}", status.code().unwrap_or(-1), err.trim()));
                }
                break;
            }
            None => thread::sleep(std::time::Duration::from_millis(500)),
        }
    }
    std::fs::rename(&part, dest)?;
    Ok(())
}

/// sha256 of a file on disk, via coreutils (streams; models are gigabytes).
pub fn sha256_of(path: &Path) -> Result<String> {
    let out = Command::new("sha256sum").arg(path).output()?;
    if !out.status.success() { return Err(anyhow!("sha256sum failed for {}", path.display())); }
    Ok(String::from_utf8_lossy(&out.stdout).split_whitespace().next().unwrap_or("").to_string())
}
