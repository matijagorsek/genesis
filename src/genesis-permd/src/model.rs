//! Core types: tiers, session modes, the intent envelope a tool call is described with,
//! and the decision the broker returns.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Permission tiers, ordered from least to most consequential.
/// Mirrors the table in docs/genesis-brief.html (Safety and permissions).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// R0: read inside project or allowed dirs, list windows, read the accessibility tree, query the index.
    ReadLocal,
    /// R1: fetch a URL, web search.
    ReadNetwork,
    /// W1: edit files inside the project, run tests, git commit.
    WriteProject,
    /// W2: write anywhere in $HOME, change dconf, send input to the desktop, act in the user's browser profile.
    WriteUser,
    /// S1: user-scope system changes: flatpak --user, systemd --user, package managers in toolboxes,
    /// and package installs that override policy (the Phase 0 `pip --break-system-packages` case).
    SystemUser,
    /// S2: system-wide: bootc / rpm-ostree, flatpak system, NetworkManager, users and groups, kernel params.
    System,
    /// X: secrets. The model never sees them; the broker injects at execution time only.
    Secrets,
    /// N: irreversible or out of policy: rm -rf outside the project, disk operations, sending messages,
    /// purchases, disabling the sandbox. Never automatic.
    Never,
}

impl Tier {
    pub fn code(self) -> &'static str {
        match self {
            Tier::ReadLocal => "R0",
            Tier::ReadNetwork => "R1",
            Tier::WriteProject => "W1",
            Tier::WriteUser => "W2",
            Tier::SystemUser => "S1",
            Tier::System => "S2",
            Tier::Secrets => "X",
            Tier::Never => "N",
        }
    }
    pub fn all() -> [Tier; 8] {
        [
            Tier::ReadLocal,
            Tier::ReadNetwork,
            Tier::WriteProject,
            Tier::WriteUser,
            Tier::SystemUser,
            Tier::System,
            Tier::Secrets,
            Tier::Never,
        ]
    }
}

impl fmt::Display for Tier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.code())
    }
}

/// Session modes, from most to least cautious. Mirrors the ask / accept-edits / bypass ladder common to coding agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Read-only by default; every write is a prompt. The launcher's default.
    #[default]
    Assist,
    /// Project writes are allowed without asking; user and system scope still prompt.
    AutoEdit,
    /// Long tasks: user and user-scope system changes are allowed after a snapshot; system-wide still prompts.
    Autonomous,
}

/// What the broker decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Allow,
    /// Allowed, but the transaction layer must snapshot first (Autonomous mode, W2/S1).
    AllowWithSnapshot,
    Prompt,
    Deny,
}

/// Network access requested by a tool call.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Network {
    #[serde(default)]
    pub domains: Vec<String>,
}

/// Privilege the tool call wants to run with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Privilege {
    #[default]
    User,
    Root,
}

/// The intent envelope. Every tool call the agent wants to make is described with one of these
/// before it runs. Built by genesis-agentd from the MCP tool name and arguments.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Intent {
    /// Session this call belongs to.
    pub session_id: String,
    /// Which app or surface originated the session (launcher, terminal, editor, browser).
    #[serde(default)]
    pub origin: String,
    /// Tool name, e.g. "shell", "fs.write", "fs.read", "browser.navigate", "flatpak.install", "sysconfig.set".
    pub tool: String,
    /// Shell command line, when the tool is a shell.
    #[serde(default)]
    pub command: Option<String>,
    /// Filesystem paths the call reads.
    #[serde(default)]
    pub reads: Vec<String>,
    /// Filesystem paths the call writes, creates or deletes.
    #[serde(default)]
    pub writes: Vec<String>,
    #[serde(default)]
    pub network: Network,
    #[serde(default)]
    pub privilege: Privilege,
    /// Set by the caller when the call would touch secrets (keyring, tokens, ssh keys).
    #[serde(default)]
    pub touches_secrets: bool,
    /// Set by the caller when the action cannot be undone (send message, purchase, wipe).
    #[serde(default)]
    pub irreversible: bool,
}

/// The broker's answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub request_id: String,
    pub session_id: String,
    pub tier: Tier,
    pub verdict: Verdict,
    pub mode: Mode,
    /// True when the session has ingested untrusted content and the verdict was raised because of it.
    pub tainted: bool,
    /// Human-readable reason, shown in the prompt dialog and the audit log.
    pub reason: String,
    /// Id of the policy rule that produced the tier, when one matched.
    pub rule: Option<String>,
}

/// Runtime state of one agent session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub mode: Mode,
    /// Project roots the session may write to at W1.
    pub project_roots: Vec<String>,
    /// Once the session has read untrusted content (web page, downloaded file), W2 and above prompt regardless of mode.
    pub tainted: bool,
    /// Once the session has read the user's own documents (outside the project), going online asks
    /// first: what was read must not leave the machine on the word of a hostile file.
    #[serde(default)]
    pub holds_private: bool,
}

impl Session {
    pub fn new(id: impl Into<String>, mode: Mode, project_roots: Vec<String>) -> Self {
        Session {
            id: id.into(),
            mode,
            project_roots,
            tainted: false,
            holds_private: false,
        }
    }
}
