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

/// Tool-capable flash (ctx class). Retry count is measured, not a constant.
pub const FLASH_WRITE_MODELS: &[&str] = &["qwen3:0.6b", "qwen3:1.7b"];

/// Tool-capable fillers. List order is a prior; measured tok/s can override later.
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

pub fn local_num_ctx(model: &str) -> u32 {
    if is_flash_model(model) {
        4096
    } else if is_fast_model(model) {
        8192
    } else {
        16384
    }
}

/// Last name in `WRITE_MODELS`. No further quality hop; give it room to finish.
pub fn is_last_writer(id: &str) -> bool {
    WRITE_MODELS.last().copied() == Some(id)
}

pub fn local_max_tokens(model: &str) -> u32 {
    if is_flash_model(model) {
        1536
    } else if is_last_writer(model) {
        4096
    } else if is_fast_model(model) {
        3072
    } else {
        4096
    }
}

pub fn local_timeout_secs(model: &str) -> u64 {
    if is_flash_model(model) {
        90
    } else if is_last_writer(model) {
        600
    } else if is_fast_model(model) {
        240
    } else {
        900
    }
}

/// One row from Ollama `api/ps`. Size, context, and keep-alive are measured.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LoadedRunner {
    pub name: String,
    pub size: u64,
    pub context_length: u32,
    pub expires_at: String,
}

/// Keep-alive clock has already passed. Still listed in `api/ps` = zombie.
pub fn keep_alive_lapsed(expires_at: &str, now: chrono::DateTime<chrono::Utc>) -> bool {
    let Ok(t) = chrono::DateTime::parse_from_rfc3339(expires_at) else {
        return false;
    };
    t.with_timezone(&chrono::Utc) <= now
}

/// Loaded past twice shalt's cap for this name. Talking to it keeps the huge window.
pub fn ctx_oversized(name: &str, context_length: u32) -> bool {
    context_length > 0 && context_length > local_num_ctx(name).saturating_mul(2)
}

/// Current load or a previous measured load past the cap. Do not POST at it.
pub fn review_ctx_unusable(name: &str, current_ctx: u32, last_ctx: u32) -> bool {
    ctx_oversized(name, current_ctx) || ctx_oversized(name, last_ctx)
}

/// A stall is capacity, not quality, when something other than the writer
/// occupies the runner (heavy VRAM or an oversized context). Hopping UP then
/// makes the stall worse.
pub fn stall_is_contention(loaded: &[LoadedRunner], writer: &str) -> bool {
    loaded.iter().any(|m| occupant_blocks_writer(m, writer))
}

fn occupant_blocks_writer(m: &LoadedRunner, writer: &str) -> bool {
    if m.name.is_empty() || m.name == writer {
        return false;
    }
    let gb = m.size as f64 / 1e9;
    gb >= 8.0 || ctx_oversized(&m.name, m.context_length)
}

/// Other-session 35B-128k. Do not unload it; writers may load beside it.
pub fn leave_runner_loaded(name: &str) -> bool {
    name.contains("35b-128k")
}

/// generate/chat keep_alive for `writer` would refresh a blocking occupant.
pub fn pin_keep_alive_safe(loaded: &[LoadedRunner], writer: &str) -> bool {
    occupants_to_stop(loaded, writer).is_empty()
}

/// Writer is already in `api/ps`. Chat it; do not wait on a leftover occupant.
pub fn writer_resident(loaded: &[LoadedRunner], writer: &str) -> bool {
    !writer.is_empty() && loaded.iter().any(|m| m.name == writer)
}

/// Pin still waits when an expired occupant (or oversized self-load) sits
/// next to a resident writer. Skipping that wait starves the fill.
pub fn pin_should_wait(
    loaded: &[LoadedRunner],
    writer: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    if writer_needs_reload(loaded, writer) {
        return true;
    }
    if !stall_is_contention(loaded, writer) {
        return false;
    }
    !writer_resident(loaded, writer)
        || !zombies_to_stop(loaded, writer, now).is_empty()
        || !oversized_blockers_to_stop(loaded, writer).is_empty()
}

/// Writer is loaded past twice its cap. Talking to it keeps the huge window — stop and reload.
pub fn writer_needs_reload(loaded: &[LoadedRunner], writer: &str) -> bool {
    !writer.is_empty()
        && loaded
            .iter()
            .any(|m| m.name == writer && ctx_oversized(&m.name, m.context_length))
}

/// Unprotected blocker whose keep-alive already lapsed. Stop even if a writer is resident.
pub fn zombies_to_stop(
    loaded: &[LoadedRunner],
    writer: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<String> {
    loaded
        .iter()
        .filter(|m| {
            occupant_blocks_writer(m, writer)
                && !leave_runner_loaded(&m.name)
                && keep_alive_lapsed(&m.expires_at, now)
        })
        .map(|m| m.name.clone())
        .collect()
}

/// Heavy leftover loaded past twice its cap. Stop even if keep-alive is live.
/// Never 35B-128k.
pub fn oversized_blockers_to_stop(loaded: &[LoadedRunner], writer: &str) -> Vec<String> {
    occupants_to_stop(loaded, writer)
        .into_iter()
        .filter(|n| {
            loaded
                .iter()
                .any(|m| m.name == *n && ctx_oversized(&m.name, m.context_length))
        })
        .collect()
}

/// Occupants that block `writer` and are safe to `ollama stop`. Never 35B-128k.
pub fn occupants_to_stop(loaded: &[LoadedRunner], writer: &str) -> Vec<String> {
    loaded
        .iter()
        .filter(|m| occupant_blocks_writer(m, writer) && !leave_runner_loaded(&m.name))
        .map(|m| m.name.clone())
        .collect()
}

/// Repeating `ollama stop` on an unload-resistant runner refreshes it.
/// If keep-alive already lapsed and it is still listed, it is a zombie — stop again.
pub fn should_issue_stop(
    still_loaded: bool,
    stop_already_failed: bool,
    keep_alive_lapsed: bool,
) -> bool {
    if !still_loaded {
        return false;
    }
    if keep_alive_lapsed {
        return true;
    }
    !stop_already_failed
}

/// GET-only poll rounds after a stop. Unload-resistant occupants need the expire window, not another stop.
/// An already-lapsed keep-alive will not GC because we wait — try the writer beside it.
pub fn contend_wait_rounds(stop_failed_still_loaded: bool, keep_alive_lapsed: bool) -> u32 {
    if keep_alive_lapsed {
        8
    } else if stop_failed_still_loaded {
        360
    } else {
        40
    }
}

/// Remaining blockers have already lapsed. Loading a writer beside them is better than waiting.
pub fn expired_blockers_only(
    loaded: &[LoadedRunner],
    writer: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    let stop = occupants_to_stop(loaded, writer);
    !stop.is_empty()
        && stop.iter().all(|n| {
            loaded
                .iter()
                .any(|m| m.name == *n && keep_alive_lapsed(&m.expires_at, now))
        })
}

/// Flash writers only stall next to a 27B/35B. Skip them when ps is unknown or leftover.
pub fn skip_flash_writers(
    loaded: &[LoadedRunner],
    stop_failed: &[String],
    ps_unknown: bool,
) -> bool {
    if ps_unknown {
        return true;
    }
    loaded.iter().any(|m| {
        let gb = m.size as f64 / 1e9;
        gb >= 8.0 && (m.name.contains("27b") || m.name.contains("35b"))
    }) || stop_failed.iter().any(|n| {
        n.contains("27b") || (n.contains("35b") && !leave_runner_loaded(n))
    })
}

/// Chat/load this writer: pin is safe, a write-pool model is already in, or leftovers are expired.
pub fn writer_load_ok(
    loaded: &[LoadedRunner],
    writer: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    pin_keep_alive_safe(loaded, writer)
        || writer_swap_ok(loaded, writer)
        || expired_blockers_only(loaded, writer, now)
}

/// A leftover occupant must not kill the write chain. Hop 2B→4B→8B beside it.
pub fn hop_up_ok(
    loaded: &[LoadedRunner],
    current: &str,
    next: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    writer_load_ok(loaded, next, now)
        || writer_resident(loaded, current)
        || !occupants_to_stop(loaded, next).is_empty()
}

/// A write-pool model is already in. Swap to `next` without a keep_alive on the occupant.
pub fn writer_swap_ok(loaded: &[LoadedRunner], next: &str) -> bool {
    is_write_model(next) && loaded.iter().any(|m| is_write_model(&m.name))
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
    pick_fast_model_filtered(installed, false)
}

/// When a 27B/35B is still resident, flash writers only stall. Start at 2B+.
pub fn pick_fast_model_filtered(
    installed: &[String],
    skip_flash: bool,
) -> Option<(String, String)> {
    let pick = |skip: bool| {
        WRITE_MODELS.iter().find(|id| {
            if skip && is_flash_writer(id) {
                return false;
            }
            installed.iter().any(|have| have == *id)
        })
    };
    pick(skip_flash)
        .or_else(|| pick(false))
        .map(|id| ("qwen".into(), (*id).to_string()))
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
    pick_escalate_model_filtered(installed, current, false)
}

pub fn pick_escalate_model_filtered(
    installed: &[String],
    current: &str,
    skip_flash: bool,
) -> Option<(String, String)> {
    if !is_write_model(current) {
        return pick_fast_model_filtered(installed, skip_flash);
    }
    let mut seen = current.is_empty();
    for id in WRITE_MODELS {
        if *id == current {
            seen = true;
            continue;
        }
        if seen && skip_flash && is_flash_writer(id) {
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
        assert_eq!(local_max_tokens("qwen3:4b"), 3072);
        assert_eq!(local_timeout_secs("qwen3:4b"), 240);
        assert!(is_last_writer("qwen3:8b"));
        assert!(!is_last_writer("qwen3:4b"));
        assert_eq!(local_max_tokens("qwen3:8b"), 4096);
        assert_eq!(local_timeout_secs("qwen3:8b"), 600);
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
        let blocked = pick_fast_model_filtered(&installed, true).expect("mid");
        assert_eq!(blocked.1, "qwen3.5:2b-mlx");
        assert!(is_write_model("qwen3:0.6b"));
        assert!(is_flash_writer("qwen3:0.6b"));
        assert!(!is_write_model("gemma3:1b"));
        assert!(!is_flash_writer("gemma3:1b"));
        assert!(is_flash_model("qwen3:0.6b"));
        assert_eq!(local_num_ctx("qwen3:0.6b"), 4096);
        let next = pick_escalate_model(&installed, "qwen3:0.6b").expect("1.7b");
        assert_eq!(next.1, "qwen3:1.7b");
        let skip = pick_escalate_model_filtered(&installed, "qwen3:0.6b", true).expect("2b");
        assert_eq!(skip.1, "qwen3.5:2b-mlx");
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

    #[test]
    fn a_heavy_or_oversized_occupant_is_contention_not_a_quality_hop() {
        let zombie = LoadedRunner {
            name: "qwen3.8:27b-mlx".into(),
            size: 31_899_864_808,
            context_length: 262_144,
            expires_at: "2026-09-23T09:42:12.211808-07:00".into(),
        };
        let writer = LoadedRunner {
            name: "qwen3:4b".into(),
            size: 2_000_000_000,
            context_length: 8192,
            ..Default::default()
        };
        let flash = LoadedRunner {
            name: "qwen3:1.7b".into(),
            size: 1_200_000_000,
            context_length: 4096,
            ..Default::default()
        };
        let protected = LoadedRunner {
            name: "qwen3.5:35b-128k".into(),
            size: 20_000_000_000,
            context_length: 131_072,
            ..Default::default()
        };
        assert!(stall_is_contention(&[zombie.clone()], "qwen3:4b"));
        assert!(stall_is_contention(&[zombie.clone(), writer.clone()], "qwen3:4b"));
        assert!(!stall_is_contention(&[writer.clone()], "qwen3:4b"));
        assert!(!stall_is_contention(&[flash.clone(), writer.clone()], "qwen3:4b"));
        assert!(stall_is_contention(&[protected.clone()], "qwen3.5:2b-mlx"));
        assert!(ctx_oversized("qwen3.8:27b-mlx", 262_144));
        assert!(!ctx_oversized("qwen3.8:27b-mlx", 16_384));
        assert!(!ctx_oversized("qwen3:4b", 8192));
        assert!(review_ctx_unusable("qwen3.8:27b-mlx", 0, 262_144));
        assert!(review_ctx_unusable("qwen3.8:27b-mlx", 262_144, 0));
        assert!(!review_ctx_unusable("qwen3.8:27b-mlx", 0, 0));
        assert!(!review_ctx_unusable("qwen3.8:27b-mlx", 16_384, 16_384));
        assert!(!pin_keep_alive_safe(&[zombie.clone()], "qwen3.5:2b-mlx"));
        assert!(!pin_keep_alive_safe(&[zombie.clone(), writer.clone()], "qwen3:4b"));
        assert!(pin_keep_alive_safe(&[writer.clone()], "qwen3:4b"));
        assert!(pin_keep_alive_safe(&[protected.clone()], "qwen3.5:2b-mlx"));
        assert!(
            pin_keep_alive_safe(&[protected.clone(), writer.clone()], "qwen3:4b"),
            "35B-128k must not block a quality hop to the next writer"
        );
        assert!(leave_runner_loaded("qwen3.5:35b-128k"));
        assert!(!leave_runner_loaded("qwen3.8:27b-mlx"));
        assert_eq!(
            occupants_to_stop(&[zombie.clone()], "qwen3.5:2b-mlx"),
            vec!["qwen3.8:27b-mlx".to_string()]
        );
        assert!(occupants_to_stop(&[protected.clone()], "qwen3.5:2b-mlx").is_empty());
        assert!(occupants_to_stop(&[writer.clone()], "qwen3:4b").is_empty());
        assert!(should_issue_stop(true, false, false));
        assert!(!should_issue_stop(true, true, false));
        assert!(!should_issue_stop(false, true, false));
        assert!(
            should_issue_stop(true, true, true),
            "expired occupant is a zombie — stop again"
        );
        assert!(!should_issue_stop(false, true, true));
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-23T16:46:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert!(keep_alive_lapsed(&zombie.expires_at, now));
        assert!(!keep_alive_lapsed("", now));
        assert!(!keep_alive_lapsed("2026-09-23T17:30:00Z", now));
        assert!(contend_wait_rounds(true, false) > contend_wait_rounds(false, false));
        assert!(contend_wait_rounds(true, true) < contend_wait_rounds(true, false));
        assert!(expired_blockers_only(&[zombie.clone()], "qwen3.5:2b-mlx", now));
        assert!(!expired_blockers_only(&[writer.clone()], "qwen3:4b", now));
        assert!(writer_load_ok(&[zombie.clone()], "qwen3.5:2b-mlx", now));
        assert!(writer_load_ok(&[writer.clone()], "qwen3:4b", now));
        assert!(
            hop_up_ok(&[zombie.clone()], "qwen3.5:2b-mlx", "qwen3:4b", now),
            "2B stall with leftover 27B must hop to 4B, not die"
        );
        assert!(hop_up_ok(&[writer.clone()], "qwen3:4b", "qwen3:8b", now));
        assert!(skip_flash_writers(&[zombie.clone()], &[], false));
        assert!(skip_flash_writers(
            &[],
            &["qwen3.8:27b-mlx".into()],
            false
        ));
        assert!(
            skip_flash_writers(&[], &[], true),
            "hung api/ps must not pin 0.6B"
        );
        assert!(!skip_flash_writers(&[writer.clone()], &[], false));
        assert!(writer_swap_ok(&[zombie.clone(), writer.clone()], "qwen3:8b"));
        assert!(
            writer_swap_ok(&[zombie.clone(), writer.clone()], "qwen3:8b"),
            "2B resident → hop to 8B even if 4B never loaded"
        );
        assert!(!writer_swap_ok(&[zombie.clone()], "qwen3:4b"));
        assert!(writer_resident(&[zombie.clone(), writer.clone()], "qwen3:4b"));
        assert!(!writer_resident(&[zombie.clone()], "qwen3:4b"));
        let writer_8 = LoadedRunner {
            name: "qwen3:8b".into(),
            size: 3_860_000_000,
            context_length: 8192,
            expires_at: "2026-09-23T18:00:00Z".into(),
        };
        assert!(
            pin_should_wait(&[zombie.clone(), writer_8.clone()], "qwen3:8b", now),
            "resident 8B next to expired 27B must still stop the zombie"
        );
        assert!(!pin_should_wait(&[writer_8.clone()], "qwen3:8b", now));
        assert!(pin_should_wait(&[zombie.clone()], "qwen3:8b", now));
        assert_eq!(
            zombies_to_stop(&[zombie.clone()], "qwen3.5:2b-mlx", now),
            vec!["qwen3.8:27b-mlx".to_string()]
        );
        assert!(zombies_to_stop(&[protected.clone()], "qwen3.5:2b-mlx", now).is_empty());
        let live_27 = LoadedRunner {
            name: "qwen3.8:27b-mlx".into(),
            size: 31_899_864_808,
            context_length: 262_144,
            expires_at: "2026-09-23T18:00:00Z".into(),
        };
        assert!(zombies_to_stop(&[live_27.clone()], "qwen3.5:2b-mlx", now).is_empty());
        assert_eq!(
            oversized_blockers_to_stop(&[live_27.clone(), writer_8.clone()], "qwen3:8b"),
            vec!["qwen3.8:27b-mlx".to_string()]
        );
        assert!(
            pin_should_wait(&[live_27.clone(), writer_8.clone()], "qwen3:8b", now),
            "resident 8B next to a 256k 27B must stop the leftover, not starve"
        );
        assert!(oversized_blockers_to_stop(&[protected.clone()], "qwen3:8b").is_empty());
        let fat_writer = LoadedRunner {
            name: "qwen3.5:2b-mlx".into(),
            size: 3_400_000_000,
            context_length: 262_144,
            ..Default::default()
        };
        assert!(writer_needs_reload(&[fat_writer.clone()], "qwen3.5:2b-mlx"));
        assert!(!writer_needs_reload(&[writer.clone()], "qwen3:4b"));
        assert!(!writer_needs_reload(&[zombie.clone()], "qwen3:4b"));
    }
}
