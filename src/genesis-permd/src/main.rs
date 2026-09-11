//! genesis-permd: the Genesis permission broker.
//!
//!   genesis-permd serve                      run the D-Bus service (Linux)
//!   genesis-permd check --cmd "..."          classify one shell command from the CLI
//!   genesis-permd check --intent intent.json evaluate a full intent envelope
//!   genesis-permd policy dump                print the effective policy as TOML
//!   genesis-permd audit verify               verify the audit chain

mod audit;
mod broker;
mod dbus;
mod model;
mod policy;

use anyhow::Result;
use broker::Broker;
use clap::{Parser, Subcommand};
use model::{Intent, Mode};
use policy::Policy;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "genesis-permd", version, about = "Genesis permission broker")]
struct Cli {
    /// Extra policy files or directories, applied after the system defaults.
    #[arg(long, global = true)]
    policy: Vec<PathBuf>,
    /// Audit log path (default: $GENESIS_AUDIT or ~/.local/state/genesis/audit.jsonl).
    #[arg(long, global = true)]
    audit: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the D-Bus service on the session bus.
    Serve,
    /// Evaluate one tool call from the command line.
    Check {
        #[arg(long, default_value = "assist")]
        mode: String,
        #[arg(long)]
        cmd: Option<String>,
        #[arg(long)]
        intent: Option<PathBuf>,
        /// Project roots the session may write to.
        #[arg(long)]
        root: Vec<String>,
        /// Pretend the session has read untrusted content.
        #[arg(long)]
        tainted: bool,
        #[arg(long)]
        json: bool,
    },
    /// Policy inspection.
    Policy {
        #[command(subcommand)]
        cmd: PolicyCmd,
    },
    /// Audit log tools.
    Audit {
        #[command(subcommand)]
        cmd: AuditCmd,
    },
}

#[derive(Subcommand)]
enum PolicyCmd {
    /// Print the effective policy as TOML.
    Dump,
    /// Print the tier -> verdict table for every mode.
    Table,
}

#[derive(Subcommand)]
enum AuditCmd {
    /// Verify the hash chain of the audit log.
    Verify,
}

fn default_policy_sources() -> Vec<PathBuf> {
    let mut v = vec![
        PathBuf::from("/usr/share/genesis/policy.d"),
        PathBuf::from("/etc/genesis/policy.d"),
    ];
    if let Some(home) = policy::home_dir() {
        v.push(PathBuf::from(format!("{}/.config/genesis/policy.toml", home)));
    }
    v
}

fn default_audit_path() -> PathBuf {
    if let Ok(p) = std::env::var("GENESIS_AUDIT") {
        return PathBuf::from(p);
    }
    let home = policy::home_dir().unwrap_or_else(|| "/tmp".into());
    PathBuf::from(format!("{}/.local/state/genesis/audit.jsonl", home))
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive("info".parse()?))
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();
    let mut sources = default_policy_sources();
    sources.extend(cli.policy.iter().cloned());
    let policy = Policy::load(&sources)?;
    let audit_path = cli.audit.clone().unwrap_or_else(default_audit_path);

    match cli.cmd {
        Cmd::Serve => serve(policy, audit_path),
        Cmd::Check { mode, cmd, intent, root, tainted, json } => {
            let mode: Mode = serde_json::from_value(serde_json::Value::String(mode))?;
            let mut broker = Broker::new(policy, &audit_path)?;
            let roots = if root.is_empty() { vec![std::env::current_dir()?.display().to_string()] } else { root };
            broker.open_session("cli", mode, roots, "cli")?;
            if tainted {
                broker.mark_tainted("cli", "cli --tainted")?;
            }
            let intent = match (cmd, intent) {
                (Some(c), _) => Intent { session_id: "cli".into(), origin: "cli".into(), tool: "shell".into(), command: Some(c), ..Default::default() },
                (None, Some(p)) => {
                    let mut i: Intent = serde_json::from_str(&std::fs::read_to_string(p)?)?;
                    i.session_id = "cli".into();
                    i
                }
                (None, None) => anyhow::bail!("give --cmd or --intent"),
            };
            let d = broker.evaluate(&intent)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&d)?);
            } else {
                println!("{:<8} {:<20} {}", d.tier.code(), format!("{:?}", d.verdict).to_lowercase(), d.reason);
                if let Some(r) = &d.rule {
                    println!("rule     {}", r);
                }
                println!("audit    {}", audit_path.display());
            }
            Ok(())
        }
        Cmd::Policy { cmd } => match cmd {
            PolicyCmd::Dump => {
                println!("{}", toml::to_string_pretty(&policy)?);
                Ok(())
            }
            PolicyCmd::Table => {
                println!("{:<12} {:<20} {:<20} {:<20}", "tier", "assist", "auto_edit", "autonomous");
                for t in model::Tier::all() {
                    let v = |m| format!("{:?}", policy.verdict_for(m, t)).to_lowercase();
                    println!("{:<12} {:<20} {:<20} {:<20}", format!("{} {:?}", t.code(), t), v(Mode::Assist), v(Mode::AutoEdit), v(Mode::Autonomous));
                }
                Ok(())
            }
        },
        Cmd::Audit { cmd } => match cmd {
            AuditCmd::Verify => {
                let n = audit::AuditLog::verify(&audit_path)?;
                println!("ok: {} entries, chain intact ({})", n, audit_path.display());
                Ok(())
            }
        },
    }
}

#[cfg(target_os = "linux")]
fn serve(policy: Policy, audit_path: PathBuf) -> Result<()> {
    use std::sync::{Arc, Mutex};
    let broker = Arc::new(Mutex::new(Broker::new(policy, &audit_path)?));
    tracing::info!(audit = %audit_path.display(), "genesis-permd starting");
    tokio::runtime::Runtime::new()?.block_on(dbus::serve(broker))
}

#[cfg(not(target_os = "linux"))]
fn serve(_policy: Policy, _audit_path: PathBuf) -> Result<()> {
    anyhow::bail!("the D-Bus service runs on Linux only; use `check` here")
}
