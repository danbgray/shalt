//! The single `shalt ui` process, recorded in ~/.shalt/ui.json.

use crate::org::Org;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

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

fn kill_cmd(sig: &str, pid: u32) -> std::io::Result<std::process::ExitStatus> {
    Command::new("kill")
        .args([sig, &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
}

pub fn pid_alive(pid: u32) -> bool {
    kill_cmd("-0", pid).map(|s| s.success()).unwrap_or(false)
}

/// The running shalt ui, if the recorded pid is alive and `/api/health` answers.
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
    let _ = kill_cmd("-TERM", pid);
    for _ in 0..20 {
        if !pid_alive(pid) {
            clear();
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let _ = kill_cmd("-KILL", pid);
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
