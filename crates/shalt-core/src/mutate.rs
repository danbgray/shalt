//! Mutation-test the oracle: break the implementation, see which scenarios notice.

use crate::config::Config;
use crate::runner::run_suite;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize)]
pub struct Mutant {
    pub path: String,
    pub line: usize,
    pub operator: String,
    pub before: String,
    pub after: String,
    #[serde(default)]
    pub killed_by: Vec<String>,
    pub status: String,
}

impl Mutant {
    pub fn describe(&self) -> String {
        format!("{}:{}  {}  {} -> {}", self.path, self.line, self.operator, self.before, self.after)
    }
}

#[derive(Debug, Clone, Default)]
pub struct MutationReport {
    pub mutants: Vec<Mutant>,
    pub baseline_green: Vec<String>,
    pub kills: HashMap<String, i64>,
    pub error: String,
}

impl MutationReport {
    pub fn killed(&self) -> Vec<&Mutant> {
        self.mutants.iter().filter(|m| m.status == "killed").collect()
    }
    pub fn survived(&self) -> Vec<&Mutant> {
        self.mutants.iter().filter(|m| m.status == "survived").collect()
    }
    pub fn invalid(&self) -> Vec<&Mutant> {
        self.mutants.iter().filter(|m| m.status == "invalid").collect()
    }
    pub fn score(&self) -> f64 {
        let considered = self.killed().len() + self.survived().len();
        if considered == 0 {
            0.0
        } else {
            (1000.0 * self.killed().len() as f64 / considered as f64).round() / 10.0
        }
    }
    pub fn vacuous(&self) -> Vec<String> {
        let mut v: Vec<_> = self
            .baseline_green
            .iter()
            .filter(|r| self.kills.get(*r).copied().unwrap_or(0) == 0)
            .cloned()
            .collect();
        v.sort();
        v
    }
    pub fn exercised(&self) -> HashMap<String, HashSet<String>> {
        let mut out: HashMap<String, HashSet<String>> = HashMap::new();
        for m in self.killed() {
            for rid in &m.killed_by {
                out.entry(rid.clone()).or_default().insert(m.path.clone());
            }
        }
        out
    }
    pub fn blind_spots(&self) -> HashMap<String, Vec<Mutant>> {
        let ex = self.exercised();
        let mut out: HashMap<String, Vec<Mutant>> = HashMap::new();
        for m in self.survived() {
            for rid in &self.baseline_green {
                if ex.get(rid).map(|s| s.contains(&m.path)).unwrap_or(false) {
                    out.entry(rid.clone()).or_default().push(m.clone());
                }
            }
        }
        out
    }
    pub fn weak_oracles(&self) -> HashMap<String, String> {
        let mut out = HashMap::new();
        for rid in self.vacuous() {
            out.insert(rid, "detected no mutation at all".into());
        }
        for (rid, ms) in self.blind_spots() {
            out.insert(
                rid,
                format!("ran {} mutated version(s) of code it executes without noticing", ms.len()),
            );
        }
        out
    }
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "score": self.score(),
            "killed": self.killed().len(),
            "survived": self.survived().len(),
            "invalid": self.invalid().len(),
            "baseline_green": self.baseline_green.len(),
            "kills": self.kills,
            "vacuous": self.vacuous(),
            "weak_oracles": self.weak_oracles(),
            "survivors": self.survived().iter().map(|m| m.describe()).collect::<Vec<_>>(),
        })
    }
}

const TEXT_OPS: &[(&str, &str, &str)] = &[
    ("==", "!=", "comparison"),
    ("!=", "==", "comparison"),
    ("<=", ">", "comparison"),
    (">=", "<", "comparison"),
    ("&&", "||", "boolean"),
    ("||", "&&", "boolean"),
    ("True", "False", "boolean-literal"),
    ("False", "True", "boolean-literal"),
    ("true", "false", "boolean-literal"),
    ("false", "true", "boolean-literal"),
    (" and ", " or ", "boolean"),
    (" or ", " and ", "boolean"),
];
const COMMENT_PREFIXES: &[&str] = &["#", "//", "--", "*", "/*"];
const TEXT_EXTENSIONS: &[&str] = &[".py", ".js", ".mjs", ".ts", ".go", ".java", ".rb", ".cs", ".kt", ".rs"];

pub fn text_mutants(root: &Path, src_dir: &Path) -> Vec<(Mutant, String)> {
    let mut out = Vec::new();
    if !src_dir.exists() {
        return out;
    }
    let mut paths: Vec<PathBuf> = walkdir::WalkDir::new(src_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| {
            p.extension()
                .and_then(|s| s.to_str())
                .map(|ext| TEXT_EXTENSIONS.iter().any(|e| e.strip_prefix('.').unwrap_or(e) == ext))
                .unwrap_or(false)
                && !p.components().any(|c| c.as_os_str() == "__pycache__")
        })
        .collect();
    paths.sort();
    for path in paths {
        let Ok(text) = fs::read_to_string(&path) else { continue };
        let lines: Vec<&str> = text.split('\n').collect();
        let lines: Vec<&str> = if lines.last() == Some(&"") {
            lines[..lines.len() - 1].to_vec()
        } else {
            lines
        };
        for (idx, line) in lines.iter().enumerate() {
            let stripped = line.trim();
            if stripped.is_empty() || COMMENT_PREFIXES.iter().any(|p| stripped.starts_with(p)) {
                continue;
            }
            for (pattern, replacement, op) in TEXT_OPS {
                let mut from = 0;
                while let Some(at) = line[from..].find(pattern) {
                    let start = from + at;
                    let end = start + pattern.len();
                    let mut new_line = String::new();
                    new_line.push_str(&line[..start]);
                    new_line.push_str(replacement);
                    new_line.push_str(&line[end..]);
                    let mut mutated = lines.iter().map(|s| s.to_string()).collect::<Vec<_>>();
                    mutated[idx] = new_line;
                    let rel = path
                        .strip_prefix(root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .replace('\\', "/");
                    out.push((
                        Mutant {
                            path: rel,
                            line: idx + 1,
                            operator: (*op).into(),
                            before: (*pattern).into(),
                            after: (*replacement).into(),
                            killed_by: vec![],
                            status: "pending".into(),
                        },
                        mutated.join("\n") + "\n",
                    ));
                    from = end;
                }
            }
        }
    }
    out
}

pub fn run_campaign(root: &Path, cfg: &Config, engine: &str, budget: usize, seed: u64) -> MutationReport {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let src_dir = root.join(&cfg.src);
    let mut report = MutationReport::default();
    let engine = if engine == "auto" {
        if cfg.stack == "python" { "text" } else { "text" }
    } else {
        engine
    };
    if engine != "text" && engine != "python" {
        report.error = format!("unknown mutation engine {engine:?}");
        return report;
    }
    if !src_dir.exists() {
        report.error = format!("no implementation directory at {}/", cfg.src);
        return report;
    }
    let baseline = run_suite(&root, cfg);
    if baseline.harness_error || !baseline.collection_error.is_empty() {
        report.error = "the suite does not run cleanly yet, so there is no baseline to mutate against. Get it green first.".into();
        return report;
    }
    report.baseline_green = baseline
        .results
        .iter()
        .filter(|(_, r)| r.outcome == "passed")
        .map(|(rid, _)| rid.clone())
        .collect();
    report.baseline_green.sort();
    if report.baseline_green.is_empty() {
        report.error = "no scenario is green, so nothing can be shown to detect a mutation.".into();
        return report;
    }
    let mut candidates = text_mutants(&root, &src_dir);
    if candidates.is_empty() {
        report.error = format!("the {engine} engine found nothing to mutate under {}/", cfg.src);
        return report;
    }
    // deterministic shuffle
    {
        let mut rng = seed;
        for i in (1..candidates.len()).rev() {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            let j = (rng as usize) % (i + 1);
            candidates.swap(i, j);
        }
    }
    candidates.truncate(budget);

    for (mut mutant, mutated_text) in candidates {
        let target = root.join(&mutant.path);
        let original = fs::read_to_string(&target).unwrap_or_default();
        let _ = fs::write(&target, &mutated_text);
        let run = run_suite(&root, cfg);
        if run.harness_error
            || !run.collection_error.is_empty()
            || !report.baseline_green.iter().any(|rid| run.results.contains_key(rid))
        {
            mutant.status = "invalid".into();
        } else {
            let killers: Vec<String> = report
                .baseline_green
                .iter()
                .filter(|rid| run.results.get(*rid).map(|r| r.outcome == "failed").unwrap_or(false))
                .cloned()
                .collect();
            mutant.killed_by = killers.clone();
            mutant.status = if killers.is_empty() { "survived" } else { "killed" }.into();
            for rid in killers {
                *report.kills.entry(rid).or_insert(0) += 1;
            }
        }
        let _ = fs::write(&target, original);
        touch(&target);
        report.mutants.push(mutant);
    }
    touch_tree(&src_dir);
    let after = run_suite(&root, cfg);
    let mut still_green: Vec<String> = after
        .results
        .iter()
        .filter(|(_, r)| r.outcome == "passed")
        .map(|(rid, _)| rid.clone())
        .collect();
    still_green.sort();
    if still_green != report.baseline_green {
        report.error = "the workspace did not return to its baseline after mutating, so these results cannot be trusted.".into();
    }
    report
}

fn touch(p: &Path) {
    if let Ok(f) = fs::File::options().write(true).open(p) {
        let _ = f.set_modified(SystemTime::now());
    }
}

fn touch_tree(dir: &Path) {
    for (p, is_link) in crate::integrity::iter_files(dir) {
        if !is_link {
            touch(&p);
        }
    }
}
