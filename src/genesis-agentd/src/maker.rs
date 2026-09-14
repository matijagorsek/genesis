//! Maker tools: scaffold a project from a template, run a live preview, install the result as an app.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSpec {
    pub cmd: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub open: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Template {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub dev: RunSpec,
    #[serde(default)]
    pub run: Option<RunSpec>,
    #[serde(default)]
    pub entry: String,
}

pub fn templates_dir() -> PathBuf {
    if let Ok(p) = std::env::var("GENESIS_TEMPLATES") {
        return PathBuf::from(p);
    }
    let system = Path::new("/usr/share/genesis/templates");
    if system.is_dir() {
        return system.to_path_buf();
    }
    // dev checkout: ../../templates relative to the workspace, or ./templates
    for c in ["templates", "../templates", "../../templates"] {
        let p = PathBuf::from(c);
        if p.join("web-static").is_dir() {
            return std::fs::canonicalize(p).unwrap_or(PathBuf::from(c));
        }
    }
    system.to_path_buf()
}

pub fn list_templates() -> Vec<Template> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(templates_dir()) {
        let mut dirs: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.join("genesis.json").is_file()).collect();
        dirs.sort();
        for d in dirs {
            if let Ok(t) = serde_json::from_str::<Template>(&std::fs::read_to_string(d.join("genesis.json")).unwrap_or_default()) {
                out.push(t);
            }
        }
    }
    out
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') && !name.starts_with('-')
}

/// Copy a template into `dest`, substituting `{{name}}` in text files and `__name__` in file names.
pub fn scaffold(template_id: &str, name: &str, dest: &Path) -> Result<Template> {
    if !valid_name(name) {
        return Err(anyhow!("project name must be letters, digits, - or _"));
    }
    let src = templates_dir().join(template_id);
    if !src.join("genesis.json").is_file() {
        return Err(anyhow!("unknown template {} (have: {})", template_id, list_templates().iter().map(|t| t.id.clone()).collect::<Vec<_>>().join(", ")));
    }
    if dest.exists() && std::fs::read_dir(dest).map(|mut d| d.next().is_some()).unwrap_or(false) {
        return Err(anyhow!("{} already exists and is not empty", dest.display()));
    }
    std::fs::create_dir_all(dest)?;
    copy_tree(&src, dest, name)?;
    let t: Template = serde_json::from_str(&std::fs::read_to_string(dest.join("genesis.json"))?)?;
    Ok(t)
}

fn copy_tree(src: &Path, dest: &Path, name: &str) -> Result<()> {
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let fname = e.file_name().to_string_lossy().replace("__name__", name);
        let target = dest.join(&fname);
        if e.path().is_dir() {
            std::fs::create_dir_all(&target)?;
            copy_tree(&e.path(), &target, name)?;
        } else {
            let bytes = std::fs::read(e.path())?;
            match String::from_utf8(bytes) {
                Ok(text) => std::fs::write(&target, text.replace("{{name}}", name).replace("{name}", name))?,
                Err(err) => std::fs::write(&target, err.into_bytes())?,
            }
        }
    }
    Ok(())
}

pub struct Preview {
    pub child: Child,
    pub url: Option<String>,
    pub cmd: String,
}

/// Running previews, keyed by project path.
#[derive(Default)]
pub struct Previews {
    pub running: HashMap<String, Preview>,
}

impl Previews {
    pub fn start(&mut self, project: &Path) -> Result<String> {
        let key = project.display().to_string();
        if let Some(p) = self.running.get_mut(&key) {
            if p.child.try_wait()?.is_none() {
                return Ok(format!("already running: {}", p.url.clone().unwrap_or_else(|| p.cmd.clone())));
            }
            self.running.remove(&key);
        }
        let manifest = project.join("genesis.json");
        let t: Template = serde_json::from_str(&std::fs::read_to_string(&manifest).with_context(|| format!("no genesis.json in {}; scaffold a template first or add one", project.display()))?)?;
        let name = project.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let port = if t.dev.port > 0 { free_port(t.dev.port) } else { 0 };
        let cmd = t.dev.cmd.replace("{port}", &port.to_string()).replace("{name}", &name);
        let log = std::fs::File::create(project.join(".genesis-preview.log"))?;
        let child = Command::new("/bin/sh").args(["-lc", &cmd]).current_dir(project).stdin(Stdio::null()).stdout(Stdio::from(log.try_clone()?)).stderr(Stdio::from(log)).spawn().with_context(|| format!("starting {}", cmd))?;
        let url = if port > 0 { Some(format!("http://127.0.0.1:{}/", port)) } else { None };
        // give servers a moment; if the process already died, report the log
        std::thread::sleep(std::time::Duration::from_millis(900));
        let mut p = Preview { child, url: url.clone(), cmd: cmd.clone() };
        if let Some(status) = p.child.try_wait()? {
            let out = std::fs::read_to_string(project.join(".genesis-preview.log")).unwrap_or_default();
            if port > 0 {
                return Err(anyhow!("preview command exited with {}: {}", status, out.chars().take(600).collect::<String>()));
            }
            // non-server dev commands (tests, scripts) are expected to exit: return their output
            return Ok(format!("ran `{}` (exit {}):\n{}", cmd, status.code().unwrap_or(-1), out.chars().take(4000).collect::<String>()));
        }
        self.running.insert(key, p);
        Ok(match url { Some(u) => format!("preview running at {}", u), None => format!("running `{}` in the background", cmd) })
    }

    pub fn stop(&mut self, project: &Path) -> Result<String> {
        let key = project.display().to_string();
        match self.running.remove(&key) {
            Some(mut p) => {
                let _ = p.child.kill();
                let _ = p.child.wait();
                Ok("preview stopped".into())
            }
            None => Ok("no preview was running".into()),
        }
    }

    pub fn url(&self, project: &Path) -> Option<String> {
        self.running.get(&project.display().to_string()).and_then(|p| p.url.clone())
    }
}

impl Drop for Previews {
    fn drop(&mut self) {
        for (_, p) in self.running.iter_mut() {
            let _ = p.child.kill();
        }
    }
}

fn free_port(preferred: u16) -> u16 {
    for p in preferred..preferred + 50 {
        if std::net::TcpListener::bind(("127.0.0.1", p)).is_ok() {
            return p;
        }
    }
    preferred
}

/// Write a desktop entry so the project shows up in the app menu. Returns the entry path.
pub fn install_app(project: &Path, display_name: &str) -> Result<PathBuf> {
    let t: Template = serde_json::from_str(&std::fs::read_to_string(project.join("genesis.json")).context("no genesis.json in the project")?)?;
    let run = t.run.clone().unwrap_or(t.dev.clone());
    let name = project.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let port = if run.port > 0 { run.port } else { t.dev.port };
    let cmd = run.cmd.replace("{port}", &port.to_string()).replace("{name}", &name);
    let open = run.open.clone().map(|o| o.replace("{port}", &port.to_string()));
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let apps = Path::new(&home).join(".local/share/applications");
    std::fs::create_dir_all(&apps)?;
    let entry = apps.join(format!("genesis-{}.desktop", name));
    let exec = match open {
        Some(url) => format!("sh -c 'cd {} && ({} &) && sleep 1 && xdg-open {}'", shell_quote(&project.display().to_string()), cmd, url),
        None => format!("sh -c 'cd {} && {}'", shell_quote(&project.display().to_string()), cmd),
    };
    let content = format!("[Desktop Entry]\nType=Application\nName={}\nComment=Made with Genesis\nExec={}\nPath={}\nTerminal={}\nCategories=Utility;\nX-Genesis-Project={}\n", display_name, exec, project.display(), if run.open.is_some() { "false" } else { "true" }, project.display());
    std::fs::write(&entry, content)?;
    Ok(entry)
}

/// One thing made with Genesis: a project folder with a genesis.json, plus whether it is in the app menu.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Made {
    pub name: String,
    pub path: String,
    pub template: String,
    pub installed: bool,
    pub made_at: String,
    #[serde(default)]
    pub prompts: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Recipe {
    #[serde(default)]
    pub template: String,
    #[serde(default)]
    pub prompts: Vec<String>,
    #[serde(default)]
    pub made_at: String,
}

/// Append a prompt to the project's recipe (the shareable "how this was made" record).
pub fn record_recipe(project: &Path, template: Option<&str>, prompt: &str) {
    let f = project.join(".genesis-recipe.json");
    let mut r: Recipe = std::fs::read_to_string(&f).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    if let Some(t) = template { if r.template.is_empty() { r.template = t.to_string(); } }
    if r.made_at.is_empty() { r.made_at = time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339).unwrap_or_default(); }
    if !prompt.trim().is_empty() && r.prompts.last().map(|l| l != prompt).unwrap_or(true) { r.prompts.push(prompt.trim().to_string()); }
    let _ = std::fs::write(&f, serde_json::to_string_pretty(&r).unwrap_or_default());
}

/// Everything made under `projects_dir`, newest first.
pub fn made_here(projects_dir: &Path) -> Vec<Made> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let apps = Path::new(&home).join(".local/share/applications");
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(projects_dir) {
        for e in rd.flatten() {
            let p = e.path();
            let manifest = p.join("genesis.json");
            if !manifest.is_file() { continue; }
            let name = p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let t: Option<Template> = std::fs::read_to_string(&manifest).ok().and_then(|x| serde_json::from_str(&x).ok());
            let recipe: Recipe = std::fs::read_to_string(p.join(".genesis-recipe.json")).ok().and_then(|x| serde_json::from_str(&x).ok()).unwrap_or_default();
            let mtime = std::fs::metadata(&manifest).and_then(|m| m.modified()).ok().map(|m| time::OffsetDateTime::from(m).format(&time::format_description::well_known::Rfc3339).unwrap_or_default()).unwrap_or_default();
            out.push(Made {
                name: name.clone(),
                path: p.display().to_string(),
                template: if recipe.template.is_empty() { t.map(|t| t.id).unwrap_or_default() } else { recipe.template },
                installed: apps.join(format!("genesis-{}.desktop", name)).is_file(),
                made_at: if recipe.made_at.is_empty() { mtime } else { recipe.made_at },
                prompts: recipe.prompts,
            });
        }
    }
    out.sort_by(|a, b| b.made_at.cmp(&a.made_at));
    out
}

/// Bundle a project (files plus its recipe) into ~/Downloads/<name>-genesis.tar.gz for sharing.
pub fn export_bundle(project: &Path) -> Result<PathBuf> {
    if !project.join("genesis.json").is_file() { return Err(anyhow!("{} is not a Genesis project", project.display())); }
    let name = project.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "project".into());
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let downloads = Path::new(&home).join("Downloads");
    std::fs::create_dir_all(&downloads)?;
    let out = downloads.join(format!("{}-genesis.tar.gz", name));
    let parent = project.parent().ok_or_else(|| anyhow!("project has no parent directory"))?;
    let status = Command::new("tar").arg("-czf").arg(&out).arg("-C").arg(parent)
        .args(["--exclude=.genesis-preview.log", "--exclude=node_modules", "--exclude=__pycache__", "--exclude=.venv"]).arg(&name)
        .status().context("running tar")?;
    if !status.success() { return Err(anyhow!("tar failed with {}", status)); }
    Ok(out)
}

/// A proactive card: something Genesis noticed that the person may want to act on. Local sources only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notice {
    pub kind: String,
    pub title: String,
    pub text: String,
    /// A request to hand to the maker, or empty when the card is informational.
    pub prompt: String,
    pub project: String,
}

/// What Genesis noticed: made projects whose last run failed, and made projects never installed to the menu.
pub fn notices(projects_dir: &Path) -> Vec<Notice> {
    let mut out = Vec::new();
    for m in made_here(projects_dir) {
        let log = Path::new(&m.path).join(".genesis-preview.log");
        if let Ok(text) = std::fs::read_to_string(&log) {
            // judge the end of the last run only: an error near the end, and no success marker after it
            let tail: String = text.chars().rev().take(600).collect::<String>().chars().rev().collect();
            let err_pos = ["Traceback", "FAILED", "Error:", "error:", "ModuleNotFoundError", "SyntaxError"].iter().filter_map(|k| tail.rfind(k)).max();
            let ok_pos = ["\nOK", "passed", "exit=0", "Serving", "on http"].iter().filter_map(|k| tail.rfind(k)).max();
            let failed = match (err_pos, ok_pos) { (Some(e), Some(o)) => e > o, (Some(_), None) => true, _ => false };
            if failed {
                out.push(Notice { kind: "failing".into(), title: format!("{} did not run cleanly last time", m.name), text: "Its last preview or test run ended with an error.".into(), prompt: format!("The last run of this project failed. Read .genesis-preview.log, find the cause and fix it, then run it again."), project: m.path.clone() });
                continue;
            }
        }
        if !m.installed && !m.prompts.is_empty() {
            out.push(Notice { kind: "not-installed".into(), title: format!("{} is not in your app menu yet", m.name), text: "Install it and it opens like any other program.".into(), prompt: format!("Install this project as an app named \"{}\" using the install_app tool.", m.name), project: m.path.clone() });
        }
    }
    // things in Downloads: calendar invitations, and a large download that just finished
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let downloads = Path::new(&home).join("Downloads");
    if let Ok(rd) = std::fs::read_dir(&downloads) {
        let now = std::time::SystemTime::now();
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| std::cmp::Reverse(e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH)));
        for e in entries.into_iter().take(40) {
            let p = e.path(); let name = e.file_name().to_string_lossy().to_string();
            let Ok(md) = e.metadata() else { continue };
            let age = md.modified().ok().and_then(|m| now.duration_since(m).ok()).map(|d| d.as_secs()).unwrap_or(u64::MAX);
            if name.to_lowercase().ends_with(".ics") && age < 7 * 86400 {
                out.push(Notice { kind: "calendar".into(), title: format!("{} is a calendar invitation", name), text: "Open it to add the event to your calendar.".into(), prompt: String::new(), project: p.display().to_string() });
            } else if md.len() > 200_000_000 && age < 86400 && !name.ends_with(".part") {
                out.push(Notice { kind: "download".into(), title: format!("{} finished downloading", name), text: format!("{:.1} GB, in Downloads.", md.len() as f64 / 1e9), prompt: String::new(), project: p.display().to_string() });
            }
        }
    }
    // git repositories under the projects folder with changes left uncommitted for more than a day
    if let Ok(rd) = std::fs::read_dir(projects_dir) {
        for e in rd.flatten().filter(|e| e.path().join(".git").is_dir()) {
            let p = e.path();
            let dirty = std::process::Command::new("git").args(["-C", &p.display().to_string(), "status", "--porcelain"]).output().map(|o| !o.stdout.is_empty()).unwrap_or(false);
            if !dirty { continue; }
            let idx_age = std::fs::metadata(p.join(".git/index")).and_then(|m| m.modified()).ok().and_then(|m| std::time::SystemTime::now().duration_since(m).ok()).map(|d| d.as_secs()).unwrap_or(0);
            if idx_age > 86400 {
                let name = p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                out.push(Notice { kind: "uncommitted".into(), title: format!("{} has uncommitted changes since yesterday", name), text: "Genesis can review them and write a commit.".into(), prompt: "Look at the uncommitted changes in this repository (git status, git diff), summarise them, and if they look complete commit them with a clear message.".into(), project: p.display().to_string() });
            }
        }
    }
    out.truncate(8);
    out
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Tests that set HOME must not overlap (cargo runs tests in parallel).
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn scaffold_substitutes_names_and_lists_templates() {
        let repo_templates = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates");
        std::env::set_var("GENESIS_TEMPLATES", repo_templates.display().to_string());
        assert!(list_templates().iter().any(|t| t.id == "web-static"));
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("mytool");
        let t = scaffold("python-cli", "mytool", &dest).unwrap();
        assert_eq!(t.id, "python-cli");
        assert!(dest.join("mytool.py").is_file());
        assert!(dest.join("test_mytool.py").is_file());
        let text = std::fs::read_to_string(dest.join("mytool.py")).unwrap();
        assert!(text.contains("prog=\"mytool\""));
        assert!(!text.contains("{{name}}"));
        assert!(scaffold("python-cli", "bad name", &dir.path().join("x")).is_err());
        assert!(scaffold("nope", "ok", &dir.path().join("y")).is_err());
    }

    #[test]
    fn gallery_lists_projects_with_recipes() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let repo_templates = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates");
        std::env::set_var("GENESIS_TEMPLATES", repo_templates.display().to_string());
        let projects = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", home.path().display().to_string());
        let a = projects.path().join("timer");
        scaffold("web-static", "timer", &a).unwrap();
        record_recipe(&a, Some("web-static"), "a pomodoro timer with a bell");
        record_recipe(&a, None, "make the bell louder");
        install_app(&a, "Timer").unwrap();
        std::fs::create_dir_all(projects.path().join("not-genesis")).unwrap();
        let made = made_here(projects.path());
        assert_eq!(made.len(), 1);
        let bundle = export_bundle(&a).unwrap();
        assert!(bundle.ends_with("timer-genesis.tar.gz") && bundle.is_file());
        let listing = String::from_utf8(Command::new("tar").arg("-tzf").arg(&bundle).output().unwrap().stdout).unwrap();
        assert!(listing.contains("timer/genesis.json") && listing.contains("timer/.genesis-recipe.json"), "{}", listing);
        assert!(export_bundle(projects.path()).is_err());
        assert_eq!(made[0].name, "timer");
        assert!(made[0].installed);
        assert_eq!(made[0].template, "web-static");
        assert_eq!(made[0].prompts, vec!["a pomodoro timer with a bell", "make the bell louder"]);
    }

    #[test]
    fn notices_flag_failed_runs_and_uninstalled_projects() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let repo_templates = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates");
        std::env::set_var("GENESIS_TEMPLATES", repo_templates.display().to_string());
        let projects = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", home.path().display().to_string());
        let a = projects.path().join("broken");
        scaffold("python-cli", "broken", &a).unwrap();
        record_recipe(&a, Some("python-cli"), "a thing");
        std::fs::write(a.join(".genesis-preview.log"), "Traceback (most recent call last):\n  boom\n").unwrap();
        let b = projects.path().join("fresh");
        scaffold("web-static", "fresh", &b).unwrap();
        record_recipe(&b, Some("web-static"), "a site");
        let n = notices(projects.path());
        let kinds: Vec<(String, String)> = n.iter().map(|x| (x.kind.clone(), x.project.clone())).collect();
        assert!(kinds.iter().any(|(k, p)| k == "failing" && p.ends_with("broken")), "{:?}", kinds);
        assert!(kinds.iter().any(|(k, p)| k == "not-installed" && p.ends_with("fresh")), "{:?}", kinds);
    }

    #[test]
    fn install_app_writes_desktop_entry() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let repo_templates = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates");
        std::env::set_var("GENESIS_TEMPLATES", repo_templates.display().to_string());
        let dir = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", home.path().display().to_string());
        let dest = dir.path().join("board");
        scaffold("web-static", "board", &dest).unwrap();
        let entry = install_app(&dest, "Board").unwrap();
        let text = std::fs::read_to_string(&entry).unwrap();
        assert!(text.contains("Name=Board"));
        assert!(text.contains("http.server"));
        assert!(text.contains("xdg-open http://127.0.0.1:5300/"));
    }
}
