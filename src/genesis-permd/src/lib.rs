//! genesis-permd as a library: the broker, policy and audit chain, used in-process by genesis-agentd.
pub mod audit;
pub mod broker;
pub mod model;
pub mod policy;

pub use broker::Broker;
pub use model::{Decision, Intent, Mode, Network, Privilege, Session, Tier, Verdict};
pub use policy::{command_write_paths, Policy};

/// Default policy sources: system defaults, system overrides, user override.
pub fn default_policy_sources() -> Vec<std::path::PathBuf> {
    let mut v = vec![std::path::PathBuf::from("/usr/share/genesis/policy.d"), std::path::PathBuf::from("/etc/genesis/policy.d")];
    if let Some(home) = policy::home_dir() {
        v.push(std::path::PathBuf::from(format!("{}/.config/genesis/policy.toml", home)));
    }
    v
}

/// Default audit path: $GENESIS_AUDIT or ~/.local/state/genesis/audit.jsonl.
pub fn default_audit_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("GENESIS_AUDIT") {
        return std::path::PathBuf::from(p);
    }
    let home = policy::home_dir().unwrap_or_else(|| "/tmp".into());
    std::path::PathBuf::from(format!("{}/.local/state/genesis/audit.jsonl", home))
}
