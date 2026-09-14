use crate::config::Config;
use crate::ledger::RunResult;
use crate::reports::read_report;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

pub struct SuiteRun {
    pub results: HashMap<String, RunResult>,
    pub stdout: String,
    pub stderr: String,
    pub run_id: String,
    pub duration: f64,
    pub returncode: i32,
    pub harness_error: bool,
    pub collection_error: String,
}

pub fn run_suite(root: &Path, cfg: &Config) -> SuiteRun {
    let report = root.join(&cfg.report);
    if report.exists() {
        let _ = std::fs::remove_file(&report);
    }
    if let Some(parent) = report.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let started = Instant::now();
    let cmd_s = cfg.subst(&cfg.command, root);
    let mut cmd = if cfg.uses_shell() {
        let mut c = Command::new("sh");
        c.arg("-c").arg(&cmd_s);
        c
    } else {
        let mut parts = shell_words(&cmd_s);
        if parts.is_empty() {
            parts = vec!["false".into()];
        }
        let mut c = Command::new(&parts[0]);
        c.args(&parts[1..]);
        c
    };
    cmd.current_dir(root);
    let mut pythonpath = vec![
        root.display().to_string(),
        root.join(&cfg.src).display().to_string(),
    ];
    if let Ok(existing) = std::env::var("PYTHONPATH") {
        if !existing.is_empty() {
            pythonpath.push(existing);
        }
    }
    cmd.env("PYTHONPATH", pythonpath.join(":"));
    for (k, v) in &cfg.env {
        cmd.env(k, cfg.subst(v, root));
    }
    let output = cmd.output();
    let duration = started.elapsed().as_secs_f64();
    match output {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return SuiteRun {
                results: HashMap::new(),
                stdout: String::new(),
                stderr: format!("runner command not found: {e}. Check [runner].command in shalt.toml."),
                run_id: "run-failed".into(),
                duration: 0.0,
                returncode: 127,
                harness_error: true,
                collection_error: String::new(),
            };
        }
        Err(e) => {
            return SuiteRun {
                results: HashMap::new(),
                stdout: String::new(),
                stderr: e.to_string(),
                run_id: "run-failed".into(),
                duration,
                returncode: 1,
                harness_error: true,
                collection_error: String::new(),
            };
        }
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).chars().rev().take(8000).collect::<String>().chars().rev().collect::<String>();
            let stderr = String::from_utf8_lossy(&out.stderr).chars().rev().take(4000).collect::<String>().chars().rev().collect::<String>();
            let rc = out.status.code().unwrap_or(1);
            let results = read_report(&report, &cfg.format);
            let mut collection_error = String::new();
            if results.is_empty() && rc != 0 {
                let combo = format!("{stdout}{stderr}");
                collection_error = combo.chars().rev().take(3000).collect::<String>().chars().rev().collect();
            }
            SuiteRun {
                results,
                stdout,
                stderr,
                run_id: chrono::Utc::now().format("run-%Y%m%d-%H%M%S").to_string(),
                duration: (duration * 100.0).round() / 100.0,
                returncode: rc,
                harness_error: rc == 3 || rc == 4,
                collection_error,
            }
        }
    }
}

fn shell_words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    for c in s.chars() {
        match c {
            '"' => in_q = !in_q,
            ' ' if !in_q => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn failure_digest(run: &SuiteRun, allowed: Option<&std::collections::HashSet<String>>, limit: usize) -> String {
    let mut parts = Vec::new();
    for (rid, r) in &run.results {
        if let Some(a) = allowed {
            if !a.contains(rid) {
                continue;
            }
        }
        if r.outcome == "failed" {
            parts.push(format!("--- {rid} ({}) ---\n{}", r.nodeid, r.detail.chars().take(1500).collect::<String>()));
        }
        if parts.len() >= limit {
            break;
        }
    }
    if parts.is_empty() {
        parts.push("no visible failing scenario detail available".into());
    }
    parts.join("\n\n")
}

#[allow(dead_code)]
fn _timeout_unused() -> Duration {
    Duration::from_secs(1)
}
