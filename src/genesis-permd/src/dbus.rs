//! D-Bus service `org.genesis.Permission1` on the session bus (Linux only).
//! All payloads are JSON strings so the interface stays stable while the Rust types evolve.

#![cfg(target_os = "linux")]

use crate::broker::Broker;
use crate::model::{Intent, Mode};
use anyhow::Result;
use std::sync::{Arc, Mutex};
use zbus::{connection, interface};

pub struct PermissionService {
    pub broker: Arc<Mutex<Broker>>,
}

fn err(e: impl std::fmt::Display) -> zbus::fdo::Error {
    zbus::fdo::Error::Failed(e.to_string())
}

#[interface(name = "org.genesis.Permission1")]
impl PermissionService {
    /// Open a session. mode: assist | auto_edit | autonomous. project_roots: JSON array of paths.
    fn open_session(&self, id: &str, mode: &str, project_roots: &str, origin: &str) -> zbus::fdo::Result<String> {
        let mode: Mode = serde_json::from_value(serde_json::Value::String(mode.into())).map_err(err)?;
        let roots: Vec<String> = serde_json::from_str(project_roots).map_err(err)?;
        let s = self.broker.lock().unwrap().open_session(id, mode, roots, origin).map_err(err)?;
        serde_json::to_string(&s).map_err(err)
    }

    fn set_mode(&self, id: &str, mode: &str) -> zbus::fdo::Result<()> {
        let mode: Mode = serde_json::from_value(serde_json::Value::String(mode.into())).map_err(err)?;
        self.broker.lock().unwrap().set_mode(id, mode).map_err(err)
    }

    fn mark_tainted(&self, id: &str, source: &str) -> zbus::fdo::Result<()> {
        self.broker.lock().unwrap().mark_tainted(id, source).map_err(err)
    }

    /// Evaluate an intent (JSON, see model::Intent). Returns a Decision as JSON.
    fn evaluate(&self, intent_json: &str) -> zbus::fdo::Result<String> {
        let intent: Intent = serde_json::from_str(intent_json).map_err(err)?;
        let d = self.broker.lock().unwrap().evaluate(&intent).map_err(err)?;
        serde_json::to_string(&d).map_err(err)
    }

    /// A UI answers a pending prompt. Returns the final verdict as a string.
    fn resolve(&self, request_id: &str, allowed: bool, by: &str) -> zbus::fdo::Result<String> {
        let v = self.broker.lock().unwrap().resolve(request_id, allowed, by).map_err(err)?;
        Ok(format!("{:?}", v).to_lowercase())
    }

    /// Pending prompts as a JSON array of Decisions.
    fn pending(&self) -> zbus::fdo::Result<String> {
        let p = self.broker.lock().unwrap().pending();
        serde_json::to_string(&p).map_err(err)
    }

    #[zbus(property)]
    fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }
}

pub async fn serve(broker: Arc<Mutex<Broker>>) -> Result<()> {
    let _conn = connection::Builder::session()?
        .name("org.genesis.Permission1")?
        .serve_at("/org/genesis/Permission1", PermissionService { broker })?
        .build()
        .await?;
    tracing::info!("org.genesis.Permission1 on the session bus at /org/genesis/Permission1");
    tokio::signal::ctrl_c().await?;
    Ok(())
}
