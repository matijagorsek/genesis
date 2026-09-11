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

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn install_app_writes_desktop_entry() {
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
