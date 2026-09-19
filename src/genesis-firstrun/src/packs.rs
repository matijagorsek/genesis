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
    /// From the signed pack definition when one is present; the download is checked against it.
    #[serde(default)]
    pub sha256: Option<String>,
}

/// A pack definition resolved to exact files and checksums, pulled as a signed OCI artifact by
/// `genesis-packs refresh` (see the publish-packs workflow). None when no signed copy is on disk.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SignedPack {
    pub id: String,
    #[serde(default)]
    pub resolved_at: String,
    pub files: Vec<SignedFile>,
}
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SignedFile { pub role: String, pub repo: String, pub name: String, #[serde(default)] pub size: u64, pub sha256: String }

pub fn signed_pack(id: &str) -> Option<SignedPack> {
    // per user, like the refresh timer that writes it: ~/.local/state/genesis/packs/<id>.json
    let dir = std::env::var("GENESIS_PACKS_STATE").unwrap_or_else(|_| {
        let base = std::env::var("XDG_STATE_HOME").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| format!("{}/.local/state", std::env::var("HOME").unwrap_or_default()));
        format!("{}/genesis/packs", base)
    });
    let text = std::fs::read_to_string(Path::new(&dir).join(format!("{}.json", id))).ok()?;
    let sp: SignedPack = serde_json::from_str(&text).ok()?;
    if sp.id == id && !sp.files.is_empty() { Some(sp) } else { None }
}

/// Plan from a signed definition: no registry call, exact files, checksums to verify against.
pub fn plan_signed(sp: &SignedPack, models_dir: &Path) -> Vec<PlannedFile> {
    sp.files.iter().map(|f| {
        let base = Path::new(&f.name).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or(f.name.clone());
        PlannedFile {
            role: f.role.clone(), repo: f.repo.clone(), filename: base.clone(), size: f.size,
            url: format!("https://huggingface.co/{}/resolve/main/{}", f.repo, f.name),
            dest: models_dir.join(&f.role).join(&base).display().to_string(),
            sha256: Some(f.sha256.clone()),
        }
    }).collect()
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
                sha256: None,
            });
        }
    }
    Ok(out)
}

/// Render the llama-swap config for a pack from the files on disk. Mirrors prototype/router.yaml.
/// Server tuning derived from the hardware profile: how many threads llama-server may use and whether
/// to offload layers to a GPU. Using every core for generation thrashes (measured 1.6 tok/s at 6 of 6
/// threads vs 7 tok/s at 3 or 4), and a software Vulkan device (llvmpipe, virtio) is slower than the CPU.
#[derive(Debug, Clone, Copy)]
pub struct Tuning { pub threads: u32, pub ngl: u32 }

pub fn tuning_for(cores: u32, gpu_names: &[String], compute_ready: bool) -> Tuning {
    let threads = if cores <= 2 { cores.max(1) } else { (cores - 2).clamp(2, 16) };
    let software = |n: &str| { let l = n.to_lowercase(); l.contains("virtio") || l.contains("llvmpipe") || l.contains("qxl") || l.contains("vmware") || l.contains("bochs") };
    let real_gpu = gpu_names.iter().any(|n| !software(n)) || compute_ready;
    Tuning { threads, ngl: if real_gpu { 99 } else { 0 } }
}

pub fn render_router(models_dir: &Path, port_base: u16) -> Result<String> {
    render_router_tuned(models_dir, port_base, Tuning { threads: 4, ngl: 99 })
}

pub fn render_router_tuned(models_dir: &Path, port_base: u16, t: Tuning) -> Result<String> {
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
    // The small always-loaded models go to the GPU whole. The big ones (coder, chat) leave -ngl unset on a
    // GPU, so llama.cpp's --fit (on by default) places as many layers as the memory still free allows and
    // shrinks the context before it fails: an 8 GB card holds the small models AND a 6 GB coder only that
    // way. On CPU-only machines -ngl 0 stays explicit.
    let big_ngl = if t.ngl == 0 { " -ngl 0".to_string() } else { String::new() };
    let mut y = String::new();
    y.push_str("# Generated by genesis-firstrun from the installed pack. Edit via `genesis apply`, not by hand.\n");
    y.push_str(&format!("healthCheckTimeout: 600\nstartPort: {}\nlogLevel: info\n\nmacros:\n  server: \"llama-server --port ${{PORT}} --host 127.0.0.1 -ngl {} -t {} --jinja --flash-attn auto\"\n  big: \"llama-server --port ${{PORT}} --host 127.0.0.1{} -t {} --jinja --flash-attn auto\"\n\nmodels:\n", port_base, t.ngl, t.threads, big_ngl, t.threads));
    let mut hot = Vec::new();
    let mut big = Vec::new();
    if let Some(f) = find("fast", false) {
        let mm = find("fast", true).map(|m| format!(" --mmproj {}", m)).unwrap_or_default();
        y.push_str(&format!("  fast:\n    cmd: |\n      ${{server}} -m {}{}\n      -c 16384 --temp 0.7 --top-p 0.8 --top-k 20 --reasoning off\n    aliases: [ \"auto\", \"genesis-fast\" ]\n    ttl: 0\n\n", f, mm));
        hot.push("fast");
    }
    if let Some(f) = find("code", false) {
        let mm = find("code", true).map(|m| format!(" --mmproj {}", m)).unwrap_or_default();
        // speculative decoding: a pack may ship a small draft model for the coder (role "draft"); llama.cpp
        // then proposes tokens with it and the big model verifies, roughly doubling generation on dense models
        let draft = find("draft", false).filter(|_| t.ngl > 0).map(|d| format!(" -md {} --draft-max 16 --draft-min 4", d)).unwrap_or_default();
        y.push_str(&format!("  code:\n    cmd: |\n      ${{big}} -m {}{}{}\n      -c {} --temp 0.6 --top-p 0.95 --top-k 20 --min-p 0.0 --reasoning auto --reasoning-budget {}\n      --cache-type-k q8_0 --cache-type-v q8_0\n    aliases: [ \"genesis-code\" ]\n    ttl: 600\n\n", f, mm, draft, if t.ngl == 0 { 8192 } else { 32768 }, if t.ngl == 0 { 512 } else { 4096 }));
        big.push("code");
    }
    if let Some(f) = find("chat", false) {
        y.push_str(&format!("  chat:\n    cmd: |\n      ${{big}} -m {}\n      -c 32768 --temp 0.7 --top-p 0.8 --top-k 20\n    aliases: [ \"genesis-chat\" ]\n    ttl: 600\n\n", f));
        big.push("chat");
    }
    if let Some(f) = find("fim", false) {
        // fill-in-the-middle for Babel's inline completion: small, short context. Resident on a machine with
        // a GPU, where holding it costs nothing anyone notices; on a CPU pack it unloads when idle (see embed)
        y.push_str(&format!("  fim:\n    cmd: |\n      ${{server}} -m {}\n      -c 4096 --temp 0.2 --top-p 0.9 --reasoning off\n    aliases: [ \"genesis-fim\" ]\n    ttl: {}\n\n", f, if t.ngl == 0 { 300 } else { 0 }));
        if t.ngl > 0 { hot.push("fim"); }
    }
    if let Some(f) = find("rerank", false) {
        // reranker for the file index: scores query/passage pairs (llama-server --reranking, /v1/rerank)
        y.push_str(&format!("  rerank:\n    cmd: |\n      ${{server}} -m {} --reranking -c 4096 -b 4096 -ub 4096\n    aliases: [ \"genesis-rerank\" ]\n    ttl: 600\n\n", f));
        big.push("rerank");
    }
    if let Some(f) = find("embed", false) {
        // The embedding model was resident for the life of the session on every machine, in a group that
        // never swaps out. On a GPU that is the right trade: a search answers without a reload. Without one
        // it cost between 1.0 and 1.6 GB alongside a 3.4 GB main model, on packs whose whole reason to exist
        // is a machine with 4 to 8 GB of RAM — the evaluation found three makes running on under half the
        // memory of the others for exactly this reason. On a CPU pack it now unloads five minutes after the
        // last search, and indexes in smaller batches, which is the same memory again at the moment of use.
        let (batch, ttl) = if t.ngl == 0 { (2048, 300) } else { (8192, 0) };
        y.push_str(&format!("  embed:\n    cmd: |\n      ${{server}} -m {} --embedding --pooling last -c 8192 -b {} -ub {}\n    aliases: [ \"genesis-embed\", \"text-embedding-3-small\" ]\n    ttl: {}\n\n", f, batch, batch, ttl));
        if t.ngl > 0 { hot.push("embed"); }
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
    fn tuning_from_profile() {
        let t = tuning_for(6, &["Other GPU 1af4:1050 (virtio-pci)".into()], false);
        assert_eq!((t.threads, t.ngl), (4, 0));
        let t = tuning_for(16, &["NVIDIA GeForce RTX 4090".into()], true);
        assert_eq!((t.threads, t.ngl), (14, 99));
        let t = tuning_for(2, &[], false);
        assert_eq!((t.threads, t.ngl), (2, 0));
        let t = tuning_for(8, &["Apple M4 Max GPU (unified memory)".into()], false);
        assert_eq!(t.ngl, 99);
    }

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
    fn draft_model_enables_speculative_decoding_on_gpu() {
        let dir = tempfile::tempdir().unwrap();
        for (role, f) in [("code", "b.gguf"), ("draft", "d.gguf")] { std::fs::create_dir_all(dir.path().join(role)).unwrap(); std::fs::write(dir.path().join(role).join(f), b"x").unwrap(); }
        let gpu = render_router_tuned(dir.path(), 10001, Tuning { threads: 4, ngl: 99 }).unwrap();
        assert!(gpu.contains("-md ") && gpu.contains("--draft-max 16"));
        let cpu = render_router_tuned(dir.path(), 10001, Tuning { threads: 4, ngl: 0 }).unwrap();
        assert!(!cpu.contains("-md "), "no draft on CPU: it would slow generation down");
        // the big models leave -ngl to llama.cpp's fit on a GPU, and stay on the CPU explicitly without one
        assert!(gpu.contains("big: \"llama-server --port ${PORT} --host 127.0.0.1 -t 4"));
        assert!(cpu.contains("big: \"llama-server --port ${PORT} --host 127.0.0.1 -ngl 0 -t 4"));
        assert!(gpu.contains("${big} -m"));
    }

    #[test]
    fn a_machine_without_a_gpu_does_not_hold_the_small_models_forever() {
        let dir = tempfile::tempdir().unwrap();
        for (role, f) in [("fast", "a.gguf"), ("embed", "e.gguf"), ("fim", "f.gguf")] {
            std::fs::create_dir_all(dir.path().join(role)).unwrap();
            std::fs::write(dir.path().join(role).join(f), b"x").unwrap();
        }
        // With a GPU, holding the embedding model costs nothing anyone notices and a search never waits.
        let gpu = render_router_tuned(dir.path(), 10001, Tuning { threads: 4, ngl: 99 }).unwrap();
        assert!(gpu.contains("members: [ fast, fim, embed ]"), "all three stay resident: {}", gpu);
        assert!(gpu.contains("-b 8192 -ub 8192"));

        // Without one, embed cost 1.0 to 1.6 GB beside a 3.4 GB main model on packs meant for 4 to 8 GB
        // machines. It unloads when idle, and it is not in the group that never swaps out.
        let cpu = render_router_tuned(dir.path(), 10001, Tuning { threads: 4, ngl: 0 }).unwrap();
        assert!(cpu.contains("members: [ fast ]"), "only the main model stays resident: {}", cpu);
        let embed = cpu.split("  embed:").nth(1).unwrap().split("\n\n").next().unwrap();
        assert!(embed.contains("ttl: 300"), "embed unloads when idle: {}", embed);
        assert!(embed.contains("-b 2048 -ub 2048"), "and indexes in smaller batches: {}", embed);
        let fim = cpu.split("  fim:").nth(1).unwrap().split("\n\n").next().unwrap();
        assert!(fim.contains("ttl: 300"), "the completion model too: {}", fim);
        // the main model is still never unloaded: a reload in front of someone typing is the thing to avoid
        let fast = cpu.split("  fast:").nth(1).unwrap().split("\n\n").next().unwrap();
        assert!(fast.contains("ttl: 0"), "{}", fast);
    }

    #[test]
    fn router_renders_from_files_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        for (role, f) in [("fast", "a-Q8_0.gguf"), ("fast", "mmproj-F16.gguf"), ("code", "b-Q4_K_M.gguf"), ("embed", "e.gguf"), ("fim", "f.gguf"), ("rerank", "r.gguf")] {
            std::fs::create_dir_all(dir.path().join(role)).unwrap();
            std::fs::write(dir.path().join(role).join(f), b"x").unwrap();
        }
        let y = render_router(dir.path(), 10001).unwrap();
        assert!(y.contains("  fast:"));
        assert!(y.contains("--mmproj"));
        assert!(y.contains("  code:"));
        assert!(y.contains("  embed:"));
        assert!(!y.contains("  chat:"));
        assert!(y.contains("  fim:"));
        assert!(y.contains("members: [ fast, fim, embed ]"));
        assert!(y.contains("  rerank:"));
        assert!(y.contains("members: [ code, rerank ]"));
    }
}
