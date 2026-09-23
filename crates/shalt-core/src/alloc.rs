//! Project model pool: random first, then fit, weighted by cost vs time.

use crate::board::{Board, PoolSlot};
use crate::jobs::{Job, JobStatus};
use crate::ledger::Ledger;
use crate::spec::Feature;
use crate::tokens::{agent_fits, epic_name, normalize_backend, AgentFit};
use rand::seq::SliceRandom;
use rand::Rng;

#[derive(Debug, Clone)]
pub struct AllocPick {
    pub backend: String,
    pub model: String,
    pub why: String,
}

#[derive(Debug, Clone)]
pub struct AllocChange {
    pub rid: String,
    pub backend: String,
    pub model: String,
    pub why: String,
}

pub fn default_pool() -> Vec<PoolSlot> {
    vec![
        PoolSlot {
            backend: "qwen".into(),
            model: crate::api::DEFAULT_QWEN_MODEL.into(),
        },
        PoolSlot {
            backend: "grok".into(),
            model: crate::api::DEFAULT_GROK_MODEL.into(),
        },
        PoolSlot {
            backend: "claude".into(),
            model: "claude-sonnet-4-5".into(),
        },
    ]
}

pub fn is_local(backend: &str) -> bool {
    matches!(normalize_backend(backend).as_str(), "qwen" | "ollama")
}

/// Relative cost of a token. Local is nearly free; cloud is the unit.
pub fn cost_factor(backend: &str) -> f64 {
    if is_local(backend) {
        0.05
    } else {
        1.0
    }
}

/// Prior if we have no measured duration. Local is slower.
pub fn time_prior(backend: &str) -> f64 {
    if is_local(backend) {
        2.5
    } else {
        1.0
    }
}

fn prefer_weights(prefer: &str) -> (f64, f64, f64) {
    match prefer {
        "cheap" => (0.35, 0.55, 0.10),
        "fast" => (0.35, 0.10, 0.55),
        _ => (0.40, 0.30, 0.30),
    }
}

fn avg_secs(fits: &[AgentFit], backend: &str, model: &str, epic: &str) -> Option<f64> {
    let hit = fits
        .iter()
        .find(|f| f.backend == backend && f.model == model && f.epic == epic && f.jobs > 0)
        .or_else(|| {
            fits.iter()
                .find(|f| f.backend == backend && f.model == model && f.epic.is_empty() && f.jobs > 0)
        })?;
    if hit.secs > 0 && hit.jobs > 0 {
        Some(hit.secs as f64 / hit.jobs as f64)
    } else {
        None
    }
}

fn samples(fits: &[AgentFit], backend: &str, model: &str, epic: &str) -> usize {
    fits.iter()
        .find(|f| f.backend == backend && f.model == model && f.epic == epic)
        .or_else(|| {
            fits.iter()
                .find(|f| f.backend == backend && f.model == model && f.epic.is_empty())
        })
        .map(|f| f.jobs)
        .unwrap_or(0)
}

fn accuracy(fits: &[AgentFit], backend: &str, model: &str, epic: &str) -> Option<f64> {
    fits.iter()
        .find(|f| f.backend == backend && f.model == model && f.epic == epic && f.accuracy.is_some())
        .or_else(|| {
            fits.iter().find(|f| {
                f.backend == backend && f.model == model && f.epic.is_empty() && f.accuracy.is_some()
            })
        })
        .and_then(|f| f.accuracy)
}

fn score_slot(
    slot: &PoolSlot,
    fits: &[AgentFit],
    epic: &str,
    prefer: &str,
    fastest: f64,
) -> f64 {
    let (wa, wc, wt) = prefer_weights(prefer);
    let acc = accuracy(fits, &slot.backend, &slot.model, epic).unwrap_or(1.0);
    let acc_pen = (acc - 1.0).abs();
    let cost = cost_factor(&slot.backend);
    let time = avg_secs(fits, &slot.backend, &slot.model, epic)
        .map(|s| (s / fastest).clamp(0.5, 4.0))
        .unwrap_or_else(|| time_prior(&slot.backend));
    wa * acc_pen + wc * cost + wt * time
}

pub fn pick_for<R: Rng>(
    board: &Board,
    ledger: &Ledger,
    jobs: &[Job],
    project_id: &str,
    epic: &str,
    rng: &mut R,
) -> AllocPick {
    let mut pool = if board.pool.is_empty() {
        default_pool()
    } else {
        board.pool.clone()
    };
    if crate::compose::backend_quota_exhausted(jobs, "grok") {
        pool.retain(|s| crate::tokens::normalize_backend(&s.backend) != "grok");
    }
    if pool.is_empty() {
        pool = vec![PoolSlot {
            backend: "qwen".into(),
            model: crate::api::DEFAULT_QWEN_MODEL.into(),
        }];
    }
    let prefer = if board.prefer.is_empty() {
        "balanced"
    } else {
        board.prefer.as_str()
    };
    let fits = agent_fits(board, jobs, ledger, project_id);
    let under: Vec<_> = pool
        .iter()
        .filter(|s| samples(&fits, &s.backend, &s.model, epic) < 2)
        .cloned()
        .collect();
    let total: usize = pool
        .iter()
        .map(|s| samples(&fits, &s.backend, &s.model, epic))
        .sum();
    let explore = !under.is_empty() || total < pool.len() || rng.gen_bool(0.15);
    if explore {
        let choice = if !under.is_empty() {
            under.choose(rng).cloned().unwrap()
        } else {
            pool.choose(rng).cloned().unwrap_or_else(|| pool[0].clone())
        };
        return AllocPick {
            why: if total < pool.len() || !under.is_empty() {
                "explore — random until this model has enough history on this epic".into()
            } else {
                "explore — keep sampling so fit does not freeze".into()
            },
            backend: choice.backend,
            model: choice.model,
        };
    }
    let fastest = pool
        .iter()
        .filter_map(|s| avg_secs(&fits, &s.backend, &s.model, epic))
        .fold(f64::INFINITY, f64::min);
    let fastest = if fastest.is_finite() { fastest.max(1.0) } else { 1.0 };
    let mut best: Option<(f64, PoolSlot)> = None;
    for s in &pool {
        let sc = score_slot(s, &fits, epic, prefer, fastest);
        match &best {
            Some((b, _)) if *b <= sc => {}
            _ => best = Some((sc, s.clone())),
        }
    }
    let (sc, slot) = best.unwrap_or_else(|| (0.0, pool[0].clone()));
    let acc = accuracy(&fits, &slot.backend, &slot.model, epic)
        .map(|a| format!("{a:.1}×"))
        .unwrap_or_else(|| "no ratio yet".into());
    let kind = if is_local(&slot.backend) {
        "local · nearly free · slower"
    } else {
        "cloud · paid · faster"
    };
    AllocPick {
        why: format!("fit · {acc} · {kind} · prefer {prefer} · score {sc:.2}"),
        backend: slot.backend,
        model: slot.model,
    }
}

/// Fill tickets that have no agent. Does not overwrite a human assignment.
pub fn allocate_unassigned<R: Rng>(
    board: &mut Board,
    ledger: &Ledger,
    jobs: &[Job],
    project_id: &str,
    rng: &mut R,
) -> Vec<AllocChange> {
    if board.pool.is_empty() {
        board.pool = default_pool();
    }
    if board.prefer.is_empty() {
        board.prefer = "balanced".into();
    }
    let rids: Vec<(String, String)> = board
        .items
        .iter()
        .filter(|i| i.backend.is_empty() && i.model.is_empty())
        .map(|i| (i.rid.clone(), epic_name(ledger, &i.rid)))
        .collect();
    let mut out = Vec::new();
    for (rid, epic) in rids {
        let pick = pick_for(board, ledger, jobs, project_id, &epic, rng);
        board.set_item_agent(&rid, &pick.backend, &pick.model);
        out.push(AllocChange {
            rid,
            backend: pick.backend,
            model: pick.model,
            why: pick.why,
        });
    }
    crate::tokens::stamp_forecasts(board, ledger, jobs, project_id, &[]);
    out
}

pub fn allocate_unassigned_now(
    board: &mut Board,
    ledger: &Ledger,
    jobs: &[Job],
    project_id: &str,
) -> Vec<AllocChange> {
    allocate_unassigned(board, ledger, jobs, project_id, &mut rand::thread_rng())
}

fn live_rids(jobs: &[Job], project_id: &str) -> std::collections::HashSet<String> {
    jobs.iter()
        .filter(|j| {
            j.project_id == project_id
                && matches!(
                    j.status,
                    JobStatus::Running | JobStatus::Pending | JobStatus::Waiting
                )
                && !j.rid.is_empty()
        })
        .map(|j| j.rid.clone())
        .collect()
}

/// Reshuffle agents on idle tickets. Running work is left alone.
pub fn reallocate<R: Rng>(
    board: &mut Board,
    ledger: &Ledger,
    jobs: &[Job],
    project_id: &str,
    rng: &mut R,
) -> Vec<AllocChange> {
    if board.pool.is_empty() {
        board.pool = default_pool();
    }
    if board.prefer.is_empty() {
        board.prefer = "balanced".into();
    }
    let held = live_rids(jobs, project_id);
    let rids: Vec<(String, String)> = board
        .items
        .iter()
        .filter(|i| !held.contains(&i.rid))
        .map(|i| (i.rid.clone(), epic_name(ledger, &i.rid)))
        .collect();
    let mut out = Vec::new();
    for (rid, epic) in rids {
        let pick = pick_for(board, ledger, jobs, project_id, &epic, rng);
        board.set_item_agent(&rid, &pick.backend, &pick.model);
        out.push(AllocChange {
            rid,
            backend: pick.backend,
            model: pick.model,
            why: pick.why,
        });
    }
    crate::tokens::stamp_forecasts(board, ledger, jobs, project_id, &[]);
    out
}

pub fn reallocate_now(
    board: &mut Board,
    ledger: &Ledger,
    jobs: &[Job],
    project_id: &str,
) -> Vec<AllocChange> {
    reallocate(board, ledger, jobs, project_id, &mut rand::thread_rng())
}

/// Seat backlog on the open sprint and assign every ticket that has no agent.
pub fn prepare_board(
    board: &mut Board,
    ledger: &Ledger,
    jobs: &[Job],
    project_id: &str,
    features: &[Feature],
) -> bool {
    let seated = crate::sprint::seat_sprint_slice(board, ledger);
    let assigned = allocate_unassigned_now(board, ledger, jobs, project_id);
    let stamped = crate::tokens::stamp_forecasts(board, ledger, jobs, project_id, features);
    seated > 0 || !assigned.is_empty() || stamped > 0
}

/// Fast coder vs slower reviewer. Inner loop (tests + code) stays Fast so
/// Ollama does not unload a tiny MLX model to fetch 27B every turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    Fast,
    Review,
}

/// Measured >>200 tok/s on this Mac with thinking off. Gemma / 0.5b have no tools.
pub const FLASH_MODELS: &[&str] = &[
    "qwen3:0.6b",
    "qwen3:1.7b",
    "qwen2.5:0.5b",
    "gemma3:1b",
    "llama3.2:1b",
    "gemma3:270m",
];

/// Tool-capable flash: cheap first tries. Three attempts still beat one 27B pass.
pub const FLASH_WRITE_MODELS: &[&str] = &["qwen3:0.6b", "qwen3:1.7b"];

/// Tries on the same flash writer before escalating. Trash is discarded each time.
pub const FLASH_ATTEMPTS: usize = 3;

/// Tool-capable fillers. Flash first, then 2B/4B/8B. Never gemma / 0.5b.
pub const WRITE_MODELS: &[&str] = &[
    "qwen3:0.6b",
    "qwen3:1.7b",
    "qwen3.5:2b-mlx",
    "qwen3:4b",
    "qwen3:8b",
];

/// Ctx/timeout class: mid writers plus aliases.
pub const FAST_MODELS: &[&str] = &[
    "qwen3.5:2b-mlx",
    "qwen3:4b",
    "qwen3:8b",
    "qwen3.5:2b",
    "qwen2.5:3b",
];

pub const REVIEW_MODELS: &[&str] = &[
    crate::api::DEFAULT_QWEN_MODEL,
    "qwen3.8:27b-mlx",
    "qwen3.5:35b-128k",
    "qwen3.8:27b-mtp-q4_K_M",
    "qwen3.5:35b",
];

/// Ollama keep_alive for local chat. Long enough to cover a ticket without reload.
pub const LOCAL_KEEP_ALIVE: &str = "45m";

pub fn is_fast_model(id: &str) -> bool {
    FAST_MODELS.iter().any(|m| *m == id) || is_flash_model(id)
}

pub fn is_flash_model(id: &str) -> bool {
    FLASH_MODELS.iter().any(|m| *m == id)
}

pub fn is_write_model(id: &str) -> bool {
    WRITE_MODELS.iter().any(|m| *m == id)
}

pub fn is_flash_writer(id: &str) -> bool {
    FLASH_WRITE_MODELS.iter().any(|m| *m == id)
}

/// Stay on this flash model for another try. Mid writers escalate immediately.
pub fn flash_retry(model: &str, attempts: usize) -> bool {
    is_flash_writer(model) && attempts < FLASH_ATTEMPTS
}

pub fn local_num_ctx(model: &str) -> u32 {
    if is_flash_model(model) {
        4096
    } else if is_fast_model(model) {
        8192
    } else {
        16384
    }
}

pub fn local_max_tokens(model: &str) -> u32 {
    if is_flash_model(model) {
        1536
    } else if is_fast_model(model) {
        3072
    } else {
        4096
    }
}

pub fn local_timeout_secs(model: &str) -> u64 {
    if is_flash_model(model) {
        90
    } else if is_fast_model(model) {
        240
    } else {
        900
    }
}

/// Tests and code stay on the fast local model. Author/designer/auditors review.
pub fn lane_for_role(role: &str) -> Lane {
    match role {
        "author" | "designer" | "auditor" | "code_auditor" => Lane::Review,
        _ => Lane::Fast,
    }
}

/// Inner-loop build turns stay Fast. Loading 27B mid-ticket is slower than a
/// weaker coder that is already resident.
pub fn lane_for_build_turn(_turn: usize) -> Lane {
    Lane::Fast
}

pub fn pick_fast_model(installed: &[String]) -> Option<(String, String)> {
    for id in WRITE_MODELS {
        if installed.iter().any(|have| have == id) {
            return Some(("qwen".into(), (*id).to_string()));
        }
    }
    None
}

pub fn pick_review_model(installed: &[String], grok_ready: bool) -> (String, String) {
    if let Some(p) = pick_audit_model(installed) {
        return p;
    }
    if grok_ready {
        return ("grok".into(), crate::api::DEFAULT_GROK_MODEL.into());
    }
    (
        "qwen".into(),
        crate::api::DEFAULT_QWEN_MODEL.to_string(),
    )
}

/// Local auditor only. Skip cloud — the inner loop is a local quality chain.
pub fn pick_audit_model(installed: &[String]) -> Option<(String, String)> {
    for id in REVIEW_MODELS {
        if installed.iter().any(|have| have == id) {
            return Some(("qwen".into(), (*id).to_string()));
        }
    }
    None
}

/// Next writer after `current` (0.6 → 1.7 → 2b → 4b → 8b). None after 8B. Never gemma.
pub fn pick_escalate_model(installed: &[String], current: &str) -> Option<(String, String)> {
    if !is_write_model(current) {
        return pick_fast_model(installed);
    }
    let mut seen = current.is_empty();
    for id in WRITE_MODELS {
        if *id == current {
            seen = true;
            continue;
        }
        if seen && installed.iter().any(|have| have == id) {
            return Some(("qwen".into(), (*id).to_string()));
        }
    }
    None
}

#[cfg(test)]
mod lane_tests {
    use super::*;

    #[test]
    fn inner_loop_stays_fast() {
        assert_eq!(lane_for_build_turn(1), Lane::Fast);
        assert_eq!(lane_for_build_turn(2), Lane::Fast);
        assert_eq!(lane_for_build_turn(3), Lane::Fast);
        assert_eq!(lane_for_role("stepwright"), Lane::Fast);
        assert_eq!(lane_for_role("implementer"), Lane::Fast);
        assert_eq!(lane_for_role("author"), Lane::Review);
        assert_eq!(lane_for_role("designer"), Lane::Review);
        assert_eq!(lane_for_role("auditor"), Lane::Review);
        assert_eq!(lane_for_role("code_auditor"), Lane::Review);
    }

    #[test]
    fn fast_prefers_the_tiny_mlx_model() {
        let installed = vec![
            "qwen3.8:27b-mlx".into(),
            "qwen3.5:2b-mlx".into(),
            "qwen3:8b".into(),
        ];
        let pick = pick_fast_model(&installed).expect("fast");
        assert_eq!(pick.1, "qwen3.5:2b-mlx");
        let rev = pick_review_model(&installed, false);
        assert_eq!(rev.1, "qwen3.8:27b-mlx");
        assert!(is_fast_model("qwen3.5:2b-mlx"));
        assert!(is_write_model("qwen3.5:2b-mlx"));
        assert!(!is_fast_model("qwen3.8:27b-mlx"));
        assert_eq!(local_num_ctx("qwen3.5:2b-mlx"), 8192);
        assert_eq!(local_num_ctx("qwen3.8:27b-mlx"), 16384);
        assert_eq!(local_max_tokens("qwen3.5:2b-mlx"), 3072);
        assert_eq!(local_timeout_secs("qwen3.5:2b-mlx"), 240);
        assert_eq!(local_timeout_secs("qwen3.8:27b-mlx"), 900);
    }

    #[test]
    fn write_pool_starts_at_flash_and_skips_gemma() {
        let installed = vec![
            "qwen3.8:27b-mlx".into(),
            "qwen3.5:2b-mlx".into(),
            "qwen3:0.6b".into(),
            "qwen3:1.7b".into(),
            "qwen3:8b".into(),
            "gemma3:1b".into(),
            "qwen2.5:0.5b".into(),
        ];
        let pick = pick_fast_model(&installed).expect("flash");
        assert_eq!(pick.1, "qwen3:0.6b");
        assert!(is_write_model("qwen3:0.6b"));
        assert!(is_flash_writer("qwen3:0.6b"));
        assert!(!is_write_model("gemma3:1b"));
        assert!(!is_flash_writer("gemma3:1b"));
        assert!(is_flash_model("qwen3:0.6b"));
        assert_eq!(local_num_ctx("qwen3:0.6b"), 4096);
        assert!(flash_retry("qwen3:0.6b", 1));
        assert!(flash_retry("qwen3:0.6b", 2));
        assert!(!flash_retry("qwen3:0.6b", 3));
        assert!(!flash_retry("qwen3.5:2b-mlx", 1));
        let next = pick_escalate_model(&installed, "qwen3:0.6b").expect("1.7b");
        assert_eq!(next.1, "qwen3:1.7b");
        let mid = pick_escalate_model(&installed, "qwen3:1.7b").expect("2b");
        assert_eq!(mid.1, "qwen3.5:2b-mlx");
        let eight = pick_escalate_model(&installed, "qwen3.5:2b-mlx").expect("8b");
        assert_eq!(eight.1, "qwen3:8b");
        assert!(pick_escalate_model(&installed, "qwen3:8b").is_none());
    }

    #[test]
    fn missing_fast_model_is_none() {
        assert!(pick_fast_model(&["qwen3.8:27b-mlx".into()]).is_none());
        assert!(pick_fast_model(&["gemma3:1b".into(), "qwen2.5:0.5b".into()]).is_none());
    }

    #[test]
    fn escalate_2b_to_8b_then_stop() {
        let installed = vec![
            "qwen3.5:2b-mlx".into(),
            "qwen3:8b".into(),
            "qwen3.8:27b-mlx".into(),
        ];
        let next = pick_escalate_model(&installed, "qwen3.5:2b-mlx").expect("8b");
        assert_eq!(next.1, "qwen3:8b");
        assert!(pick_escalate_model(&installed, "qwen3:8b").is_none());
        assert_eq!(
            pick_audit_model(&installed).expect("audit").1,
            "qwen3.8:27b-mlx"
        );
        assert!(pick_audit_model(&["qwen3.5:2b-mlx".into()]).is_none());
    }
}
