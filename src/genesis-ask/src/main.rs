//! genesis-ask: Genesis in the terminal.
//!
//!   genesis-ask list the ten biggest files here      -> shows the command, runs it when you confirm
//!   genesis-ask --print "..."                         -> prints only the command (used by the Ctrl+G binding)
//!
//! The sentence goes to the local model service (nothing leaves the machine). The command is classified
//! by the same permission policy the maker uses, so you see what it would touch before it runs.

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use genesis_permd::{command_write_paths, default_audit_path, default_policy_sources, Broker, Intent, Mode, Policy, Tier, Verdict};
use std::io::{self, BufRead, Write};

#[derive(Parser, Debug)]
#[command(name = "genesis-ask", about = "Say what you want; get the command; run it when you confirm")]
struct Cli {
    /// Print the command only, never run it
    #[arg(long)]
    print: bool,
    /// Run without asking (still refuses commands the policy never allows)
    #[arg(short = 'y', long)]
    yes: bool,
    #[arg(long, env = "GENESIS_ENDPOINT", default_value = "http://127.0.0.1:8080/v1")]
    endpoint: String,
    /// Model name at the local endpoint (the small, always-hot one by default)
    #[arg(long, env = "GENESIS_ASK_MODEL", default_value = "fast")]
    model: String,
    /// What you want done, in plain words
    #[arg(trailing_var_arg = true)]
    words: Vec<String>,
}

const SYSTEM: &str = "You turn a request into exactly one shell command line for a Fedora Linux desktop (bash, GNU coreutils, KDE). \
Reply with the command only: no explanation, no code fences, no leading $. Prefer safe, common tools. \
If the request cannot be done with a command, reply with a comment starting with # explaining why in one line.";

fn ask_model(cli: &Cli, question: &str) -> Result<String> {
    let cwd = std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default();
    let body = serde_json::json!({
        "model": cli.model, "temperature": 0.1, "max_tokens": 200,
        "messages": [
            {"role": "system", "content": SYSTEM},
            {"role": "user", "content": format!("Current directory: {}\nRequest: {}", cwd, question)}
        ]
    });
    let resp: serde_json::Value = ureq::post(&format!("{}/chat/completions", cli.endpoint.trim_end_matches('/')))
        .set("Authorization", "Bearer local").timeout(std::time::Duration::from_secs(60)).send_json(body)
        .map_err(|e| anyhow!("the local model service did not answer ({}). Is Genesis set up? Open Genesis Settings.", e))?
        .into_json().context("bad reply from the model service")?;
    let text = resp["choices"][0]["message"]["content"].as_str().unwrap_or("").trim().to_string();
    Ok(clean(&text))
}

/// Strip fences, a leading `$ ` and surrounding whitespace; keep the first non-empty line.
fn clean(text: &str) -> String {
    let mut lines: Vec<&str> = text.lines().map(|l| l.trim()).filter(|l| !l.is_empty() && !l.starts_with("```")).collect();
    if lines.is_empty() { return String::new(); }
    let first = lines.remove(0);
    let first = first.trim_start_matches("$ ").trim_start_matches('`').trim_end_matches('`').trim();
    first.to_string()
}

/// Classify the command with the shared policy: what would it touch?
fn classify(cmd: &str) -> Result<(Tier, Verdict, String)> {
    let policy = Policy::load(&default_policy_sources())?;
    let mut broker = Broker::new(policy, default_audit_path())?;
    let cwd = std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_else(|_| "/".into());
    let sid = format!("ask-{}", std::process::id());
    broker.open_session(&sid, Mode::Assist, vec![cwd.clone()], "terminal")?;
    let mut intent = Intent { session_id: sid, origin: "terminal".into(), tool: "shell".into(), command: Some(cmd.to_string()), ..Default::default() };
    intent.writes = command_write_paths(cmd);
    let d = broker.evaluate(&intent)?;
    Ok((d.tier, d.verdict, d.reason))
}

fn describe(tier: Tier) -> &'static str {
    match tier {
        Tier::ReadLocal => "reads only",
        Tier::ReadNetwork => "uses the network",
        Tier::WriteProject => "changes files in this folder",
        Tier::WriteUser => "changes files outside this folder",
        Tier::SystemUser => "installs or changes settings for you",
        Tier::System => "changes the system",
        Tier::Secrets => "touches secrets",
        Tier::Never => "not allowed by policy",
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let question = cli.words.join(" ").trim().to_string();
    if question.is_empty() {
        eprintln!("usage: genesis-ask <what you want, in plain words>   (or: ask ...; or type it and press Ctrl+G)");
        std::process::exit(2);
    }
    let cmd = ask_model(&cli, &question)?;
    if cmd.is_empty() { return Err(anyhow!("the model gave no command")); }
    if cli.print {
        if cmd.starts_with('#') { std::process::exit(1); }
        println!("{}", cmd);
        return Ok(());
    }
    if cmd.starts_with('#') {
        println!("{}", cmd.trim_start_matches('#').trim());
        return Ok(());
    }
    let (tier, verdict, reason) = classify(&cmd)?;
    eprintln!("\x1b[1m{}\x1b[0m", cmd);
    eprintln!("\x1b[2m{} · {}\x1b[0m", describe(tier), reason);
    if verdict == Verdict::Deny {
        eprintln!("Not running this: the policy never allows it.");
        std::process::exit(3);
    }
    let mut run = cli.yes;
    let mut cmd = cmd;
    if !run {
        eprint!("[Enter] run   e edit   n cancel > ");
        io::stderr().flush()?;
        let mut line = String::new();
        io::stdin().lock().read_line(&mut line)?;
        match line.trim() {
            "" | "y" | "yes" => run = true,
            "e" | "edit" => {
                eprint!("command > ");
                io::stderr().flush()?;
                let mut edited = String::new();
                io::stdin().lock().read_line(&mut edited)?;
                if !edited.trim().is_empty() { cmd = edited.trim().to_string(); run = true; }
            }
            _ => {}
        }
    }
    if !run { eprintln!("cancelled"); return Ok(()); }
    let status = std::process::Command::new("bash").arg("-c").arg(&cmd).status().context("running the command")?;
    std::process::exit(status.code().unwrap_or(1));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cleans_model_output() {
        assert_eq!(clean("```bash\n$ ls -la\n```"), "ls -la");
        assert_eq!(clean("`du -sh *`\nsome explanation"), "du -sh *");
        assert_eq!(clean("  \n"), "");
    }
}
