//! Shell execution. On Linux, inside bubblewrap when available: read-only system, the project bound
//! read-write, a private /tmp, no network unless allowed. Elsewhere (dev on macOS) a plain subprocess.

use anyhow::Result;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::Duration;

pub struct ShellResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub sandboxed: bool,
    pub timed_out: bool,
}

pub fn bwrap_available() -> bool {
    cfg!(target_os = "linux") && which("bwrap")
}

fn which(bin: &str) -> bool {
    std::env::var_os("PATH").map(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file())).unwrap_or(false)
}

/// Build the bubblewrap argument list for a project directory.
pub fn bwrap_args(project: &Path, allow_network: bool) -> Vec<String> {
    let p = project.display().to_string();
    let mut a: Vec<String> = vec![
        "--ro-bind", "/usr", "/usr",
        "--ro-bind-try", "/etc", "/etc",
        "--ro-bind-try", "/opt", "/opt",
        "--symlink", "usr/lib", "/lib",
        "--symlink", "usr/lib64", "/lib64",
        "--symlink", "usr/bin", "/bin",
        "--symlink", "usr/sbin", "/sbin",
        "--dev", "/dev",
        "--proc", "/proc",
        "--tmpfs", "/tmp",
        "--tmpfs", "/var",
        "--tmpfs", "/run",
        "--tmpfs", "/home",
        "--bind", &p, &p,
        "--chdir", &p,
        "--unshare-pid", "--unshare-ipc", "--unshare-uts",
        "--die-with-parent", "--new-session",
        "--setenv", "HOME", "/tmp", "--setenv", "GENESIS_SANDBOX", "1",
    ].into_iter().map(String::from).collect();
    if !allow_network {
        a.push("--unshare-net".into());
    }
    // toolchains commonly live in the user's home; expose read-only the ones the project needs
    for extra in ["/var/lib/genesis/models", "/usr/lib/genesis"] {
        if Path::new(extra).exists() {
            a.extend(["--ro-bind".to_string(), extra.to_string(), extra.to_string()]);
        }
    }
    a
}

pub fn run_shell(project: &Path, command: &str, allow_network: bool, timeout: Duration, stop: Option<&std::sync::atomic::AtomicBool>) -> Result<ShellResult> {
    let sandboxed = bwrap_available();
    let mut cmd = if sandboxed {
        let mut c = Command::new("bwrap");
        c.args(bwrap_args(project, allow_network));
        c.args(["/bin/sh", "-lc", command]);
        c
    } else {
        let mut c = Command::new("/bin/sh");
        c.args(["-lc", command]);
        c.current_dir(project);
        c.env("GENESIS_SANDBOX", "0");
        c
    };
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let start = std::time::Instant::now();
    let mut timed_out = false;
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if start.elapsed() > timeout || stop.map(|s| s.load(std::sync::atomic::Ordering::SeqCst)).unwrap_or(false) {
            let _ = child.kill();
            timed_out = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let Output { status, stdout, stderr } = child.wait_with_output()?;
    Ok(ShellResult {
        exit_code: status.code().unwrap_or(-1),
        stdout: truncate(String::from_utf8_lossy(&stdout).to_string(), 16_000),
        stderr: truncate(String::from_utf8_lossy(&stderr).to_string(), 4_000),
        sandboxed,
        timed_out,
    })
}

fn truncate(s: String, max: usize) -> String {
    if s.len() <= max {
        s
    } else {
        let mut t: String = s.chars().take(max).collect();
        t.push_str("\n…[truncated]");
        t
    }
}
