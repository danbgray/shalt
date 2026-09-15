use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

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

pub fn preset(stack: &str) -> Option<Preset> {
    Some(match stack {
        "python" => Preset {
            label: "Python / pytest-bdd",
            command: "python3 -m pytest -q --no-header -p shalt_report {steps}",
            format: "shalt",
            report: ".shalt/last_run.json",
            src: "src",
            steps: "steps",
            note: "Reporter is .shalt/shalt_report.py (not in steps/).",
        },
        "javascript" => Preset {
            label: "JavaScript / cucumber-js",
            command: "npx cucumber-js {spec} --require {steps} --format message:{report}",
            format: "cucumber-messages",
            report: ".shalt/messages.ndjson",
            src: "src",
            steps: "steps",
            note: "Needs @cucumber/cucumber.",
        },
        "rust" => Preset {
            label: "Rust / cucumber-rs",
            command: "cargo test --test cucumber",
            format: "cucumber-json",
            report: ".shalt/cucumber.json",
            src: "src",
            steps: "tests",
            note: "Report path via [runner].env SHALT_REPORT.",
        },
        "go" => Preset {
            label: "Go / godog",
            command: "godog run --format=cucumber --paths={spec} > {report}",
            format: "cucumber-json",
            report: ".shalt/cucumber.json",
            src: "internal",
            steps: "features",
            note: "",
        },
        "java" => Preset {
            label: "Java / cucumber-jvm",
            command: "mvn -q test -Dcucumber.features={spec} -Dcucumber.plugin=json:{report}",
            format: "cucumber-json",
            report: "target/cucumber.json",
            src: "src/main/java",
            steps: "src/test/java",
            note: "",
        },
        "ruby" => Preset {
            label: "Ruby / cucumber",
            command: "bundle exec cucumber {spec} -r {steps} --format json --out {report}",
            format: "cucumber-json",
            report: ".shalt/cucumber.json",
            src: "lib",
            steps: "features/step_definitions",
            note: "",
        },
        "dotnet" => Preset {
            label: ".NET / Reqnroll",
            command: "dotnet test -- Reqnroll.Output.Cucumber={report}",
            format: "cucumber-json",
            report: ".shalt/cucumber.json",
            src: "src",
            steps: "Tests",
            note: "",
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
}

impl Default for Config {
    fn default() -> Self {
        let p = preset("python").unwrap();
        Self {
            command: p.command.into(),
            format: p.format.into(),
            report: p.report.into(),
            src: p.src.into(),
            steps: p.steps.into(),
            stack: "python".into(),
            name: String::new(),
            timeout: 900,
            env: HashMap::new(),
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
        let req = root.join("requirements.txt");
        if !req.exists() {
            std::fs::write(&req, "pytest>=8\npytest-bdd>=8\n").map_err(|e| e.to_string())?;
        }
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
        std::fs::write(
            root.join("requirements.txt"),
            "pytest>=8\npytest-bdd>=8\n",
        )
        .map_err(|e| e.to_string())?;
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
