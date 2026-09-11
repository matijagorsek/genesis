//! genesis-probe: detect hardware, pick a model pack, write the profile.
//!
//!   genesis-probe                       print a summary
//!   genesis-probe --json                print the profile as JSON
//!   genesis-probe --write               write /etc/genesis/profile.json (or --out PATH)
//!   genesis-probe --fake gpu=nvidia,vram=24,ram=64,disk=400   evaluate packs for a pretend machine

mod detect;
mod model;
mod select;

use anyhow::{Context, Result};
use clap::Parser;
use model::{BudgetKind, Cpu, Gpu, GpuVendor, Profile};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "genesis-probe", version, about = "Genesis hardware probe and pack selector")]
struct Cli {
    /// Directory with pack definitions (default: /usr/share/genesis/packs, or ./packs when present).
    #[arg(long)]
    packs: Option<PathBuf>,
    /// Print the full profile as JSON.
    #[arg(long)]
    json: bool,
    /// Write the profile to the given path (default /etc/genesis/profile.json).
    #[arg(long)]
    write: bool,
    #[arg(long)]
    out: Option<PathBuf>,
    /// Path whose filesystem holds the models (for free-disk check).
    #[arg(long, default_value = "/var/lib/genesis")]
    models_dir: PathBuf,
    /// Pretend hardware: gpu=nvidia|amd|intel|apple|none,vram=GB,ram=GB,disk=GB
    #[arg(long)]
    fake: Option<String>,
    /// Alternate sysfs / procfs roots (tests).
    #[arg(long, default_value = "/sys", hide = true)]
    sysfs: PathBuf,
    #[arg(long, default_value = "/proc", hide = true)]
    procfs: PathBuf,
}

fn now() -> String {
    time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339).unwrap_or_default()
}

fn parse_fake(spec: &str) -> Result<(Vec<Gpu>, u64, u64, bool)> {
    let mut gpu = "none".to_string();
    let mut vram_gb = 0.0f64;
    let mut ram_gb = 16.0f64;
    let mut disk_gb = 500.0f64;
    for kv in spec.split(',') {
        let (k, v) = kv.split_once('=').with_context(|| format!("bad fake spec item {}", kv))?;
        match k.trim() {
            "gpu" => gpu = v.trim().to_lowercase(),
            "vram" => vram_gb = v.trim().parse()?,
            "ram" => ram_gb = v.trim().parse()?,
            "disk" => disk_gb = v.trim().parse()?,
            other => anyhow::bail!("unknown fake key {}", other),
        }
    }
    let unified = gpu == "apple";
    let gpus = match gpu.as_str() {
        "none" => vec![],
        v => {
            let vendor = match v {
                "nvidia" => GpuVendor::Nvidia,
                "amd" => GpuVendor::Amd,
                "intel" => GpuVendor::Intel,
                "apple" => GpuVendor::Apple,
                _ => GpuVendor::Other,
            };
            vec![Gpu { vendor, name: format!("fake {} GPU", v), vram_mb: (vram_gb * 1024.0) as u64, driver: "fake".into(), pci_id: String::new(), compute_ready: true }]
        }
    };
    Ok((gpus, (ram_gb * 1024.0) as u64, (disk_gb * 1024.0) as u64, unified))
}

pub fn build_profile(gpus: Vec<Gpu>, ram_mb: u64, cpu: Cpu, disk_free_mb: u64, unified: bool, host_os: &str, packs: &[model::PackFile]) -> Profile {
    let (budget_mb, budget_kind) = select::budget(&gpus, ram_mb, unified);
    let (fits, recommended) = select::evaluate(packs, &gpus, ram_mb, unified, budget_mb, disk_free_mb);
    let mut profile = Profile { version: 1, probed_at: now(), host_os: host_os.into(), gpus, ram_mb, cpu, disk_free_mb, unified_memory: unified, budget_mb, budget_kind, recommended_pack: recommended, packs: fits, notes: vec![] };
    profile.notes = select::notes_for(&profile);
    profile
}

fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive("info".parse()?)).with_writer(std::io::stderr).init();
    let cli = Cli::parse();

    let packs_dir = cli.packs.clone().unwrap_or_else(|| {
        let local = PathBuf::from("packs");
        if local.is_dir() { local } else { PathBuf::from("/usr/share/genesis/packs") }
    });
    let packs = select::load_packs(&packs_dir)?;

    let profile = if let Some(spec) = &cli.fake {
        let (gpus, ram_mb, disk_mb, unified) = parse_fake(spec)?;
        build_profile(gpus, ram_mb, Cpu { model: "fake".into(), cores: 8, avx2: true, avx512: false, amx: false }, disk_mb, unified, "fake", &packs)
    } else {
        let raw = detect::detect(&cli.sysfs, &cli.procfs);
        let unified = raw.gpus.iter().any(|g| g.vendor == GpuVendor::Apple);
        let disk = detect::disk_free_mb(&cli.models_dir);
        build_profile(raw.gpus, raw.ram_mb, raw.cpu, disk, unified, &raw.host_os, &packs)
    };

    if cli.write {
        let out = cli.out.clone().unwrap_or_else(|| PathBuf::from("/etc/genesis/profile.json"));
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&out, serde_json::to_string_pretty(&profile)?)?;
        tracing::info!(path = %out.display(), pack = ?profile.recommended_pack, "profile written");
    }

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&profile)?);
    } else {
        print_summary(&profile);
    }
    Ok(())
}

fn print_summary(p: &Profile) {
    println!("Genesis hardware profile ({})", p.host_os);
    if p.gpus.is_empty() {
        println!("  GPU      none");
    }
    for g in &p.gpus {
        println!("  GPU      {} · {} MB · driver {} · compute {}", g.name, g.vram_mb, if g.driver.is_empty() { "-" } else { &g.driver }, if g.compute_ready { "ready" } else { "no" });
    }
    println!("  RAM      {} MB", p.ram_mb);
    println!("  CPU      {} · {} threads · avx2 {} · avx512 {}", p.cpu.model, p.cpu.cores, p.cpu.avx2, p.cpu.avx512);
    println!("  Disk     {} MB free for models", p.disk_free_mb);
    println!("  Budget   {} MB ({})", p.budget_mb, match p.budget_kind { BudgetKind::Vram => "VRAM", BudgetKind::Unified => "unified memory", BudgetKind::Ram => "60% of RAM, CPU inference" });
    println!();
    println!("  {:<10} {:<6} {:>8}  {}", "pack", "fits", "disk", "reason");
    for f in &p.packs {
        let mark = if Some(&f.id) == p.recommended_pack.as_ref() { "*" } else { " " };
        println!("{} {:<10} {:<6} {:>6.0} GB  {}", mark, f.id, if f.fits { "yes" } else { "no" }, f.disk_gb, f.reason);
    }
    println!();
    match &p.recommended_pack {
        Some(id) => println!("  recommended: {}", id),
        None => println!("  recommended: none fits"),
    }
    for n in &p.notes {
        println!("  note: {}", n);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packs() -> Vec<model::PackFile> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs");
        select::load_packs(&dir).expect("repo packs load")
    }

    fn rec(spec: &str) -> Option<String> {
        let (gpus, ram, disk, unified) = parse_fake(spec).unwrap();
        build_profile(gpus, ram, Cpu::default(), disk, unified, "fake", &packs()).recommended_pack
    }

    #[test]
    fn repo_packs_parse() {
        let p = packs();
        assert!(p.len() >= 7);
        assert!(p.iter().any(|x| x.id == "gpu-24"));
    }

    #[test]
    fn selection_per_tier() {
        assert_eq!(rec("gpu=nvidia,vram=24,ram=64,disk=400").as_deref(), Some("gpu-24"));
        assert_eq!(rec("gpu=nvidia,vram=48,ram=128,disk=800").as_deref(), Some("gpu-48"));
        assert_eq!(rec("gpu=amd,vram=16,ram=32,disk=400").as_deref(), Some("gpu-16"));
        assert_eq!(rec("gpu=nvidia,vram=8,ram=32,disk=400").as_deref(), Some("gpu-8"));
        assert_eq!(rec("gpu=none,ram=16,disk=400").as_deref(), Some("cpu"));
        assert_eq!(rec("gpu=none,ram=8,disk=400").as_deref(), Some("tiny"));
    }

    #[test]
    fn apple_unified_36gb_gets_the_mac_pack() {
        let r = rec("gpu=apple,ram=36,disk=400");
        assert!(matches!(r.as_deref(), Some("mac-36") | Some("mac-36-q5")), "{:?}", r);
    }

    #[test]
    fn low_disk_blocks_big_packs() {
        assert_eq!(rec("gpu=nvidia,vram=24,ram=64,disk=20").as_deref(), Some("cpu"));
    }

    #[test]
    fn budget_rules() {
        let g = Gpu { vendor: GpuVendor::Nvidia, name: "x".into(), vram_mb: 24 * 1024, driver: "nvidia".into(), pci_id: String::new(), compute_ready: true };
        assert_eq!(select::budget(&[g.clone()], 64 * 1024, false), (24 * 1024, BudgetKind::Vram));
        assert_eq!(select::budget(&[], 32 * 1024, false), (32 * 1024 * 6 / 10, BudgetKind::Ram));
        assert_eq!(select::budget(&[], 36 * 1024, true), (36 * 1024 * 3 / 4, BudgetKind::Unified));
        let small = Gpu { vram_mb: 2048, ..g };
        assert_eq!(select::budget(&[small], 32 * 1024, false).1, BudgetKind::Ram);
    }

    #[test]
    fn sysfs_and_procfs_parsing() {
        let dir = tempfile::tempdir().unwrap();
        let sys = dir.path().join("sys");
        let card = sys.join("class/drm/card0/device");
        std::fs::create_dir_all(&card).unwrap();
        std::fs::write(card.join("vendor"), "0x1002\n").unwrap();
        std::fs::write(card.join("device"), "0x744c\n").unwrap();
        std::fs::write(card.join("mem_info_vram_total"), format!("{}\n", 24u64 * 1024 * 1024 * 1024)).unwrap();
        std::fs::create_dir_all(sys.join("class/drm/card0-DP-1")).unwrap();
        let proc_ = dir.path().join("proc");
        std::fs::create_dir_all(&proc_).unwrap();
        std::fs::write(proc_.join("meminfo"), "MemTotal:       65536000 kB\nMemFree: 1 kB\n").unwrap();
        std::fs::write(proc_.join("cpuinfo"), "processor\t: 0\nmodel name\t: AMD Ryzen 9 7950X\nflags\t\t: fpu avx2 avx512f\nprocessor\t: 1\n").unwrap();
        let gpus = detect::gpus_linux(&sys);
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].vendor, GpuVendor::Amd);
        assert_eq!(gpus[0].vram_mb, 24 * 1024);
        assert_eq!(gpus[0].pci_id, "1002:744c");
        assert_eq!(detect::ram_linux(&proc_), 64000);
        let cpu = detect::cpu_linux(&proc_);
        assert_eq!(cpu.cores, 2);
        assert!(cpu.avx2 && cpu.avx512);
        assert_eq!(cpu.model, "AMD Ryzen 9 7950X");
    }
}
