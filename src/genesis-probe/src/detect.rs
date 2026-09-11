//! Hardware detection. Linux reads sysfs and procfs; macOS has a small fallback so the tool can be
//! exercised on the dev machine. Everything degrades to "unknown" rather than failing.

use crate::model::{Cpu, Gpu, GpuVendor};
use std::fs;
use std::path::Path;
use std::process::Command;

pub struct Raw {
    pub gpus: Vec<Gpu>,
    pub ram_mb: u64,
    pub cpu: Cpu,
    pub host_os: String,
}

pub fn detect(sysfs_root: &Path, proc_root: &Path) -> Raw {
    if cfg!(target_os = "linux") || sysfs_root != Path::new("/sys") {
        Raw {
            gpus: gpus_linux(sysfs_root),
            ram_mb: ram_linux(proc_root),
            cpu: cpu_linux(proc_root),
            host_os: "linux".into(),
        }
    } else {
        detect_macos()
    }
}

// ---------------- Linux ----------------

fn read_trim(p: &Path) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

pub fn gpus_linux(sysfs: &Path) -> Vec<Gpu> {
    let mut out = Vec::new();
    let drm = sysfs.join("class/drm");
    let Ok(entries) = fs::read_dir(&drm) else { return out };
    let mut names: Vec<String> = entries.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).collect();
    names.sort();
    for name in names {
        // card0, card1 ... but not card0-DP-1 connectors or renderD128
        if !name.starts_with("card") || name.contains('-') {
            continue;
        }
        let dev = drm.join(&name).join("device");
        let Some(vendor) = read_trim(&dev.join("vendor")) else { continue };
        let device = read_trim(&dev.join("device")).unwrap_or_default();
        let vendor_id = vendor.trim_start_matches("0x").to_lowercase();
        let device_id = device.trim_start_matches("0x").to_lowercase();
        let driver = fs::read_link(dev.join("driver")).ok().and_then(|p| p.file_name().map(|f| f.to_string_lossy().to_string())).unwrap_or_default();
        let gv = match vendor_id.as_str() {
            "10de" => GpuVendor::Nvidia,
            "1002" | "1022" => GpuVendor::Amd,
            "8086" => GpuVendor::Intel,
            _ => GpuVendor::Other,
        };
        let mut vram_mb = read_trim(&dev.join("mem_info_vram_total")).and_then(|s| s.parse::<u64>().ok()).map(|b| b / (1024 * 1024)).unwrap_or(0);
        let mut gname = String::new();
        if gv == GpuVendor::Nvidia {
            if let Some((n, v)) = nvidia_smi() {
                gname = n;
                if vram_mb == 0 {
                    vram_mb = v;
                }
            }
        }
        if gname.is_empty() {
            gname = product_name(&dev).unwrap_or_else(|| format!("{:?} GPU {}:{}", gv, vendor_id, device_id));
        }
        let compute_ready = match gv {
            GpuVendor::Nvidia => driver == "nvidia",
            GpuVendor::Amd => driver == "amdgpu" && Path::new("/dev/kfd").exists(),
            GpuVendor::Intel => driver == "i915" || driver == "xe",
            _ => false,
        };
        out.push(Gpu { vendor: gv, name: gname, vram_mb, driver, pci_id: format!("{}:{}", vendor_id, device_id), compute_ready });
    }
    out
}

fn product_name(dev: &Path) -> Option<String> {
    // amdgpu exposes product_name on newer kernels; otherwise nothing standard in sysfs.
    read_trim(&dev.join("product_name")).filter(|s| !s.is_empty())
}

fn nvidia_smi() -> Option<(String, u64)> {
    let out = Command::new("nvidia-smi").args(["--query-gpu=name,memory.total", "--format=csv,noheader,nounits"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&out.stdout).lines().next()?.to_string();
    let mut it = line.split(',').map(|s| s.trim());
    let name = it.next()?.to_string();
    let mb = it.next()?.parse::<u64>().ok()?;
    Some((name, mb))
}

pub fn ram_linux(proc_root: &Path) -> u64 {
    let Some(text) = read_trim(&proc_root.join("meminfo")) else { return 0 };
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            let kb: u64 = rest.trim().trim_end_matches("kB").trim().parse().unwrap_or(0);
            return kb / 1024;
        }
    }
    0
}

pub fn cpu_linux(proc_root: &Path) -> Cpu {
    let mut cpu = Cpu::default();
    let Some(text) = read_trim(&proc_root.join("cpuinfo")) else { return cpu };
    let mut cores = 0u32;
    for line in text.lines() {
        if line.starts_with("processor") {
            cores += 1;
        } else if cpu.model.is_empty() && line.starts_with("model name") {
            cpu.model = line.split(':').nth(1).unwrap_or("").trim().to_string();
        } else if line.starts_with("flags") && !cpu.avx2 && !cpu.avx512 {
            let flags: Vec<&str> = line.split(':').nth(1).unwrap_or("").split_whitespace().collect();
            cpu.avx2 = flags.contains(&"avx2");
            cpu.avx512 = flags.iter().any(|f| f.starts_with("avx512"));
            cpu.amx = flags.iter().any(|f| f.starts_with("amx"));
        }
    }
    cpu.cores = cores;
    cpu
}

// ---------------- macOS fallback (dev machine only) ----------------

fn detect_macos() -> Raw {
    let sysctl = |k: &str| Command::new("sysctl").args(["-n", k]).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let ram_mb = sysctl("hw.memsize").parse::<u64>().unwrap_or(0) / (1024 * 1024);
    let model = sysctl("machdep.cpu.brand_string");
    let cores = sysctl("hw.ncpu").parse::<u32>().unwrap_or(0);
    let apple = model.starts_with("Apple");
    let gpus = if apple {
        vec![Gpu { vendor: GpuVendor::Apple, name: format!("{} GPU (unified memory)", model), vram_mb: 0, driver: "metal".into(), pci_id: String::new(), compute_ready: true }]
    } else {
        vec![]
    };
    Raw { gpus, ram_mb, cpu: Cpu { model, cores, avx2: !apple, avx512: false, amx: false }, host_os: "macos".into() }
}

// ---------------- disk ----------------

#[cfg(unix)]
pub fn disk_free_mb(path: &Path) -> u64 {
    // walk up until something exists (the models dir may not exist before first boot)
    let mut p = path.to_path_buf();
    while !p.exists() {
        if !p.pop() {
            return 0;
        }
    }
    match nix::sys::statvfs::statvfs(&p) {
        Ok(s) => (s.blocks_available() as u64 * s.fragment_size() as u64) / (1024 * 1024),
        Err(_) => 0,
    }
}

#[cfg(not(unix))]
pub fn disk_free_mb(_path: &Path) -> u64 {
    0
}
