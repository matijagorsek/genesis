//! Pack files, download planning against the Hugging Face API, and router config rendering.

use anyhow::{anyhow, Context, Result};
use globset::{Glob, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Pack {
    pub id: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub disk_gb: f64,
    #[serde(default)]
    pub models: Vec<PackModel>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PackModel {
    pub role: String,
    #[serde(default)]
    pub name: String,
    pub repo: String,
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub size_gb: f64,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub offload: bool,
}

pub fn load_pack(dir: &Path, id: &str) -> Result<Pack> {
    let p = dir.join(format!("{}.json", id));
    let text = std::fs::read_to_string(&p).with_context(|| format!("reading pack {}", p.display()))?;
    Ok(serde_json::from_str(&text)?)
}

pub fn list_packs(dir: &Path) -> Result<Vec<Pack>> {
    let mut out = Vec::new();
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().map(|e| e == "json").unwrap_or(false) && p.file_name().map(|f| f != "schema.json").unwrap_or(false)).collect();
    files.sort();
    for f in files {
        if let Ok(p) = serde_json::from_str::<Pack>(&std::fs::read_to_string(&f)?) {
            out.push(p);
        }
    }
    Ok(out)
}

/// One file to fetch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlannedFile {
    pub role: String,
    pub repo: String,
    pub filename: String,
    pub size: u64,
    pub url: String,
    pub dest: String,
}

#[derive(Debug, Deserialize)]
struct HfSibling {
    rfilename: String,
    #[serde(default)]
    size: Option<u64>,
}
#[derive(Debug, Deserialize)]
struct HfModel {
    #[serde(default)]
    siblings: Vec<HfSibling>,
}

/// Ask the Hugging Face API which files match a model's include globs. Uses curl so the binary stays small.
pub fn hf_files(repo: &str) -> Result<Vec<(String, u64)>> {
    let url = format!("https://huggingface.co/api/models/{}?blobs=true", repo);
    let out = Command::new("curl").args(["-fsSL", "--max-time", "60", &url]).output().context("running curl")?;
    if !out.status.success() {
        return Err(anyhow!("hf api {}: {}", repo, String::from_utf8_lossy(&out.stderr).trim()));
    }
    let m: HfModel = serde_json::from_slice(&out.stdout).with_context(|| format!("parsing hf api response for {}", repo))?;
    Ok(m.siblings.into_iter().map(|s| (s.rfilename, s.size.unwrap_or(0))).collect())
}

pub fn match_includes(files: &[(String, u64)], include: &[String]) -> Result<Vec<(String, u64)>> {
    let mut b = GlobSetBuilder::new();
    for g in include {
        b.add(Glob::new(g)?);
    }
    let set = b.build()?;
    Ok(files.iter().filter(|(f, _)| set.is_match(f)).cloned().collect())
}

/// Build the download plan for a pack. `models_dir` is where role subdirectories live.
pub fn plan(pack: &Pack, models_dir: &Path, lister: &dyn Fn(&str) -> Result<Vec<(String, u64)>>) -> Result<Vec<PlannedFile>> {
    let mut out = Vec::new();
    for m in &pack.models {
        if m.optional {
            continue;
        }
        let files = lister(&m.repo)?;
        let matched = match_includes(&files, &m.include)?;
        if matched.is_empty() {
            return Err(anyhow!("no files in {} match {:?}", m.repo, m.include));
        }
        for (f, size) in matched {
            let base = Path::new(&f).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or(f.clone());
            out.push(PlannedFile {
                role: m.role.clone(),
                repo: m.repo.clone(),
                filename: base.clone(),
                size,
                url: format!("https://huggingface.co/{}/resolve/main/{}", m.repo, f),
                dest: models_dir.join(&m.role).join(&base).display().to_string(),
            });
        }
    }
    Ok(out)
}

/// Render the llama-swap config for a pack from the files on disk. Mirrors prototype/router.yaml.
pub fn render_router(models_dir: &Path, port_base: u16) -> Result<String> {
    let models_dir = std::fs::canonicalize(models_dir).unwrap_or_else(|_| models_dir.to_path_buf());
    let models_dir = models_dir.as_path();
    let find = |role: &str, mmproj: bool| -> Option<String> {
        let dir = models_dir.join(role);
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).ok()?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().map(|e| e == "gguf").unwrap_or(false)).collect();
        files.sort();
        files.into_iter().find(|p| {
            let n = p.file_name().unwrap().to_string_lossy();
            if mmproj { n.starts_with("mmproj") } else { !n.starts_with("mmproj") }
        }).map(|p| p.display().to_string())
    };
    let mut y = String::new();
    y.push_str("# Generated by genesis-firstrun from the installed pack. Edit via `genesis apply`, not by hand.\n");
    y.push_str(&format!("healthCheckTimeout: 600\nstartPort: {}\nlogLevel: info\n\nmacros:\n  server: \"llama-server --port ${{PORT}} --host 127.0.0.1 -ngl 99 --jinja --flash-attn on\"\n\nmodels:\n", port_base));
    let mut hot = Vec::new();
    let mut big = Vec::new();
    if let Some(f) = find("fast", false) {
        let mm = find("fast", true).map(|m| format!(" --mmproj {}", m)).unwrap_or_default();
        y.push_str(&format!("  fast:\n    cmd: |\n      ${{server}} -m {}{}\n      -c 16384 --temp 0.7 --top-p 0.8 --top-k 20 --reasoning off\n    aliases: [ \"auto\", \"genesis-fast\" ]\n    ttl: 0\n\n", f, mm));
        hot.push("fast");
    }
    if let Some(f) = find("code", false) {
        let mm = find("code", true).map(|m| format!(" --mmproj {}", m)).unwrap_or_default();
        y.push_str(&format!("  code:\n    cmd: |\n      ${{server}} -m {}{}\n      -c 32768 --temp 0.6 --top-p 0.95 --top-k 20 --min-p 0.0 --reasoning auto --reasoning-budget 4096\n      --cache-type-k q8_0 --cache-type-v q8_0\n    aliases: [ \"genesis-code\" ]\n    ttl: 600\n\n", f, mm));
        big.push("code");
    }
    if let Some(f) = find("chat", false) {
        y.push_str(&format!("  chat:\n    cmd: |\n      ${{server}} -m {}\n      -c 32768 --temp 0.7 --top-p 0.8 --top-k 20\n    aliases: [ \"genesis-chat\" ]\n    ttl: 600\n\n", f));
        big.push("chat");
    }
    if let Some(f) = find("embed", false) {
        y.push_str(&format!("  embed:\n    cmd: |\n      ${{server}} -m {} --embedding --pooling last -c 8192 -b 8192 -ub 8192\n    aliases: [ \"genesis-embed\", \"text-embedding-3-small\" ]\n    ttl: 0\n\n", f));
        hot.push("embed");
    }
    y.push_str("groups:\n");
    if !hot.is_empty() {
        y.push_str(&format!("  hot:\n    swap: false\n    exclusive: false\n    persistent: true\n    members: [ {} ]\n", hot.join(", ")));
    }
    if !big.is_empty() {
        y.push_str(&format!("  big:\n    swap: true\n    exclusive: false\n    members: [ {} ]\n", big.join(", ")));
    }
    Ok(y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_matches_includes_and_skips_optional() {
        let pack = Pack {
            id: "t".into(),
            description: String::new(),
            disk_gb: 1.0,
            models: vec![
                PackModel { role: "fast".into(), name: "x".into(), repo: "org/x".into(), include: vec!["*Q8_0*.gguf".into(), "mmproj-F16.gguf".into()], license: String::new(), size_gb: 1.0, optional: false, offload: false },
                PackModel { role: "chat".into(), name: "y".into(), repo: "org/y".into(), include: vec!["*.gguf".into()], license: String::new(), size_gb: 9.0, optional: true, offload: false },
            ],
        };
        let lister = |repo: &str| -> Result<Vec<(String, u64)>> {
            Ok(match repo {
                "org/x" => vec![("x-Q8_0.gguf".into(), 100), ("x-Q4_K_M.gguf".into(), 50), ("mmproj-F16.gguf".into(), 10), ("mmproj-BF16.gguf".into(), 10), ("README.md".into(), 1)],
                _ => vec![("y.gguf".into(), 900)],
            })
        };
        let p = plan(&pack, Path::new("/m"), &lister).unwrap();
        let names: Vec<&str> = p.iter().map(|f| f.filename.as_str()).collect();
        assert_eq!(names, vec!["x-Q8_0.gguf", "mmproj-F16.gguf"]);
        assert_eq!(p[0].dest, "/m/fast/x-Q8_0.gguf");
        assert!(p[0].url.ends_with("/org/x/resolve/main/x-Q8_0.gguf"));
        assert_eq!(p.iter().map(|f| f.size).sum::<u64>(), 110);
    }

    #[test]
    fn router_renders_from_files_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        for (role, f) in [("fast", "a-Q8_0.gguf"), ("fast", "mmproj-F16.gguf"), ("code", "b-Q4_K_M.gguf"), ("embed", "e.gguf")] {
            std::fs::create_dir_all(dir.path().join(role)).unwrap();
            std::fs::write(dir.path().join(role).join(f), b"x").unwrap();
        }
        let y = render_router(dir.path(), 10001).unwrap();
        assert!(y.contains("  fast:"));
        assert!(y.contains("--mmproj"));
        assert!(y.contains("  code:"));
        assert!(y.contains("  embed:"));
        assert!(!y.contains("  chat:"));
        assert!(y.contains("members: [ fast, embed ]"));
        assert!(y.contains("members: [ code ]"));
    }
}
