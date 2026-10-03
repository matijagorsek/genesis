//! Shell execution. On Linux, inside bubblewrap when available: read-only system, the project bound
//! read-write, a private /tmp, no network unless allowed. Elsewhere (dev on macOS) a plain subprocess.

use anyhow::Result;
use std::path::Path;
use std::process::{Command, Stdio};
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
        "--unshare-pid", "--unshare-ipc", "--unshare-uts", "--unshare-cgroup-try",
        "--die-with-parent", "--new-session", "--cap-drop", "ALL",
        // Nothing of this session's environment goes in. It was inherited whole, which handed every
        // command the model runs whatever the desktop session happens to carry — an API key someone
        // exported, the address of the session bus, the path to this daemon's own token. A command that
        // is allowed the network could have read any of it out. What a shell genuinely needs is short.
        "--clearenv",
        "--setenv", "HOME", "/tmp",
        "--setenv", "GENESIS_SANDBOX", "1",
        "--setenv", "PATH", "/usr/bin:/bin:/usr/sbin:/sbin:/usr/local/bin",
        "--setenv", "SHELL", "/bin/sh",
        "--setenv", "USER", "genesis",
    ].into_iter().map(String::from).collect();
    // how text is shown and where the machine thinks it is: no secret in any of them, and a build that
    // cannot read its own locale produces confusing failures
    for k in ["TERM", "LANG", "LC_ALL", "TZ"] {
        if let Ok(v) = std::env::var(k) {
            if !v.is_empty() {
                a.extend(["--setenv".to_string(), k.to_string(), v]);
            }
        }
    }
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

/// What a window needs on top of the sandbox: the display, and nothing else of the session. The Wayland
/// socket (and an X display where there is no Wayland), the GPU for drawing, the GTK settings so it follows
/// the day/night look, and the one folder of the person's data it keeps its own things in,
/// `~/.local/share/<project name>`, at its real path so the installed app finds the same. No session bus:
/// GLib then runs the app as a plain non-unique one. A window ran with the whole account (decision 242).
pub fn bwrap_display_args(project: &Path) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    let home = std::env::var("HOME").unwrap_or_default();
    let runtime = std::env::var("XDG_RUNTIME_DIR").unwrap_or_default();
    let mut add = |xs: &[&str]| a.extend(xs.iter().map(|x| x.to_string()));
    if !runtime.is_empty() {
        let wayland = std::env::var("WAYLAND_DISPLAY").ok().filter(|w| !w.is_empty()).unwrap_or_else(|| "wayland-0".into());
        let sock = if wayland.starts_with('/') { wayland.clone() } else { format!("{}/{}", runtime, wayland) };
        if Path::new(&sock).exists() {
            add(&["--ro-bind", &sock, &sock, "--setenv", "XDG_RUNTIME_DIR", &runtime, "--setenv", "WAYLAND_DISPLAY", &sock, "--setenv", "GDK_BACKEND", "wayland"]);
        }
    }
    if let Ok(d) = std::env::var("DISPLAY") {
        if !d.is_empty() && Path::new("/tmp/.X11-unix").exists() {
            add(&["--ro-bind", "/tmp/.X11-unix", "/tmp/.X11-unix", "--setenv", "DISPLAY", &d]);
            if let Ok(x) = std::env::var("XAUTHORITY") { if Path::new(&x).exists() { add(&["--ro-bind", &x, &x, "--setenv", "XAUTHORITY", &x]); } }
        }
    }
    add(&["--dev-bind-try", "/dev/dri", "/dev/dri"]);
    if !home.is_empty() {
        for d in [".config/gtk-4.0", ".config/gtk-3.0", ".config/kdeglobals", ".local/share/fonts", ".fonts", ".icons"] {
            let p = format!("{}/{}", home, d);
            add(&["--ro-bind-try", &p, &p]);
        }
        let name = project.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if !name.is_empty() {
            let data = format!("{}/.local/share/{}", home, name);
            let _ = std::fs::create_dir_all(&data);
            add(&["--bind", &data, &data]);
        }
        add(&["--setenv", "HOME", &home]);
    }
    a
}

/// A command that is just a toolbox call, `genesis-toolbox node -- npm test`: no chaining, no
/// substitution, no redirection. Such a command runs in the toolbox's own container, which sees the
/// project and nothing else; anything more than that goes into bubblewrap, where the toolbox cannot run.
pub fn plain_toolbox_call(command: &str) -> bool {
    let c = command.trim();
    c.starts_with("genesis-toolbox ") && !c.chars().any(|ch| matches!(ch, ';' | '&' | '|' | '`' | '$' | '<' | '>' | '\n' | '(' | ')'))
}

/// The environment that makes genesis-toolbox run its command sandboxed: the project it may see, whether
/// it may reach the network, and its ceiling on memory.
pub fn toolbox_sandboxed(c: &mut Command, project: &Path, allow_network: bool) {
    c.env("GENESIS_TOOLBOX_PROJECT", project);
    c.env("GENESIS_TOOLBOX_NETWORK", if allow_network { "1" } else { "0" });
    if let Some(max) = run_memory_max() { c.env("GENESIS_TOOLBOX_MEMORY", max.to_string()); }
}

/// Whether the toolbox's container image is already built: the first use downloads it.
pub fn toolbox_image_ready(cmd: &str) -> bool {
    let kind = cmd.split_whitespace().nth(1).unwrap_or("");
    Command::new("podman").args(["image", "exists", &format!("localhost/genesis-{}:44", kind)]).status().map(|s| s.success()).unwrap_or(false)
}

/// A program the model made or runs, started in a scope of its own with a ceiling on its memory. Without
/// one it ran inside the maker's own service, and when a made program ate the machine's memory the kernel
/// killed processes in that service: the maker went down with it, and the next job found nothing there
/// (decision 230). A quarter of the machine, between 512 MB and 4 GB; GENESIS_RUN_MEMORY_MAX overrides.
/// Where there is no user systemd (a container, a test) the program starts as it always did.
pub fn memory_capped(program: &str) -> Command {
    match (run_memory_max(), user_systemd()) {
        (Some(max), true) => {
            let mut c = Command::new("systemd-run");
            c.args(["--user", "--scope", "--quiet", "--collect", "-p", &format!("MemoryMax={}", max), "-p", "MemorySwapMax=0", "--", program]);
            c
        }
        _ => Command::new(program),
    }
}

fn run_memory_max() -> Option<u64> {
    if let Some(v) = std::env::var("GENESIS_RUN_MEMORY_MAX").ok().and_then(|v| v.parse::<u64>().ok()) { return Some(v); }
    let kb: u64 = std::fs::read_to_string("/proc/meminfo").ok()?
        .lines().find(|l| l.starts_with("MemTotal"))?.split_whitespace().nth(1)?.parse().ok()?;
    Some((kb * 1024 / 4).clamp(512 << 20, 4 << 30))
}

fn user_systemd() -> bool {
    cfg!(target_os = "linux") && which("systemd-run")
        && std::env::var("XDG_RUNTIME_DIR").map(|d| std::path::Path::new(&d).join("systemd/private").exists()).unwrap_or(false)
}

pub fn run_shell(project: &Path, command: &str, allow_network: bool, timeout: Duration, stop: Option<&std::sync::atomic::AtomicBool>) -> Result<ShellResult> {
    let sandboxed = bwrap_available() || plain_toolbox_call(command);
    // On Linux the sandbox is required: without bubblewrap a command the model wrote ran with the person's
    // whole account, and the deny list in front of it is advice to a language model, not a wall. A
    // developer's Mac has no bubblewrap and keeps running unsandboxed; GENESIS_ALLOW_UNSANDBOXED=1 says so
    // on purpose anywhere else.
    if !sandboxed && cfg!(target_os = "linux") && !cfg!(test) && std::env::var("GENESIS_ALLOW_UNSANDBOXED").map(|v| v != "1").unwrap_or(true) {
        anyhow::bail!("not run: the sandbox (bubblewrap) is not on this machine, and Genesis does not run a command the model wrote without it. Reinstalling the image restores it.");
    }
    let mut cmd = if plain_toolbox_call(command) && cfg!(target_os = "linux") {
        // a toolbox command runs in the toolbox's own sandbox: a container with the project and nothing else
        let mut c = Command::new("/bin/sh");
        c.args(["-lc", command]).current_dir(project);
        toolbox_sandboxed(&mut c, project, allow_network);
        c
    } else if sandboxed {
        let mut c = memory_capped("bwrap");
        c.args(bwrap_args(project, allow_network));
        c.args(["/bin/sh", "-lc", command]);
        c
    } else {
        let mut c = memory_capped("/bin/sh");
        c.args(["-lc", command]);
        c.current_dir(project);
        c.env("GENESIS_SANDBOX", "0");
        c
    };
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    // its own process group: killing the shell alone leaves `npm install` or a server running, and its
    // still-open pipe then blocks the read below (a `sleep 30` outlived Stop by the full 30 s in CI)
    #[cfg(unix)]
    { use std::os::unix::process::CommandExt; cmd.process_group(0); }
    let mut child = cmd.spawn()?;
    let pid = child.id();
    // the pipes are drained while the command runs: a command printing more than a pipe buffer would
    // otherwise stall until the timeout, whatever the loop below decides
    let (mut out_pipe, mut err_pipe) = (child.stdout.take(), child.stderr.take());
    let reader = |mut p: Option<std::process::ChildStdout>| std::thread::spawn(move || { let mut v = Vec::new(); if let Some(r) = p.as_mut() { use std::io::Read; let _ = r.take(4 << 20).read_to_end(&mut v); } v });
    let out_t = reader(out_pipe.take());
    let err_t = std::thread::spawn(move || { let mut v = Vec::new(); if let Some(r) = err_pipe.as_mut() { use std::io::Read; let _ = r.take(1 << 20).read_to_end(&mut v); } v });
    let start = std::time::Instant::now();
    let mut timed_out = false;
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if start.elapsed() > timeout || stop.map(|s| s.load(std::sync::atomic::Ordering::SeqCst)).unwrap_or(false) {
            kill_group(pid);
            let _ = child.kill();
            timed_out = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let status = child.wait()?;
    let stdout = out_t.join().unwrap_or_default();
    let stderr = err_t.join().unwrap_or_default();
    Ok(ShellResult {
        exit_code: status.code().unwrap_or(-1),
        stdout: truncate(String::from_utf8_lossy(&stdout).to_string(), 16_000),
        stderr: truncate(String::from_utf8_lossy(&stderr).to_string(), 4_000),
        sandboxed,
        timed_out,
    })
}

/// Kill the whole process group of a command Genesis started: the shell and everything it started.
/// The syscall, not the `kill` program: on a minimal Linux `kill` is only a shell builtin and there is
/// no /bin/kill, so the group survived and its open pipe blocked the read (found in CI, 18 Sep).
fn kill_group(pid: u32) {
    #[cfg(unix)]
    unsafe {
        let pgid = -(pid as i32);
        libc::kill(pgid, libc::SIGTERM);
        std::thread::sleep(Duration::from_millis(200));
        libc::kill(pgid, libc::SIGKILL);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sandbox_hands_over_none_of_this_session_environment() {
        let a = bwrap_args(std::path::Path::new("/tmp/p"), false);
        // a command the model runs used to inherit whatever the desktop session carried, including an
        // API key someone had exported and the path to this daemon's own token
        assert!(a.iter().any(|x| x == "--clearenv"), "{:?}", a);
        assert!(a.iter().any(|x| x == "--cap-drop"), "{:?}", a);
        // and what it is given instead is short and carries nothing secret
        let set: Vec<&str> = a.windows(2).filter(|w| w[0] == "--setenv").map(|w| w[1].as_str()).collect();
        for k in ["HOME", "PATH", "GENESIS_SANDBOX"] {
            assert!(set.contains(&k), "{} should be set: {:?}", k, set);
        }
        for k in ["XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS", "ANTHROPIC_API_KEY", "GENESIS_TOKEN", "SSH_AUTH_SOCK"] {
            assert!(!set.contains(&k), "{} must never be handed over: {:?}", k, set);
        }
        // with no network asked for, there is none
        assert!(a.iter().any(|x| x == "--unshare-net"));
        assert!(!bwrap_args(std::path::Path::new("/tmp/p"), true).iter().any(|x| x == "--unshare-net"));
    }

    #[test]
    fn stop_kills_the_command_and_everything_it_started() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        std::thread::spawn(move || { std::thread::sleep(Duration::from_millis(300)); s2.store(true, Ordering::SeqCst); });
        let t0 = std::time::Instant::now();
        // the shell starts a child and waits for it: killing the shell alone would leave the sleep running
        // and its open pipe would block the read for the full 40 s
        let r = run_shell(dir.path(), "sleep 40 & wait", false, Duration::from_secs(120), Some(&stop)).unwrap();
        assert!(t0.elapsed() < Duration::from_secs(10), "took {:?}", t0.elapsed());
        assert!(r.timed_out);
    }

    #[test]
    fn a_command_that_prints_a_lot_does_not_stall() {
        let dir = tempfile::tempdir().unwrap();
        let t0 = std::time::Instant::now();
        let r = run_shell(dir.path(), "head -c 400000 /dev/zero | tr '\\0' 'x'", false, Duration::from_secs(30), None).unwrap();
        assert!(!r.timed_out, "more output than a pipe buffer must not hit the timeout");
        assert!(t0.elapsed() < Duration::from_secs(10));
        assert!(r.stdout.len() >= 16_000 - 100);
    }
}
