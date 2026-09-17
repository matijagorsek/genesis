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
    start_with(shared, pack_id, files, Box::new(|_| {}), on_done)
}

/// Like `start`, with `on_file` called after each file lands (the wizard uses it to bring the small
/// model up before the rest of the pack has downloaded, so the first make does not wait for it all).
pub fn start_with(shared: Shared, pack_id: String, files: Vec<PlannedFile>, on_file: Box<dyn Fn(&PlannedFile) + Send>, on_done: Box<dyn FnOnce() -> Result<()> + Send>) {
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
                    on_file(f);
                    continue;
                }
            }
            // a mirrored copy on GHCR (publish-models) comes first when the checksum is known; Hugging Face otherwise
            let (url, auth) = match f.sha256.as_deref().and_then(ghcr_blob) {
                Some((u, t)) => { shared.lock().unwrap().current = Some(format!("{} (from GHCR)", f.filename)); (u, Some(t)) }
                None => (f.url.clone(), None),
            };
            let mut res = fetch_with(&url, auth.as_deref(), dest, &shared, done_bytes);
            for attempt in 1..=2 {
                if res.is_ok() { break; }
                shared.lock().unwrap().current = Some(format!("{} (connection dropped, resuming, try {})", f.filename, attempt + 1));
                std::thread::sleep(std::time::Duration::from_secs(10));
                res = fetch_with(&url, auth.as_deref(), dest, &shared, done_bytes);
            }
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
            on_file(f);
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

/// The GHCR mirror of a model file, addressed by its sha256 (see .github/workflows/publish-models.yml):
/// an anonymous pull token, the manifest, and the single layer's blob URL. None when not mirrored.
fn ghcr_blob(sha256: &str) -> Option<(String, String)> {
    let repo = std::env::var("GENESIS_MODELS_REPO").unwrap_or_else(|_| "matijagorsek/genesis-models".into());
    let tok = Command::new("curl").args(["-fsSL", "--max-time", "20", &format!("https://ghcr.io/token?scope=repository:{}:pull", repo)]).output().ok()?;
    let token = serde_json::from_slice::<serde_json::Value>(&tok.stdout).ok()?.get("token")?.as_str()?.to_string();
    let man = Command::new("curl").args(["-fsSL", "--max-time", "20", "-H", &format!("Authorization: Bearer {}", token),
        "-H", "Accept: application/vnd.oci.image.manifest.v1+json", &format!("https://ghcr.io/v2/{}/manifests/{}", repo, sha256)]).output().ok()?;
    if !man.status.success() { return None; }
    let m: serde_json::Value = serde_json::from_slice(&man.stdout).ok()?;
    let digest = m.get("layers")?.as_array()?.first()?.get("digest")?.as_str()?.to_string();
    Some((format!("https://ghcr.io/v2/{}/blobs/{}", repo, digest), token))
}

/// curl with resume; progress is sampled from the growing file size.
fn fetch_with(url: &str, bearer: Option<&str>, dest: &Path, shared: &Shared, base_done: u64) -> Result<()> {
    let part = dest.with_extension(format!("{}.part", dest.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default()));
    let mut cmd = Command::new("curl");
    // HTTP/1.1 on purpose: a long transfer over HTTP/2 ended with "stream was not closed cleanly" twenty
    // minutes into a pack (evaluation, 17 Sep), which curl does not retry; --retry-all-errors resumes (-C -)
    cmd.args(["-fL", "--http1.1", "--retry", "8", "--retry-delay", "5", "--retry-all-errors", "-C", "-", "--silent", "--show-error"]);
    if let Some(t) = bearer { cmd.arg("-H").arg(format!("Authorization: Bearer {}", t)); }
    let mut child = cmd
        .arg("-o")
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
