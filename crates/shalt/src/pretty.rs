//! Cucumber pretty / progress / summary report.

use crate::term;
use shalt_core::ledger::{Ledger, RunResult, GREEN, ORPHAN, PENDING, RED, STALE};
use shalt_core::spec::{Feature, Scenario};
use shalt_core::tags::Pick;
use std::collections::HashMap;
use std::io::IsTerminal;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Pretty,
    Progress,
    Play,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Passed,
    Failed,
    Undefined,
    Pending,
    Skipped,
}

impl Kind {
    fn glyph(self) -> char {
        match self {
            Kind::Passed => '.',
            Kind::Failed => 'F',
            Kind::Undefined => 'U',
            Kind::Pending => 'P',
            Kind::Skipped => '-',
        }
    }

    fn label(self) -> &'static str {
        match self {
            Kind::Passed => "passed",
            Kind::Failed => "failed",
            Kind::Undefined => "undefined",
            Kind::Pending => "pending",
            Kind::Skipped => "skipped",
        }
    }

    fn paint(self, s: &str) -> String {
        match self {
            Kind::Passed => term::ok(s),
            Kind::Failed => term::bad(s),
            Kind::Undefined | Kind::Pending => term::warn(s),
            Kind::Skipped => term::mute(s),
        }
    }
}

pub fn resolve(name: &str) -> Format {
    match name {
        "progress" => Format::Progress,
        "play" => Format::Play,
        "pretty" => Format::Pretty,
        _ => {
            if std::io::stdout().is_terminal() {
                Format::Pretty
            } else {
                Format::Progress
            }
        }
    }
}

pub fn kind_of(
    sc: &Scenario,
    led: &Ledger,
    results: Option<&HashMap<String, RunResult>>,
) -> Kind {
    if let Some(rid) = sc.rid.as_deref() {
        if let Some(results) = results {
            if let Some(r) = results.get(rid) {
                return if r.outcome == "passed" {
                    Kind::Passed
                } else {
                    Kind::Failed
                };
            }
        }
        if let Some(e) = led.entries.get(rid) {
            return match e.status.as_str() {
                GREEN => Kind::Passed,
                RED => Kind::Failed,
                STALE => Kind::Pending,
                PENDING | ORPHAN => Kind::Undefined,
                _ => Kind::Undefined,
            };
        }
    }
    Kind::Undefined
}

fn failure_of(
    sc: &Scenario,
    led: &Ledger,
    results: Option<&HashMap<String, RunResult>>,
) -> Option<String> {
    if let Some(rid) = sc.rid.as_deref() {
        if let Some(results) = results {
            if let Some(r) = results.get(rid) {
                if r.outcome != "passed" && !r.detail.is_empty() {
                    return Some(r.detail.clone());
                }
            }
        }
        if let Some(e) = led.entries.get(rid) {
            if e.status == RED {
                return e.failure.clone();
            }
        }
    }
    None
}

#[derive(Default)]
pub struct Counts {
    pub failed: usize,
    pub skipped: usize,
    pub undefined: usize,
    pub pending: usize,
    pub passed: usize,
}

impl Counts {
    fn add(&mut self, k: Kind) {
        match k {
            Kind::Failed => self.failed += 1,
            Kind::Skipped => self.skipped += 1,
            Kind::Undefined => self.undefined += 1,
            Kind::Pending => self.pending += 1,
            Kind::Passed => self.passed += 1,
        }
    }

    fn total(&self) -> usize {
        self.failed + self.skipped + self.undefined + self.pending + self.passed
    }

    fn phrase(&self, word: &str) -> String {
        let n = self.total();
        let noun = if n == 1 {
            word.to_string()
        } else {
            format!("{word}s")
        };
        let parts = [
            (self.failed, "failed"),
            (self.skipped, "skipped"),
            (self.undefined, "undefined"),
            (self.pending, "pending"),
            (self.passed, "passed"),
        ];
        let inner: Vec<String> = parts
            .iter()
            .filter(|(c, _)| *c > 0)
            .map(|(c, l)| format!("{c} {l}"))
            .collect();
        if inner.is_empty() {
            format!("{n} {noun}")
        } else {
            format!("{n} {noun} ({})", inner.join(", "))
        }
    }
}

fn tally(features: &[Feature], picks: &[Pick], led: &Ledger, results: Option<&HashMap<String, RunResult>>) -> (Counts, Counts) {
    let mut scenarios = Counts::default();
    let mut steps = Counts::default();
    for p in picks {
        let sc = &features[p.feature].scenarios[p.scenario];
        let k = kind_of(sc, led, results);
        scenarios.add(k);
        let n = sc.steps.len().max(1);
        match k {
            Kind::Passed => {
                for _ in 0..n {
                    steps.add(Kind::Passed);
                }
            }
            Kind::Failed => {
                steps.add(Kind::Failed);
                for _ in 1..n {
                    steps.add(Kind::Skipped);
                }
            }
            Kind::Undefined => {
                for _ in 0..n {
                    steps.add(Kind::Undefined);
                }
            }
            Kind::Pending => {
                for _ in 0..n {
                    steps.add(Kind::Pending);
                }
            }
            Kind::Skipped => {
                for _ in 0..n {
                    steps.add(Kind::Skipped);
                }
            }
        }
    }
    (scenarios, steps)
}

fn duration_str(d: f64) -> String {
    let m = (d / 60.0).floor() as u64;
    let s = d - (m as f64 * 60.0);
    format!("{m}m{s:.3}s")
}

fn comment_line(left: &str, comment: &str) -> String {
    let pad = 52usize.saturating_sub(left.len()).max(1);
    format!(
        "{left}{}{}",
        " ".repeat(pad),
        term::mute(&format!("# {comment}"))
    )
}

fn spec_loc(file: &str, line: usize) -> String {
    let path = if file.contains('/') || file.starts_with("spec") {
        file.to_string()
    } else {
        format!("spec/{file}")
    };
    format!("{path}:{line}")
}

fn paint_tags(tags: &[String], indent: &str) {
    if tags.is_empty() {
        return;
    }
    let painted: Vec<String> = tags
        .iter()
        .map(|t| {
            if t.starts_with("@rid:") {
                term::mute(t)
            } else {
                term::accent(t)
            }
        })
        .collect();
    println!("{indent}{}", painted.join(" "));
}

pub fn render(
    features: &[Feature],
    picks: &[Pick],
    led: &Ledger,
    results: Option<&HashMap<String, RunResult>>,
    format: Format,
    duration: Option<f64>,
) {
    let (sc_counts, step_counts) = tally(features, picks, led, results);
    match format {
        Format::Progress => print_progress(features, picks, led, results),
        Format::Play => print_play(features, picks, led, results),
        Format::Pretty => print_pretty(features, picks, led, results),
    }
    let failing = failing_locs(features, picks, led, results);
    if !failing.is_empty() && format != Format::Play {
        println!();
        println!("{}", term::bad("Failing Scenarios:"));
        for loc in &failing {
            println!("{loc}");
        }
    }
    println!();
    println!("{}", sc_counts.phrase("scenario"));
    println!("{}", step_counts.phrase("step"));
    if let Some(d) = duration {
        println!("{}", duration_str(d));
    }
}

fn failing_locs(
    features: &[Feature],
    picks: &[Pick],
    led: &Ledger,
    results: Option<&HashMap<String, RunResult>>,
) -> Vec<String> {
    let mut out = Vec::new();
    for p in picks {
        let sc = &features[p.feature].scenarios[p.scenario];
        if kind_of(sc, led, results) == Kind::Failed {
            out.push(format!("shalt {}", spec_loc(&sc.feature_file, sc.line)));
        }
    }
    out
}

fn print_pretty(
    features: &[Feature],
    picks: &[Pick],
    led: &Ledger,
    results: Option<&HashMap<String, RunResult>>,
) {
    let mut last_fi: Option<usize> = None;
    for p in picks {
        if last_fi != Some(p.feature) {
            if last_fi.is_some() {
                println!();
            }
            let f = &features[p.feature];
            paint_tags(&f.tags, "");
            println!("{}", term::bold(&term::keyword(&format!("Feature: {}", f.name))));
            println!();
            last_fi = Some(p.feature);
        }
        let sc = &features[p.feature].scenarios[p.scenario];
        let k = kind_of(sc, led, results);
        paint_tags(&sc.tags, "  ");
        let head = format!("  {}: {}", sc.keyword, sc.name);
        let loc = spec_loc(&sc.feature_file, sc.line);
        println!("{}", k.paint(&comment_line(&head, &loc)));
        for (i, step) in sc.steps.iter().enumerate() {
            let sk = step_kind(k, i, sc.steps.len());
            println!("{}", sk.paint(&format!("    {step}")));
        }
        if let Some(fail) = failure_of(sc, led, results) {
            for line in fail.lines() {
                println!("      {}", term::bad(line));
            }
        }
        println!();
    }
}

fn step_kind(scenario: Kind, i: usize, n: usize) -> Kind {
    match scenario {
        Kind::Failed if i + 1 < n => Kind::Skipped,
        other => other,
    }
}

fn print_progress(
    features: &[Feature],
    picks: &[Pick],
    led: &Ledger,
    results: Option<&HashMap<String, RunResult>>,
) {
    let mut buf = String::new();
    for p in picks {
        let sc = &features[p.feature].scenarios[p.scenario];
        let k = kind_of(sc, led, results);
        buf.push_str(&k.paint(&k.glyph().to_string()));
    }
    println!("{buf}");
}

fn print_play(
    features: &[Feature],
    picks: &[Pick],
    led: &Ledger,
    results: Option<&HashMap<String, RunResult>>,
) {
    println!("PLAY start");
    for p in picks {
        let sc = &features[p.feature].scenarios[p.scenario];
        let k = kind_of(sc, led, results);
        let rid = sc.rid.clone().unwrap_or_default();
        match k {
            Kind::Passed => println!("PLAY ok {rid} {}", sc.name),
            Kind::Failed => println!("PLAY failed {rid} {}", sc.name),
            other => println!("PLAY {} {rid} {}", other.label(), sc.name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_is_cucumber_shaped() {
        assert_eq!(duration_str(1.204), "0m1.204s");
        assert_eq!(duration_str(65.0), "1m5.000s");
    }

    #[test]
    fn summary_omits_zero_buckets() {
        let c = Counts {
            failed: 1,
            passed: 2,
            ..Counts::default()
        };
        assert_eq!(c.phrase("scenario"), "3 scenarios (1 failed, 2 passed)");
        let one = Counts {
            passed: 1,
            ..Counts::default()
        };
        assert_eq!(one.phrase("scenario"), "1 scenario (1 passed)");
    }
}
