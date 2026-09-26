use crate::spec::Feature;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

pub const SCHEMA: &str = "shalt.ledger/1";
pub const PENDING: &str = "pending";
pub const RED: &str = "red";
pub const GREEN: &str = "green";
pub const STALE: &str = "stale";
pub const ORPHAN: &str = "orphan";

fn now() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub rid: String,
    pub name: String,
    pub feature: String,
    pub feature_file: String,
    #[serde(default)]
    pub line: usize,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub epic: String,
    #[serde(default)]
    pub actor: String,
    #[serde(default)]
    pub capability: String,
    #[serde(default)]
    pub benefit: String,
    #[serde(default = "pending")]
    pub status: String,
    #[serde(default)]
    pub spec_hash: String,
    #[serde(default)]
    pub verified_spec_hash: String,
    #[serde(default)]
    pub last_green_at: Option<String>,
    #[serde(default)]
    pub last_run_at: Option<String>,
    #[serde(default)]
    pub failure: Option<String>,
    #[serde(default)]
    pub mutants_killed: Option<i64>,
    #[serde(default)]
    pub blind_spots: Option<i64>,
    #[serde(default)]
    pub history: Vec<Value>,
    /// First non-harness suite colour for this Then. Green-first is a suspicion.
    #[serde(default)]
    pub first_colour: Option<String>,
    #[serde(default)]
    pub first_colour_at: Option<String>,
    #[serde(default)]
    pub was_ever_red: bool,
}

#[cfg(test)]
mod first_colour_tests {
    use super::*;
    use std::collections::HashMap;

    fn entry(rid: &str) -> Entry {
        Entry {
            rid: rid.into(),
            name: "S".into(),
            feature: "F".into(),
            feature_file: "f.feature".into(),
            line: 1,
            tags: vec![],
            epic: String::new(),
            actor: String::new(),
            capability: String::new(),
            benefit: String::new(),
            status: PENDING.into(),
            spec_hash: "h".into(),
            verified_spec_hash: String::new(),
            last_green_at: None,
            last_run_at: None,
            failure: None,
            mutants_killed: None,
            blind_spots: None,
            history: vec![],
            first_colour: None,
            first_colour_at: None,
            was_ever_red: false,
        }
    }

    #[test]
    fn red_then_green_is_evidence() {
        let mut led = Ledger::default();
        led.entries.insert("S-1".into(), entry("S-1"));
        let mut first = HashMap::new();
        first.insert(
            "S-1".into(),
            RunResult {
                outcome: "failed".into(),
                detail: "no".into(),
                nodeid: "t".into(),
            },
        );
        led.apply_run(&first, "r1", "");
        assert_eq!(led.entries["S-1"].first_colour.as_deref(), Some(RED));
        assert!(led.entries["S-1"].was_ever_red);
        let mut second = HashMap::new();
        second.insert(
            "S-1".into(),
            RunResult {
                outcome: "passed".into(),
                detail: String::new(),
                nodeid: "t".into(),
            },
        );
        led.apply_run(&second, "r2", "");
        assert_eq!(led.entries["S-1"].status, GREEN);
        assert_eq!(led.entries["S-1"].first_colour.as_deref(), Some(RED));
        assert!(led.unfalsified_greens().is_empty());
    }

    #[test]
    fn green_first_is_suspicion() {
        let mut led = Ledger::default();
        led.entries.insert("S-1".into(), entry("S-1"));
        let mut first = HashMap::new();
        first.insert(
            "S-1".into(),
            RunResult {
                outcome: "passed".into(),
                detail: String::new(),
                nodeid: "t".into(),
            },
        );
        led.apply_run(&first, "r1", "");
        assert_eq!(led.entries["S-1"].first_colour.as_deref(), Some(GREEN));
        assert!(!led.entries["S-1"].was_ever_red);
        assert_eq!(led.unfalsified_greens().len(), 1);
    }
}

fn pending() -> String {
    PENDING.to_string()
}

fn ledger_body_eq(a: &str, b: &str) -> bool {
    let strip = |s: &str| -> Option<Value> {
        let mut v: Value = serde_json::from_str(s).ok()?;
        if let Some(o) = v.as_object_mut() {
            o.remove("generated_at");
        }
        Some(v)
    };
    match (strip(a), strip(b)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

pub fn is_harness_failure(s: &str) -> bool {
    let t = s.to_ascii_lowercase();
    t.contains("could not compile")
        || t.contains("error[e")
        || t.contains("unresolved import")
        || t.contains("cannot find module")
        || t.contains("cannot find crate")
        || t.contains("failed to compile")
        || t.contains("expected one of")
        || t.contains("importerror")
        || t.contains("no module named")
        || (t.contains("compiling ") && t.contains("error:"))
}

impl Ledger {
    pub fn harness_error(&self) -> Option<&str> {
        self.suite_error.as_deref().filter(|s| !s.is_empty())
    }

    /// First scenario in spec order that is not green or orphan.
    pub fn next_ungreen<'a>(
        &self,
        features: &'a [Feature],
        skip: &HashSet<String>,
    ) -> Option<(&'a Feature, &'a crate::spec::Scenario)> {
        for f in features {
            for s in &f.scenarios {
                let Some(rid) = s.rid.as_deref() else {
                    continue;
                };
                if skip.contains(rid) {
                    continue;
                }
                match self.entries.get(rid).map(|e| e.status.as_str()) {
                    Some(GREEN) | Some(ORPHAN) => continue,
                    _ => return Some((f, s)),
                }
            }
        }
        None
    }

    /// Suite compile dump, including ledgers that copied it onto every scenario
    /// before `suite_error` existed.
    pub fn display_harness_error(&self) -> Option<String> {
        if let Some(s) = self.harness_error() {
            return Some(s.to_string());
        }
        let dumps: Vec<&str> = self
            .entries
            .values()
            .filter_map(|e| e.failure.as_deref())
            .filter(|s| is_harness_failure(s) && *s != "tests did not compile")
            .collect();
        if dumps.len() < 2 {
            return None;
        }
        let first = dumps[0];
        let n = dumps.iter().filter(|d| **d == first).count();
        let need = dumps.len().div_ceil(2).max(2);
        if n >= need {
            Some(first.to_string())
        } else {
            None
        }
    }
}

impl Entry {
    pub fn record(&mut self, event: &str, extra: Value) {
        let mut obj = serde_json::json!({"at": now(), "event": event});
        if let Value::Object(map) = extra {
            if let Value::Object(o) = &mut obj {
                o.extend(map);
            }
        }
        self.history.push(obj);
        if self.history.len() > 50 {
            let n = self.history.len();
            self.history = self.history.split_off(n - 50);
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Ledger {
    pub entries: HashMap<String, Entry>,
    pub spec_lock: Value,
    pub regressions: Vec<Value>,
    pub mutation: Value,
    /// Suite-level compile/harness dump. Not a per-scenario assertion.
    pub suite_error: Option<String>,
    /// Then or #observe: changed after Play locked the surface.
    pub amendments: Vec<Value>,
}

impl Ledger {
    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw: Value = serde_json::from_str(&fs::read_to_string(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let schema = raw.get("schema").and_then(|s| s.as_str()).unwrap_or("");
        if schema != SCHEMA {
            return Err(format!("unsupported ledger schema: {schema:?}"));
        }
        let mut entries = HashMap::new();
        if let Some(obj) = raw.get("scenarios").and_then(|s| s.as_object()) {
            for (k, v) in obj {
                let e: Entry = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
                entries.insert(k.clone(), e);
            }
        }
        Ok(Self {
            entries,
            spec_lock: raw.get("spec_lock").cloned().unwrap_or(Value::Object(Default::default())),
            regressions: raw
                .get("regressions")
                .and_then(|r| r.as_array())
                .cloned()
                .unwrap_or_default(),
            mutation: raw.get("mutation").cloned().unwrap_or(Value::Object(Default::default())),
            suite_error: raw
                .get("suite_error")
                .and_then(|s| s.as_str())
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty()),
            amendments: raw
                .get("amendments")
                .and_then(|a| a.as_array())
                .cloned()
                .unwrap_or_default(),
        })
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut scenarios = serde_json::Map::new();
        let mut keys: Vec<_> = self.entries.keys().cloned().collect();
        keys.sort();
        for k in keys {
            scenarios.insert(k.clone(), serde_json::to_value(&self.entries[&k]).unwrap());
        }
        let payload = serde_json::json!({
            "schema": SCHEMA,
            "generated_at": now(),
            "summary": self.summary(),
            "spec_lock": self.spec_lock,
            "regressions": self.regressions,
            "mutation": self.mutation,
            "suite_error": self.suite_error,
            "amendments": self.amendments,
            "scenarios": scenarios,
        });
        let raw = serde_json::to_string_pretty(&payload)? + "\n";
        // Play/ensure_play_lock rewrites this file. A new generated_at must not
        // trip GuardedTurn while a filler is mid-turn.
        if let Ok(old) = fs::read_to_string(path) {
            if ledger_body_eq(&old, &raw) {
                return Ok(());
            }
        }
        fs::write(path, raw)
    }

    pub fn sync_spec(&mut self, features: &[Feature]) -> HashMap<String, i64> {
        let mut stats = HashMap::from([
            ("added".into(), 0i64),
            ("staled".into(), 0),
            ("orphaned".into(), 0),
            ("unchanged".into(), 0),
            ("restored".into(), 0),
        ]);
        let mut seen = HashSetLike::default();
        for f in features {
            for s in &f.scenarios {
                let Some(rid) = &s.rid else { continue };
                seen.insert(rid.clone());
                let h = s.spec_hash(&f.background);
                if let Some(e) = self.entries.get_mut(rid) {
                    e.name = s.name.clone();
                    e.feature = f.name.clone();
                    e.feature_file = f.file.clone();
                    e.line = s.line;
                    e.tags = s.all_tags();
                    let st = f.story();
                    e.epic = if s.epic().is_empty() { f.epic() } else { s.epic() };
                    e.actor = st.actor;
                    e.capability = st.capability;
                    e.benefit = st.benefit;
                    if e.status == ORPHAN {
                        e.status = PENDING.to_string();
                        e.verified_spec_hash.clear();
                        e.record("restored_to_spec", serde_json::json!({}));
                        *stats.get_mut("restored").unwrap() += 1;
                    }
                    if e.spec_hash != h {
                        let prev = e.status.clone();
                        e.spec_hash = h.clone();
                        e.last_run_at = None;
                        if prev == GREEN {
                            e.status = STALE.to_string();
                            e.record("spec_changed", serde_json::json!({"was": prev, "spec_hash": h}));
                            *stats.get_mut("staled").unwrap() += 1;
                        } else {
                            e.record("spec_changed", serde_json::json!({"was": prev, "spec_hash": h}));
                        }
                    } else {
                        *stats.get_mut("unchanged").unwrap() += 1;
                    }
                } else {
                    let st = f.story();
                    let mut e = Entry {
                        rid: rid.clone(),
                        name: s.name.clone(),
                        feature: f.name.clone(),
                        feature_file: f.file.clone(),
                        tags: s.all_tags(),
                        spec_hash: h.clone(),
                        line: s.line,
                        epic: if s.epic().is_empty() { f.epic() } else { s.epic() },
                        actor: st.actor,
                        capability: st.capability,
                        benefit: st.benefit,
                        status: PENDING.to_string(),
                        verified_spec_hash: String::new(),
                        last_green_at: None,
                        last_run_at: None,
                        failure: None,
                        mutants_killed: None,
                        blind_spots: None,
                        history: vec![],
                        first_colour: None,
                        first_colour_at: None,
                        was_ever_red: false,
                    };
                    e.record("added", serde_json::json!({"spec_hash": h}));
                    self.entries.insert(rid.clone(), e);
                    *stats.get_mut("added").unwrap() += 1;
                }
            }
        }
        for (rid, e) in self.entries.iter_mut() {
            if !seen.contains(rid) && e.status != ORPHAN {
                e.status = ORPHAN.to_string();
                e.record("removed_from_spec", serde_json::json!({}));
                *stats.get_mut("orphaned").unwrap() += 1;
            }
        }
        stats
    }

    pub fn apply_run(
        &mut self,
        results: &HashMap<String, RunResult>,
        run_id: &str,
        blocked: &str,
    ) -> ApplyOut {
        let nows = now();
        let mut new_regressions = Vec::new();
        if !blocked.is_empty() && is_harness_failure(blocked) {
            self.suite_error = Some(blocked.chars().take(4000).collect());
        } else if blocked.is_empty() {
            self.suite_error = None;
        }
        let rids: Vec<String> = self.entries.keys().cloned().collect();
        for rid in rids {
            let e = self.entries.get_mut(&rid).unwrap();
            if e.status == ORPHAN {
                continue;
            }
            match results.get(&rid) {
                None if !blocked.is_empty() => {
                    if !is_harness_failure(blocked) {
                        if e.first_colour.is_none() {
                            e.first_colour = Some(RED.to_string());
                            e.first_colour_at = Some(nows.clone());
                        }
                        e.was_ever_red = true;
                    }
                    if e.status == GREEN {
                        let reg = serde_json::json!({
                            "at": nows, "rid": rid, "name": e.name, "run": run_id,
                            "detail": "suite stopped collecting"
                        });
                        self.regressions.push(reg.clone());
                        new_regressions.push(reg);
                        e.record("REGRESSION", serde_json::json!({"run": run_id}));
                    }
                    e.status = RED.to_string();
                    e.failure = Some(if is_harness_failure(blocked) {
                        "tests did not compile".into()
                    } else {
                        blocked.chars().take(2000).collect()
                    });
                    e.last_run_at = Some(nows.clone());
                }
                None => {
                    if e.status == GREEN {
                        let reg = serde_json::json!({
                            "at": nows, "rid": rid, "name": e.name, "run": run_id,
                            "detail": "the test that proved this scenario is gone"
                        });
                        self.regressions.push(reg.clone());
                        new_regressions.push(reg);
                        e.record("REGRESSION", serde_json::json!({"run": run_id, "why": "test_unbound"}));
                    } else if e.status == STALE || e.status == RED {
                        e.record("test_unbound", serde_json::json!({"was": e.status}));
                    }
                    e.verified_spec_hash.clear();
                    e.status = PENDING.to_string();
                    e.failure = Some("no test bound to this scenario".into());
                    // The suite ran. No binding is a survey result, not "we never looked."
                    e.last_run_at = Some(nows.clone());
                }
                Some(r) => {
                    e.last_run_at = Some(nows.clone());
                    let colour = if r.outcome == "passed" { GREEN } else { RED };
                    if e.first_colour.is_none() {
                        e.first_colour = Some(colour.to_string());
                        e.first_colour_at = Some(nows.clone());
                        e.record(
                            "first_colour",
                            serde_json::json!({ "run": run_id, "colour": colour }),
                        );
                    }
                    if colour == RED {
                        e.was_ever_red = true;
                    }
                    if r.outcome == "passed" {
                        if e.status == RED {
                            e.record("fixed", serde_json::json!({"run": run_id}));
                        }
                        e.status = GREEN.to_string();
                        e.verified_spec_hash = e.spec_hash.clone();
                        e.last_green_at = Some(nows.clone());
                        e.failure = None;
                    } else {
                        let was = e.status.clone();
                        if was == GREEN {
                            let reg = serde_json::json!({
                                "at": nows, "rid": rid, "name": e.name, "run": run_id,
                                "detail": r.detail.chars().take(400).collect::<String>()
                            });
                            self.regressions.push(reg.clone());
                            new_regressions.push(reg);
                            e.record("REGRESSION", serde_json::json!({"run": run_id}));
                        }
                        e.status = RED.to_string();
                        e.failure = Some(r.detail.chars().take(2000).collect());
                    }
                }
            }
        }
        let unknown: Vec<String> = results.keys().filter(|k| !self.entries.contains_key(*k)).cloned().collect();
        ApplyOut {
            regressions: new_regressions,
            unknown_rids: unknown,
            summary: self.summary(),
        }
    }

    pub fn apply_mutation(&mut self, report: &crate::mutate::MutationReport) {
        self.mutation = report.to_json();
        let blind = report.blind_spots();
        for (rid, e) in self.entries.iter_mut() {
            if report.baseline_green.iter().any(|g| g == rid) {
                e.mutants_killed = Some(*report.kills.get(rid).unwrap_or(&0));
                e.blind_spots = Some(blind.get(rid).map(|v| v.len() as i64).unwrap_or(0));
                if e.mutants_killed == Some(0) {
                    e.record("VACUOUS", serde_json::json!({"detail": "green but detected no mutation"}));
                } else if e.blind_spots.unwrap_or(0) > 0 {
                    e.record(
                        "BLIND_SPOT",
                        serde_json::json!({"count": e.blind_spots, "detail": "ran mutated code without noticing"}),
                    );
                }
            }
        }
    }

    pub fn summary(&self) -> HashMap<String, Value> {
        let mut out = HashMap::from([
            (GREEN.to_string(), 0i64),
            (RED.to_string(), 0),
            (PENDING.to_string(), 0),
            (STALE.to_string(), 0),
            (ORPHAN.to_string(), 0),
        ]);
        for e in self.entries.values() {
            *out.entry(e.status.clone()).or_insert(0) += 1;
        }
        let total = self.entries.len() as i64;
        let live = total - out.get(ORPHAN).copied().unwrap_or(0);
        let green = out.get(GREEN).copied().unwrap_or(0);
        let pct = if live > 0 {
            (1000.0 * green as f64 / live as f64).round() / 10.0
        } else {
            0.0
        };
        let mut v: HashMap<String, Value> = out
            .into_iter()
            .map(|(k, n)| (k, Value::from(n)))
            .collect();
        v.insert("total".into(), Value::from(total));
        v.insert("completion_pct".into(), Value::from(pct));
        v.insert(
            "never_red".into(),
            Value::from(self.unfalsified_greens().len() as i64),
        );
        v
    }

    /// Green without a recorded fail. Those are the greens to distrust.
    pub fn unfalsified_greens(&self) -> Vec<&Entry> {
        self.entries
            .values()
            .filter(|e| e.status == GREEN && !e.was_ever_red)
            .collect()
    }

    pub fn note_amendment(&mut self, rid: &str, field: &str, was: &str, new_val: &str) {
        let row = serde_json::json!({
            "at": now(),
            "rid": rid,
            "field": field,
            "was": was,
            "now": new_val,
        });
        self.amendments.push(row.clone());
        if let Some(e) = self.entries.get_mut(rid) {
            e.first_colour = None;
            e.first_colour_at = None;
            e.was_ever_red = false;
            if e.status == GREEN {
                e.status = STALE.to_string();
            }
            e.record("amendment", row);
        }
    }

    /// True after a suite pass on the current spec. Spec edits clear last_run_at.
    pub fn suite_surveyed(&self) -> bool {
        let live: Vec<&Entry> = self
            .entries
            .values()
            .filter(|e| e.status != ORPHAN)
            .collect();
        !live.is_empty() && live.iter().any(|e| e.last_run_at.is_some())
    }

    /// Scenarios the spec names but no test bound — tests (or the Gherkin) need a pass.
    pub fn unbound_after_survey(&self) -> usize {
        self.entries
            .values()
            .filter(|e| {
                e.status != ORPHAN
                    && e.last_run_at.is_some()
                    && (e.status == PENDING
                        || e.failure
                            .as_deref()
                            .unwrap_or("")
                            .to_lowercase()
                            .contains("no test bound"))
            })
            .count()
    }

    /// Duplicate step definitions. Cucumber reports this as `[ambiguous]`;
    /// that is a tests miss, not a Gherkin miss.
    pub fn duplicate_step_failures(&self) -> usize {
        self.entries
            .values()
            .filter(|e| {
                if e.status == ORPHAN || e.status == GREEN {
                    return false;
                }
                e.failure
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains("multiple step definitions match")
            })
            .count()
    }

    /// Failures that look like the Gherkin is wrong, not the code.
    pub fn spec_shaped_failures(&self) -> usize {
        self.entries
            .values()
            .filter(|e| {
                if e.status == ORPHAN || e.status == GREEN {
                    return false;
                }
                let f = e.failure.as_deref().unwrap_or("").to_lowercase();
                if f.contains("multiple step definitions match") {
                    return false;
                }
                f.contains("underspec")
                    || f.contains("not specified")
                    || f.contains("contradict")
                    || f.contains("example is missing")
                    || f.contains("as a human")
                    || f.contains("ask_human")
                    || (f.contains("ambiguous") && !f.contains("step definition"))
            })
            .count()
    }

    /// A remaining ticket has no suite result on the current spec (edits clear last_run_at).
    pub fn remaining_need_survey(&self, focus_journey: &str) -> bool {
        let focus = focus_journey.trim();
        self.entries.values().any(|e| {
            e.status != GREEN
                && e.status != ORPHAN
                && (focus.is_empty() || e.epic == focus)
                && e.last_run_at.is_none()
        })
    }

    pub fn by_status(&self, status: &str) -> Vec<&Entry> {
        self.entries.values().filter(|e| e.status == status).collect()
    }
}

#[derive(Debug, Clone, Default)]
pub struct RunResult {
    pub outcome: String,
    pub detail: String,
    pub nodeid: String,
}

#[derive(Debug)]
pub struct ApplyOut {
    pub regressions: Vec<Value>,
    pub unknown_rids: Vec<String>,
    pub summary: HashMap<String, Value>,
}

#[derive(Default)]
struct HashSetLike(std::collections::HashSet<String>);
impl HashSetLike {
    fn insert(&mut self, s: String) {
        self.0.insert(s);
    }
    fn contains(&self, s: &str) -> bool {
        self.0.contains(s)
    }
}

#[cfg(test)]
mod save_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn unchanged_ledger_does_not_rewrite_generated_at() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ledger.json");
        let led = Ledger::default();
        led.save(&path).unwrap();
        let first = fs::read(&path).unwrap();
        std::thread::sleep(Duration::from_millis(1100));
        led.save(&path).unwrap();
        let second = fs::read(&path).unwrap();
        assert_eq!(first, second, "Play must not bump ledger hash mid-fill");
    }
}
