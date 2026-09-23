//! Measured decode speed. How many fills fit in one auditor pass — not a try count.

use crate::org::Org;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SpeedBook {
    #[serde(default)]
    pub models: BTreeMap<String, SpeedSample>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SpeedSample {
    pub tok_s: f64,
    pub secs: f64,
    pub n: u32,
}

fn path() -> PathBuf {
    Org::home_dir().join("speed.json")
}

pub fn load() -> SpeedBook {
    let p = path();
    let Ok(raw) = fs::read_to_string(&p) else {
        return SpeedBook::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save(book: &SpeedBook) {
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Ok(raw) = serde_json::to_string_pretty(book) {
        let _ = fs::write(p, raw);
    }
}

pub fn sample(model: &str) -> Option<SpeedSample> {
    if model.is_empty() {
        return None;
    }
    load().models.get(model).cloned()
}

/// Running average of decode rate and wall time for `model`.
pub fn record(model: &str, tok_s: f64, secs: f64) {
    if model.is_empty() {
        return;
    }
    if tok_s <= 0.0 && secs <= 0.0 {
        return;
    }
    let mut book = load();
    let e = book.models.entry(model.to_string()).or_default();
    let n = e.n as f64;
    if tok_s > 0.0 {
        e.tok_s = if n == 0.0 {
            tok_s
        } else {
            (e.tok_s * n + tok_s) / (n + 1.0)
        };
    }
    if secs > 0.0 {
        e.secs = if n == 0.0 {
            secs
        } else {
            (e.secs * n + secs) / (n + 1.0)
        };
    }
    e.n += 1;
    save(&book);
}

pub fn record_fill(model: &str, secs: f64, completion: i64) {
    let tok_s = if secs > 0.0 && completion > 0 {
        completion as f64 / secs
    } else {
        0.0
    };
    record(model, tok_s, secs);
}

/// How many fills fit in one auditor pass. Measured. 1000 tok/s vs 25 → 40.
/// Unknown → 2 (more than one, so we measure). Never a constant like 3.
pub fn fill_budget(
    last_secs: f64,
    writer: Option<&SpeedSample>,
    auditor: Option<&SpeedSample>,
) -> usize {
    let fill = if last_secs > 0.0 {
        last_secs
    } else {
        writer.map(|s| s.secs).unwrap_or(0.0)
    };
    let audit = auditor.map(|s| s.secs).unwrap_or(0.0);
    let w_rate = writer.map(|s| s.tok_s).unwrap_or(0.0);
    let a_rate = auditor.map(|s| s.tok_s).unwrap_or(0.0);
    if fill > 0.0 && audit > 0.0 {
        return ((audit / fill).floor() as usize).max(1);
    }
    if w_rate > 0.0 && a_rate > 0.0 {
        return ((w_rate / a_rate).floor() as usize).max(1);
    }
    2
}

pub fn fill_budget_for(writer: &str, auditor: &str, last_secs: f64) -> usize {
    fill_budget(last_secs, sample(writer).as_ref(), sample(auditor).as_ref())
}

/// Room for another fill after `fills` already ran.
pub fn another_fill_fits(fills: usize, last_secs: f64, writer: &str, auditor: &str) -> bool {
    fills < fill_budget_for(writer, auditor, last_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(tok_s: f64, secs: f64) -> SpeedSample {
        SpeedSample { tok_s, secs, n: 1 }
    }

    #[test]
    fn unknown_speeds_budget_is_more_than_one() {
        assert_eq!(fill_budget(4.0, None, None), 2);
    }

    #[test]
    fn budget_is_audit_time_over_fill_time() {
        let w = s(250.0, 5.0);
        let a = s(22.0, 90.0);
        assert_eq!(fill_budget(5.0, Some(&w), Some(&a)), 18);
    }

    #[test]
    fn a_thousand_tok_s_buys_more_fills_than_two_hundred() {
        let flash = s(1000.0, 0.0);
        let mid = s(200.0, 0.0);
        let audit = s(25.0, 0.0);
        assert_eq!(fill_budget(0.0, Some(&flash), Some(&audit)), 40);
        assert_eq!(fill_budget(0.0, Some(&mid), Some(&audit)), 8);
        assert!(
            fill_budget(0.0, Some(&flash), Some(&audit))
                > fill_budget(0.0, Some(&mid), Some(&audit))
        );
    }

    #[test]
    fn slower_than_one_audit_is_a_single_fill() {
        let w = s(10.0, 100.0);
        let a = s(25.0, 80.0);
        assert_eq!(fill_budget(100.0, Some(&w), Some(&a)), 1);
    }
}
