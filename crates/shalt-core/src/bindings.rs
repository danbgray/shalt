//! Which scenarios actually have step definitions — a file on disk is not enough.

use crate::config::Config;
use crate::mockups::journey_slug;
use crate::runner::list_step_files;
use crate::spec::{Feature, Scenario};
use regex::Regex;
use serde::Serialize;
use std::path::Path;
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub struct StepDef {
    pub kw: String,
    pub pattern: String,
    pub regex: bool,
    pub stub: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct JourneyTests {
    pub journey: String,
    pub scenarios: usize,
    pub bound: usize,
}

fn attr_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?im)^\s*#\[(given|when|then)\s*(?:\((.*)\)\s*)?\]\s*$"#).unwrap()
    })
}

fn py_attr_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?im)^\s*@(given|when|then)\s*\((.*)\)\s*$"#).unwrap()
    })
}

fn js_attr_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?im)^\s*(Given|When|Then)\s*\((.*)\)\s*,?"#).unwrap()
    })
}

fn kw_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)^(Given|When|Then|And|But)\s+").unwrap())
}

/// First line of a Gherkin step, without Given/When/Then/And/But.
pub fn step_text(step: &str) -> String {
    let line = step.lines().next().unwrap_or(step).trim();
    kw_re().replace(line, "").to_string()
}

/// Keyword (given/when/then) and cucumber-js phrase with `{string}` captures.
/// `And`/`But` inherit `last_kw`.
pub fn step_kw_and_phrase(step: &str, last_kw: &str) -> (String, String) {
    let line = step.lines().next().unwrap_or(step).trim();
    let lower = line.to_ascii_lowercase();
    let (raw_kw, rest) = if let Some(r) = strip_kw(&lower, line, "given ") {
        ("given", r)
    } else if let Some(r) = strip_kw(&lower, line, "when ") {
        ("when", r)
    } else if let Some(r) = strip_kw(&lower, line, "then ") {
        ("then", r)
    } else if let Some(r) = strip_kw(&lower, line, "and ") {
        (if last_kw.is_empty() { "given" } else { last_kw }, r)
    } else if let Some(r) = strip_kw(&lower, line, "but ") {
        (if last_kw.is_empty() { "given" } else { last_kw }, r)
    } else {
        (
            if last_kw.is_empty() { "given" } else { last_kw },
            line,
        )
    };
    (raw_kw.to_string(), quoted_to_string_capture(rest.trim()))
}

fn strip_kw<'a>(lower: &str, line: &'a str, kw: &str) -> Option<&'a str> {
    if lower.starts_with(kw) {
        Some(line[kw.len()..].trim())
    } else {
        None
    }
}

fn quoted_to_string_capture(s: &str) -> String {
    let mut out = String::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' {
                if b[i] == b'\\' && i + 1 < b.len() {
                    i += 2;
                    continue;
                }
                i += 1;
            }
            if i < b.len() {
                i += 1;
            }
            out.push_str("{string}");
            continue;
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}

pub fn step_has_table(step: &str) -> bool {
    step.lines()
        .skip(1)
        .any(|l| l.trim().starts_with('|'))
}

fn extract_string(inner: &str) -> (String, bool) {
    let inner = inner.trim();
    let lower = inner.to_ascii_lowercase();
    let regex = lower.starts_with("regex");
    // Only rust attrs (`expr = "..."`, `regex = "..."`). A JS one-liner
    // `Given('x', function () { this.n = 1; })` contains `=` in the body.
    let rest = if lower.starts_with("expr") || lower.starts_with("regex") {
        inner.split_once('=').map(|(_, r)| r.trim()).unwrap_or(inner)
    } else {
        inner
    };
    let rest = rest.trim().trim_end_matches(',').trim();
    if let Some(s) = strip_quoted(rest) {
        let regex = regex || looks_like_regex(&s);
        return (s, regex);
    }
    if let Some(idx) = rest.find('"') {
        if let Some(s) = strip_quoted(&rest[idx..]) {
            let regex = regex || looks_like_regex(&s);
            return (s, regex);
        }
    }
    if let Some(idx) = rest.find("r#") {
        if let Some(s) = strip_raw(&rest[idx..]) {
            return (s, true);
        }
    }
    (String::new(), regex)
}

fn looks_like_regex(s: &str) -> bool {
    s.contains('(') || s.contains('[') || s.contains('\\') || s.contains('+') || s.contains('*')
}

fn strip_quoted(s: &str) -> Option<String> {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() >= 2 && (b[0] == b'"' || b[0] == b'\'') {
        let q = b[0];
        let mut out = String::new();
        let mut i = 1;
        while i < b.len() {
            if b[i] == b'\\' && i + 1 < b.len() {
                out.push(b[i + 1] as char);
                i += 2;
                continue;
            }
            if b[i] == q {
                return Some(out);
            }
            out.push(b[i] as char);
            i += 1;
        }
    }
    None
}

fn strip_raw(s: &str) -> Option<String> {
    let s = s.trim();
    let rest = s.strip_prefix('r')?;
    let hashes = rest.chars().take_while(|c| *c == '#').count();
    let rest = &rest[hashes..];
    let quote = rest.strip_prefix('"')?;
    let end = "#".repeat(hashes);
    let close = format!("\"{end}");
    quote.find(&close).map(|i| quote[..i].to_string())
}

fn body_is_stub(lines: &[&str], start: usize) -> bool {
    let mut chunk = String::new();
    for line in lines.iter().skip(start).take(8) {
        if attr_re().is_match(line) || py_attr_re().is_match(line) || js_attr_re().is_match(line) {
            break;
        }
        if line.trim().starts_with("fn main") {
            break;
        }
        chunk.push_str(line);
        chunk.push('\n');
        if line.contains('}') && chunk.contains('{') {
            break;
        }
    }
    let blob = chunk.to_ascii_lowercase();
    if blob.contains("todo!")
        || blob.contains("unimplemented!")
        || blob.contains("todo(")
        || blob.contains("pytest.skip")
        || blob.contains("raise notimplemented")
        || blob.contains("return 'pending'")
        || blob.contains("return \"pending\"")
        || blob.contains("not implemented")
    {
        return true;
    }
    let statements: Vec<&str> = chunk
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("//"))
        .collect();
    if statements.iter().any(|l| *l == "pass" || *l == "..." || *l == "todo!()") {
        return true;
    }
    let compact: String = chunk.chars().filter(|c| !c.is_whitespace()).collect();
    compact.contains("{}") || compact.contains("{...}")
}

/// Parse cucumber-family step definitions from a source file.
pub fn parse_step_defs(src: &str) -> Vec<StepDef> {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let (kw, inner) = if let Some(c) = attr_re().captures(line) {
            (c[1].to_ascii_lowercase(), c.get(2).map(|m| m.as_str()).unwrap_or("").to_string())
        } else if let Some(c) = py_attr_re().captures(line) {
            (c[1].to_ascii_lowercase(), c.get(2).map(|m| m.as_str()).unwrap_or("").to_string())
        } else if let Some(c) = js_attr_re().captures(line) {
            (c[1].to_ascii_lowercase(), c.get(2).map(|m| m.as_str()).unwrap_or("").to_string())
        } else {
            continue;
        };
        let (pattern, regex) = extract_string(&inner);
        if pattern.is_empty() {
            continue;
        }
        out.push(StepDef {
            kw,
            pattern,
            regex,
            stub: body_is_stub(&lines, i + 1),
        });
    }
    out
}

fn cucumber_regex(pat: &str) -> Option<Regex> {
    let mut out = String::from("^");
    let b = pat.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'{' {
            if let Some(end) = pat[i..].find('}') {
                let name = &pat[i + 1..i + end];
                let piece = match name {
                    "string" => r#""[^"]*""#,
                    "int" => r"-?\d+",
                    "float" => r"-?\d+(?:\.\d+)?",
                    "word" => r"\S+",
                    _ => r".+?",
                };
                out.push_str(piece);
                i += end + 1;
                continue;
            }
        }
        out.push_str(&regex::escape(&pat[i..i + 1]));
        i += 1;
    }
    out.push('$');
    Regex::new(&out).ok()
}

pub fn def_matches(def: &StepDef, step: &str) -> bool {
    if def.stub {
        return false;
    }
    let text = step_text(step);
    if def.regex {
        return Regex::new(&format!("^(?:{})$", def.pattern))
            .ok()
            .map(|re| re.is_match(&text))
            .unwrap_or(false);
    }
    cucumber_regex(&def.pattern)
        .map(|re| re.is_match(&text))
        .unwrap_or(false)
}

fn scenario_steps<'a>(feature: &'a Feature, sc: &'a Scenario) -> Vec<&'a str> {
    let mut out: Vec<&str> = feature.background.iter().map(String::as_str).collect();
    out.extend(sc.steps.iter().map(String::as_str));
    out
}

pub fn matching_defs<'a>(defs: &'a [StepDef], step: &str) -> Vec<&'a StepDef> {
    defs.iter().filter(|d| def_matches(d, step)).collect()
}

pub fn scenario_is_bound(feature: &Feature, sc: &Scenario, defs: &[StepDef]) -> bool {
    let steps = scenario_steps(feature, sc);
    if steps.is_empty() {
        return false;
    }
    // Exactly one non-stub definition. Two copies (ambiguous) do not bind.
    steps.iter().all(|st| matching_defs(defs, st).len() == 1)
}

pub fn load_step_defs(root: &Path) -> Vec<StepDef> {
    let cfg = Config::load(root).unwrap_or_default();
    let mut out = Vec::new();
    for rel in list_step_files(root, &cfg.steps) {
        let body = std::fs::read_to_string(root.join(&rel)).unwrap_or_default();
        out.extend(parse_step_defs(&body));
    }
    out
}

pub fn journey_of<'a>(feature: &'a Feature) -> String {
    let j = journey_slug(feature);
    if j.is_empty() {
        feature.epic()
    } else {
        j
    }
}

pub fn in_scope(feature: &Feature, focus_journey: &str) -> bool {
    let focus = focus_journey.trim();
    focus.is_empty() || journey_of(feature) == focus
}

pub fn unbound_scenarios<'a>(
    features: &'a [Feature],
    defs: &[StepDef],
    focus_journey: &str,
) -> Vec<&'a Scenario> {
    let mut out = Vec::new();
    for f in features {
        if !in_scope(f, focus_journey) {
            continue;
        }
        for s in &f.scenarios {
            if !scenario_is_bound(f, s, defs) {
                out.push(s);
            }
        }
    }
    out
}

pub fn journey_tests(features: &[Feature], defs: &[StepDef]) -> Vec<JourneyTests> {
    let mut order: Vec<String> = Vec::new();
    for f in features {
        let j = journey_of(f);
        if !order.contains(&j) {
            order.push(j);
        }
    }
    order
        .into_iter()
        .map(|journey| {
            let mut scenarios = 0usize;
            let mut bound = 0usize;
            for f in features {
                if journey_of(f) != journey {
                    continue;
                }
                for s in &f.scenarios {
                    scenarios += 1;
                    if scenario_is_bound(f, s, defs) {
                        bound += 1;
                    }
                }
            }
            JourneyTests {
                journey,
                scenarios,
                bound,
            }
        })
        .filter(|j| j.scenarios > 0)
        .collect()
}

/// True when Play should still be writing tests for this scope, not running or building.
pub fn steps_needed(root: &Path, features: &[Feature], focus_journey: &str) -> bool {
    if features.iter().all(|f| f.scenarios.is_empty()) {
        return false;
    }
    let defs = load_step_defs(root);
    !unbound_scenarios(features, &defs, focus_journey).is_empty()
}

/// Which journey the next tests job should bind. Completes one journey before the next.
pub fn pick_steps_journey(features: &[Feature], defs: &[StepDef], focus_journey: &str) -> Option<String> {
    let focus = focus_journey.trim();
    let rows = journey_tests(features, defs);
    if !focus.is_empty() {
        return rows
            .into_iter()
            .find(|j| j.journey == focus && j.bound < j.scenarios)
            .map(|j| j.journey);
    }
    rows.into_iter()
        .filter(|j| j.bound < j.scenarios)
        .min_by_key(|j| (j.bound, j.scenarios))
        .map(|j| j.journey)
}

pub fn bound_count(features: &[Feature], defs: &[StepDef], focus_journey: &str) -> usize {
    features
        .iter()
        .filter(|f| in_scope(f, focus_journey))
        .flat_map(|f| f.scenarios.iter().map(move |s| (f, s)))
        .filter(|(f, s)| scenario_is_bound(f, s, defs))
        .count()
}

pub fn remaining_in_scope(features: &[Feature], focus_journey: &str) -> usize {
    features
        .iter()
        .filter(|f| in_scope(f, focus_journey))
        .map(|f| f.scenarios.len())
        .sum()
}

/// Prompt: bind this journey for real. Do not stub. Do not boil the ocean.
pub fn stepwright_focus_prompt(features: &[Feature], journey: &str, stack: &str) -> String {
    let mut lines = vec![
        match stack {
            "javascript" => "Write cucumber-js ESM step definitions under steps/ and contract/interface.md. Keep steps/world.js. One steps/<journey>.steps.js for this journey. import { Given, When, Then } from '@cucumber/cucumber'; function (not arrow) World; {string}/{int} captures; import from '../src/....js'. Do not duplicate a Given/When/Then phrase.".into(),
            "python" => "Write pytest-bdd step definitions under steps/ and contract/interface.md.".into(),
            _ => "Write shalt's Rust step harness in tests/shalt.rs and contract/interface.md. cucumber-rs 0.23: expr = captures, cucumber::gherkin::Step for tables, SHALT_REPORT Json writer, .run(\"spec\").".into(),
        },
        format!("This turn binds journey `{journey}` only. Do not try to cover every journey."),
        "Every Given/When/Then those scenarios use must have a real step that calls the contract.".into(),
        "No empty bodies, no todo!(), unimplemented!(), pass, or comments that say skip/stub.".into(),
        "If a scenario cannot be tested as written, skip it — do not invent a vacuous assertion.".into(),
        "Keep existing steps that already work. Add what this journey is missing. Write each file at most once, then done().".into(),
        String::new(),
        format!("Scenarios in `{journey}`:"),
    ];
    let mut n = 0usize;
    for f in features {
        if journey_of(f) != journey {
            continue;
        }
        if !f.background.is_empty() && n == 0 {
            lines.push("Background:".into());
            for st in &f.background {
                lines.push(format!("  {st}"));
            }
        }
        for s in &f.scenarios {
            n += 1;
            if n > 14 {
                lines.push("…".into());
                break;
            }
            let rid = s.rid.clone().unwrap_or_default();
            lines.push(format!("- {} {}", rid, s.name));
            for st in &s.steps {
                let first = st.lines().next().unwrap_or(st);
                lines.push(format!("    {first}"));
            }
        }
        if n > 14 {
            break;
        }
    }
    if n == 0 {
        lines.push("(no scenarios in this journey)".into());
    }
    lines.join("\n")
}
