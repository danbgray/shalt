//! The single `shalt ui` process, recorded in ~/.shalt/ui.json.

use crate::jobs::JobQueue;
use crate::org::{Org, YOU_STOPPED};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Child `shalt ui --foreground` so the parent can detach and return.
pub const SERVE_ENV: &str = "SHALT_UI_SERVE";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiInstance {
    pub pid: u32,
    pub port: u16,
    pub url: String,
    pub root: String,
    pub started_at: String,
}

fn path() -> PathBuf {
    Org::home_dir().join("ui.json")
}

pub fn load() -> Option<UiInstance> {
    let p = path();
    if !p.exists() {
        return None;
    }
    serde_json::from_str(&fs::read_to_string(p).unwrap_or_default()).ok()
}

pub fn save(inst: Option<&UiInstance>) -> std::io::Result<()> {
    let p = path();
    if let Some(dir) = p.parent() {
        fs::create_dir_all(dir)?;
    }
    match inst {
        None => {
            let _ = fs::remove_file(&p);
            Ok(())
        }
        Some(u) => fs::write(p, serde_json::to_string_pretty(u)? + "\n"),
    }
}

fn kill_cmd(sig: &str, target: &str) -> std::io::Result<std::process::ExitStatus> {
    Command::new("kill")
        .args([sig, target])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
}

pub fn pid_alive(pid: u32) -> bool {
    kill_cmd("-0", &pid.to_string())
        .map(|s| s.success())
        .unwrap_or(false)
}

/// The running shalt ui, if the recorded pid is alive and `/api/health` answers.
pub fn desk_url() -> Option<String> {
    current().map(|u| u.url.trim_end_matches('/').to_string())
}

pub fn current() -> Option<UiInstance> {
    let u = load()?;
    if !pid_alive(u.pid) {
        let _ = save(None);
        return None;
    }
    if !is_our_ui(u.port) {
        return None;
    }
    Some(u)
}

pub fn record(inst: UiInstance) {
    let _ = save(Some(&inst));
}

pub fn clear() {
    let _ = save(None);
}

/// Who is listening on the port, if we can tell.
pub fn occupant(port: u16) -> Option<(u32, String)> {
    if let Some(u) = current() {
        if u.port == port {
            return Some((u.pid, format!("shalt ui {}", u.url)));
        }
    }
    let out = Command::new("lsof")
        .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-t"])
        .output()
        .ok()?;
    let pid: u32 = String::from_utf8_lossy(&out.stdout).lines().next()?.trim().parse().ok()?;
    let comm = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "?".into());
    Some((pid, comm))
}

pub fn health(port: u16) -> Option<serde_json::Value> {
    let url = format!("http://127.0.0.1:{port}/api/health");
    let r = ureq::get(&url)
        .timeout(std::time::Duration::from_millis(400))
        .call()
        .ok()?;
    r.into_json().ok()
}

pub fn is_our_ui(port: u16) -> bool {
    health(port)
        .and_then(|v| v.get("ok").and_then(|o| o.as_bool()))
        .unwrap_or(false)
}

pub fn stop_pid(pid: u32) -> Result<(), String> {
    if !pid_alive(pid) {
        clear();
        return Ok(());
    }
    let _ = kill_cmd("-TERM", &pid.to_string());
    for _ in 0..20 {
        if !pid_alive(pid) {
            clear();
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let _ = kill_cmd("-KILL", &pid.to_string());
    std::thread::sleep(std::time::Duration::from_millis(100));
    clear();
    if pid_alive(pid) {
        Err(format!("pid {pid} did not exit"))
    } else {
        Ok(())
    }
}

pub fn stop_current() -> Result<Option<UiInstance>, String> {
    let Some(u) = load() else {
        return Ok(None);
    };
    stop_pid(u.pid)?;
    Ok(Some(u))
}

/// One OS process whose argv0 / comm is the shalt binary.
#[derive(Debug, Clone)]
pub struct Proc {
    pub pid: u32,
    pub args: String,
}

/// What `shalt stop` parked and killed.
#[derive(Debug, Clone, Default)]
pub struct StopReport {
    pub procs: Vec<Proc>,
    pub jobs: usize,
    pub projects: usize,
    pub failed: Vec<u32>,
}

fn basename(s: &str) -> &str {
    Path::new(s.trim().trim_matches('"'))
        .file_name()
        .and_then(|x| x.to_str())
        .unwrap_or(s.trim())
}

/// True when this process *is* shalt, not merely mentioning the repo in its command line.
pub fn looks_like_shalt_proc(comm: &str, args: &str) -> bool {
    if basename(comm) == "shalt" {
        return true;
    }
    let argv0 = args.split_whitespace().next().unwrap_or("");
    basename(argv0) == "shalt"
}

fn parse_pid_args_line(line: &str) -> Option<(u32, String)> {
    let line = line.trim();
    let (pid, args) = line.split_once(char::is_whitespace)?;
    let pid: u32 = pid.parse().ok()?;
    Some((pid, args.trim().to_string()))
}

fn parent_pid() -> Option<u32> {
    #[cfg(unix)]
    {
        Some(std::os::unix::process::parent_id())
    }
    #[cfg(not(unix))]
    {
        None
    }
}

fn pgid_of(pid: u32) -> Option<u32> {
    let out = Command::new("ps")
        .args(["-o", "pgid=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

fn comm_of(pid: u32) -> String {
    Command::new("ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn children_of(pid: u32) -> Vec<u32> {
    let out = match Command::new("pgrep")
        .args(["-P", &pid.to_string()])
        .output()
    {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect()
}

fn descendants_of(pid: u32) -> Vec<u32> {
    let mut seen = HashSet::new();
    let mut stack = children_of(pid);
    let mut out = Vec::new();
    while let Some(c) = stack.pop() {
        if !seen.insert(c) {
            continue;
        }
        out.push(c);
        stack.extend(children_of(c));
    }
    out
}

/// Every live shalt binary, including `target/debug/shalt` and a detached UI.
pub fn list_shalt_procs() -> Vec<Proc> {
    let out = match Command::new("ps")
        .args(["-ax", "-o", "pid=", "-o", "args="])
        .output()
    {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    let mut procs = Vec::new();
    let mut seen = HashSet::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let Some((pid, args)) = parse_pid_args_line(line) else {
            continue;
        };
        if !looks_like_shalt_proc("", &args) {
            continue;
        }
        if seen.insert(pid) {
            procs.push(Proc { pid, args });
        }
    }
    procs
}

fn park_all_work() -> (usize, usize) {
    let mut org = Org::load();
    let projects = org.pause_all(YOU_STOPPED);
    let _ = org.save();
    let mut q = JobQueue::load();
    let jobs = q.pause_all("paused — shalt stop\n");
    let _ = q.save();
    (jobs.len(), projects.len())
}

/// Park Play, then SIGTERM/KILL every shalt process except `self_pid` (and its parent).
/// Does not touch Ollama.
pub fn stop_everything(self_pid: u32) -> StopReport {
    let (jobs, projects) = park_all_work();
    let parent = parent_pid();
    let mut procs: Vec<Proc> = list_shalt_procs()
        .into_iter()
        .filter(|p| p.pid != self_pid && Some(p.pid) != parent)
        .collect();
    if let Some(u) = load() {
        if u.pid != self_pid
            && Some(u.pid) != parent
            && pid_alive(u.pid)
            && procs.iter().all(|p| p.pid != u.pid)
        {
            procs.push(Proc {
                pid: u.pid,
                args: format!("shalt ui {}", u.url),
            });
        }
    }
    let shalt_pids: Vec<u32> = procs.iter().map(|p| p.pid).collect();
    let skip: HashSet<u32> = shalt_pids.iter().copied().chain([self_pid]).chain(parent).collect();
    let mut extra = Vec::new();
    for pid in &shalt_pids {
        for kid in descendants_of(*pid) {
            if skip.contains(&kid) || extra.contains(&kid) {
                continue;
            }
            if comm_of(kid) == "ollama" {
                continue;
            }
            extra.push(kid);
        }
    }
    for pid in &shalt_pids {
        if pgid_of(*pid) == Some(*pid) {
            let _ = kill_cmd("-TERM", &format!("-{pid}"));
        } else {
            let _ = kill_cmd("-TERM", &pid.to_string());
        }
    }
    for kid in &extra {
        let _ = kill_cmd("-TERM", &kid.to_string());
    }
    for _ in 0..20 {
        if shalt_pids.iter().all(|p| !pid_alive(*p)) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    for pid in shalt_pids.iter().chain(extra.iter()) {
        if pid_alive(*pid) {
            let _ = kill_cmd("-KILL", &pid.to_string());
        }
    }
    std::thread::sleep(Duration::from_millis(100));
    clear();
    let _ = park_all_work();
    let failed: Vec<u32> = shalt_pids.into_iter().filter(|p| pid_alive(*p)).collect();
    StopReport {
        procs,
        jobs,
        projects,
        failed,
    }
}

pub fn log_path() -> PathBuf {
    Org::home_dir().join("ui.log")
}

/// Spawn `exe ui --foreground` detached. Parent should wait on [`wait_until_up`].
pub fn spawn_detached(exe: &Path, root: &Path, port: Option<u16>) -> Result<u32, String> {
    let log = log_path();
    if let Some(dir) = log.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .map_err(|e| format!("{}: {e}", log.display()))?;
    let err = file.try_clone().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(exe);
    cmd.arg("--root")
        .arg(root)
        .arg("ui")
        .arg("--foreground")
        .arg("--no-open")
        .env(SERVE_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::from(file))
        .stderr(Stdio::from(err));
    if let Some(p) = port {
        cmd.arg("--port").arg(p.to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let child = cmd.spawn().map_err(|e| e.to_string())?;
    Ok(child.id())
}

/// Until `/api/health` answers or `timeout`.
pub fn wait_until_up(timeout: Duration) -> Option<UiInstance> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Some(u) = current() {
            return Some(u);
        }
        std::thread::sleep(Duration::from_millis(80));
    }
    current()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shalt_binary_matches() {
        assert!(looks_like_shalt_proc(
            "shalt",
            "/Users/me/.cargo/bin/shalt ui --foreground"
        ));
        assert!(looks_like_shalt_proc("shalt", "target/debug/shalt play"));
        assert!(looks_like_shalt_proc(
            "",
            "/Users/me/target/debug/shalt --root . play"
        ));
    }

    #[test]
    fn nearby_commands_do_not_match() {
        assert!(!looks_like_shalt_proc("cargo", "cargo test -p shalt-core"));
        assert!(!looks_like_shalt_proc("rg", "rg shalt"));
        assert!(!looks_like_shalt_proc(
            "vim",
            "vim /Users/danielgray/shalt/README.md"
        ));
        assert!(!looks_like_shalt_proc("ollama", "ollama serve"));
        assert!(!looks_like_shalt_proc(
            "zsh",
            "zsh -c cd /Users/danielgray/shalt"
        ));
    }

    #[test]
    fn pid_args_line_splits() {
        let (pid, args) = parse_pid_args_line(
            "  12345 /Users/me/.cargo/bin/shalt ui --foreground",
        )
        .unwrap();
        assert_eq!(pid, 12345);
        assert!(args.ends_with("shalt ui --foreground"));
        assert!(looks_like_shalt_proc("", &args));
    }
}
