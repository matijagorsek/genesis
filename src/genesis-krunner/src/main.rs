//! genesis-krunner: a KRunner D-Bus runner (org.kde.krunner1).
//! Registered via /usr/share/krunner/dbusplugins/genesis.desktop; activated on demand by D-Bus.
//! Matches "ask <text>", "genesis <text>" or "g <text>" and opens the workspace with that prompt.

#[cfg(target_os = "linux")]
mod runner {
    use anyhow::Result;
    use std::collections::HashMap;
    use zbus::{connection, interface, zvariant::{OwnedValue, Type, Value}};
    use serde::{Deserialize, Serialize};

    pub const WORKSPACE_URL: &str = "http://127.0.0.1:11520/";

    /// KRunner match: (id, text, icon, type, relevance, properties). type 100 = ExactMatch, 70 = PossibleMatch.
    #[derive(Debug, Clone, Serialize, Deserialize, Type)]
    pub struct Match(String, String, String, i32, f64, HashMap<String, OwnedValue>);

    /// KRunner action: (id, text, icon)
    #[derive(Debug, Clone, Serialize, Deserialize, Type)]
    pub struct Action(String, String, String);

    pub struct GenesisRunner;

    fn strip_prefix(q: &str) -> Option<String> {
        let q = q.trim();
        for p in ["ask ", "genesis ", "g "] {
            if let Some(rest) = q.strip_prefix(p) {
                let r = rest.trim();
                if !r.is_empty() { return Some(r.to_string()); }
            }
        }
        None
    }

    #[interface(name = "org.kde.krunner1")]
    impl GenesisRunner {
        #[zbus(name = "Match")]
        fn match_query(&self, query: &str) -> Vec<Match> {
            let Some(text) = strip_prefix(query) else { return vec![] };
            let mut props = HashMap::new();
            props.insert("subtext".to_string(), Value::from("Runs on this machine · opens the Genesis workspace").try_into().unwrap());
            vec![Match(text.clone(), format!("Ask Genesis: {}", text), "genesis".into(), 100, 0.9, props)]
        }

        #[zbus(name = "Actions")]
        fn actions(&self) -> Vec<Action> {
            vec![]
        }

        #[zbus(name = "Run")]
        fn run(&self, match_id: &str, _action_id: &str) {
            let url = format!("{}?prompt={}", WORKSPACE_URL, urlencode(match_id));
            let _ = std::process::Command::new("xdg-open").arg(url).spawn();
        }
    }

    fn urlencode(s: &str) -> String {
        let mut out = String::new();
        for b in s.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
                b' ' => out.push('+'),
                _ => out.push_str(&format!("%{:02X}", b)),
            }
        }
        out
    }

    pub async fn serve() -> Result<()> {
        let _conn = connection::Builder::session()?
            .name("org.genesis.Runner")?
            .serve_at("/runner", GenesisRunner)?
            .build()
            .await?;
        tracing::info!("org.genesis.Runner on the session bus at /runner");
        // exit after a while idle; D-Bus activation restarts us on the next query
        tokio::time::sleep(std::time::Duration::from_secs(600)).await;
        Ok(())
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_writer(std::io::stderr).init();
    if std::env::args().any(|a| a == "--version") {
        println!("genesis-krunner {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        tokio::runtime::Runtime::new()?.block_on(runner::serve())
    }
    #[cfg(not(target_os = "linux"))]
    {
        anyhow::bail!("KRunner integration is Linux only")
    }
}
