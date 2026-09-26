//! Token estimates on tickets, spend on jobs, accuracy on the sprint retro.

use crate::board::{Board, BoardItem, SprintRetro};
use crate::jobs::{kind_phase, Job, JobKind, JobStatus};
use crate::ledger::{Ledger, GREEN, ORPHAN};
use crate::spec::{Feature, Scenario};
use chrono::Utc;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub const DEFAULT_TICKET_TOKENS: i64 = 100_000;
/// Cloud prior. Local (Qwen) is slower — see `suggest_secs_for`.
pub const DEFAULT_TICKET_SECS: i64 = 180;

#[derive(Debug, Clone, Serialize)]
pub struct Assignment {
    pub rid: String,
    pub epic: String,
    pub backend: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EpicTicket {
    pub rid: String,
    pub name: String,
    pub status: String,
    pub estimate: i64,
    pub forecast_secs: i64,
    pub spent_tokens: i64,
    pub spent_secs: i64,
    pub backend: String,
    pub model: String,
    pub sprint: String,
    pub inherited: bool,
    pub suggest: i64,
    pub suggest_secs: i64,
    pub epic: String,
    pub file: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EpicAlloc {
    pub name: String,
    pub estimated: i64,
    pub ticket_sum: i64,
    pub spent: i64,
    pub forecast_secs: i64,
    pub spent_secs: i64,
    pub backend: String,
    pub model: String,
    pub tickets: usize,
    pub remaining: usize,
    pub items: Vec<EpicTicket>,
    #[serde(default)]
    pub suggest_backend: String,
    #[serde(default)]
    pub suggest_model: String,
    #[serde(default)]
    pub suggest_why: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelScore {
    pub backend: String,
    pub model: String,
    pub jobs: usize,
    pub spent: i64,
    pub estimated: i64,
    pub tickets: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommandCenter {
    pub allocated: i64,
    pub spent: i64,
    pub forecast_secs: i64,
    pub spent_secs: i64,
    pub accuracy: Option<f64>,
    pub epics: Vec<EpicAlloc>,
    pub models: Vec<ModelScore>,
    #[serde(default)]
    pub fits: Vec<AgentFit>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentFit {
    pub backend: String,
    pub model: String,
    pub epic: String,
    pub jobs: usize,
    pub tickets: usize,
    pub estimated: i64,
    pub spent: i64,
    pub accuracy: Option<f64>,
    pub suggest: i64,
    #[serde(default)]
    pub secs: i64,
    #[serde(default)]
    pub suggest_secs: i64,
}

pub fn job_secs(j: &Job) -> i64 {
    let a = chrono::NaiveDateTime::parse_from_str(&j.created_at, "%Y-%m-%dT%H:%M:%SZ").ok();
    let b = chrono::NaiveDateTime::parse_from_str(&j.finished_at, "%Y-%m-%dT%H:%M:%SZ").ok();
    match (a, b) {
        (Some(a), Some(b)) => (b - a).num_seconds().max(0),
        _ => 0,
    }
}

/// Wall time for the dashboard: finished jobs use `finished_at`, live jobs count until now.
pub fn job_wall_secs(j: &Job) -> i64 {
    let done = job_secs(j);
    if done > 0 {
        return done;
    }
    if !matches!(
        j.status,
        JobStatus::Running | JobStatus::Pending | JobStatus::Waiting
    ) {
        return 0;
    }
    let Some(a) = chrono::NaiveDateTime::parse_from_str(&j.created_at, "%Y-%m-%dT%H:%M:%SZ").ok()
    else {
        return 0;
    };
    (Utc::now().naive_utc() - a).num_seconds().max(0)
}

pub fn parse_agent(spec: &str) -> (String, String) {
    let spec = spec.trim();
    if spec.is_empty() {
        return (String::new(), String::new());
    }
    match spec.split_once("::") {
        Some((b, m)) => (b.trim().to_string(), m.trim().to_string()),
        None => (spec.to_string(), String::new()),
    }
}

pub fn agent_label(backend: &str, model: &str) -> String {
    match (backend.is_empty(), model.is_empty()) {
        (true, true) => String::new(),
        (false, true) => backend.to_string(),
        (true, false) => model.to_string(),
        (false, false) => format!("{backend} {model}"),
    }
}

pub(crate) fn normalize_backend(backend: &str) -> String {
    match backend {
        "ollama" => "qwen".into(),
        other => other.to_string(),
    }
}

pub(crate) fn epic_name(ledger: &Ledger, rid: &str) -> String {
    ledger
        .entries
        .get(rid)
        .map(|e| e.epic.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_default()
}

fn ticket_file(ledger: &Ledger, rid: &str) -> String {
    ledger
        .entries
        .get(rid)
        .map(|e| e.feature_file.clone())
        .filter(|s| !s.is_empty())
        .map(|f| {
            if f.starts_with("spec/") {
                f
            } else {
                format!("spec/{f}")
            }
        })
        .unwrap_or_default()
}

pub(crate) fn ticket_name(ledger: &Ledger, rid: &str) -> String {
    ledger
        .entries
        .get(rid)
        .map(|e| e.name.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| rid.to_string())
}

pub(crate) fn ticket_status(ledger: &Ledger, rid: &str) -> String {
    ledger
        .entries
        .get(rid)
        .map(|e| e.status.clone())
        .unwrap_or_else(|| "pending".into())
}

/// Agent that will run this ticket: ticket override, else epic, else empty (inherit last job).
pub fn agent_for_item(board: &Board, item: &BoardItem, epic: &str) -> (String, String, bool) {
    if !item.backend.is_empty() || !item.model.is_empty() {
        return (item.backend.clone(), item.model.clone(), false);
    }
    if let Some(e) = board.epics.iter().find(|e| e.name == epic) {
        if !e.backend.is_empty() || !e.model.is_empty() {
            return (e.backend.clone(), e.model.clone(), true);
        }
    }
    (String::new(), String::new(), true)
}

/// Next unfinished ticket in the sprint (or whole board), lowest rank first.
pub fn next_assignment(
    board: &Board,
    ledger: &Ledger,
    sprint_id: Option<&str>,
) -> Option<Assignment> {
    let mut items: Vec<&BoardItem> = board.items.iter().collect();
    items.sort_by_key(|i| i.rank);
    for it in items {
        if let Some(sid) = sprint_id {
            if it.sprint_id.as_deref() != Some(sid) {
                continue;
            }
        }
        let status = ticket_status(ledger, &it.rid);
        if status == GREEN || status == ORPHAN {
            continue;
        }
        let epic = epic_name(ledger, &it.rid);
        let focus = board.focus_journey.trim();
        if !focus.is_empty() && epic != focus {
            continue;
        }
        let (backend, model, _) = agent_for_item(board, it, &epic);
        return Some(Assignment {
            rid: it.rid.clone(),
            epic,
            backend,
            model,
        });
    }
    None
}

/// Next unfinished ticket whose epoch is not already held by a live job.
pub fn next_free_assignment(
    board: &Board,
    ledger: &Ledger,
    sprint_id: Option<&str>,
    jobs: &[Job],
    project_id: &str,
    kind: crate::jobs::JobKind,
) -> Option<Assignment> {
    let live: Vec<&Job> = jobs
        .iter()
        .filter(|j| {
            j.project_id == project_id
                && matches!(
                    j.status,
                    JobStatus::Running | JobStatus::Pending | JobStatus::Waiting
                )
        })
        .collect();
    let held: BTreeSet<String> = live
        .iter()
        .map(|j| crate::jobs::epoch_of(j))
        .filter(|e| !e.is_empty())
        .collect();
    let held_rids: BTreeSet<&str> = live
        .iter()
        .map(|j| j.rid.as_str())
        .filter(|r| !r.is_empty())
        .collect();
    let map = if kind == crate::jobs::JobKind::Build {
        Some(crate::deps::work_map(
            board, ledger, jobs, project_id, sprint_id,
        ))
    } else {
        None
    };
    let mut items: Vec<&BoardItem> = board.items.iter().collect();
    items.sort_by_key(|i| i.rank);
    for it in items {
        if let Some(sid) = sprint_id {
            if it.sprint_id.as_deref() != Some(sid) {
                continue;
            }
        }
        let status = ticket_status(ledger, &it.rid);
        if status == GREEN || status == ORPHAN {
            continue;
        }
        if held_rids.contains(it.rid.as_str()) {
            continue;
        }
        let epic = epic_name(ledger, &it.rid);
        let focus = board.focus_journey.trim();
        if !focus.is_empty() && epic != focus {
            continue;
        }
        let epoch = crate::jobs::epoch_id(project_id, kind, &it.rid, &epic);
        if held.contains(&epoch) {
            continue;
        }
        if let Some(m) = &map {
            if !m.is_ready(&it.rid) {
                continue;
            }
        }
        let (backend, model, _) = agent_for_item(board, it, &epic);
        return Some(Assignment {
            rid: it.rid.clone(),
            epic,
            backend,
            model,
        });
    }
    None
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct FocusItem {
    pub rid: String,
    pub name: String,
    pub epic: String,
    pub file: String,
    pub status: String,
    pub estimate: i64,
    pub forecast_secs: i64,
    pub spent_tokens: i64,
    pub spent_secs: i64,
    pub backend: String,
    pub model: String,
    pub job_id: String,
    pub kind: String,
    pub line: String,
    pub files: Vec<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub tokens: i64,
    #[serde(default)]
    pub turn: u32,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct WorkFocus {
    pub now: Option<FocusItem>,
    pub next: Vec<FocusItem>,
    pub just_wrote: Option<FocusItem>,
}

fn job_card_name(j: &Job) -> String {
    match j.kind {
        JobKind::Design => "Drawing storyboards".into(),
        JobKind::Author => {
            let t = j.prompt.lines().next().unwrap_or("").trim();
            if t.is_empty() {
                "Writing the spec".into()
            } else {
                t.chars().take(72).collect()
            }
        }
        _ => kind_phase(j.kind).into(),
    }
}

fn wrote_files(j: &Job) -> Vec<String> {
    let mut out: Vec<String> = j.draft.files.keys().cloned().collect();
    for line in j.log.lines() {
        let t = line.trim();
        if let Some(p) = t.strip_prefix("[write_file]") {
            let p = p.trim().split_whitespace().next().unwrap_or("");
            if p.contains('.') {
                out.push(p.to_string());
            }
        } else if t.starts_with("wrote ") && t.contains(':') && !t.contains(" bytes") {
            if let Some((_, list)) = t.split_once(':') {
                for p in list.split(',') {
                    let p = p.trim();
                    if p.contains('.') && !p.contains(' ') {
                        out.push(p.to_string());
                    }
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out.truncate(12);
    out
}

fn spent_for_rid(jobs: &[Job], project_id: &str, rid: &str) -> (i64, i64) {
    jobs.iter()
        .filter(|j| j.project_id == project_id && j.rid == rid)
        .fold((0, 0), |(tok, secs), j| {
            (tok + job_tokens(j), secs + job_secs(j))
        })
}

fn item_from_ticket(
    board: &Board,
    ledger: &Ledger,
    jobs: &[Job],
    project_id: &str,
    rid: &str,
    job: Option<&Job>,
) -> FocusItem {
    let epic = epic_name(ledger, rid);
    let (backend, model, _) = board
        .items
        .iter()
        .find(|i| i.rid == rid)
        .map(|i| agent_for_item(board, i, &epic))
        .unwrap_or_default();
    let row = board.items.iter().find(|i| i.rid == rid);
    let estimate = row.map(|i| i.token_estimate).unwrap_or(0);
    let forecast_secs = row.map(|i| i.time_estimate_secs).unwrap_or(0);
    let (spent_tokens, spent_secs) = spent_for_rid(jobs, project_id, rid);
    let entry = ledger.entries.get(rid);
    let mut it = FocusItem {
        rid: rid.into(),
        name: ticket_name(ledger, rid),
        epic,
        file: entry
            .map(|e| {
                if e.feature_file.is_empty() {
                    String::new()
                } else if e.feature_file.starts_with("spec/") {
                    e.feature_file.clone()
                } else {
                    format!("spec/{}", e.feature_file)
                }
            })
            .unwrap_or_default(),
        status: ticket_status(ledger, rid),
        estimate,
        forecast_secs,
        spent_tokens,
        spent_secs,
        backend,
        model,
        ..Default::default()
    };
    if let Some(j) = job {
        if !j.backend.is_empty() {
            it.backend = j.backend.clone();
        }
        if !j.model.is_empty() {
            it.model = j.model.clone();
        }
        it.job_id = j.id.clone();
        it.kind = kind_phase(j.kind).into();
        it.line = crate::jobs::status_line(j);
        it.files = wrote_files(j);
        it.created_at = j.created_at.clone();
        it.tokens = job_tokens(j);
        it.turn = crate::jobs::job_turn(&j.log);
        if it.file.is_empty() {
            if let Some(f) = it.files.iter().find(|f| f.ends_with(".feature")) {
                it.file = f.clone();
            }
        }
    }
    it
}

/// Now / next / just-wrote for the PM desk. Sprint tickets first, then the rest.
pub fn work_focus(
    board: &Board,
    ledger: &Ledger,
    jobs: &[Job],
    project_id: &str,
    sprint_id: Option<&str>,
) -> WorkFocus {
    let live = crate::jobs::pick_live_job(jobs.iter().filter(|j| j.project_id == project_id));
    let now = if let Some(j) = live {
        let rid = if !j.rid.is_empty() {
            j.rid.clone()
        } else if matches!(j.kind, JobKind::Author | JobKind::Design) {
            String::new()
        } else {
            next_assignment(board, ledger, sprint_id)
                .map(|a| a.rid)
                .unwrap_or_default()
        };
        if rid.is_empty() {
            Some(FocusItem {
                name: job_card_name(j),
                job_id: j.id.clone(),
                kind: kind_phase(j.kind).into(),
                line: crate::jobs::status_line(j),
                backend: j.backend.clone(),
                model: j.model.clone(),
                files: wrote_files(j),
                epic: j.epic.clone(),
                created_at: j.created_at.clone(),
                tokens: job_tokens(j),
                turn: crate::jobs::job_turn(&j.log),
                ..Default::default()
            })
        } else {
            Some(item_from_ticket(
                board,
                ledger,
                jobs,
                project_id,
                &rid,
                Some(j),
            ))
        }
    } else {
        next_assignment(board, ledger, sprint_id)
            .map(|a| item_from_ticket(board, ledger, jobs, project_id, &a.rid, None))
    };
    let skip = now.as_ref().map(|n| n.rid.as_str()).unwrap_or("");
    let mut next = Vec::new();
    let mut items: Vec<&BoardItem> = board.items.iter().collect();
    items.sort_by_key(|i| i.rank);
    let push_next = |it: &BoardItem, next: &mut Vec<FocusItem>| {
        if it.rid == skip {
            return;
        }
        let status = ticket_status(ledger, &it.rid);
        if status == GREEN || status == ORPHAN {
            return;
        }
        if next.iter().any(|n| n.rid == it.rid) {
            return;
        }
        next.push(item_from_ticket(
            board, ledger, jobs, project_id, &it.rid, None,
        ));
    };
    if let Some(sid) = sprint_id {
        for it in &items {
            if it.sprint_id.as_deref() == Some(sid) {
                push_next(it, &mut next);
                if next.len() >= 8 {
                    break;
                }
            }
        }
    }
    if next.len() < 8 {
        for it in &items {
            push_next(it, &mut next);
            if next.len() >= 8 {
                break;
            }
        }
    }
    let just_wrote = jobs
        .iter()
        .rev()
        .find(|j| {
            j.project_id == project_id
                && j.status == JobStatus::Done
                && live.map(|l| l.id.as_str()) != Some(j.id.as_str())
        })
        .map(|j| {
            let rid = if !j.rid.is_empty() {
                j.rid.clone()
            } else {
                String::new()
            };
            if rid.is_empty() {
                let files = wrote_files(j);
                let kind = kind_phase(j.kind);
                FocusItem {
                    job_id: j.id.clone(),
                    kind: kind.into(),
                    name: format!(
                        "{} · {} file{}",
                        kind,
                        files.len(),
                        if files.len() == 1 { "" } else { "s" }
                    ),
                    backend: j.backend.clone(),
                    model: j.model.clone(),
                    files,
                    epic: j.epic.clone(),
                    spent_tokens: job_tokens(j),
                    spent_secs: job_secs(j),
                    ..Default::default()
                }
            } else {
                item_from_ticket(board, ledger, jobs, project_id, &rid, Some(j))
            }
        });
    WorkFocus {
        now,
        next,
        just_wrote,
    }
}

/// Estimate vs spend per agent × model × epic. Empty epic is that agent on all work.
pub fn agent_fits(board: &Board, jobs: &[Job], ledger: &Ledger, project_id: &str) -> Vec<AgentFit> {
    #[derive(Default)]
    struct Acc {
        jobs: usize,
        spent: i64,
        estimated: i64,
        secs: i64,
        rids: BTreeSet<String>,
    }
    let mut buckets: BTreeMap<(String, String, String), Acc> = BTreeMap::new();
    for j in jobs {
        if j.project_id != project_id {
            continue;
        }
        let spent = job_tokens(j);
        if spent <= 0 {
            continue;
        }
        let backend = normalize_backend(&j.backend);
        if backend.is_empty() {
            continue;
        }
        let model = j.model.clone();
        let epic = if !j.epic.is_empty() {
            j.epic.clone()
        } else if !j.rid.is_empty() {
            epic_name(ledger, &j.rid)
        } else {
            String::new()
        };
        let est = board
            .items
            .iter()
            .find(|i| i.rid == j.rid)
            .map(|i| i.token_estimate)
            .unwrap_or(0);
        for epic_key in [epic, String::new()] {
            let acc = buckets
                .entry((backend.clone(), model.clone(), epic_key))
                .or_default();
            acc.jobs += 1;
            acc.spent += spent;
            acc.secs += job_secs(j);
            if !j.rid.is_empty() && acc.rids.insert(j.rid.clone()) {
                acc.estimated += est;
            }
        }
    }
    let mut out: Vec<_> = buckets
        .into_iter()
        .map(|((backend, model, epic), acc)| {
            let accuracy = if acc.estimated > 0 {
                Some(((10.0 * acc.spent as f64 / acc.estimated as f64).round()) / 10.0)
            } else {
                None
            };
            let suggest = match accuracy {
                Some(a) => ((DEFAULT_TICKET_TOKENS as f64) * a.clamp(0.5, 3.0)).round() as i64,
                None => DEFAULT_TICKET_TOKENS,
            };
            let avg = if acc.jobs > 0 && acc.secs > 0 {
                acc.secs / acc.jobs as i64
            } else {
                0
            };
            let suggest_secs = if avg > 0 { avg } else { prior_secs(&backend) };
            AgentFit {
                backend,
                model,
                epic,
                jobs: acc.jobs,
                tickets: acc.rids.len(),
                estimated: acc.estimated,
                spent: acc.spent,
                accuracy,
                suggest,
                secs: acc.secs,
                suggest_secs,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        a.epic
            .cmp(&b.epic)
            .then(a.backend.cmp(&b.backend))
            .then(a.model.cmp(&b.model))
    });
    out
}

/// Agent whose accuracy is closest to 1.0× for this epic (else overall).
pub fn suggest_assignment<'a>(fits: &'a [AgentFit], epic: &str) -> Option<&'a AgentFit> {
    let scoped: Vec<_> = fits
        .iter()
        .filter(|f| f.epic == epic && !f.backend.is_empty() && f.accuracy.is_some())
        .collect();
    let pool: Vec<_> = if scoped.is_empty() {
        fits.iter()
            .filter(|f| f.epic.is_empty() && !f.backend.is_empty() && f.accuracy.is_some())
            .collect()
    } else {
        scoped
    };
    pool.into_iter().min_by(|a, b| {
        let da = (a.accuracy.unwrap_or(1.0) - 1.0).abs();
        let db = (b.accuracy.unwrap_or(1.0) - 1.0).abs();
        da.partial_cmp(&db)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.spent.cmp(&b.spent))
    })
}

fn fit_for<'a>(
    fits: &'a [AgentFit],
    backend: &str,
    model: &str,
    epic: &str,
) -> Option<&'a AgentFit> {
    let backend = normalize_backend(backend);
    fits.iter()
        .find(|f| f.backend == backend && f.model == model && f.epic == epic)
        .or_else(|| {
            fits.iter()
                .find(|f| f.backend == backend && f.epic == epic && f.accuracy.is_some())
        })
        .or_else(|| {
            fits.iter()
                .find(|f| f.backend == backend && f.model == model && f.epic.is_empty())
        })
        .or_else(|| {
            fits.iter()
                .find(|f| f.backend == backend && f.epic.is_empty() && f.accuracy.is_some())
        })
}

pub fn suggest_estimate_for(
    fits: &[AgentFit],
    backend: &str,
    model: &str,
    epic: &str,
    board: &Board,
) -> i64 {
    fit_for(fits, backend, model, epic)
        .map(|f| f.suggest)
        .unwrap_or_else(|| suggest_estimate(board))
}

fn prior_secs(backend: &str) -> i64 {
    match normalize_backend(backend).as_str() {
        "qwen" | "ollama" => (DEFAULT_TICKET_SECS as f64 * 2.5).round() as i64,
        _ => DEFAULT_TICKET_SECS,
    }
}

/// Learned duration for this agent×epic, else a local-vs-cloud prior. Not a cap.
pub fn suggest_secs_for(fits: &[AgentFit], backend: &str, model: &str, epic: &str) -> i64 {
    if let Some(f) = fit_for(fits, backend, model, epic) {
        if f.jobs > 0 && f.secs > 0 {
            return (f.secs / f.jobs as i64).max(30);
        }
        if f.suggest_secs > 0 {
            return f.suggest_secs;
        }
    }
    prior_secs(backend)
}

fn scenario_for<'a>(features: &'a [Feature], rid: &str) -> Option<(&'a Scenario, &'a [String])> {
    for f in features {
        for s in &f.scenarios {
            if s.rid.as_deref() == Some(rid) {
                return Some((s, f.background.as_slice()));
            }
        }
    }
    None
}

/// How big this scenario looks in the spec. Used until we have spent actuals.
pub fn scenario_weight(s: &Scenario, background: &[String]) -> i64 {
    let mut steps: i64 = 0;
    let mut table_rows: i64 = 0;
    let mut docs: i64 = 0;
    for line in background.iter().chain(s.steps.iter()) {
        let t = line.trim();
        if t.starts_with('|') {
            table_rows += 1;
        } else if t.contains("<<<") {
            docs += 1;
        } else if t.starts_with("Given ")
            || t.starts_with("When ")
            || t.starts_with("Then ")
            || t.starts_with("And ")
            || t.starts_with("But ")
        {
            steps += 1;
        }
    }
    let example_rows = s
        .examples
        .iter()
        .filter(|l| l.trim_start().starts_with('|'))
        .count() as i64;
    let steps = steps.max(1);
    (18_000 * steps + 4_000 * table_rows + 8_000 * example_rows + 20_000 * docs)
        .clamp(20_000, 450_000)
}

fn forecast_for_ticket(
    features: &[Feature],
    rid: &str,
    fits: &[AgentFit],
    backend: &str,
    model: &str,
    epic: &str,
    board: &Board,
) -> (i64, i64) {
    let ratio = fit_for(fits, backend, model, epic)
        .and_then(|f| f.accuracy)
        .unwrap_or(1.0)
        .clamp(0.5, 3.0);
    let weight = scenario_for(features, rid)
        .map(|(s, bg)| scenario_weight(s, bg))
        .unwrap_or_else(|| suggest_estimate_for(fits, backend, model, epic, board));
    let tokens = ((weight as f64) * ratio).round() as i64;
    let tokens = tokens.clamp(20_000, 450_000);
    let prior = prior_secs(backend).max(30);
    let secs = ((prior as f64) * (tokens as f64) / (DEFAULT_TICKET_TOKENS as f64))
        .round()
        .clamp(45.0, 1_800.0) as i64;
    (tokens, secs)
}

/// Fill or refresh forecasts from scenario size × learned ratio. Not a cap.
/// Does not overwrite a ticket that already has spend (that's the actual).
pub fn stamp_forecasts(
    board: &mut Board,
    ledger: &Ledger,
    jobs: &[Job],
    project_id: &str,
    features: &[Feature],
) -> usize {
    let fits = agent_fits(board, jobs, ledger, project_id);
    let snaps: Vec<(String, String, String, String, i64, i64)> = board
        .items
        .iter()
        .map(|it| {
            let epic = epic_name(ledger, &it.rid);
            let (backend, model, _) = agent_for_item(board, it, &epic);
            (
                it.rid.clone(),
                epic,
                backend,
                model,
                it.token_estimate,
                it.time_estimate_secs,
            )
        })
        .collect();
    let mut n = 0;
    for (rid, epic, backend, model, tok, secs) in snaps {
        let (spent_tok, spent_secs) = spent_for_rid(jobs, project_id, &rid);
        let has_spec = scenario_for(features, &rid).is_some();
        let (next_tok, next_secs) = if has_spec {
            forecast_for_ticket(features, &rid, &fits, &backend, &model, &epic, board)
        } else {
            (
                if tok <= 0 {
                    suggest_estimate_for(&fits, &backend, &model, &epic, board)
                } else {
                    tok
                },
                if secs <= 0 {
                    suggest_secs_for(&fits, &backend, &model, &epic)
                } else {
                    secs
                },
            )
        };
        let rewrite = has_spec && spent_tok == 0 && spent_secs == 0;
        let fill_tok = tok <= 0;
        let fill_secs = secs <= 0;
        if let Some(it) = board.items.iter_mut().find(|i| i.rid == rid) {
            if (rewrite || fill_tok) && it.token_estimate != next_tok {
                it.token_estimate = next_tok;
                n += 1;
            }
            if (rewrite || fill_secs) && it.time_estimate_secs != next_secs {
                it.time_estimate_secs = next_secs;
                n += 1;
            }
        }
    }
    n
}

fn empty_alloc(name: &str) -> EpicAlloc {
    EpicAlloc {
        name: name.into(),
        estimated: 0,
        ticket_sum: 0,
        spent: 0,
        forecast_secs: 0,
        spent_secs: 0,
        backend: String::new(),
        model: String::new(),
        tickets: 0,
        remaining: 0,
        items: Vec::new(),
        suggest_backend: String::new(),
        suggest_model: String::new(),
        suggest_why: String::new(),
    }
}

pub fn command_center(
    board: &Board,
    jobs: &[Job],
    ledger: &Ledger,
    project_id: &str,
    sprint_id: Option<&str>,
) -> CommandCenter {
    let fits = agent_fits(board, jobs, ledger, project_id);
    let mut epic_map: BTreeMap<String, EpicAlloc> = BTreeMap::new();
    for e in &board.epics {
        let mut row = empty_alloc(&e.name);
        row.estimated = e.token_estimate;
        row.backend = e.backend.clone();
        row.model = e.model.clone();
        epic_map.insert(e.name.clone(), row);
    }

    let mut model_est: BTreeMap<(String, String), (i64, usize)> = BTreeMap::new();

    for it in &board.items {
        if let Some(sid) = sprint_id {
            if it.sprint_id.as_deref() != Some(sid) {
                continue;
            }
        }
        let epic = epic_name(ledger, &it.rid);
        let key = if epic.is_empty() {
            "Ungrouped".to_string()
        } else {
            epic.clone()
        };
        let (backend, model, inherited) = agent_for_item(board, it, &epic);
        let status = ticket_status(ledger, &it.rid);
        let (spent_tokens, spent_secs) = spent_for_rid(jobs, project_id, &it.rid);
        let row = epic_map
            .entry(key.clone())
            .or_insert_with(|| empty_alloc(&key));
        row.ticket_sum += it.token_estimate;
        row.forecast_secs += it.time_estimate_secs;
        row.tickets += 1;
        if status != GREEN && status != ORPHAN {
            row.remaining += 1;
        }
        row.items.push(EpicTicket {
            rid: it.rid.clone(),
            name: ticket_name(ledger, &it.rid),
            status,
            estimate: it.token_estimate,
            forecast_secs: it.time_estimate_secs,
            spent_tokens,
            spent_secs,
            backend: backend.clone(),
            model: model.clone(),
            sprint: it
                .sprint_id
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "backlog".into()),
            inherited,
            suggest: suggest_estimate_for(&fits, &backend, &model, &epic, board),
            suggest_secs: suggest_secs_for(&fits, &backend, &model, &epic),
            epic: epic.clone(),
            file: ticket_file(ledger, &it.rid),
        });
        let mk = if backend.is_empty() && model.is_empty() {
            (String::new(), String::new())
        } else {
            (backend, model)
        };
        let slot = model_est.entry(mk).or_insert((0, 0));
        slot.0 += it.token_estimate;
        slot.1 += 1;
    }

    for e in epic_map.values_mut() {
        if e.estimated <= 0 {
            e.estimated = e.ticket_sum;
        }
        e.items.sort_by(|a, b| a.rid.cmp(&b.rid));
        if let Some(f) = suggest_assignment(&fits, &e.name) {
            e.suggest_backend = f.backend.clone();
            e.suggest_model = f.model.clone();
            let acc = f
                .accuracy
                .map(|a| format!("{a:.1}×"))
                .unwrap_or_else(|| "—".into());
            let scope = if f.epic.is_empty() {
                "all work"
            } else {
                f.epic.as_str()
            };
            e.suggest_why = format!(
                "{acc} on {scope} · {} job{}",
                f.jobs,
                if f.jobs == 1 { "" } else { "s" }
            );
        }
    }

    for j in jobs {
        if j.project_id != project_id {
            continue;
        }
        if let Some(sid) = sprint_id {
            if j.sprint_id != sid {
                continue;
            }
        }
        let n = job_tokens(j);
        let secs = job_secs(j);
        if n <= 0 && secs <= 0 {
            continue;
        }
        let key = if j.epic.is_empty() {
            None
        } else {
            Some(j.epic.clone())
        };
        if let Some(k) = key {
            if let Some(e) = epic_map.get_mut(&k) {
                e.spent += n;
                e.spent_secs += secs;
            } else {
                let mut row = empty_alloc(&k);
                row.spent = n;
                row.spent_secs = secs;
                row.backend = j.backend.clone();
                row.model = j.model.clone();
                epic_map.insert(k, row);
            }
        }
    }

    let mut models: BTreeMap<(String, String), ModelScore> = BTreeMap::new();
    for j in jobs {
        if j.project_id != project_id {
            continue;
        }
        if let Some(sid) = sprint_id {
            if !j.sprint_id.is_empty() && j.sprint_id != sid {
                continue;
            }
        }
        let key = (j.backend.clone(), j.model.clone());
        let spent = job_tokens(j);
        let row = models.entry(key.clone()).or_insert_with(|| ModelScore {
            backend: key.0.clone(),
            model: key.1.clone(),
            jobs: 0,
            spent: 0,
            estimated: 0,
            tickets: 0,
        });
        row.jobs += 1;
        row.spent += spent;
    }
    for (key, (est, n)) in model_est {
        let row = models.entry(key.clone()).or_insert_with(|| ModelScore {
            backend: key.0.clone(),
            model: key.1.clone(),
            jobs: 0,
            spent: 0,
            estimated: 0,
            tickets: 0,
        });
        row.estimated += est;
        row.tickets += n;
    }
    let mut models: Vec<_> = models.into_values().collect();
    models.sort_by(|a, b| b.spent.cmp(&a.spent).then(a.backend.cmp(&b.backend)));

    let mut epics: Vec<_> = epic_map.into_values().collect();
    epics.sort_by(|a, b| b.estimated.cmp(&a.estimated).then(a.name.cmp(&b.name)));

    let allocated: i64 = epics.iter().map(|e| e.estimated).sum();
    let forecast_secs: i64 = epics.iter().map(|e| e.forecast_secs).sum();
    let spent: i64 = if let Some(sid) = sprint_id {
        spent_for_sprint(jobs, project_id, sid)
    } else {
        jobs.iter()
            .filter(|j| j.project_id == project_id)
            .map(job_tokens)
            .sum()
    };
    let spent_secs: i64 = jobs
        .iter()
        .filter(|j| {
            j.project_id == project_id
                && sprint_id
                    .map(|sid| j.sprint_id == sid || j.sprint_id.is_empty())
                    .unwrap_or(true)
        })
        .map(job_secs)
        .sum();
    let accuracy = if allocated > 0 {
        Some(((10.0 * spent as f64 / allocated as f64).round()) / 10.0)
    } else {
        None
    };

    CommandCenter {
        allocated,
        spent,
        forecast_secs,
        spent_secs,
        accuracy,
        epics,
        models,
        fits,
    }
}

/// Project-level forecast vs actual for the inbox. Same numbers as Command, plus live wall time.
#[derive(Debug, Clone, Serialize, Default)]
pub struct SpendSnapshot {
    pub estimated: i64,
    pub spent: i64,
    pub forecast_secs: i64,
    pub spent_secs: i64,
}

pub fn spend_snapshot(
    board: &Board,
    jobs: &[Job],
    ledger: &Ledger,
    project_id: &str,
) -> SpendSnapshot {
    let cc = command_center(board, jobs, ledger, project_id, None);
    let spent_secs: i64 = jobs
        .iter()
        .filter(|j| j.project_id == project_id)
        .map(job_wall_secs)
        .sum();
    SpendSnapshot {
        estimated: cc.allocated,
        spent: cc.spent,
        forecast_secs: cc.forecast_secs,
        spent_secs,
    }
}

pub fn job_tokens(j: &Job) -> i64 {
    if j.prompt_tokens + j.completion_tokens > 0 {
        j.prompt_tokens + j.completion_tokens
    } else {
        0
    }
}

pub fn spent_for_sprint(jobs: &[Job], project_id: &str, sprint_id: &str) -> i64 {
    jobs.iter()
        .filter(|j| j.project_id == project_id && j.sprint_id == sprint_id)
        .map(job_tokens)
        .sum()
}

pub fn estimated_for_sprint(board: &Board, sprint_id: &str) -> (usize, i64) {
    let items: Vec<_> = board
        .items
        .iter()
        .filter(|i| i.sprint_id.as_deref() == Some(sprint_id))
        .collect();
    let estimated = items.iter().map(|i| i.token_estimate).sum();
    (items.len(), estimated)
}

/// Next ticket default, scaled by the last closed sprint's spent/estimated.
pub fn suggest_estimate(board: &Board) -> i64 {
    let last = board
        .sprints
        .iter()
        .rev()
        .find(|s| s.closed_at.is_some() && s.estimated > 0);
    match last {
        Some(s) => {
            let ratio = (s.spent as f64 / s.estimated as f64).clamp(0.5, 3.0);
            ((DEFAULT_TICKET_TOKENS as f64) * ratio).round() as i64
        }
        None => DEFAULT_TICKET_TOKENS,
    }
}

pub fn retro(
    board: &Board,
    jobs: &[Job],
    project_id: &str,
    sprint_id: &str,
) -> Option<SprintRetro> {
    let sprint = board.sprints.iter().find(|s| s.id == sprint_id)?;
    let (tickets, live_est) = estimated_for_sprint(board, sprint_id);
    let live_spent = spent_for_sprint(jobs, project_id, sprint_id);
    let estimated = if sprint.closed_at.is_some() && sprint.estimated > 0 {
        sprint.estimated
    } else {
        live_est
    };
    let spent = if sprint.closed_at.is_some() && sprint.spent > 0 {
        sprint.spent
    } else {
        live_spent
    };
    let accuracy = if estimated > 0 {
        Some(((10.0 * spent as f64 / estimated as f64).round()) / 10.0)
    } else {
        None
    };
    Some(SprintRetro {
        sprint_id: sprint.id.clone(),
        title: sprint.title.clone(),
        tickets,
        estimated,
        spent,
        accuracy,
        bias: spent - estimated,
        closed: sprint.closed_at.is_some(),
        suggest: suggest_estimate(board),
    })
}
