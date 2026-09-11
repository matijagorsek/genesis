//! Hardware profile and pack definitions.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GpuVendor {
    Nvidia,
    Amd,
    Intel,
    Apple,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Gpu {
    pub vendor: GpuVendor,
    pub name: String,
    /// Dedicated video memory in MB; 0 when unknown or shared.
    pub vram_mb: u64,
    /// Kernel driver bound to the device (nvidia, amdgpu, i915, xe, nouveau, ...).
    pub driver: String,
    /// PCI vendor:device ids, e.g. "10de:2684".
    pub pci_id: String,
    /// True when a compute stack is usable: nvidia proprietary/open driver, or amdgpu with /dev/kfd.
    pub compute_ready: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Cpu {
    pub model: String,
    pub cores: u32,
    pub avx2: bool,
    pub avx512: bool,
    pub amx: bool,
}

/// The profile written to /etc/genesis/profile.json.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    pub version: u32,
    pub probed_at: String,
    pub host_os: String,
    pub gpus: Vec<Gpu>,
    pub ram_mb: u64,
    pub cpu: Cpu,
    /// Free space on the models volume (/var/lib/genesis or the given path) in MB.
    pub disk_free_mb: u64,
    /// Unified memory (Apple Silicon, some APUs): GPU shares system RAM.
    pub unified_memory: bool,
    /// Memory budget for resident models in MB: best VRAM, or 60% of RAM for CPU/unified.
    pub budget_mb: u64,
    /// Which class of accelerator the budget refers to.
    pub budget_kind: BudgetKind,
    pub recommended_pack: Option<String>,
    /// Every shipped pack, with whether it fits and why.
    pub packs: Vec<PackFit>,
    /// Anything the probe wants the first-run wizard to tell the user.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetKind {
    Vram,
    Unified,
    Ram,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PackFit {
    pub id: String,
    pub description: String,
    pub disk_gb: f64,
    pub fits: bool,
    pub reason: String,
    /// Largest required model in the pack, in MB.
    pub largest_model_mb: u64,
    /// How specifically the trigger matched this machine (vendor +2, unified-memory flag +1).
    pub specificity: u8,
}

// ---- pack files (packs/*.json in the repo, /usr/share/genesis/packs in the image) ----

#[derive(Debug, Clone, Deserialize)]
pub struct PackFile {
    pub id: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub trigger: Trigger,
    #[serde(default)]
    pub disk_gb: f64,
    #[serde(default)]
    pub models: Vec<PackModel>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Trigger {
    #[serde(default)]
    pub min_vram_gb: Option<f64>,
    #[serde(default)]
    pub min_ram_gb: Option<f64>,
    #[serde(default)]
    pub unified_memory: Option<bool>,
    /// "any" | "none" | "nvidia" | "amd" | "intel" | "apple"
    #[serde(default)]
    pub gpu: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PackModel {
    pub role: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub size_gb: f64,
    /// Optional models do not gate whether the pack fits.
    #[serde(default)]
    pub optional: bool,
    /// Offload models are expected to exceed the budget and run with layers on the CPU; they do not gate the fit.
    #[serde(default)]
    pub offload: bool,
}
