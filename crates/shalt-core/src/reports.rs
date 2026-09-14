use crate::ledger::RunResult;
use crate::spec::RID_PREFIX;
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

fn rid_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^@rid:(S-[0-9a-f]{8})$").unwrap())
}

fn rid_from_tags(tags: &[Value]) -> Option<String> {
    for t in tags {
        let mut name = if t.is_object() {
            t.get("name")?.as_str()?.trim().to_string()
        } else {
            t.as_str()?.trim().to_string()
        };
        if name.is_empty() {
            continue;
        }
        if !name.starts_with('@') {
            name = format!("@{name}");
        }
        if let Some(c) = rid_re().captures(&name) {
            return Some(c[1].to_string());
        }
        let _ = RID_PREFIX;
    }
    None
}

fn merge(out: &mut HashMap<String, RunResult>, rid: String, outcome: &str, detail: String, nodeid: String) {
    match out.get(&rid) {
        None => {
            out.insert(rid, RunResult { outcome: outcome.into(), detail, nodeid });
        }
        Some(prev) if prev.outcome == "passed" && outcome == "failed" => {
            out.insert(rid, RunResult { outcome: outcome.into(), detail, nodeid });
        }
        _ => {}
    }
}

pub fn parse_cucumber_json(text: &str) -> HashMap<String, RunResult> {
    let data: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let features = if data.is_array() { data.as_array().unwrap().clone() } else { vec![data] };
    let mut results = HashMap::new();
    for feature in features {
        let elements = feature.get("elements").and_then(|e| e.as_array()).cloned().unwrap_or_default();
        for el in elements {
            let ty = el.get("type").and_then(|t| t.as_str());
            if ty.is_some() && ty != Some("scenario") && ty != Some("scenario_outline") {
                continue;
            }
            let tags = el.get("tags").and_then(|t| t.as_array()).cloned().unwrap_or_default();
            let Some(rid) = rid_from_tags(&tags) else { continue };
            let mut failures = Vec::new();
            let mut statuses = Vec::new();
            if let Some(steps) = el.get("steps").and_then(|s| s.as_array()) {
                for step in steps {
                    let res = step.get("result").cloned().unwrap_or(Value::Object(Default::default()));
                    let status = res.get("status").and_then(|s| s.as_str()).unwrap_or("unknown").to_lowercase();
                    statuses.push(status.clone());
                    if status != "passed" {
                        let kw = step.get("keyword").and_then(|k| k.as_str()).unwrap_or("").trim();
                        let msg = res
                            .get("error_message")
                            .and_then(|m| m.as_str())
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| format!("step status: {status}"));
                        failures.push(format!("{kw} {}\n    {msg}", step.get("name").and_then(|n| n.as_str()).unwrap_or("")).trim().to_string());
                    }
                }
            }
            let outcome = if !statuses.is_empty() && statuses.iter().all(|s| s == "passed") {
                "passed"
            } else {
                "failed"
            };
            if statuses.is_empty() {
                failures.push("scenario reported no steps".into());
            }
            let nodeid = format!(
                "{}::{}",
                feature.get("uri").and_then(|u| u.as_str()).unwrap_or("?"),
                el.get("name").and_then(|n| n.as_str()).unwrap_or("?")
            );
            merge(&mut results, rid, outcome, failures.join("\n").chars().take(4000).collect(), nodeid);
        }
    }
    results
}

pub fn parse_native(text: &str) -> HashMap<String, RunResult> {
    let data: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let mut out = HashMap::new();
    if let Some(obj) = data.get("results").and_then(|r| r.as_object()) {
        for (k, v) in obj {
            out.insert(
                k.clone(),
                RunResult {
                    outcome: v.get("outcome").and_then(|o| o.as_str()).unwrap_or("failed").into(),
                    detail: v.get("detail").and_then(|d| d.as_str()).unwrap_or("").into(),
                    nodeid: v.get("nodeid").and_then(|n| n.as_str()).unwrap_or("").into(),
                },
            );
        }
    }
    out
}

pub fn parse_cucumber_messages(text: &str) -> HashMap<String, RunResult> {
    let mut pickle_rid = HashMap::new();
    let mut pickle_name = HashMap::new();
    let mut pickle_uri = HashMap::new();
    let mut case_pickle = HashMap::new();
    let mut started_case = HashMap::new();
    let mut step_status: HashMap<String, Vec<(String, String)>> = HashMap::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(env) = serde_json::from_str::<Value>(line) else { continue };
        if let Some(pk) = env.get("pickle") {
            let tags = pk.get("tags").and_then(|t| t.as_array()).cloned().unwrap_or_default();
            if let Some(rid) = rid_from_tags(&tags) {
                if let Some(id) = pk.get("id").and_then(|i| i.as_str()) {
                    pickle_rid.insert(id.to_string(), rid);
                }
            }
            if let Some(id) = pk.get("id").and_then(|i| i.as_str()) {
                pickle_name.insert(id.to_string(), pk.get("name").and_then(|n| n.as_str()).unwrap_or("?").into());
                pickle_uri.insert(id.to_string(), pk.get("uri").and_then(|n| n.as_str()).unwrap_or("?").into());
            }
        } else if let Some(tc) = env.get("testCase") {
            if let Some(id) = tc.get("id").and_then(|i| i.as_str()) {
                case_pickle.insert(id.to_string(), tc.get("pickleId").and_then(|p| p.as_str()).unwrap_or("").into());
            }
        } else if let Some(tcs) = env.get("testCaseStarted") {
            if let Some(id) = tcs.get("id").and_then(|i| i.as_str()) {
                started_case.insert(id.to_string(), tcs.get("testCaseId").and_then(|p| p.as_str()).unwrap_or("").into());
            }
        } else if let Some(tsf) = env.get("testStepFinished") {
            let res = tsf.get("testStepResult").cloned().unwrap_or(Value::Null);
            let status = res.get("status").and_then(|s| s.as_str()).unwrap_or("UNKNOWN").to_lowercase();
            let msg = res.get("message").and_then(|m| m.as_str()).unwrap_or("").to_string();
            if let Some(sid) = tsf.get("testCaseStartedId").and_then(|s| s.as_str()) {
                step_status.entry(sid.into()).or_default().push((status, msg));
            }
        }
    }
    let mut results = HashMap::new();
    for (started_id, statuses) in step_status {
        let case_id: &str = started_case.get(&started_id).map(String::as_str).unwrap_or("");
        let pid: String = case_pickle.get(case_id).cloned().unwrap_or_default();
        let Some(rid) = pickle_rid.get(&pid).cloned() else { continue };
        let outcome = if !statuses.is_empty() && statuses.iter().all(|(s, _)| s == "passed") {
            "passed"
        } else {
            "failed"
        };
        let detail: String = statuses
            .iter()
            .filter(|(s, _)| s != "passed")
            .map(|(s, m)| format!("[{s}] {m}").trim().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let nodeid = format!(
            "{}::{}",
            pickle_uri.get(&pid).map(String::as_str).unwrap_or("?"),
            pickle_name.get(&pid).map(String::as_str).unwrap_or("?")
        );
        merge(&mut results, rid, outcome, detail.chars().take(4000).collect(), nodeid);
    }
    results
}

pub fn read_report(path: &Path, fmt: &str) -> HashMap<String, RunResult> {
    if !path.exists() {
        return HashMap::new();
    }
    let text = std::fs::read_to_string(path).unwrap_or_default();
    if text.trim().is_empty() {
        return HashMap::new();
    }
    match fmt {
        "shalt" => parse_native(&text),
        "cucumber-json" => parse_cucumber_json(&text),
        "cucumber-messages" => parse_cucumber_messages(&text),
        _ => HashMap::new(),
    }
}
