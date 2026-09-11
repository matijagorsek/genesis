//! genesis-txd CLI.
//!
//!   genesis-txd list                      transactions, newest first
//!   genesis-txd show <tx>                 what a transaction touched and whether it can be undone
//!   genesis-txd undo <tx>                 roll back
//!   genesis-txd begin --session S         open a transaction (for scripts)
//!   genesis-txd pre <tx> PATH...          save pre-images before writing paths
//!   genesis-txd commit <tx>

use anyhow::Result;
use clap::{Parser, Subcommand};
use genesis_txd::{default_store, Status, Store};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "genesis-txd", version, about = "Genesis transactions and undo")]
struct Cli {
    /// Store directory (default: /var/lib/genesis/tx as root, else ~/.local/state/genesis/tx; env GENESIS_TX_STORE)
    #[arg(long, global = true)]
    store: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    List,
    Show { tx: String },
    Undo { tx: String },
    Begin {
        #[arg(long)]
        session: String,
        #[arg(long, default_value = "cli")]
        origin: String,
    },
    Pre { tx: String, paths: Vec<PathBuf> },
    Commit { tx: String },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive("info".parse()?)).with_writer(std::io::stderr).init();
    let cli = Cli::parse();
    let store = Store::open(cli.store.clone().unwrap_or_else(default_store))?;
    match cli.cmd {
        Cmd::List => {
            let all = store.list()?;
            if all.is_empty() {
                println!("no transactions in {}", store.root.display());
            }
            for t in all {
                println!("{:<12} {:<11} {:<24} session {}  {} entr{}", t.id, format!("{:?}", t.status).to_lowercase(), t.started_at, t.session_id, t.entries.len(), if t.entries.len() == 1 { "y" } else { "ies" });
            }
        }
        Cmd::Show { tx } => {
            let t = store.load(&tx)?;
            for l in store.summary(&t) {
                println!("{}", l);
            }
            if t.status == Status::Committed || t.status == Status::Open {
                println!("undo: genesis-txd undo {}", t.id);
            }
        }
        Cmd::Undo { tx } => {
            let mut t = store.load(&tx)?;
            for l in store.rollback(&mut t)? {
                println!("{}", l);
            }
            println!("{} rolled back", t.id);
        }
        Cmd::Begin { session, origin } => {
            let t = store.begin(&session, &origin)?;
            println!("{}", t.id);
        }
        Cmd::Pre { tx, paths } => {
            let mut t = store.load(&tx)?;
            for p in paths {
                store.pre_write(&mut t, &p)?;
                println!("saved pre-image: {}", p.display());
            }
        }
        Cmd::Commit { tx } => {
            let mut t = store.load(&tx)?;
            store.commit(&mut t)?;
            println!("{} committed ({} entries)", t.id, t.entries.len());
        }
    }
    Ok(())
}
