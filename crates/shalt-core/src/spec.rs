use crate::narrative::{parse_story, Story};
use regex::Regex;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use thiserror::Error;

pub const RID_PREFIX: &str = "@rid:";
pub const HOLDOUT_TAG: &str = "@holdout";
pub const EPIC_PREFIX: &str = "@epic:";

fn rid_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"@rid:(S-[0-9a-f]{8})").unwrap())
}
fn epic_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^@epic:([A-Za-z0-9_.\-]+)$").unwrap())
}

pub fn new_rid() -> String {
    format!("S-{:08x}", rand::random::<u32>())
}

fn sha_text(text: &str) -> String {
    let mut h = Sha256::new();
    h.update(text.as_bytes());
    format!("sha256:{}", &hex::encode(h.finalize())[..32])
}

pub fn file_hash(p: &Path) -> std::io::Result<String> {
    let bytes = fs::read(p)?;
    let mut h = Sha256::new();
    h.update(&bytes);
    Ok(hex::encode(h.finalize())[..16].to_string())
}

fn norm_tag(t: &str) -> String {
    let t = t.trim();
    if t.starts_with('@') {
        t.to_string()
    } else {
        format!("@{t}")
    }
}

fn step_lines(steps: &[gherkin::Step]) -> Vec<String> {
    let mut out = Vec::new();
    for s in steps {
        let mut line = format!("{} {}", s.keyword.trim(), s.value.trim());
        if let Some(doc) = &s.docstring {
            line.push_str(&format!("\n<<<{}>>>", doc.trim()));
        }
        if let Some(tbl) = &s.table {
            for row in &tbl.rows {
                line.push_str(&format!("\n|{}|", row.join("|")));
            }
        }
        out.push(line);
    }
    out
}

fn examples_lines(examples: &[gherkin::Examples]) -> Vec<String> {
    let mut out = Vec::new();
    for ex in examples {
        if let Some(tbl) = &ex.table {
            for row in &tbl.rows {
                out.push(format!("|{}|", row.join("|")));
            }
        }
    }
    out
}

#[derive(Debug, Clone)]
pub struct Scenario {
    pub rid: Option<String>,
    pub name: String,
    pub keyword: String,
    pub tags: Vec<String>,
    pub steps: Vec<String>,
    pub examples: Vec<String>,
    pub feature_name: String,
    pub feature_file: String,
    pub line: usize,
    pub tag_lines: Vec<usize>,
    pub rid_count: usize,
    pub inherited_tags: Vec<String>,
}

impl Scenario {
    pub fn all_tags(&self) -> Vec<String> {
        let mut t = self.tags.clone();
        for it in &self.inherited_tags {
            if !t.contains(it) {
                t.push(it.clone());
            }
        }
        t
    }

    pub fn block_start(&self) -> usize {
        self.tag_lines.iter().copied().min().unwrap_or(self.line)
    }

    pub fn is_holdout(&self) -> bool {
        self.tags.iter().any(|t| t == HOLDOUT_TAG)
    }

    pub fn epic(&self) -> String {
        for t in self.all_tags() {
            if let Some(c) = epic_re().captures(&t) {
                return c[1].to_string();
            }
        }
        String::new()
    }

    pub fn canonical(&self, background: &[String]) -> String {
        let mut tags: Vec<_> = self
            .tags
            .iter()
            .filter(|t| !t.starts_with(RID_PREFIX))
            .cloned()
            .collect();
        tags.sort();
        format!(
            "FEATURE:{}\nBACKGROUND:\n{}\nTAGS:{}\n{}:{}\nSTEPS:\n{}\nEXAMPLES:\n{}",
            self.feature_name.trim(),
            background.join("\n"),
            tags.join(","),
            self.keyword.trim().to_uppercase(),
            self.name.trim(),
            self.steps.join("\n"),
            self.examples.join("\n")
        )
    }

    pub fn spec_hash(&self, background: &[String]) -> String {
        sha_text(&self.canonical(background))
    }
}

#[derive(Debug, Clone)]
pub struct Feature {
    pub name: String,
    pub file: String,
    pub tags: Vec<String>,
    pub background: Vec<String>,
    pub scenarios: Vec<Scenario>,
    pub description: String,
}

impl Feature {
    pub fn story(&self) -> Story {
        parse_story(&self.description)
    }

    pub fn epic(&self) -> String {
        for t in &self.tags {
            if let Some(c) = epic_re().captures(t) {
                return c[1].to_string();
            }
        }
        let p = Path::new(&self.file);
        let parts: Vec<_> = p.iter().collect();
        if parts.len() > 1 {
            parts[0].to_string_lossy().into_owned()
        } else {
            String::new()
        }
    }

    pub fn blocks(&self, total_lines: usize) -> Vec<(&Scenario, usize, usize)> {
        let mut anchors: Vec<usize> = self.scenarios.iter().map(|s| s.block_start()).collect();
        anchors.sort_unstable();
        anchors.dedup();
        self.scenarios
            .iter()
            .map(|sc| {
                let start = sc.block_start();
                let end = anchors
                    .iter()
                    .copied()
                    .find(|&a| a > start)
                    .unwrap_or(total_lines + 1);
                (sc, start, end)
            })
            .collect()
    }
}

#[derive(Debug, Error)]
#[error("spec parse error")]
pub struct SpecParseError {
    pub errors: HashMap<String, String>,
}

impl SpecParseError {
    pub fn message(&self) -> String {
        self.errors
            .iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

fn rids_in_tags(tags: &[String]) -> Vec<String> {
    tags.iter()
        .filter_map(|t| {
            rid_re()
                .captures(t)
                .map(|c| c[1].to_string())
        })
        .collect()
}

fn tag_lines_for(raw_lines: &[String], scenario_line: usize) -> Vec<usize> {
    let mut lines = Vec::new();
    if scenario_line == 0 {
        return lines;
    }
    let mut i = scenario_line; // 1-indexed scenario keyword line
    // walk up through tag lines
    while i > 1 {
        let prev = raw_lines.get(i - 2).map(|s| s.trim().to_string()).unwrap_or_default();
        if prev.starts_with('@') {
            lines.push(i - 1);
            i -= 1;
        } else if prev.is_empty() {
            // blank line between tags? stop unless next-up is a tag
            break;
        } else {
            break;
        }
    }
    lines.sort_unstable();
    lines
}

fn convert_scenario(
    sc: &gherkin::Scenario,
    feature_name: &str,
    rel: &str,
    inherited: &[String],
    extra_bg: &[String],
    raw_lines: &[String],
    extra_tags: &[String],
) -> Scenario {
    let mut tags: Vec<String> = sc.tags.iter().map(|t| norm_tag(t)).collect();
    tags.extend(extra_tags.iter().cloned());
    let rids = rids_in_tags(&tags);
    let line = sc.position.line as usize;
    Scenario {
        rid: rids.first().cloned(),
        rid_count: rids.len(),
        name: sc.name.clone(),
        keyword: sc.keyword.clone(),
        tags,
        steps: {
            let mut s = extra_bg.to_vec();
            s.extend(step_lines(&sc.steps));
            // background is hashed separately; don't duplicate into steps
            let _ = extra_bg;
            step_lines(&sc.steps)
        },
        examples: examples_lines(&sc.examples),
        feature_name: feature_name.to_string(),
        feature_file: rel.to_string(),
        line,
        tag_lines: tag_lines_for(raw_lines, line),
        inherited_tags: inherited.to_vec(),
    }
}

pub fn parse_text(source: &str, rel: &str) -> Result<Option<Feature>, String> {
    let gf = gherkin::Feature::parse(source, gherkin::GherkinEnv::default())
        .map_err(|e| e.to_string())?;
    let raw_lines: Vec<String> = source.split('\n').map(|s| s.trim_end_matches('\r').to_string()).collect();
    let feature_tags: Vec<String> = gf.tags.iter().map(|t| norm_tag(t)).collect();
    let mut background = Vec::new();
    if let Some(bg) = &gf.background {
        background.extend(step_lines(&bg.steps));
    }
    let mut scenarios = Vec::new();
    for sc in &gf.scenarios {
        scenarios.push(convert_scenario(
            sc,
            &gf.name,
            rel,
            &feature_tags,
            &[],
            &raw_lines,
            &[],
        ));
    }
    for rule in &gf.rules {
        let mut rule_bg = background.clone();
        if let Some(bg) = &rule.background {
            rule_bg.extend(step_lines(&bg.steps));
        }
        let rule_tags: Vec<String> = rule.tags.iter().map(|t| norm_tag(t)).collect();
        for sc in &rule.scenarios {
            let mut inherited = feature_tags.clone();
            inherited.extend(rule_tags.iter().cloned());
            let scn = convert_scenario(sc, &gf.name, rel, &inherited, &[], &raw_lines, &[]);
            let _ = rule_bg;
            scenarios.push(scn);
        }
    }
    // Fix: rule background should be in Feature.background only at feature level.
    // Python walks children and extends a single background list including rule backgrounds.
    // Recompute background the Python way: feature background only on Feature.background;
    // rule background is included when hashing scenarios under that rule via... wait.
    // Python `_walk_children` extends `background` for the whole feature including rules.
    // That means a rule background is appended to the shared list and applies to ALL subsequent
    // scenarios including those after the rule? That's a sequential walk. Feature-level
    // background first, then rule backgrounds accumulate.
    // For hashing, Scenario.spec_hash(f.background) uses the Feature.background list.
    // So rule backgrounds ARE in Feature.background in Python if they appeared as children.
    // I'll flatten: feature bg + all rule bgs into Feature.background to match sequential extend.
    for rule in &gf.rules {
        if let Some(bg) = &rule.background {
            background.extend(step_lines(&bg.steps));
        }
    }

    Ok(Some(Feature {
        name: gf.name,
        file: rel.to_string(),
        tags: feature_tags,
        background,
        scenarios,
        description: gf.description.unwrap_or_default(),
    }))
}

pub fn load_specs(spec_dir: &Path, strict: bool) -> Result<Vec<Feature>, SpecParseError> {
    let mut out = Vec::new();
    let mut errors = HashMap::new();
    if !spec_dir.exists() {
        return Ok(out);
    }
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in walkdir::WalkDir::new(spec_dir).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file()
            && entry.path().extension().and_then(|s| s.to_str()) == Some("feature")
        {
            paths.push(entry.path().to_path_buf());
        }
    }
    paths.sort();
    for p in paths {
        let rel = p
            .strip_prefix(spec_dir)
            .unwrap_or(&p)
            .to_string_lossy()
            .replace('\\', "/");
        match fs::read_to_string(&p) {
            Ok(text) => match parse_text(&text, &rel) {
                Ok(Some(f)) => out.push(f),
                Ok(None) => {}
                Err(e) => {
                    errors.insert(rel, e.lines().next().unwrap_or("parse error").chars().take(200).collect());
                }
            },
            Err(e) => {
                errors.insert(rel, e.to_string());
            }
        }
    }
    if !errors.is_empty() && strict {
        return Err(SpecParseError { errors });
    }
    Ok(out)
}

pub fn stamp_rids(spec_dir: &Path) -> std::io::Result<HashMap<String, String>> {
    let mut minted = HashMap::new();
    let mut paths: Vec<PathBuf> = walkdir::WalkDir::new(spec_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file() && e.path().extension().and_then(|s| s.to_str()) == Some("feature"))
        .map(|e| e.into_path())
        .collect();
    paths.sort();
    let mut existing: HashSet<String> = HashSet::new();
    for path in &paths {
        let text = fs::read_to_string(path)?;
        for cap in rid_re().captures_iter(&text) {
            existing.insert(cap[1].to_string());
        }
    }
    for path in &paths {
        let raw = fs::read(path)?;
        let newline: &[u8] = if raw.windows(2).any(|w| w == b"\r\n") {
            b"\r\n"
        } else {
            b"\n"
        };
        let text = String::from_utf8_lossy(&raw);
        let rel = path
            .strip_prefix(spec_dir)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let feat = match parse_text(&text.replace("\r\n", "\n"), &rel) {
            Ok(Some(f)) => f,
            _ => continue,
        };
        let mut lines: Vec<String> = if newline == b"\r\n" {
            text.replace("\r\n", "\n").split('\n').map(|s| s.to_string()).collect()
        } else {
            text.split('\n').map(|s| s.to_string()).collect()
        };
        // drop trailing empty from split
        if lines.last().map(|s| s.is_empty()).unwrap_or(false) {
            lines.pop();
        }
        let mut scenarios = feat.scenarios;
        scenarios.sort_by_key(|s| std::cmp::Reverse(s.block_start()));
        for sc in scenarios {
            if sc.rid.is_some() {
                continue;
            }
            let mut rid = new_rid();
            while existing.contains(&rid) {
                rid = new_rid();
            }
            existing.insert(rid.clone());
            minted.insert(rid.clone(), sc.name.clone());
            let idx = sc.block_start().saturating_sub(1);
            let indent = lines
                .get(idx)
                .map(|line| {
                    line.chars().take_while(|c| c.is_whitespace()).collect::<String>()
                })
                .unwrap_or_else(|| "  ".to_string());
            lines.insert(idx, format!("{indent}{RID_PREFIX}{rid}"));
        }
        let mut out = lines.join(std::str::from_utf8(newline).unwrap());
        out.push_str(std::str::from_utf8(newline).unwrap());
        fs::write(path, out.as_bytes())?;
    }
    Ok(minted)
}

pub fn duplicate_rids(features: &[Feature]) -> Vec<(String, String)> {
    let mut problems = Vec::new();
    for f in features {
        for s in &f.scenarios {
            if s.rid_count > 1 {
                problems.push((f.file.clone(), s.name.clone()));
            }
        }
    }
    let mut seen: HashMap<String, String> = HashMap::new();
    for f in features {
        for s in &f.scenarios {
            if let Some(rid) = &s.rid {
                if let Some(prev) = seen.get(rid) {
                    problems.push((
                        f.file.clone(),
                        format!("{} reuses id {rid} from {prev}", s.name),
                    ));
                } else {
                    seen.insert(rid.clone(), s.name.clone());
                }
            }
        }
    }
    problems
}

pub fn strip_holdouts(text: &str) -> String {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let normalized = text.replace("\r\n", "\n");
    let feat = match parse_text(&normalized, "<memory>") {
        Ok(Some(f)) => f,
        _ => return text.to_string(),
    };
    let mut lines: Vec<&str> = normalized.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    let ranges: Vec<_> = feat
        .blocks(lines.len())
        .into_iter()
        .filter(|(sc, _, _)| sc.is_holdout())
        .map(|(_, start, end)| (start, end))
        .collect();
    let mut lines: Vec<String> = lines.into_iter().map(|s| s.to_string()).collect();
    for (start, end) in ranges.into_iter().rev() {
        let s = start.saturating_sub(1);
        let e = end.saturating_sub(1).min(lines.len());
        if s < e {
            lines.drain(s..e);
        }
    }
    let mut out = lines.join(newline);
    out.push_str(newline);
    out
}

pub fn holdout_rids(features: &[Feature]) -> HashSet<String> {
    features
        .iter()
        .flat_map(|f| f.scenarios.iter())
        .filter(|s| s.is_holdout())
        .filter_map(|s| s.rid.clone())
        .collect()
}
