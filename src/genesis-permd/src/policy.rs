//! Policy: how tiers map to verdicts per mode, plus the rules that classify tool calls.
//! Loaded from TOML (system policy under /usr/share/genesis/policy.d, overrides under /etc/genesis/policy.d
//! and ~/.config/genesis/policy.toml). Later files override earlier ones key by key.

use crate::model::{Intent, Mode, Privilege, Session, Tier, Verdict};
use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Verdict per tier for one mode.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModeTable(pub BTreeMap<Tier, Verdict>);

/// A command-pattern rule: if the shell command matches, the call is at least `tier`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandRule {
    pub id: String,
    /// Substrings or globs matched against the whole command line (glob syntax; `*` matches anything).
    pub patterns: Vec<String>,
    pub tier: Tier,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathRules {
    /// Globs under which writes count as W1 even outside project roots (e.g. ~/Genesis/**).
    #[serde(default)]
    pub project_like: Vec<String>,
    /// Globs that are always secrets (X): reads and writes are denied to the model.
    #[serde(default)]
    pub secrets: Vec<String>,
    /// Globs that are system scope (S2) when written.
    #[serde(default)]
    pub system: Vec<String>,
    /// Globs that are user-scope system config (S1) when written.
    #[serde(default)]
    pub system_user: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkRules {
    /// Domains allowed at R1 without a prompt in Assist mode (package registries, docs).
    #[serde(default)]
    pub allow: Vec<String>,
    /// Domains that are never allowed (metadata endpoints, localhost admin ports, known exfil).
    #[serde(default)]
    pub deny: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub modes: BTreeMap<Mode, ModeTable>,
    #[serde(default)]
    pub commands: Vec<CommandRule>,
    #[serde(default)]
    pub paths: PathRules,
    #[serde(default)]
    pub network: NetworkRules,
    /// Tools that are read-only by nature.
    #[serde(default)]
    pub read_tools: Vec<String>,
    /// Tools that are project writes by nature.
    #[serde(default)]
    pub write_tools: Vec<String>,
    /// Tools whose tier is fixed regardless of arguments.
    #[serde(default)]
    pub tool_tiers: BTreeMap<String, Tier>,
}

impl Default for PathRules {
    fn default() -> Self {
        PathRules {
            project_like: vec!["~/Genesis/**".into(), "~/Projects/**".into()],
            secrets: vec![
                "~/.ssh/**".into(),
                "~/.gnupg/**".into(),
                "~/.config/genesis/keys/**".into(),
                "~/.config/genesis/settings.json".into(),
                "~/.claude/**".into(),
                "~/.aws/**".into(),
                "~/.kube/**".into(),
                "~/.netrc".into(),
                "**/.env".into(),
                "**/*.pem".into(),
                "**/id_rsa*".into(),
                "**/id_ed25519*".into(),
            ],
            system: vec![
                "/usr/**".into(),
                "/etc/**".into(),
                "/boot/**".into(),
                "/var/lib/genesis/**".into(),
                "/var/lib/flatpak/**".into(),
            ],
            system_user: vec![
                "~/.local/share/flatpak/**".into(),
                "~/.local/share/applications/**".into(),
                "~/.local/share/icons/**".into(),
                "~/.config/systemd/**".into(),
                "~/.config/genesis/**".into(),
                "~/.local/bin/**".into(),
            ],
        }
    }
}

impl Default for NetworkRules {
    fn default() -> Self {
        NetworkRules {
            allow: vec![
                "registry.npmjs.org".into(),
                "pypi.org".into(),
                "files.pythonhosted.org".into(),
                "crates.io".into(),
                "static.crates.io".into(),
                "github.com".into(),
                "objects.githubusercontent.com".into(),
                "huggingface.co".into(),
                "flathub.org".into(),
                "dl.flathub.org".into(),
                "docs.rs".into(),
            ],
            deny: vec![
                "169.254.169.254".into(),
                "metadata.google.internal".into(),
                "localhost".into(),
                "127.0.0.1".into(),
            ],
        }
    }
}

impl Policy {
    /// The built-in default policy. Shipped as /usr/share/genesis/policy.d/00-default.toml too.
    pub fn default_policy() -> Policy {
        use Tier::*;
        use Verdict::*;
        let mut modes = BTreeMap::new();
        let table = |rows: [(Tier, Verdict); 8]| ModeTable(rows.into_iter().collect());
        modes.insert(
            Mode::Assist,
            table([
                (ReadLocal, Allow),
                (ReadNetwork, Prompt),
                (WriteProject, Prompt),
                (WriteUser, Prompt),
                (SystemUser, Prompt),
                (System, Prompt),
                (Secrets, Deny),
                (Never, Deny),
            ]),
        );
        modes.insert(
            Mode::AutoEdit,
            table([
                (ReadLocal, Allow),
                (ReadNetwork, Allow),
                (WriteProject, Allow),
                (WriteUser, Prompt),
                (SystemUser, Prompt),
                (System, Prompt),
                (Secrets, Deny),
                (Never, Deny),
            ]),
        );
        modes.insert(
            Mode::Autonomous,
            table([
                (ReadLocal, Allow),
                (ReadNetwork, Allow),
                (WriteProject, Allow),
                (WriteUser, AllowWithSnapshot),
                (SystemUser, AllowWithSnapshot),
                (System, Prompt),
                (Secrets, Deny),
                (Never, Deny),
            ]),
        );

        let commands = vec![
            CommandRule {
                id: "never.rm-root".into(),
                patterns: vec!["*rm -rf /".into(), "*rm -rf / *".into(), "*rm -rf ~".into(), "*rm -rf ~/".into(), "*rm -rf ~ *".into(), "*rm -rf ~/ *".into(), "*rm -rf $HOME*".into(), "*--no-preserve-root*".into(), "*rm -rf /home*".into(), "*rm -rf /usr*".into(), "*rm -rf /etc*".into(), "*rm -rf /var*".into(), "*rm -rf /boot*".into()],
                tier: Never,
                reason: "recursive delete of home or root".into(),
            },
            CommandRule {
                id: "never.disk".into(),
                patterns: vec!["*mkfs.*".into(), "*mkfs *".into(), "*dd if=*of=/dev/*".into(), "*parted *".into(), "*sgdisk *".into(), "*wipefs *".into(), "*fdisk /dev/*".into()],
                tier: Never,
                reason: "disk or partition operation".into(),
            },
            CommandRule {
                id: "never.sandbox".into(),
                patterns: vec!["*genesis-permd*".into(), "*systemctl * stop genesis-*".into(), "*systemctl * disable genesis-*".into(), "*setenforce 0*".into()],
                tier: Never,
                reason: "would disable Genesis safety services".into(),
            },
            CommandRule {
                id: "never.mok".into(),
                patterns: vec!["*mokutil *".into(), "*efibootmgr*".into()],
                tier: Never,
                reason: "Secure Boot or firmware boot entries".into(),
            },
            CommandRule {
                id: "system.image".into(),
                patterns: vec!["*bootc *".into(), "*rpm-ostree *".into(), "*dnf * install*".into(), "*dnf5 * install*".into(), "*flatpak install --system*".into(), "*flatpak install -y*".into()],
                tier: System,
                reason: "system-wide package or image change".into(),
            },
            CommandRule {
                id: "system.config".into(),
                patterns: vec!["*nmcli *".into(), "*hostnamectl *".into(), "*timedatectl set*".into(), "*useradd *".into(), "*usermod *".into(), "*passwd *".into(), "*grubby *".into()],
                tier: System,
                reason: "system configuration".into(),
            },
            CommandRule {
                id: "system.sudo".into(),
                patterns: vec!["sudo *".into(), "*| sudo *".into(), "*&& sudo *".into(), "pkexec *".into(), "doas *".into()],
                tier: System,
                reason: "privilege escalation".into(),
            },
            CommandRule {
                id: "system_user.pip-override".into(),
                patterns: vec!["*--break-system-packages*".into()],
                tier: SystemUser,
                reason: "package install that overrides the distro's Python policy (PEP 668)".into(),
            },
            CommandRule {
                id: "system_user.global-install".into(),
                patterns: vec!["pip install*".into(), "pip3 install*".into(), "npm install -g*".into(), "npm i -g*".into(), "cargo install*".into(), "pipx install*".into(), "flatpak install*".into(), "brew install*".into(), "*systemctl --user enable*".into()],
                tier: SystemUser,
                reason: "installs into the user's environment outside the project".into(),
            },
            CommandRule {
                id: "system_user.home".into(),
                patterns: vec!["*> ~/.*".into(), "*>> ~/.*".into(), "*~/.config/*".into(), "*~/.bashrc*".into(), "*~/.zshrc*".into(), "*gsettings set*".into(), "*kwriteconfig*".into(), "*~/.profile*".into(), "*~/.bash_profile*".into(), "*~/.config/autostart/*".into(), "*~/.config/systemd/user/*".into(), "*crontab *".into(), "*~/.local/share/applications/*".into()],
                tier: SystemUser,
                reason: "changes user configuration or what runs at login".into(),
            },
            CommandRule {
                id: "system.remote-exec".into(),
                patterns: vec!["*curl *| sh*".into(), "*curl *| bash*".into(), "*curl *|sh*".into(), "*curl *|bash*".into(), "*wget *| sh*".into(), "*wget *| bash*".into(), "*wget *|sh*".into(), "*wget *|bash*".into(), "*| sudo sh*".into(), "*| sudo bash*".into(), "*bash <(curl*".into(), "*sh <(curl*".into(), "*bash <(wget*".into(), "*sh <(wget*".into()],
                tier: Tier::System,
                reason: "runs code straight from the network".into(),
            },
            CommandRule {
                id: "never.messaging".into(),
                patterns: vec!["*sendmail*".into(), "*mail -s*".into(), "*curl * -X POST *slack.com*".into()],
                tier: Never,
                reason: "sends a message on the user's behalf".into(),
            },
        ];

        Policy {
            modes,
            commands,
            paths: PathRules::default(),
            network: NetworkRules::default(),
            read_tools: vec!["fs.read".into(), "fs.list".into(), "fs.search".into(), "index.query".into(), "desktop.tree".into(), "desktop.screenshot".into(), "git.status".into(), "git.diff".into(), "git.log".into()],
            write_tools: vec!["fs.write".into(), "fs.edit".into(), "fs.mkdir".into(), "fs.delete".into(), "git.commit".into(), "git.checkout".into()],
            tool_tiers: [
                ("web.fetch".to_string(), ReadNetwork),
                ("web.search".to_string(), ReadNetwork),
                ("browser.navigate".to_string(), ReadNetwork),
                ("browser.act".to_string(), WriteUser),
                ("browser.read".to_string(), ReadLocal),
                ("desktop.input".to_string(), WriteUser),
                ("desktop.notify".to_string(), WriteUser),
                ("flatpak.install_user".to_string(), SystemUser),
                ("flatpak.install_system".to_string(), System),
                ("sysconfig.set".to_string(), System),
                ("bootc.upgrade".to_string(), System),
                ("keyring.read".to_string(), Secrets),
                ("message.send".to_string(), Never),
                ("payment".to_string(), Never),
            ]
            .into_iter()
            .collect(),
        }
    }

    /// Load the default policy and overlay every TOML file found in the given directories/files, in order.
    pub fn load(sources: &[PathBuf]) -> Result<Policy> {
        let mut policy = Policy::default_policy();
        for src in sources {
            if src.is_dir() {
                let mut files: Vec<PathBuf> = std::fs::read_dir(src)
                    .with_context(|| format!("reading policy dir {}", src.display()))?
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.extension().map(|e| e == "toml").unwrap_or(false))
                    .collect();
                files.sort();
                for f in files {
                    policy.overlay_file(&f)?;
                }
            } else if src.is_file() {
                policy.overlay_file(src)?;
            }
        }
        Ok(policy)
    }

    fn overlay_file(&mut self, path: &Path) -> Result<()> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let overlay: PolicyOverlay = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        self.apply(overlay);
        tracing::info!(file = %path.display(), "policy overlay applied");
        Ok(())
    }

    fn apply(&mut self, o: PolicyOverlay) {
        if let Some(modes) = o.modes {
            for (mode, table) in modes {
                let entry = self.modes.entry(mode).or_insert_with(|| ModeTable(BTreeMap::new()));
                for (tier, verdict) in table.0 {
                    entry.0.insert(tier, verdict);
                }
            }
        }
        if let Some(cmds) = o.commands {
            for c in cmds {
                self.commands.retain(|x| x.id != c.id);
                self.commands.push(c);
            }
        }
        if let Some(p) = o.paths {
            if let Some(v) = p.project_like { self.paths.project_like.extend(v); }
            if let Some(v) = p.secrets { self.paths.secrets.extend(v); }
            if let Some(v) = p.system { self.paths.system.extend(v); }
            if let Some(v) = p.system_user { self.paths.system_user.extend(v); }
        }
        if let Some(n) = o.network {
            if let Some(v) = n.allow { self.network.allow.extend(v); }
            if let Some(v) = n.deny { self.network.deny.extend(v); }
        }
        if let Some(t) = o.tool_tiers {
            self.tool_tiers.extend(t);
        }
    }

    pub fn verdict_for(&self, mode: Mode, tier: Tier) -> Verdict {
        self.modes
            .get(&mode)
            .and_then(|t| t.0.get(&tier).copied())
            // Unknown combination: fail closed.
            .unwrap_or(Verdict::Prompt)
    }

    pub fn compiled(&self) -> Result<CompiledPolicy> {
        let build = |globs: &[String]| -> Result<GlobSet> {
            let mut b = GlobSetBuilder::new();
            for g in globs {
                b.add(Glob::new(&expand_home(g))?);
            }
            Ok(b.build()?)
        };
        let mut cmd_sets = Vec::new();
        for rule in &self.commands {
            let mut b = GlobSetBuilder::new();
            for p in &rule.patterns {
                b.add(Glob::new(p).with_context(|| format!("rule {} pattern {}", rule.id, p))?);
            }
            cmd_sets.push((rule.clone(), b.build()?));
        }
        Ok(CompiledPolicy {
            policy: self.clone(),
            project_like: build(&self.paths.project_like)?,
            secrets: build(&self.paths.secrets)?,
            system: build(&self.paths.system)?,
            system_user: build(&self.paths.system_user)?,
            commands: cmd_sets,
        })
    }
}

/// Partial policy for overlays: every field optional.
#[derive(Debug, Default, Deserialize)]
pub struct PolicyOverlay {
    pub modes: Option<BTreeMap<Mode, ModeTable>>,
    pub commands: Option<Vec<CommandRule>>,
    pub paths: Option<PathRulesOverlay>,
    pub network: Option<NetworkRulesOverlay>,
    pub tool_tiers: Option<BTreeMap<String, Tier>>,
}
#[derive(Debug, Default, Deserialize)]
pub struct PathRulesOverlay {
    pub project_like: Option<Vec<String>>,
    pub secrets: Option<Vec<String>>,
    pub system: Option<Vec<String>>,
    pub system_user: Option<Vec<String>>,
}
#[derive(Debug, Default, Deserialize)]
pub struct NetworkRulesOverlay {
    pub allow: Option<Vec<String>>,
    pub deny: Option<Vec<String>>,
}

pub struct CompiledPolicy {
    pub policy: Policy,
    project_like: GlobSet,
    secrets: GlobSet,
    system: GlobSet,
    system_user: GlobSet,
    commands: Vec<(CommandRule, GlobSet)>,
}

/// Every spelling of home, then the real path when it exists (symlinks resolved).
pub fn canonical_path(p: &str) -> String {
    let mut s = expand_home(p);
    if let Some(home) = home_dir() {
        for v in ["${HOME}", "$HOME"] {
            s = s.replace(v, &home);
        }
    }
    match std::fs::canonicalize(&s) {
        Ok(c) => c.display().to_string(),
        Err(_) => {
            // the file may not exist yet: canonicalise the deepest existing parent
            let path = std::path::Path::new(&s);
            if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
                if let Ok(c) = std::fs::canonicalize(parent) { return c.join(name).display().to_string(); }
            }
            s
        }
    }
}

pub fn expand_home(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return format!("{}/{}", home, rest);
        }
    } else if p == "~" {
        if let Some(home) = home_dir() {
            return home;
        }
    }
    p.to_string()
}

pub fn home_dir() -> Option<String> {
    std::env::var("HOME").ok().filter(|h| !h.is_empty())
}

/// Result of classifying one intent.
#[derive(Debug, Clone)]
pub struct Classification {
    pub tier: Tier,
    pub rule: Option<String>,
    pub reason: String,
}

impl CompiledPolicy {
    /// Map an intent to the highest tier any of its aspects reaches.
    pub fn classify(&self, intent: &Intent, session: &Session) -> Classification {
        let mut best = Classification {
            tier: Tier::ReadLocal,
            rule: None,
            reason: "read-only".into(),
        };
        let mut raise = |tier: Tier, rule: Option<String>, reason: String| {
            if tier > best.tier {
                best = Classification { tier, rule, reason };
            }
        };

        // 1. explicit flags from the caller
        if intent.irreversible {
            raise(Tier::Never, Some("flag.irreversible".into()), "caller marked the action irreversible".into());
        }
        if intent.touches_secrets {
            raise(Tier::Secrets, Some("flag.secrets".into()), "touches secrets".into());
        }
        if intent.privilege == Privilege::Root {
            raise(Tier::System, Some("flag.root".into()), "requests root privilege".into());
        }

        // 2. tool-level tiers
        if let Some(t) = self.policy.tool_tiers.get(&intent.tool) {
            raise(*t, Some(format!("tool.{}", intent.tool)), format!("tool {} is {}", intent.tool, t));
        } else if self.policy.write_tools.iter().any(|t| t == &intent.tool) {
            raise(Tier::WriteProject, Some(format!("tool.{}", intent.tool)), "write tool".into());
        }

        // 3. paths: secrets first, then system, then user scope, then project. Matched on the canonical
        // path (home expanded in every spelling, symlinks resolved), so a link inside the project that
        // points at a key is still a key.
        for p in intent.reads.iter().chain(intent.writes.iter()) {
            let path = canonical_path(p);
            if self.secrets.is_match(&path) || (path != expand_home(p) && self.secrets.is_match(expand_home(p))) {
                raise(Tier::Secrets, Some("path.secrets".into()), format!("{} is a secret", p));
            }
        }
        for p in &intent.writes {
            let path = canonical_path(p);
            if self.system.is_match(&path) {
                raise(Tier::System, Some("path.system".into()), format!("writes system path {}", p));
            } else if self.system_user.is_match(&path) {
                raise(Tier::SystemUser, Some("path.system_user".into()), format!("writes user-scope system path {}", p));
            } else if in_roots(&path, &session.project_roots) || self.project_like.is_match(&path) {
                raise(Tier::WriteProject, Some("path.project".into()), format!("writes inside project: {}", p));
            } else {
                raise(Tier::WriteUser, Some("path.user".into()), format!("writes outside the project: {}", p));
            }
        }

        // 4. network
        for d in &intent.network.domains {
            if self.policy.network.deny.iter().any(|x| x == d) {
                raise(Tier::Never, Some("net.deny".into()), format!("domain {} is denied", d));
            } else {
                raise(Tier::ReadNetwork, Some("net".into()), format!("network access to {}", d));
            }
        }
        // a shell that asked for network ("*") cannot be filtered by domain, but a command that names a
        // denied host (cloud metadata, loopback services) is refused outright
        if intent.network.domains.iter().any(|d| d == "*") {
            if let Some(cmd) = &intent.command {
                let lower = cmd.to_lowercase();
                if let Some(hit) = self.policy.network.deny.iter().find(|x| lower.contains(&x.to_lowercase())) {
                    raise(Tier::Never, Some("net.deny".into()), format!("command reaches denied host {}", hit));
                }
            }
        }

        // 5. shell commands: rules, then heuristics
        if let Some(cmd) = &intent.command {
            let cmd = cmd.trim();
            // any path-like word that names a secret (keys, tokens, wallets) makes this a secrets action,
            // whether it is read, copied or encoded
            for tok in cmd.split(|c: char| c.is_whitespace() || c == '\'' || c == '"' || c == '=' || c == ',') {
                let t = tok.trim_matches(|c| c == '(' || c == ')' || c == ';' || c == '|' || c == '&');
                if t.starts_with('/') || t.starts_with("~/") || t.starts_with("$HOME/") {
                    let path = expand_home(&t.replace("$HOME", "~"));
                    if self.secrets.is_match(&path) {
                        raise(Tier::Secrets, Some("shell.secrets".into()), format!("{} is a secret", t));
                    }
                }
            }
            let mut matched = false;
            for (rule, set) in &self.commands {
                if set.is_match(cmd) {
                    // Recursive deletes whose every path argument lies inside the project are ordinary
                    // project work (cleaning build output), not the "wipe home" case the rule exists for.
                    if rule.id == "never.rm-root" && rm_targets_inside_roots(cmd, &session.project_roots) {
                        continue;
                    }
                    matched = true;
                    raise(rule.tier, Some(rule.id.clone()), rule.reason.clone());
                }
            }
            if !matched && intent.writes.is_empty() {
                // A shell command with no declared paths. Paths named in a write context that lie outside the
                // project are W2; read-only classics without redirects stay R0; everything else may write
                // inside the project (W1).
                let outside: Vec<String> = command_write_paths(cmd).into_iter().filter(|p| !in_roots(&expand_home(p), &session.project_roots)).collect();
                if let Some(p) = outside.first() {
                    if self.system.is_match(expand_home(p)) {
                        raise(Tier::System, Some("shell.system-path".into()), format!("shell command writes system path {}", p));
                    } else {
                        raise(Tier::WriteUser, Some("shell.outside-project".into()), format!("shell command writes outside the project: {}", p));
                    }
                } else {
                    let first = cmd.split_whitespace().next().unwrap_or("");
                    let has_redirect = cmd.contains('>') || cmd.contains("| tee");
                    let readonly = !has_redirect && matches!(first, "ls" | "cat" | "head" | "tail" | "grep" | "rg" | "find" | "ps" | "pwd" | "echo" | "printf" | "wc" | "sort" | "uniq" | "which" | "env" | "date" | "uname" | "df" | "du" | "stat" | "file" | "less" | "git");
                    let git_ro = first == "git" && cmd.split_whitespace().nth(1).map(|s| matches!(s, "status" | "diff" | "log" | "show" | "branch")).unwrap_or(false);
                    if !(readonly && (first != "git" || git_ro)) {
                        raise(Tier::WriteProject, Some("shell.default".into()), "shell command may modify the project".into());
                    }
                }
            }
        }

        best
    }
}

fn in_roots(path: &str, roots: &[String]) -> bool {
    roots.iter().any(|r| {
        let r = expand_home(r);
        let r = r.trim_end_matches('/');
        path == r || path.starts_with(&format!("{}/", r))
    })
}

/// For an `rm` command line: true when there is at least one path argument and every path argument
/// (absolute or ~-relative) resolves inside one of the project roots. Relative paths are treated as
/// inside the project (the shell tool runs with the project as its working directory).
fn rm_targets_inside_roots(cmd: &str, roots: &[String]) -> bool {
    if roots.is_empty() {
        return false;
    }
    let mut paths = 0usize;
    for tok in cmd.split_whitespace().skip(1) {
        if tok.starts_with('-') {
            continue;
        }
        if tok == "&&" || tok == "||" || tok == ";" || tok == "|" {
            break;
        }
        paths += 1;
        let t = tok.trim_matches(|c| c == '"' || c == '\'');
        let expanded = expand_home(t);
        let absolute = expanded.starts_with('/');
        if absolute && !in_roots(&expanded, roots) {
            return false;
        }
        if t == "~" || t == "~/" || t == "$HOME" || t.starts_with("$HOME/") && t.matches('/').count() <= 1 {
            return false;
        }
    }
    paths > 0
}

/// Best-effort extraction of paths a shell command line writes to: redirect targets and the path
/// arguments of common mutating commands. Only absolute and `~` paths are returned; relative paths
/// resolve inside the project (the tool's working directory) and are ordinary project work.
pub fn command_write_paths(cmd: &str) -> Vec<String> {
    let mutating = ["cp", "mv", "rm", "mkdir", "touch", "chmod", "chown", "install", "tee", "ln", "rsync", "truncate", "dd"];
    let mut out = Vec::new();
    let is_path = |t: &str| t.starts_with('/') || t.starts_with("~/") || t == "~" || t.starts_with("$HOME");
    let clean = |t: &str| t.trim_matches(|c| c == '"' || c == '\'' || c == ';' || c == ')' || c == '(').to_string();
    // split into simple commands on ; && || |
    for seg in cmd.split(|c| c == ';' || c == '|').map(|s| s.trim()).filter(|s| !s.is_empty()) {
        let seg = seg.trim_start_matches('&').trim();
        let toks: Vec<String> = seg.split_whitespace().map(|t| t.to_string()).collect();
        // redirects: > path, >> path, >path
        for (i, t) in toks.iter().enumerate() {
            if t == ">" || t == ">>" || t == "1>" || t == "2>" || t == "&>" {
                if let Some(n) = toks.get(i + 1) { let n = clean(n); if is_path(&n) { out.push(n); } }
            } else if let Some(rest) = t.strip_prefix(">>").or_else(|| t.strip_prefix('>')) {
                let n = clean(rest); if is_path(&n) { out.push(n); }
            }
        }
        // sed -i FILE..., and mutating commands: every path-looking non-flag argument after the command
        let first = toks.first().map(|s| s.as_str()).unwrap_or("");
        let first = if first == "sudo" { toks.get(1).map(|s| s.as_str()).unwrap_or("") } else { first };
        if first == "sed" && toks.iter().any(|t| t == "-i" || t.starts_with("-i")) {
            for t in toks.iter().skip(1) { let n = clean(t); if is_path(&n) { out.push(n); } }
        } else if mutating.contains(&first) {
            for t in toks.iter().skip(1) { let n = clean(t); if !n.starts_with('-') && is_path(&n) { out.push(n); } }
        }
    }
    out.sort();
    out.dedup();
    out
}
