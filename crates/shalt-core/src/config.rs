use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const CONFIG_NAME: &str = "shalt.toml";
pub const FORMATS: &[&str] = &["shalt", "cucumber-json", "cucumber-messages"];

#[derive(Debug, Clone)]
pub struct Preset {
    pub label: &'static str,
    pub command: &'static str,
    pub format: &'static str,
    pub report: &'static str,
    pub src: &'static str,
    pub steps: &'static str,
    pub note: &'static str,
}

/// Product default. Supported stacks: rust and javascript.
pub const DEFAULT_STACK: &str = "rust";

/// Support order. rust and javascript are first-class. Other cucumber-family
/// presets still *run*; python is next, then the rest.
pub const STACK_SUPPORT_ORDER: &[&str] = &[
    "rust",
    "javascript",
    "python",
    "go",
    "java",
    "ruby",
    "dotnet",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackSupport {
    /// We write prompts, scaffold, and stand behind this stack.
    Supported,
    /// Next in line. Preset exists; we do not treat failures as product bugs yet.
    Next,
    /// Should run if the toolchain is present. Not supported.
    Later,
}

pub fn stack_support(stack: &str) -> Option<StackSupport> {
    match stack {
        "rust" | "javascript" => Some(StackSupport::Supported),
        "python" => Some(StackSupport::Next),
        "go" | "java" | "ruby" | "dotnet" => Some(StackSupport::Later),
        _ => None,
    }
}

pub fn stack_is_set(stack: &str) -> bool {
    let s = stack.trim();
    !s.is_empty() && preset(s).is_some()
}

pub fn stack_support_note(stack: &str) -> Option<&'static str> {
    match stack_support(stack)? {
        StackSupport::Supported => None,
        StackSupport::Next => Some(
            "python is next after rust and javascript. The preset should run; it is not the supported stack yet.",
        ),
        StackSupport::Later => Some(
            "rust and javascript are the supported stacks. This preset should run if the toolchain is present; it is not supported yet.",
        ),
    }
}

pub fn preset(stack: &str) -> Option<Preset> {
    Some(match stack {
        "rust" => Preset {
            label: "Rust",
            command: "cargo test --test shalt",
            format: "cucumber-json",
            report: ".shalt/cucumber.json",
            src: "src",
            steps: "tests",
            note: "Report path via [runner].env SHALT_REPORT.",
        },
        "javascript" => Preset {
            label: "JavaScript",
            command: "npx cucumber-js {spec} --import {steps}/**/*.js --format message:{report}",
            format: "cucumber-messages",
            report: ".shalt/messages.ndjson",
            src: "src",
            steps: "steps",
            note: "Needs Node. cucumber-js ESM; report via cucumber-messages.",
        },
        "python" => Preset {
            label: "Python",
            command: "{python} -m pytest -q --no-header -p shalt_report {steps}",
            format: "shalt",
            report: ".shalt/last_run.json",
            src: "src",
            steps: "steps",
            note: "After rust and javascript. Should run; not supported yet.",
        },
        "go" => Preset {
            label: "Go",
            command: "godog run --format=cucumber --paths={spec} > {report}",
            format: "cucumber-json",
            report: ".shalt/cucumber.json",
            src: "internal",
            steps: "features",
            note: "Should run; not supported yet.",
        },
        "java" => Preset {
            label: "Java",
            command: "mvn -q test -Dcucumber.features={spec} -Dcucumber.plugin=json:{report}",
            format: "cucumber-json",
            report: "target/cucumber.json",
            src: "src/main/java",
            steps: "src/test/java",
            note: "Should run; not supported yet.",
        },
        "ruby" => Preset {
            label: "Ruby",
            command: "bundle exec cucumber {spec} -r {steps} --format json --out {report}",
            format: "cucumber-json",
            report: ".shalt/cucumber.json",
            src: "lib",
            steps: "features/step_definitions",
            note: "Should run; not supported yet.",
        },
        "dotnet" => Preset {
            label: ".NET",
            command: "dotnet test -- Reqnroll.Output.Cucumber={report}",
            format: "cucumber-json",
            report: ".shalt/cucumber.json",
            src: "src",
            steps: "Tests",
            note: "Should run; not supported yet.",
        },
        _ => return None,
    })
}

#[derive(Debug, Clone)]
pub struct Config {
    pub command: String,
    pub format: String,
    pub report: String,
    pub src: String,
    pub steps: String,
    pub stack: String,
    pub name: String,
    pub timeout: u64,
    pub env: HashMap<String, String>,
    /// Yolo empty/non-answers retry this many times before parking for a human.
    pub guess_tries: u32,
}

impl Default for Config {
    fn default() -> Self {
        let p = preset(DEFAULT_STACK).unwrap();
        Self {
            command: p.command.into(),
            format: p.format.into(),
            report: p.report.into(),
            src: p.src.into(),
            steps: p.steps.into(),
            stack: DEFAULT_STACK.into(),
            name: String::new(),
            timeout: 900,
            env: HashMap::new(),
            guess_tries: 5,
        }
    }
}

#[derive(Deserialize, Default)]
struct Raw {
    #[serde(default)]
    project: HashMap<String, String>,
    #[serde(default)]
    runner: RunnerRaw,
    #[serde(default)]
    zones: HashMap<String, String>,
    #[serde(default)]
    ask: AskRaw,
}

#[derive(Deserialize, Default)]
struct AskRaw {
    guess_tries: Option<u32>,
}

#[derive(Deserialize, Default)]
struct RunnerRaw {
    command: Option<String>,
    format: Option<String>,
    report: Option<String>,
    timeout: Option<u64>,
    #[serde(default)]
    env: HashMap<String, String>,
}

pub fn guess_tries_for(root: &Path) -> u32 {
    if let Ok(s) = std::env::var("SHALT_ASK_GUESS_TRIES") {
        if let Ok(n) = s.parse::<u32>() {
            if n > 0 {
                return n;
            }
        }
    }
    Config::load(root)
        .map(|c| c.guess_tries)
        .unwrap_or(5)
        .max(1)
}

impl Config {
    pub fn load(root: &Path) -> Result<Self, String> {
        let path = root.join(CONFIG_NAME);
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let raw: Raw = toml::from_str(&text).map_err(|e| e.to_string())?;
        let mut cfg = Self::default();
        if let Some(c) = raw.runner.command {
            cfg.command = c;
        }
        if let Some(f) = raw.runner.format {
            cfg.format = f;
        }
        if let Some(r) = raw.runner.report {
            cfg.report = r;
        }
        if let Some(s) = raw.zones.get("src") {
            cfg.src = s.clone();
        }
        if let Some(s) = raw.zones.get("steps") {
            cfg.steps = s.clone();
        }
        if let Some(s) = raw.project.get("stack") {
            cfg.stack = s.clone();
        }
        if let Some(n) = raw.project.get("name") {
            cfg.name = n.clone();
        }
        if let Some(t) = raw.runner.timeout {
            cfg.timeout = t;
        }
        if let Some(n) = raw.ask.guess_tries {
            if n > 0 {
                cfg.guess_tries = n;
            }
        }
        cfg.env = raw.runner.env;
        if !FORMATS.contains(&cfg.format.as_str()) {
            return Err(format!(
                "unknown runner format {:?} in {CONFIG_NAME}; expected one of {}",
                cfg.format,
                FORMATS.join(", ")
            ));
        }
        Ok(cfg)
    }

    pub fn placeholders(&self, root: &Path) -> HashMap<String, String> {
        HashMap::from([
            ("spec".into(), root.join("spec").display().to_string()),
            ("steps".into(), root.join(&self.steps).display().to_string()),
            ("src".into(), root.join(&self.src).display().to_string()),
            ("report".into(), root.join(&self.report).display().to_string()),
            ("root".into(), root.display().to_string()),
            ("python".into(), python_venv_bin(root).display().to_string()),
        ])
    }

    pub fn subst(&self, s: &str, root: &Path) -> String {
        let ph = self.placeholders(root);
        let mut out = s.to_string();
        for (k, v) in ph {
            out = out.replace(&format!("{{{k}}}"), &v);
        }
        out
    }

    pub fn uses_shell(&self) -> bool {
        self.command.contains('|') || self.command.contains('>') || self.command.contains("&&")
    }
}

pub fn write_config(root: &Path, stack: &str, name: &str) -> Result<Preset, String> {
    let p = preset(stack).ok_or_else(|| format!("unknown stack {stack:?}"))?;
    let env_block = if stack == "rust" || stack == "python" {
        "\n[runner.env]\nSHALT_REPORT = \"{report}\"\n"
    } else {
        ""
    };
    let name = if name.is_empty() {
        root.file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "project".into())
    } else {
        name.to_string()
    };
    let body = format!(
        r#"# shalt workspace configuration
[project]
name = "{name}"
stack = "{stack}"

[zones]
steps = "{steps}"
src = "{src}"

[runner]
command = "{command}"
format = "{format}"
report = "{report}"
timeout = 900
{env_block}
[ui]
editor = ""
color = "auto"
"#,
        steps = p.steps,
        src = p.src,
        command = p.command,
        format = p.format,
        report = p.report,
    );
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    std::fs::write(root.join(CONFIG_NAME), body).map_err(|e| e.to_string())?;
    Ok(p)
}

pub fn detect_stack(root: &Path) -> &'static str {
    if root.join("Cargo.toml").exists() {
        "rust"
    } else if root.join("package.json").exists() {
        "javascript"
    } else if root.join("go.mod").exists() {
        "go"
    } else if root.join("pom.xml").exists() || root.join("build.gradle").exists() {
        "java"
    } else if root.join("Gemfile").exists() {
        "ruby"
    } else {
        "rust"
    }
}

/// Scaffold shalt files without clobbering an existing repo's source or config.
pub fn ensure_workspace(root: &Path, stack: &str, name: &str) -> Result<Preset, String> {
    let had_config = root.join(CONFIG_NAME).exists();
    let preset = if had_config {
        preset(stack).ok_or_else(|| format!("unknown stack {stack:?}"))?
    } else {
        write_config(root, stack, name)?
    };
    let cfg = Config::load(root).unwrap_or_default();
    for d in ["spec", "contract", ".shalt"] {
        std::fs::create_dir_all(root.join(d)).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(root.join(&cfg.steps)).map_err(|e| e.to_string())?;
    if !root.join(&cfg.src).exists() {
        std::fs::create_dir_all(root.join(&cfg.src)).map_err(|e| e.to_string())?;
    }
    if stack == "python" {
        write_python_reporter(root)?;
        write_python_requirements(root, false)?;
        ignore_venv(root)?;
    }
    if stack == "rust" {
        write_rust_scaffold(root, name, false)?;
    }
    if stack == "javascript" {
        write_js_scaffold(root, name, false)?;
    }
    let gi = root.join(".shalt/.gitignore");
    if !gi.exists() {
        std::fs::write(&gi, "stage/\nbackup/\nlast_run.json\nmessages.ndjson\ncucumber.json\n")
            .map_err(|e| e.to_string())?;
    }
    let led = root.join(".shalt/ledger.json");
    if !led.exists() {
        crate::ledger::Ledger::default()
            .save(&led)
            .map_err(|e| e.to_string())?;
    }
    Ok(preset)
}

pub fn init_workspace(root: &Path, stack: &str, name: &str) -> Result<Preset, String> {
    let preset = write_config(root, stack, name)?;
    let cfg = Config::load(root).unwrap_or_default();
    for d in ["spec", "contract", ".shalt"] {
        std::fs::create_dir_all(root.join(d)).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(root.join(&cfg.steps)).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(root.join(&cfg.src)).map_err(|e| e.to_string())?;
    if stack == "python" {
        write_python_reporter(root)?;
        write_python_requirements(root, true)?;
        ignore_venv(root)?;
    }
    if stack == "rust" {
        write_rust_scaffold(root, name, false)?;
    }
    if stack == "javascript" {
        write_js_scaffold(root, name, false)?;
    }
    std::fs::write(
        root.join(".shalt/.gitignore"),
        "stage/\nbackup/\nlast_run.json\nmessages.ndjson\ncucumber.json\n",
    )
    .map_err(|e| e.to_string())?;
    crate::ledger::Ledger::default()
        .save(&root.join(".shalt/ledger.json"))
        .map_err(|e| e.to_string())?;
    Ok(preset)
}

/// Spec + plan only. No language, no Cargo.toml. Pick a stack before Play writes tests.
pub fn init_plan_workspace(root: &Path, name: &str) -> Result<(), String> {
    let name = if name.is_empty() {
        root.file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "project".into())
    } else {
        name.to_string()
    };
    let body = format!(
        r#"# shalt workspace configuration
[project]
name = "{name}"
stack = ""

[zones]
steps = "tests"
src = "src"
"#
    );
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    std::fs::write(root.join(CONFIG_NAME), body).map_err(|e| e.to_string())?;
    for d in ["spec", "contract", ".shalt"] {
        std::fs::create_dir_all(root.join(d)).map_err(|e| e.to_string())?;
    }
    std::fs::write(
        root.join(".shalt/.gitignore"),
        "stage/\nbackup/\nlast_run.json\nmessages.ndjson\ncucumber.json\n",
    )
    .map_err(|e| e.to_string())?;
    crate::ledger::Ledger::default()
        .save(&root.join(".shalt/ledger.json"))
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn python_venv_bin(root: &Path) -> PathBuf {
    if cfg!(windows) {
        root.join(".venv").join("Scripts").join("python.exe")
    } else {
        root.join(".venv").join("bin").join("python")
    }
}

pub const PYTHON_REQUIREMENTS: &str = "pytest>=8\npytest-bdd>=8\n";

fn write_python_requirements(root: &Path, overwrite: bool) -> Result<(), String> {
    let req = root.join("requirements.txt");
    if overwrite || !req.exists() {
        std::fs::write(&req, PYTHON_REQUIREMENTS).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn ignore_venv(root: &Path) -> Result<(), String> {
    let gi = root.join(".gitignore");
    let extra = ".venv/\n";
    if gi.exists() {
        let cur = std::fs::read_to_string(&gi).map_err(|e| e.to_string())?;
        if !cur.lines().any(|l| l.trim() == ".venv/" || l.trim() == ".venv") {
            std::fs::write(&gi, cur + extra).map_err(|e| e.to_string())?;
        }
    } else {
        std::fs::write(&gi, extra).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn crate_name(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    while s.starts_with('-') {
        s.remove(0);
    }
    if s.is_empty() || s.as_bytes()[0].is_ascii_digit() {
        s = format!("app-{s}");
    }
    s
}

fn rust_cargo_toml(crate_name: &str) -> String {
    format!(
        r#"[package]
name = "{crate_name}"
version = "0.1.0"
edition = "2021"

[dev-dependencies]
cucumber = {{ version = "0.23", features = ["output-json"] }}
futures = "0.3"

[[test]]
name = "shalt"
harness = false
"#
    )
}

fn write_rust_scaffold(root: &Path, name: &str, overwrite: bool) -> Result<(), String> {
    let raw = if name.is_empty() {
        root.file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "app".into())
    } else {
        name.to_string()
    };
    let crate_name = crate_name(&raw);
    let cargo = root.join("Cargo.toml");
    if overwrite || !cargo.exists() {
        std::fs::write(&cargo, rust_cargo_toml(&crate_name)).map_err(|e| e.to_string())?;
    }
    let lib = root.join("src/lib.rs");
    if overwrite || !lib.exists() {
        std::fs::create_dir_all(root.join("src")).map_err(|e| e.to_string())?;
        std::fs::write(&lib, "//! Application crate. The implementer writes this zone.\n")
            .map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(root.join("tests")).map_err(|e| e.to_string())?;
    Ok(())
}

fn js_package_json(name: &str) -> String {
    format!(
        r#"{{
  "name": "{name}",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "scripts": {{
    "test": "cucumber-js"
  }},
  "devDependencies": {{
    "@cucumber/cucumber": "^11"
  }}
}}
"#
    )
}

fn write_js_scaffold(root: &Path, name: &str, overwrite: bool) -> Result<(), String> {
    let raw = if name.is_empty() {
        root.file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "app".into())
    } else {
        name.to_string()
    };
    let pkg_name = crate_name(&raw);
    let pkg = root.join("package.json");
    if overwrite || !pkg.exists() {
        std::fs::write(&pkg, js_package_json(&pkg_name)).map_err(|e| e.to_string())?;
    }
    let index = root.join("src/index.js");
    if overwrite || !index.exists() {
        std::fs::create_dir_all(root.join("src")).map_err(|e| e.to_string())?;
        std::fs::write(
            &index,
            "/** Application module. The implementer writes this zone. */\nexport {};\n",
        )
        .map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(root.join("steps")).map_err(|e| e.to_string())?;
    ignore_node_modules(root)?;
    Ok(())
}

fn ignore_node_modules(root: &Path) -> Result<(), String> {
    let gi = root.join(".gitignore");
    let extra = "node_modules/\n";
    if gi.exists() {
        let cur = std::fs::read_to_string(&gi).map_err(|e| e.to_string())?;
        if !cur
            .lines()
            .any(|l| l.trim() == "node_modules/" || l.trim() == "node_modules")
        {
            std::fs::write(&gi, cur + extra).map_err(|e| e.to_string())?;
        }
    } else {
        std::fs::write(&gi, extra).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct StackChoice {
    pub id: &'static str,
    pub label: &'static str,
    pub support: &'static str,
}

pub fn stack_choices() -> Vec<StackChoice> {
    STACK_SUPPORT_ORDER
        .iter()
        .filter_map(|id| {
            let p = preset(id)?;
            let support = match stack_support(id)? {
                StackSupport::Supported => "supported",
                StackSupport::Next => "next",
                StackSupport::Later => "later",
            };
            Some(StackChoice {
                id,
                label: p.label,
                support,
            })
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct RestackReport {
    pub from: String,
    pub to: String,
    pub removed: Vec<String>,
    pub label: String,
    pub note: String,
}

const RUST_CONTRACT_STUB: &str = "# Public API\n\n\
The implementer writes `src/`. Step definitions import only this surface.\n\
This workspace is Rust — do not invent a Python package.\n";

const JS_CONTRACT_STUB: &str = "# Public API\n\n\
The implementer writes `src/`. Step definitions import only this surface.\n\
This workspace is JavaScript (cucumber-js, ESM) — do not invent a Python package or a Rust crate.\n";

/// Change the build language. Spec and plan stay. Leftover files from the old
/// stack (Python `steps/`, pytest contract, …) are removed. Does not Play.
pub fn restack(root: &Path, stack: &str) -> Result<RestackReport, String> {
    let stack = stack.trim();
    let p = preset(stack).ok_or_else(|| format!("unknown stack {stack:?}"))?;
    let cfg = Config::load(root).unwrap_or_default();
    let from = if cfg.stack.is_empty() {
        "unknown".into()
    } else {
        cfg.stack.clone()
    };
    let name = if cfg.name.is_empty() {
        root.file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "project".into())
    } else {
        cfg.name.clone()
    };
    write_config(root, stack, &name)?;
    let cfg = Config::load(root).unwrap_or_default();
    std::fs::create_dir_all(root.join(&cfg.steps)).map_err(|e| e.to_string())?;
    if !root.join(&cfg.src).exists() {
        std::fs::create_dir_all(root.join(&cfg.src)).map_err(|e| e.to_string())?;
    }
    if stack == "rust" {
        write_rust_scaffold(root, &name, false)?;
    }
    if stack == "javascript" {
        write_js_scaffold(root, &name, false)?;
    }
    if stack == "python" {
        write_python_reporter(root)?;
        write_python_requirements(root, false)?;
        ignore_venv(root)?;
    }
    let removed = strip_foreign_artifacts(root, stack)?;
    let mut note = if from == stack {
        format!("Already {stack}.")
    } else {
        format!("Stack {from} → {stack}.")
    };
    if !removed.is_empty() {
        note.push_str(" Removed leftover ");
        note.push_str(&removed.join(", "));
        note.push('.');
    }
    if let Some(n) = stack_support_note(stack) {
        note.push(' ');
        note.push_str(n);
    }
    if stack == "rust" {
        note.push_str(" Play writes tests/shalt.rs.");
    }
    if stack == "javascript" {
        note.push_str(" Play writes steps/.");
    }
    Ok(RestackReport {
        from,
        to: stack.into(),
        removed,
        label: p.label.into(),
        note,
    })
}

fn strip_foreign_artifacts(root: &Path, keep: &str) -> Result<Vec<String>, String> {
    let mut removed = Vec::new();
    if keep != "python" {
        for rel in [
            "requirements.txt",
            "pytest.ini",
            "conftest.py",
            ".shalt/shalt_report.py",
        ] {
            remove_file(root.join(rel), rel, &mut removed)?;
        }
        if keep == "javascript" {
            remove_python_files_in(root.join("steps"), "steps", &mut removed)?;
        } else {
            remove_python_tree(root.join("steps"), "steps/", &mut removed)?;
        }
        reset_python_contract(root, keep, &mut removed)?;
        remove_python_files_in(root.join("src"), "src", &mut removed)?;
    }
    Ok(removed)
}

fn remove_file(path: PathBuf, rel: &str, removed: &mut Vec<String>) -> Result<(), String> {
    if path.is_file() {
        std::fs::remove_file(&path).map_err(|e| format!("{rel}: {e}"))?;
        removed.push(rel.into());
    }
    Ok(())
}

fn remove_python_tree(dir: PathBuf, rel: &str, removed: &mut Vec<String>) -> Result<(), String> {
    if !dir.is_dir() {
        return Ok(());
    }
    if dir_has_python(&dir) {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("{rel}: {e}"))?;
        removed.push(rel.into());
    }
    Ok(())
}

fn remove_python_files_in(dir: PathBuf, rel: &str, removed: &mut Vec<String>) -> Result<(), String> {
    if !dir.is_dir() {
        return Ok(());
    }
    let rd = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(_) => return Ok(()),
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name();
        let n = name.to_string_lossy();
        if p.is_file() && (n.ends_with(".py") || n == "conftest.py") {
            std::fs::remove_file(&p).map_err(|e| format!("{rel}/{n}: {e}"))?;
            removed.push(format!("{rel}/{n}"));
        }
    }
    Ok(())
}

fn dir_has_python(dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return false;
    };
    rd.flatten().any(|e| {
        let n = e.file_name();
        let n = n.to_string_lossy();
        n.ends_with(".py") || n == "__pycache__"
    })
}

fn reset_python_contract(root: &Path, keep: &str, removed: &mut Vec<String>) -> Result<(), String> {
    let p = root.join("contract/interface.md");
    if !p.exists() {
        return Ok(());
    }
    let body = std::fs::read_to_string(&p).unwrap_or_default();
    let pythonish = body.contains("```python")
        || body.contains("PYTHONPATH")
        || body.contains("from mrp")
        || body.contains("pytest")
        || body.contains("pytest_bdd");
    if pythonish {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let stub = if keep == "javascript" {
            JS_CONTRACT_STUB
        } else {
            RUST_CONTRACT_STUB
        };
        std::fs::write(&p, stub).map_err(|e| e.to_string())?;
        removed.push("contract/interface.md".into());
    }
    Ok(())
}

fn write_python_reporter(root: &Path) -> Result<(), String> {
    std::fs::create_dir_all(root.join(".shalt")).map_err(|e| e.to_string())?;
    let dest = root.join(".shalt/shalt_report.py");
    if dest.exists() {
        return Ok(());
    }
    std::fs::write(dest, python_reporter_template()).map_err(|e| e.to_string())
}

/// Pytest plugin. Lives in `.shalt/`, not `steps/`, so agents do not treat it as tests.
pub fn python_reporter_template() -> &'static str {
    r#"# shalt pytest reporter — lives in .shalt/, not the stepwright's zone
import json, os, re, sys
from pathlib import Path

RID_RE = re.compile(r"^rid:(S-[0-9a-f]{8})$")
_STATE = {"node_to_rid": {}, "outcomes": {}, "report": None}

src = Path(__file__).resolve().parents[1] / "src"  # workspace root / src
if str(src) not in sys.path:
    sys.path.insert(0, str(src))

def pytest_configure(config):
    _STATE["report"] = os.environ.get("SHALT_REPORT")

try:
    import pytest_bdd  # noqa: F401
except ImportError:
    pytest_bdd = None

if pytest_bdd is not None:
    def pytest_bdd_before_scenario(request, feature, scenario):
        for tag in getattr(scenario, "tags", None) or []:
            name = str(tag).strip().lstrip("@")
            m = RID_RE.fullmatch(name)
            if m:
                _STATE["node_to_rid"][request.node.nodeid] = m.group(1)
                return

def pytest_runtest_logreport(report):
    if report.when != "call" and not (report.when == "setup" and report.failed):
        return
    rid = _STATE["node_to_rid"].get(report.nodeid)
    if not rid:
        return
    outcome = "passed" if report.passed else "failed"
    detail = str(report.longrepr) if report.failed else ""
    prev = _STATE["outcomes"].get(rid)
    if prev is None or (prev["outcome"] == "passed" and outcome == "failed"):
        _STATE["outcomes"][rid] = {"outcome": outcome, "detail": detail, "nodeid": report.nodeid}

def pytest_sessionfinish(session, exitstatus):
    path = _STATE["report"]
    if not path:
        return
    p = Path(path)
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps({
        "results": _STATE["outcomes"],
        "exitstatus": int(exitstatus),
    }, indent=2), encoding="utf-8")
"#
}
