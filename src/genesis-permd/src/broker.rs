//! The broker: sessions, classification, verdicts, prompts awaiting a human answer, and the audit trail.

use crate::audit::{now, AuditEntry, AuditLog};
use crate::model::{Decision, Intent, Mode, Session, Tier, Verdict};
use crate::policy::{CompiledPolicy, Policy};
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::path::Path;

pub struct Broker {
    policy: CompiledPolicy,
    sessions: HashMap<String, Session>,
    /// Decisions with verdict Prompt that a UI has not answered yet.
    pending: HashMap<String, Decision>,
    audit: AuditLog,
}

// Several methods are only called from the Linux D-Bus service.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
impl Broker {
    pub fn new(policy: Policy, audit_path: impl AsRef<Path>) -> Result<Broker> {
        Ok(Broker {
            policy: policy.compiled()?,
            sessions: HashMap::new(),
            pending: HashMap::new(),
            audit: AuditLog::open(audit_path)?,
        })
    }

    pub fn audit_path(&self) -> &Path {
        self.audit.path()
    }

    pub fn open_session(&mut self, id: &str, mode: Mode, project_roots: Vec<String>, origin: &str) -> Result<Session> {
        let s = Session::new(id, mode, project_roots.clone());
        self.sessions.insert(id.to_string(), s.clone());
        self.audit.append(AuditEntry {
            ts: now(),
            kind: "session.open".into(),
            session_id: id.into(),
            request_id: String::new(),
            tool: String::new(),
            tier: String::new(),
            verdict: String::new(),
            reason: format!("mode {:?}, origin {}", mode, origin),
            detail: serde_json::json!({ "project_roots": project_roots, "origin": origin }),
            prev: String::new(),
        })?;
        Ok(s)
    }

    pub fn set_mode(&mut self, id: &str, mode: Mode) -> Result<()> {
        let s = self.sessions.get_mut(id).ok_or_else(|| anyhow!("unknown session {}", id))?;
        s.mode = mode;
        self.audit.append(AuditEntry {
            ts: now(),
            kind: "session.mode".into(),
            session_id: id.into(),
            request_id: String::new(),
            tool: String::new(),
            tier: String::new(),
            verdict: String::new(),
            reason: format!("{:?}", mode),
            detail: serde_json::Value::Null,
            prev: String::new(),
        })
    }

    /// Mark the session as having read untrusted content (web page, downloaded file, email).
    pub fn mark_tainted(&mut self, id: &str, source: &str) -> Result<()> {
        let s = self.sessions.get_mut(id).ok_or_else(|| anyhow!("unknown session {}", id))?;
        s.tainted = true;
        self.audit.append(AuditEntry {
            ts: now(),
            kind: "session.tainted".into(),
            session_id: id.into(),
            request_id: String::new(),
            tool: String::new(),
            tier: String::new(),
            verdict: String::new(),
            reason: format!("untrusted content from {}", source),
            detail: serde_json::Value::Null,
            prev: String::new(),
        })
    }

    pub fn session(&self, id: &str) -> Option<&Session> {
        self.sessions.get(id)
    }

    /// The core call: classify the intent, apply the mode table, apply taint, log, and answer.
    pub fn evaluate(&mut self, intent: &Intent) -> Result<Decision> {
        let session = self
            .sessions
            .get(&intent.session_id)
            .cloned()
            .ok_or_else(|| anyhow!("unknown session {}", intent.session_id))?;

        let class = self.policy.classify(intent, &session);
        let mut verdict = self.policy.policy.verdict_for(session.mode, class.tier);
        let mut reason = class.reason.clone();

        // Capability drop after taint: once untrusted content is in context, nothing at W2 or above
        // runs without a human, whatever the mode. Denials stay denials.
        let mut raised_by_taint = false;
        if session.tainted && class.tier >= Tier::WriteUser && matches!(verdict, Verdict::Allow | Verdict::AllowWithSnapshot) {
            verdict = Verdict::Prompt;
            raised_by_taint = true;
            reason = format!("{} (session has read untrusted content)", reason);
        }

        let decision = Decision {
            request_id: uuid::Uuid::new_v4().to_string(),
            session_id: intent.session_id.clone(),
            tier: class.tier,
            verdict,
            mode: session.mode,
            tainted: raised_by_taint,
            reason,
            rule: class.rule,
        };

        self.audit.append(AuditEntry {
            ts: now(),
            kind: "decision".into(),
            session_id: decision.session_id.clone(),
            request_id: decision.request_id.clone(),
            tool: intent.tool.clone(),
            tier: decision.tier.code().into(),
            verdict: format!("{:?}", decision.verdict).to_lowercase(),
            reason: decision.reason.clone(),
            detail: serde_json::json!({
                "command": intent.command,
                "reads": intent.reads,
                "writes": intent.writes,
                "domains": intent.network.domains,
                "rule": decision.rule,
                "mode": format!("{:?}", decision.mode).to_lowercase(),
            }),
            prev: String::new(),
        })?;

        if decision.verdict == Verdict::Prompt {
            self.pending.insert(decision.request_id.clone(), decision.clone());
        }
        Ok(decision)
    }

    /// The UI answers a prompt. Returns the final verdict.
    pub fn resolve(&mut self, request_id: &str, allowed: bool, by: &str) -> Result<Verdict> {
        let d = self.pending.remove(request_id).ok_or_else(|| anyhow!("no pending prompt {}", request_id))?;
        let verdict = if allowed { Verdict::Allow } else { Verdict::Deny };
        self.audit.append(AuditEntry {
            ts: now(),
            kind: "resolve".into(),
            session_id: d.session_id.clone(),
            request_id: request_id.into(),
            tool: String::new(),
            tier: d.tier.code().into(),
            verdict: format!("{:?}", verdict).to_lowercase(),
            reason: format!("answered by {}", by),
            detail: serde_json::Value::Null,
            prev: String::new(),
        })?;
        Ok(verdict)
    }

    pub fn pending(&self) -> Vec<Decision> {
        self.pending.values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Network, Privilege};

    fn broker() -> Broker {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        // keep the tempdir alive by leaking it for the test's duration
        std::mem::forget(dir);
        let mut b = Broker::new(Policy::default_policy(), path).unwrap();
        b.open_session("s1", Mode::Assist, vec!["/home/u/Projects/app".into()], "test").unwrap();
        b
    }

    fn shell(cmd: &str) -> Intent {
        Intent { session_id: "s1".into(), tool: "shell".into(), command: Some(cmd.into()), ..Default::default() }
    }

    #[test]
    fn phase0_break_system_packages_is_s1_and_prompts_in_assist_and_autoedit() {
        let mut b = broker();
        let d = b.evaluate(&shell("pip3 install --user --break-system-packages pytest")).unwrap();
        assert_eq!(d.tier, Tier::SystemUser);
        assert_eq!(d.verdict, Verdict::Prompt);
        assert_eq!(d.rule.as_deref(), Some("system_user.pip-override"));

        b.set_mode("s1", Mode::AutoEdit).unwrap();
        let d = b.evaluate(&shell("pip3 install --user --break-system-packages pytest")).unwrap();
        assert_eq!(d.verdict, Verdict::Prompt);

        b.set_mode("s1", Mode::Autonomous).unwrap();
        let d = b.evaluate(&shell("pip3 install --user --break-system-packages pytest")).unwrap();
        assert_eq!(d.verdict, Verdict::AllowWithSnapshot);
    }

    #[test]
    fn readonly_shell_is_allowed_everywhere() {
        let mut b = broker();
        for c in ["ls -la", "git status", "cat README.md", "ps aux | sort -k6 -rn | head -5"] {
            let d = b.evaluate(&shell(c)).unwrap();
            assert_eq!(d.tier, Tier::ReadLocal, "{}", c);
            assert_eq!(d.verdict, Verdict::Allow, "{}", c);
        }
    }

    #[test]
    fn project_write_prompts_in_assist_allows_in_autoedit() {
        let mut b = broker();
        let mut i = Intent { session_id: "s1".into(), tool: "fs.write".into(), ..Default::default() };
        i.writes = vec!["/home/u/Projects/app/src/main.ts".into()];
        let d = b.evaluate(&i).unwrap();
        assert_eq!(d.tier, Tier::WriteProject);
        assert_eq!(d.verdict, Verdict::Prompt);
        b.set_mode("s1", Mode::AutoEdit).unwrap();
        assert_eq!(b.evaluate(&i).unwrap().verdict, Verdict::Allow);
    }

    #[test]
    fn write_outside_project_is_w2() {
        let mut b = broker();
        b.set_mode("s1", Mode::AutoEdit).unwrap();
        let mut i = Intent { session_id: "s1".into(), tool: "fs.write".into(), ..Default::default() };
        i.writes = vec!["/home/u/Documents/notes.md".into()];
        let d = b.evaluate(&i).unwrap();
        assert_eq!(d.tier, Tier::WriteUser);
        assert_eq!(d.verdict, Verdict::Prompt);
    }

    #[test]
    fn secrets_are_denied_in_every_mode() {
        let mut b = broker();
        for m in [Mode::Assist, Mode::AutoEdit, Mode::Autonomous] {
            b.set_mode("s1", m).unwrap();
            let mut i = Intent { session_id: "s1".into(), tool: "fs.read".into(), ..Default::default() };
            i.reads = vec!["~/.ssh/id_ed25519".into()];
            let d = b.evaluate(&i).unwrap();
            assert_eq!(d.tier, Tier::Secrets);
            assert_eq!(d.verdict, Verdict::Deny);
        }
    }

    #[test]
    fn never_tier_is_denied_even_autonomous() {
        let mut b = broker();
        b.set_mode("s1", Mode::Autonomous).unwrap();
        for c in ["rm -rf ~/", "sudo mkfs.ext4 /dev/sda1", "systemctl --user stop genesis-permd", "mokutil --disable-validation"] {
            let d = b.evaluate(&shell(c)).unwrap();
            assert_eq!(d.tier, Tier::Never, "{}", c);
            assert_eq!(d.verdict, Verdict::Deny, "{}", c);
        }
    }

    #[test]
    fn rm_inside_project_is_project_work_but_home_is_never() {
        let mut b = broker();
        b.set_mode("s1", Mode::Autonomous).unwrap();
        for c in ["rm -rf ./dist", "rm -rf /home/u/Projects/app/node_modules", "rm -rf /home/u/Projects/app/dist /home/u/Projects/app/.cache"] {
            let d = b.evaluate(&shell(c)).unwrap();
            assert_eq!(d.tier, Tier::WriteProject, "{}", c);
        }
        for c in ["rm -rf /home/u", "rm -rf /home/u/Documents", "rm -rf ~/", "rm -rf /home/u/Projects/app /home/u/Documents"] {
            let d = b.evaluate(&shell(c)).unwrap();
            assert_eq!(d.tier, Tier::Never, "{}", c);
        }
    }

    #[test]
    fn sudo_and_bootc_are_system_and_prompt_even_autonomous() {
        let mut b = broker();
        b.set_mode("s1", Mode::Autonomous).unwrap();
        for c in ["sudo dnf install foo", "bootc upgrade", "rpm-ostree install foo", "nmcli con up wifi"] {
            let d = b.evaluate(&shell(c)).unwrap();
            assert_eq!(d.tier, Tier::System, "{}", c);
            assert_eq!(d.verdict, Verdict::Prompt, "{}", c);
        }
    }

    #[test]
    fn taint_raises_user_scope_to_prompt_in_autonomous() {
        let mut b = broker();
        b.set_mode("s1", Mode::Autonomous).unwrap();
        let d = b.evaluate(&shell("flatpak install --user org.example.App")).unwrap();
        assert_eq!(d.verdict, Verdict::AllowWithSnapshot);
        b.mark_tainted("s1", "https://example.com/page").unwrap();
        let d = b.evaluate(&shell("flatpak install --user org.example.App")).unwrap();
        assert_eq!(d.verdict, Verdict::Prompt);
        assert!(d.tainted);
        // project writes are still fine after taint
        let mut i = Intent { session_id: "s1".into(), tool: "fs.write".into(), ..Default::default() };
        i.writes = vec!["/home/u/Projects/app/x.rs".into()];
        assert_eq!(b.evaluate(&i).unwrap().verdict, Verdict::Allow);
    }

    #[test]
    fn network_allowlist_and_denylist() {
        let mut b = broker();
        let i = Intent { session_id: "s1".into(), tool: "web.fetch".into(), network: Network { domains: vec!["pypi.org".into()] }, ..Default::default() };
        let d = b.evaluate(&i).unwrap();
        assert_eq!(d.tier, Tier::ReadNetwork);
        assert_eq!(d.verdict, Verdict::Prompt); // assist prompts for network
        let i = Intent { session_id: "s1".into(), tool: "web.fetch".into(), network: Network { domains: vec!["169.254.169.254".into()] }, ..Default::default() };
        let d = b.evaluate(&i).unwrap();
        assert_eq!(d.tier, Tier::Never);
        assert_eq!(d.verdict, Verdict::Deny);
    }

    #[test]
    fn root_flag_is_system() {
        let mut b = broker();
        let i = Intent { session_id: "s1".into(), tool: "shell".into(), command: Some("ls".into()), privilege: Privilege::Root, ..Default::default() };
        assert_eq!(b.evaluate(&i).unwrap().tier, Tier::System);
    }

    #[test]
    fn prompt_resolution_and_audit_chain() {
        let mut b = broker();
        let d = b.evaluate(&shell("npm install -g typescript")).unwrap();
        assert_eq!(d.verdict, Verdict::Prompt);
        assert_eq!(b.pending().len(), 1);
        assert_eq!(b.resolve(&d.request_id, true, "test-ui").unwrap(), Verdict::Allow);
        assert!(b.pending().is_empty());
        let n = AuditLog::verify(b.audit_path()).unwrap();
        assert!(n >= 3);
    }
}
