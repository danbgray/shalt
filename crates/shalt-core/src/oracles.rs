//! Then observation surfaces. Locked in the spec before Play, like a threshold.
//! Changing a Then or its surface after Play is an amendment, not a silent edit.

use crate::spec::Feature;
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ThenOracle {
    pub then: String,
    pub observe: String,
}

impl ThenOracle {
    pub fn lock_hash(&self) -> String {
        let mut h = Sha256::new();
        h.update(self.then.trim().as_bytes());
        h.update(b"\n");
        h.update(self.observe.trim().as_bytes());
        format!("sha256:{}", &hex::encode(h.finalize())[..24])
    }
}

fn is_scenario_start(t: &str) -> bool {
    t.starts_with("Scenario:") || t.starts_with("Scenario Outline:")
}

fn step_keyword(t: &str) -> Option<&'static str> {
    let t = t.trim();
    for k in ["Given ", "When ", "Then ", "And ", "But ", "* "] {
        if t.len() >= k.len() && t[..k.len()].eq_ignore_ascii_case(k) {
            return Some(k.trim());
        }
    }
    None
}

fn phrase_after_keyword(t: &str) -> String {
    let t = t.trim();
    for k in ["Given ", "When ", "Then ", "And ", "But ", "* "] {
        if t.len() >= k.len() && t[..k.len()].eq_ignore_ascii_case(k) {
            return t[k.len()..].trim().to_string();
        }
    }
    t.to_string()
}

/// `#observe:` is a spec comment (not a markdown heading). Cucumber ignores it.
pub fn observe_line(t: &str) -> Option<String> {
    let t = t.trim();
    let rest = t.strip_prefix("#observe:")?;
    let s = rest.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// Every Then (and And/But after Then) must name a door the implementation cannot fake.
pub fn missing_observe(content: &str) -> Option<String> {
    let mut scenario = String::from("(top)");
    let mut pending: Option<String> = None;
    let mut in_then = false;
    let flush = |pending: &mut Option<String>, scenario: &str| -> Option<String> {
        pending.take().map(|then| {
            format!(
                "Scenario {scenario:?} Then {then:?} needs #observe: <surface> — a door chosen after the code exists is not a different door"
            )
        })
    };
    for raw in content.lines() {
        let t = raw.trim();
        if t.is_empty() {
            continue;
        }
        if is_scenario_start(t) {
            if let Some(msg) = flush(&mut pending, &scenario) {
                return Some(msg);
            }
            scenario = t
                .split_once(':')
                .map(|(_, n)| n.trim().to_string())
                .unwrap_or_else(|| t.to_string());
            in_then = false;
            continue;
        }
        if t.starts_with("Feature:")
            || t.starts_with("Background:")
            || t.starts_with("Examples:")
            || t.starts_with("@")
        {
            continue;
        }
        if let Some(surface) = observe_line(t) {
            let _ = surface;
            if pending.take().is_none() {
                return Some(format!(
                    "Scenario {scenario:?} has #observe: with no Then above it"
                ));
            }
            continue;
        }
        if let Some(kw) = step_keyword(t) {
            if kw.eq_ignore_ascii_case("then")
                || (in_then && (kw.eq_ignore_ascii_case("and") || kw.eq_ignore_ascii_case("but")))
            {
                if let Some(msg) = flush(&mut pending, &scenario) {
                    return Some(msg);
                }
                pending = Some(phrase_after_keyword(t));
                in_then = true;
                continue;
            }
            if let Some(msg) = flush(&mut pending, &scenario) {
                return Some(msg);
            }
            in_then = false;
        }
    }
    flush(&mut pending, &scenario)
}

pub fn parse_oracles(content: &str) -> Vec<(String, Vec<ThenOracle>)> {
    let mut out: Vec<(String, Vec<ThenOracle>)> = Vec::new();
    let mut scenario = String::new();
    let mut oracles: Vec<ThenOracle> = Vec::new();
    let mut pending: Option<String> = None;
    let mut in_then = false;
    let push = |out: &mut Vec<(String, Vec<ThenOracle>)>,
                scenario: &str,
                oracles: &mut Vec<ThenOracle>| {
        if !scenario.is_empty() {
            out.push((scenario.to_string(), std::mem::take(oracles)));
        }
    };
    for raw in content.lines() {
        let t = raw.trim();
        if t.is_empty() {
            continue;
        }
        if is_scenario_start(t) {
            if pending.is_some() {
                pending = None;
            }
            push(&mut out, &scenario, &mut oracles);
            scenario = t
                .split_once(':')
                .map(|(_, n)| n.trim().to_string())
                .unwrap_or_else(|| t.to_string());
            in_then = false;
            continue;
        }
        if let Some(surface) = observe_line(t) {
            if let Some(then) = pending.take() {
                oracles.push(ThenOracle {
                    then,
                    observe: surface,
                });
            }
            continue;
        }
        if let Some(kw) = step_keyword(t) {
            if kw.eq_ignore_ascii_case("then")
                || (in_then && (kw.eq_ignore_ascii_case("and") || kw.eq_ignore_ascii_case("but")))
            {
                pending = Some(phrase_after_keyword(t));
                in_then = true;
                continue;
            }
            pending = None;
            in_then = false;
        }
    }
    push(&mut out, &scenario, &mut oracles);
    out
}

pub fn attach_oracles(feature: &mut Feature, source: &str) {
    let parsed = parse_oracles(source);
    for sc in &mut feature.scenarios {
        sc.oracles = parsed
            .iter()
            .find(|(n, _)| n == &sc.name)
            .map(|(_, o)| o.clone())
            .unwrap_or_default();
    }
}

pub fn feature_oracles_ready(features: &[Feature]) -> Result<(), String> {
    for f in features {
        for s in &f.scenarios {
            let then_n = s
                .steps
                .iter()
                .filter(|st| {
                    let k = st.split_whitespace().next().unwrap_or("");
                    k.eq_ignore_ascii_case("then")
                })
                .count();
            // And-after-Then also count in oracles; require at least one observe per Then keyword.
            if then_n > 0 && s.oracles.is_empty() {
                return Err(format!(
                    "Scenario {:?} needs #observe: on each Then before Play — lock the surface in the spec, not after the code exists",
                    s.name
                ));
            }
            let then_like = s
                .oracles
                .len();
            if then_n > 0 && then_like < then_n {
                return Err(format!(
                    "Scenario {:?} has {then_n} Then line(s) but {} #observe: — every Then needs a surface",
                    s.name,
                    then_like
                ));
            }
        }
    }
    Ok(())
}

pub fn oracle_lock_map(features: &[Feature]) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for f in features {
        for s in &f.scenarios {
            let Some(rid) = s.rid.as_ref() else { continue };
            let arr: Vec<serde_json::Value> = s
                .oracles
                .iter()
                .map(|o| {
                    serde_json::json!({
                        "then": o.then,
                        "observe": o.observe,
                        "hash": o.lock_hash(),
                    })
                })
                .collect();
            map.insert(rid.clone(), serde_json::Value::Array(arr));
        }
    }
    serde_json::Value::Object(map)
}

fn ident_calls(body: &str) -> Vec<String> {
    let re = regex::Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*\(").unwrap();
    re.captures_iter(body)
        .map(|c| c[1].to_string())
        .filter(|n| {
            !matches!(
                n.as_str(),
                "function"
                    | "if"
                    | "for"
                    | "while"
                    | "switch"
                    | "catch"
                    | "assert"
                    | "equal"
                    | "ok"
                    | "strictEqual"
                    | "deepEqual"
                    | "expect"
                    | "panic"
                    | "format"
                    | "vec"
                    | "Some"
                    | "Ok"
                    | "Err"
                    | "String"
                    | "from"
                    | "new"
                    | "Number"
                    | "encodeURIComponent"
                    | "encodeURI"
            )
        })
        .collect()
}

/// Names a When body typically *does*. A Then that calls these is acting.
const ACT_FNS: &[&str] = &[
    "createRecipe",
    "addIngredient",
    "addStep",
    "changeStep",
    "publish",
    "setVisibility",
    "attachFullVideo",
    "tagStepTimestamp",
    "createPacket",
    "subscribePatron",
    "enablePatronage",
    "setPatronOnly",
    "setAssociateTag",
    "clearAssociateTag",
    "resetStore",
    "cancelPatronage",
];

/// Then must observe. Acting, tautology, or no assert is a fake door.
pub fn then_body_lint(body: &str, when_calls: &[String]) -> Option<String> {
    let t = body.trim();
    if t.is_empty() {
        return Some("empty Then body".into());
    }
    let low = t.to_ascii_lowercase();
    if low.contains("return 'pending'") || low.contains("return \"pending\"") {
        return None; // stub, not a fake green
    }
    let calls = ident_calls(t);
    let mut acts: Vec<String> = calls
        .iter()
        .filter(|n| ACT_FNS.contains(&n.as_str()) || when_calls.iter().any(|w| w == *n))
        .cloned()
        .collect();
    acts.sort();
    acts.dedup();
    if !acts.is_empty() {
        return Some(format!(
            "Then acts ({}) — observe, don't perform the When",
            acts.join(", ")
        ));
    }
    if low.contains("assert.ok(this.") || low.contains("assert.ok( this.") {
        return Some("Then is assert.ok(this.…); a handle is not an observation".into());
    }
    if t.lines().any(|l| {
        let ll = l.to_ascii_lowercase();
        ll.contains("assert") && l.contains("||")
    }) {
        return Some("Then has an || escape; a door that always opens is not a door".into());
    }
    let asserts = low.contains("assert.")
        || low.contains("assert!")
        || low.contains("assert_eq")
        || low.contains("assert_ne")
        || low.contains("expect(")
        || low.contains("panic!");
    if !asserts {
        return Some("Then has no assert — nothing that can fail for the right reason".into());
    }
    None
}

pub fn lint_then_defs(defs: &[crate::bindings::StepDef]) -> Vec<String> {
    let when_calls: Vec<String> = defs
        .iter()
        .filter(|d| d.kw == "when")
        .flat_map(|d| ident_calls(&d.body))
        .collect();
    let mut out = Vec::new();
    for d in defs.iter().filter(|d| d.kw == "then" && !d.stub) {
        if let Some(why) = then_body_lint(&d.body, &when_calls) {
            let excerpt: String = d
                .body
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .take(4)
                .collect::<Vec<_>>()
                .join(" ");
            out.push(format!(
                "acting Then '{}' — {why}: {excerpt}",
                d.pattern
            ));
        }
    }
    out
}

pub fn oracles_ready_in_dir(spec_dir: &Path) -> Result<(), String> {
    let features = crate::spec::load_specs(spec_dir, false).map_err(|e| e.to_string())?;
    feature_oracles_ready(&features)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn then_without_observe_is_missing() {
        let spec = "@epic:a\nFeature: A\n  Scenario: S\n    When x\n    Then y\n";
        let err = missing_observe(spec).expect("missing");
        assert!(err.contains("#observe:"), "{err}");
    }

    #[test]
    fn observe_locks_the_door() {
        let spec = concat!(
            "@epic:a\nFeature: A\n  Scenario: S\n",
            "    When I publish the recipe\n",
            "    Then a stranger sees the title\n",
            "    #observe: unsigned GET of the public share URL shows the title\n",
        );
        assert_eq!(missing_observe(spec), None);
        let parsed = parse_oracles(spec);
        assert_eq!(parsed[0].1[0].observe.contains("unsigned GET"), true);
    }

    #[test]
    fn markdown_heading_is_not_observe() {
        assert!(observe_line("# observe: nope").is_none());
        assert!(observe_line("#observe: yes").is_some());
    }

    #[test]
    fn then_that_tags_then_asserts_is_acting() {
        let body = "  attachFullVideo(this.lastRecipe, 'https://example.com/pasta.mp4');\n  tagStepTimestamp(this.lastRecipe, 1, '00:00:12');\n  assert.equal(stepTimestamp(this.lastRecipe, 1), '00:00:12');\n";
        let why = then_body_lint(body, &[]).expect("act");
        assert!(why.contains("acts"), "{why}");
    }

    #[test]
    fn then_assert_ok_handle_is_acting() {
        let why = then_body_lint("  assert.ok(this.lastPacket);\n", &[]).expect("ok");
        assert!(why.contains("handle"), "{why}");
    }

    #[test]
    fn then_or_escape_is_acting() {
        let why = then_body_lint(
            "  assert.ok(String(url || '').includes('pasta-20') || String(url || '').length > 0);\n",
            &[],
        )
        .expect("or");
        assert!(why.contains("||"), "{why}");
    }

    #[test]
    fn then_that_only_reads_is_ok() {
        let body = "  const url = this.lastShare && (this.lastShare.url || this.lastShare.href);\n  assert.equal(viewRecipe(null, this.lastRecipe).visibility, 'public');\n  assert.ok(url);\n";
        assert_eq!(then_body_lint(body, &[]), None);
    }
}
