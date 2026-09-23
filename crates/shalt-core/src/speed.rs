//! Measured decode speed. Retry and pick from samples, not a magic try count.

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

/// Another fill on `writer` after `attempts` already ran.
///
/// No magic count. One try always happens in the caller. A second try is
/// taken when we have no comparison yet (so we measure) or the writer is
/// faster than the auditor. Further tries only while another fill still
/// fits inside one measured auditor pass.
pub fn retry_same_writer(writer: &str, auditor: &str, attempts: usize, last_secs: f64) -> bool {
    retry_with(
        sample(writer).as_ref(),
        sample(auditor).as_ref(),
        attempts,
        last_secs,
    )
}

pub fn retry_with(
    writer: Option<&SpeedSample>,
    auditor: Option<&SpeedSample>,
    attempts: usize,
    last_secs: f64,
) -> bool {
    if attempts == 0 {
        return true;
    }
    let fill = if last_secs > 0.0 {
        last_secs
    } else {
        writer.map(|s| s.secs).unwrap_or(0.0)
    };
    let audit = auditor.map(|s| s.secs).unwrap_or(0.0);
    let w_rate = writer.map(|s| s.tok_s).unwrap_or(0.0);
    let a_rate = auditor.map(|s| s.tok_s).unwrap_or(0.0);

    let faster = if w_rate > 0.0 && a_rate > 0.0 {
        w_rate > a_rate
    } else if fill > 0.0 && audit > 0.0 {
        fill < audit
    } else {
        // No comparison yet: one extra try so we have a measurement, then hop.
        return attempts < 2;
    };
    if !faster {
        return false;
    }
    if audit > 0.0 && fill > 0.0 {
        return (attempts as f64 + 1.0) * fill < audit;
    }
    attempts < 2
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(tok_s: f64, secs: f64) -> SpeedSample {
        SpeedSample { tok_s, secs, n: 1 }
    }

    #[test]
    fn unknown_speeds_get_one_extra_try_then_stop() {
        assert!(retry_with(None, None, 1, 4.0));
        assert!(!retry_with(None, None, 2, 4.0));
    }

    #[test]
    fn slower_writer_does_not_retry() {
        assert!(!retry_with(
            Some(&s(10.0, 100.0)),
            Some(&s(25.0, 80.0)),
            1,
            100.0
        ));
    }

    #[test]
    fn faster_writer_retries_while_another_fill_fits_in_one_audit() {
        let w = s(250.0, 5.0);
        let a = s(22.0, 90.0);
        assert!(retry_with(Some(&w), Some(&a), 1, 5.0));
        assert!(retry_with(Some(&w), Some(&a), 8, 5.0));
        // 17 * 5 = 85 < 90; 18 * 5 = 90 is not strictly inside.
        assert!(retry_with(Some(&w), Some(&a), 16, 5.0));
        assert!(!retry_with(Some(&w), Some(&a), 17, 5.0));
        // 3 is not a policy: (3+1)*5=20 < 90, so it retries because the clock says so.
        assert!(retry_with(Some(&w), Some(&a), 3, 5.0));
    }

    #[test]
    fn tok_s_alone_allows_a_second_try_when_faster() {
        let w = s(250.0, 0.0);
        let a = s(22.0, 0.0);
        assert!(retry_with(Some(&w), Some(&a), 1, 0.0));
        assert!(!retry_with(Some(&w), Some(&a), 2, 0.0));
    }
}
