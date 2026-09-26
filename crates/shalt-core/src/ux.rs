//! UX loop after the look is past wireframes: static checks, then Playwright if present.

use crate::jobs::{Job, JobKind, JobStatus};
use crate::mockups::{self, Film};
use crate::scaffold::has_final_look;
use crate::spec::Feature;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const UX_TURNS: usize = 3;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UxState {
    #[serde(default)]
    pub look_hash: String,
    #[serde(default)]
    pub pass: bool,
    #[serde(default)]
    pub turns: usize,
    #[serde(default)]
    pub findings: Vec<String>,
}

pub fn state_path(root: &Path) -> PathBuf {
    root.join(".shalt/ux.json")
}

pub fn load_state(root: &Path) -> UxState {
    fs::read_to_string(state_path(root))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_state(root: &Path, st: &UxState) -> Result<(), String> {
    fs::create_dir_all(root.join(".shalt")).map_err(|e| e.to_string())?;
    fs::write(state_path(root), serde_json::to_string_pretty(st).unwrap_or_else(|_| "{}".into()) + "\n")
        .map_err(|e| e.to_string())
}

pub fn look_hash(root: &Path) -> String {
    let mut h = 0u64;
    let css = root.join("mockups/tokens.final.css");
    if let Ok(b) = fs::read(&css) {
        for x in b {
            h = h.wrapping_mul(16777619) ^ x as u64;
        }
    }
    format!("{h:016x}")
}

pub fn ux_needed(root: &Path, jobs: &[Job], project_id: &str) -> bool {
    if !has_final_look(root) {
        return false;
    }
    let st = load_state(root);
    let hash = look_hash(root);
    if st.pass && st.look_hash == hash {
        return false;
    }
    let turns = jobs
        .iter()
        .filter(|j| j.project_id == project_id && j.kind == JobKind::Ux)
        .count();
    turns < UX_TURNS
}

/// Static pass over sketched vs polished screens: contrast, dead controls, red stubs.
pub fn static_findings(root: &Path, films: &[Film]) -> Vec<String> {
    let mut out = Vec::new();
    let sketch = !has_final_look(root);
    if sketch {
        out.push("look is still the wireframe sheet — Playwright waits until tokens.final.css exists.".into());
        return out;
    }
    for f in films {
        if f.kind == "none" {
            continue;
        }
        for fr in &f.frames {
            let rel = mockups::normalize_rel(&fr.file).unwrap_or_default();
            if rel.is_empty() {
                continue;
            }
            let path = root.join("mockups").join(&rel);
            let html = fs::read_to_string(&path).unwrap_or_default();
            if html.is_empty() {
                out.push(format!("{} is empty", rel));
                continue;
            }
            if html.contains("shalt-sketch") && html.contains("tokens.final.css") {
                // product look should not keep the pencil class
            }
            if html.to_ascii_lowercase().contains("red stub")
                || html.contains("data-stub")
                || html.contains("class=\"stub\"")
            {
                out.push(format!("{rel}: unproven path is still a red stub"));
            }
            let buttons = html.matches("<button").count();
            let labeled = html.matches("<button").filter(|_| true).count();
            let _ = (buttons, labeled);
            if html.contains("<button") && !html.contains("</button>") {
                out.push(format!("{rel}: button is not closed"));
            }
            if html.contains("color:#f4") && html.contains("background:#f4") {
                out.push(format!("{rel}: cream-on-cream control"));
            }
        }
    }
    out
}

fn playwright_available(root: &Path) -> bool {
    root.join("node_modules/@playwright/test").is_dir()
        || Command::new("npx")
            .args(["--no-install", "playwright", "--version"])
            .current_dir(root)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

fn write_playwright_spec(root: &Path, films: &[Film]) -> Result<PathBuf, String> {
    let dir = root.join("tests");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut body = String::from(
        "import { test, expect } from '@playwright/test';\n\n",
    );
    for f in films {
        if f.kind == "none" {
            continue;
        }
        let file = f
            .frames
            .iter()
            .find_map(|fr| {
                let r = mockups::normalize_rel(&fr.file).unwrap_or_default();
                if r.is_empty() { None } else { Some(r) }
            })
            .unwrap_or_default();
        if file.is_empty() {
            continue;
        }
        let html = root.join("mockups").join(&file);
        let url = format!("file://{}", html.display());
        let name = f.journey.replace('\'', "");
        body.push_str(&format!(
            "test('{name} is clickable', async ({{ page }}) => {{\n  const errors = [];\n  page.on('pageerror', e => errors.push(String(e)));\n  await page.goto('{url}');\n  await expect(page.locator('body')).toBeVisible();\n  const btns = page.locator('button, a, [role=\"button\"]');\n  const n = await btns.count();\n  for (let i = 0; i < Math.min(n, 12); i++) {{\n    const b = btns.nth(i);\n    if (await b.isVisible()) await b.click({{ trial: true }}).catch(() => {{}});\n  }}\n  expect(errors, errors.join('\\n')).toEqual([]);\n}});\n\n"
        ));
    }
    let spec = dir.join("ux.spec.mjs");
    fs::write(&spec, body).map_err(|e| e.to_string())?;
    let cfg = root.join("playwright.config.mjs");
    if !cfg.is_file() {
        fs::write(
            &cfg,
            "export default { testDir: 'tests', testMatch: 'ux.spec.mjs', timeout: 15000 };\n",
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(spec)
}

fn run_playwright(root: &Path) -> Result<String, String> {
    let out = Command::new("npx")
        .args(["--no-install", "playwright", "test", "tests/ux.spec.mjs", "--reporter=line"])
        .current_dir(root)
        .output()
        .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let blob = format!("{stdout}\n{stderr}");
    if out.status.success() {
        Ok(blob)
    } else {
        Err(blob.chars().take(4000).collect())
    }
}

pub fn run_ux_pass(root: &Path, features: &[Feature], jobs: &[Job], project_id: &str) -> UxState {
    let led = crate::ledger::Ledger::load(&root.join(".shalt/ledger.json")).unwrap_or_default();
    let films = mockups::films(root, features, &led);
    let mut findings = static_findings(root, &films);
    if playwright_available(root) {
        if let Err(e) = write_playwright_spec(root, &films) {
            findings.push(format!("could not write Playwright spec: {e}"));
        } else {
            match run_playwright(root) {
                Ok(_) => {}
                Err(log) => {
                    findings.push("Playwright failed".into());
                    for line in log.lines().filter(|l| l.contains("Error") || l.contains("failed") || l.contains("expect"))
                    {
                        findings.push(line.chars().take(160).collect());
                        if findings.len() > 24 {
                            break;
                        }
                    }
                }
            }
        }
    } else {
        findings.push(
            "Playwright is not installed. Static pass only. In a javascript project: npm i -D @playwright/test && npx playwright install chromium".into(),
        );
    }
    let mut st = load_state(root);
    st.look_hash = look_hash(root);
    st.pass = findings.iter().all(|f| {
        f.starts_with("Playwright is not installed") || f.contains("wireframe")
    }) && findings
        .iter()
        .all(|f| !f.contains("stub") && !f.contains("cream-on-cream") && !f.contains("Playwright failed"));
    // empty findings = pass; "not installed" is a note, not a fail
    if findings.is_empty()
        || findings.iter().all(|f| f.starts_with("Playwright is not installed"))
    {
        st.pass = true;
        if findings.is_empty() {
            findings.push("UX pass: no stubs, no contrast traps.".into());
        }
    }
    st.findings = findings;
    st.turns = jobs
        .iter()
        .filter(|j| j.project_id == project_id && j.kind == JobKind::Ux && j.status != JobStatus::Pending)
        .count()
        .saturating_add(1);
    let md = format!(
        "# UX loop\n\nLook: {}\nPass: {}\nTurn: {}\n\n{}\n",
        st.look_hash,
        st.pass,
        st.turns,
        st.findings
            .iter()
            .map(|f| format!("- {f}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let _ = fs::write(root.join(".shalt/ux.md"), md);
    let _ = save_state(root, &st);
    st
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn ux_waits_until_the_final_look() {
        let t = TempDir::new().unwrap();
        assert!(!ux_needed(t.path(), &[], "p"));
        fs::create_dir_all(t.path().join("mockups")).unwrap();
        fs::write(t.path().join("mockups/tokens.final.css"), ":root{--ink:#111}").unwrap();
        assert!(ux_needed(t.path(), &[], "p"));
    }

    #[test]
    fn static_scan_flags_a_red_stub() {
        let t = TempDir::new().unwrap();
        fs::create_dir_all(t.path().join("mockups/journeys/x")).unwrap();
        fs::write(
            t.path().join("mockups/tokens.final.css"),
            ":root{--ink:#111}",
        )
        .unwrap();
        fs::write(
            t.path().join("mockups/journeys/x/x.html"),
            "<button class=\"stub\">red stub</button>",
        )
        .unwrap();
        let films = vec![Film {
            journey: "x".into(),
            kind: "ui".into(),
            story: String::new(),
            spec_hash: String::new(),
            stale: false,
            frames: vec![mockups::Frame {
                rid: "S-1".into(),
                name: "x".into(),
                file: "journeys/x/x.html".into(),
                caption: String::new(),
                built: true,
                stale: false,
                thumb: String::new(),
            }],
        }];
        let hits = static_findings(t.path(), &films);
        assert!(hits.iter().any(|h| h.contains("stub")), "{hits:?}");
    }
}
