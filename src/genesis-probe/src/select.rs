//! Budget computation and pack selection.

use crate::model::{BudgetKind, Gpu, GpuVendor, PackFile, PackFit, Profile, Trigger};
use anyhow::{Context, Result};
use std::path::Path;

/// Memory available for resident models, and what kind of memory it is.
pub fn budget(gpus: &[Gpu], ram_mb: u64, unified: bool) -> (u64, BudgetKind) {
    if unified {
        // Apple Silicon and similar: the GPU can use most of RAM; keep 25% for the OS and apps.
        return (ram_mb * 3 / 4, BudgetKind::Unified);
    }
    let best = gpus.iter().filter(|g| g.compute_ready || g.vendor == GpuVendor::Nvidia || g.vendor == GpuVendor::Amd).map(|g| g.vram_mb).max().unwrap_or(0);
    if best >= 4096 {
        (best, BudgetKind::Vram)
    } else {
        (ram_mb * 6 / 10, BudgetKind::Ram)
    }
}

pub fn load_packs(dir: &Path) -> Result<Vec<PackFile>> {
    let mut packs = Vec::new();
    let mut files: Vec<_> = std::fs::read_dir(dir).with_context(|| format!("reading packs dir {}", dir.display()))?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().map(|e| e == "json").unwrap_or(false) && p.file_name().map(|f| f != "schema.json").unwrap_or(false)).collect();
    files.sort();
    for f in files {
        let text = std::fs::read_to_string(&f)?;
        let pack: PackFile = serde_json::from_str(&text).with_context(|| format!("parsing {}", f.display()))?;
        packs.push(pack);
    }
    Ok(packs)
}

fn gpu_matches(trigger: &Trigger, gpus: &[Gpu]) -> (bool, String) {
    let want = trigger.gpu.as_deref().unwrap_or("any");
    let usable: Vec<&Gpu> = gpus.iter().filter(|g| g.vendor != GpuVendor::Other).collect();
    match want {
        "any" => (true, String::new()),
        "none" => (true, String::new()), // a CPU pack always works, it just may not be the best
        "nvidia" | "amd" | "intel" | "apple" => {
            let ok = usable.iter().any(|g| format!("{:?}", g.vendor).to_lowercase() == want);
            (ok, if ok { String::new() } else { format!("needs a {} GPU", want) })
        }
        other => (false, format!("unknown gpu trigger {}", other)),
    }
}

/// Evaluate every pack against the profile and pick the best fit.
pub fn evaluate(packs: &[PackFile], gpus: &[Gpu], ram_mb: u64, unified: bool, budget_mb: u64, disk_free_mb: u64) -> (Vec<PackFit>, Option<String>) {
    let vram_gb = gpus.iter().map(|g| g.vram_mb).max().unwrap_or(0) as f64 / 1024.0;
    let ram_gb = ram_mb as f64 / 1024.0;
    let mut fits = Vec::new();
    for p in packs {
        let mut reasons = Vec::new();
        let (gok, greason) = gpu_matches(&p.trigger, gpus);
        if !gok {
            reasons.push(greason);
        }
        if let Some(min) = p.trigger.min_vram_gb {
            let have = if unified { budget_mb as f64 / 1024.0 } else { vram_gb };
            if have + 0.5 < min {
                reasons.push(format!("needs {:.0} GB of GPU memory, have {:.0}", min, have));
            }
        }
        if let Some(min) = p.trigger.min_ram_gb {
            // A machine sold as 16 GB reports about 15.5 GiB: the firmware keeps some, and the rest is
            // counted in GiB rather than the GB on the box. With half a gigabyte of slack the CPU pack —
            // the one whose description says "16 GB RAM" — was refused on a 16 GB laptop by five
            // megabytes, and a 32 GB machine missed the 36 GB packs the same way. The slack has to be
            // bigger than the gap between what is sold and what is counted, so: a gigabyte, or five per
            // cent for the large packs, whichever is more.
            let slack = (min * 0.05).max(1.0);
            if ram_gb + slack < min {
                reasons.push(format!("needs {:.0} GB RAM, have {:.0}", min, ram_gb));
            }
        }
        if let Some(u) = p.trigger.unified_memory {
            if u != unified {
                reasons.push(if u { "made for unified-memory machines".into() } else { "made for discrete GPUs".into() });
            }
        }
        let largest_mb = p.models.iter().filter(|m| !m.optional && !m.offload).map(|m| (m.size_gb * 1024.0) as u64).max().unwrap_or(0);
        // 5% tolerance: a model a hair over the budget runs with a few layers offloaded.
        if largest_mb > budget_mb * 105 / 100 {
            reasons.push(format!("largest model {:.1} GB exceeds the {:.1} GB budget", largest_mb as f64 / 1024.0, budget_mb as f64 / 1024.0));
        }
        let need_disk = (p.disk_gb * 1024.0 * 1.1) as u64;
        if disk_free_mb > 0 && need_disk > disk_free_mb {
            reasons.push(format!("needs {:.0} GB free disk, have {:.0}", need_disk as f64 / 1024.0, disk_free_mb as f64 / 1024.0));
        }
        let ok = reasons.is_empty();
        // Specificity: a pack whose trigger names this machine's platform beats a generic one.
        let vendor = gpus.iter().find(|g| g.vendor != GpuVendor::Other).map(|g| format!("{:?}", g.vendor).to_lowercase());
        let mut specificity = 0u8;
        if let (Some(want), Some(have)) = (p.trigger.gpu.as_deref(), vendor.as_deref()) {
            if want == have { specificity += 2; }
        }
        if p.trigger.unified_memory == Some(unified) { specificity += 1; }
        fits.push(PackFit { id: p.id.clone(), description: p.description.clone(), disk_gb: p.disk_gb, fits: ok, reason: if ok { "fits".into() } else { reasons.join("; ") }, largest_model_mb: largest_mb, specificity });
    }
    // Best = fitting pack with the highest specificity, then the highest hardware tier its trigger asks for
    // (min VRAM, then min RAM), then disk size as a proxy for how much it ships.
    let mut best: Option<(&PackFit, (u8, u64, u64, u64))> = None;
    for (f, p) in fits.iter().zip(packs.iter()) {
        if !f.fits {
            continue;
        }
        let score = (f.specificity, (p.trigger.min_vram_gb.unwrap_or(0.0) * 1024.0) as u64, (p.trigger.min_ram_gb.unwrap_or(0.0) * 1024.0) as u64, (f.disk_gb * 1024.0) as u64);
        if best.map(|(_, s)| score > s).unwrap_or(true) {
            best = Some((f, score));
        }
    }
    (fits.clone(), best.map(|(f, _)| f.id.clone()))
}

pub fn notes_for(profile: &Profile) -> Vec<String> {
    let mut n = Vec::new();
    if profile.gpus.is_empty() {
        n.push("No GPU detected. Models run on the CPU; expect slower answers and no reliable app building.".into());
    }
    for g in &profile.gpus {
        if g.vendor == GpuVendor::Nvidia && !g.compute_ready {
            n.push(format!("{}: NVIDIA driver not loaded (driver: {}). Use the genesis-nvidia image for full speed.", g.name, if g.driver.is_empty() { "none" } else { &g.driver }));
        }
        if g.vendor == GpuVendor::Amd && !g.compute_ready {
            n.push(format!("{}: ROCm compute not available; Vulkan will be used.", g.name));
        }
    }
    if profile.budget_kind == BudgetKind::Ram && profile.ram_mb < 16 * 1024 {
        n.push("Under 16 GB RAM: only the tiny pack fits comfortably.".into());
    }
    if profile.disk_free_mb > 0 && profile.disk_free_mb < 30 * 1024 {
        n.push(format!("Only {:.0} GB free on the models volume; larger packs will not fit.", profile.disk_free_mb as f64 / 1024.0));
    }
    n
}
