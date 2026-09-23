//! Play continues the inner loop: spec → tests → run → code → run again.

use crate::board::{verify_drift, Board};
use crate::compose::{backend_for_job, execute_author, run_role_or_failover};
use crate::config::Config;
use crate::integrity::audit;
use crate::jobs::{kind_phase, Job, JobKind, JobQueue, JobStatus};
use crate::ledger::{Ledger, GREEN, ORPHAN, PENDING, RED, STALE};
use crate::org::Org;
use crate::roles::{run_role, RoleError};
use crate::runner::{failure_digest, harness_report, run_suite};
use crate::spec::{holdout_rids, load_specs, stamp_rids};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Author,
    /// Spec exists; designer has not written current storyboards.
    Design,
    /// Product look exists; Playwright / static UX pass still due.
    Ux,
    /// Spec exists; human has not picked rust / javascript / python yet.
    Language,
    Steps,
    Build,
    Run,
    Idle,
}

pub fn next_stage(root: &Path) -> Stage {
    let features = load_specs(&root.join("spec"), false).unwrap_or_default();
    if features.is_empty() || features.iter().all(|f| f.scenarios.is_empty()) {
        return Stage::Author;
    }
    if crate::mockups::design_needed(root, &features) {
        return Stage::Design;
    }
    let q = crate::jobs::JobQueue::load();
    let pid = crate::org::Org::load()
        .projects
        .iter()
        .find(|p| PathBuf::from(&p.path) == root)
        .map(|p| p.id.clone())
        .unwrap_or_default();
    if crate::ux::ux_needed(root, &q.jobs, &pid) {
        return Stage::Ux;
    }
    let cfg = Config::load(root).unwrap_or_default();
    if !crate::config::stack_is_set(&cfg.stack) {
        return Stage::Language;
    }
    let board = Board::load(&root.join(".shalt/board.json"));
    let focus = board.focus_journey.clone();
    if crate::bindings::steps_needed(root, &features, &focus) {
        return Stage::Steps;
    }
    let mut led = Ledger::load(&root.join(".shalt/ledger.json")).unwrap_or_default();
    led.sync_spec(&features);
    let remaining = led
        .entries
        .values()
        .filter(|e| {
            e.status != GREEN
                && e.status != ORPHAN
                && (focus.trim().is_empty() || e.epic == focus)
        })
        .count();
    let unstamped = features
        .iter()
        .flat_map(|f| &f.scenarios)
        .filter(|s| s.rid.is_none())
        .count();
    if remaining > 0 || unstamped > 0 {
        if !led.suite_surveyed() {
            Stage::Run
        } else {
            Stage::Build
        }
    } else {
        Stage::Idle
    }
}

/// plan → language → tests → build. The desk uses this to gate Play.
pub fn work_gate(root: &Path) -> &'static str {
    match next_stage(root) {
        Stage::Author => "plan",
        Stage::Design => "design",
        Stage::Ux => "ux",
        Stage::Language => "language",
        Stage::Steps => "tests",
        Stage::Run => "run",
        Stage::Build => "build",
        Stage::Idle => "idle",
    }
}

pub fn ensure_play_lock(root: &Path) -> Result<usize, String> {
    let minted = stamp_rids(&root.join("spec")).map_err(|e| e.to_string())?;
    let features = load_specs(&root.join("spec"), false).map_err(|e| e.to_string())?;
    let mut led = Ledger::load(&root.join(".shalt/ledger.json")).unwrap_or_default();
    led.sync_spec(&features);
    let empty = led
        .spec_lock
        .as_object()
        .map(|o| o.is_empty())
        .unwrap_or(true);
    if empty {
        let hashes: serde_json::Map<String, serde_json::Value> = features
            .iter()
            .flat_map(|f| f.scenarios.iter().map(move |s| (f, s)))
            .filter_map(|(f, s)| {
                s.rid.as_ref().map(|r| {
                    (
                        r.clone(),
                        serde_json::Value::String(s.spec_hash(&f.background)),
                    )
                })
            })
            .collect();
        led.spec_lock = serde_json::json!({
            "approved_by": "play",
            "approved_at": chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            "scenario_count": features.iter().map(|f| f.scenarios.len()).sum::<usize>(),
            "scenario_hashes": hashes,
        });
    }
    let mut board = Board::load(&root.join(".shalt/board.json"));
    board.sync_new_rids(&features);
    let _ = board.save(&root.join(".shalt/board.json"));
    led.save(&root.join(".shalt/ledger.json"))
        .map_err(|e| e.to_string())?;
    Ok(minted.len())
}

fn project_root(project_id: &str) -> Result<PathBuf, String> {
    let org = Org::load();
    let p = org
        .get(project_id)
        .ok_or_else(|| format!("unknown project {project_id}"))?;
    Ok(PathBuf::from(&p.path))
}

fn inherit(q: &JobQueue, project_id: &str) -> (String, String) {
    let grok_out = crate::compose::backend_quota_exhausted(&q.jobs, "grok");
    q.jobs
        .iter()
        .rev()
        .find(|j| {
            j.project_id == project_id
                && (!j.backend.is_empty() || !j.model.is_empty())
                && !(grok_out && crate::tokens::normalize_backend(&j.backend) == "grok")
        })
        .map(|j| (j.backend.clone(), j.model.clone()))
        .unwrap_or_else(|| ("qwen".into(), String::new()))
}

/// Queue the next stage (tests, then code) for a free epoch.
/// Another ticket in the same project may already be running.
pub fn continue_project(project_id: &str) -> Result<Option<Job>, String> {
    let org = Org::load();
    let Some(p) = org.get(project_id) else {
        return Err(format!("unknown project {project_id}"));
    };
    if p.paused {
        return Ok(None);
    }
    let q = JobQueue::load();
    let root = PathBuf::from(&p.path);
    if q.authoring_open(project_id) {
        return Ok(None);
    }
    if !root.join("spec").exists() && next_stage(&root) != Stage::Author {
        return Ok(None);
    }
    let stage = next_stage(&root);
    if matches!(stage, Stage::Steps | Stage::Build | Stage::Run) {
        let _ = ensure_play_lock(&root)?;
    }
    let stage = next_stage(&root);
    let mut board = Board::load(&root.join(".shalt/board.json"));
    let features = load_specs(&root.join("spec"), false).unwrap_or_default();
    board.sync_new_rids(&features);
    board.sync_epics(&features);
    let mut led = Ledger::load(&root.join(".shalt/ledger.json")).unwrap_or_default();
    led.sync_spec(&features);
    if crate::sprint::cadence_due(&board, &led) {
        let yolo = Org::yolo_plan(project_id);
        match crate::sprint::maybe_enqueue_plan(
            project_id,
            &root,
            &mut board,
            &q.jobs,
            &led,
            yolo,
        ) {
            Ok(Some(j)) => {
                let _ = board.save(&root.join(".shalt/board.json"));
                return Ok(Some(j));
            }
            Ok(None) => {
                let _ = board.save(&root.join(".shalt/board.json"));
            }
            Err(_) => {}
        }
    }
    let kind = match stage {
        Stage::Idle => return Ok(None),
        Stage::Author => JobKind::Author,
        Stage::Design => JobKind::Design,
        Stage::Ux => JobKind::Ux,
        Stage::Language => {
            return Err("Pick a build language first, then Play writes tests.".into())
        }
        Stage::Steps => JobKind::Steps,
        Stage::Run => JobKind::Run,
        Stage::Build => play_kind_after_survey(&root, &led, &features, &board.focus_journey),
    };
    let sprint = board.active_sprint().map(|s| s.id.clone());
    let mut assign = crate::tokens::next_free_assignment(
        &board,
        &led,
        sprint.as_deref(),
        &q.jobs,
        project_id,
        kind,
    );
    if let Some(a) = assign.as_mut() {
        if a.backend.is_empty() && a.model.is_empty() {
            let pick = crate::alloc::pick_for(
                &board,
                &led,
                &q.jobs,
                project_id,
                &a.epic,
                &mut rand::thread_rng(),
            );
            a.backend = pick.backend.clone();
            a.model = pick.model.clone();
            board.set_item_agent(&a.rid, &a.backend, &a.model);
        }
    }
    let _ = crate::alloc::prepare_board(&mut board, &led, &q.jobs, project_id, &features);
    let _ = board.save(&root.join(".shalt/board.json"));
    if assign.is_none()
        && !board.items.is_empty()
        && !matches!(
            kind,
            JobKind::Run | JobKind::Author | JobKind::Design | JobKind::Steps
        )
    {
        return Ok(None);
    }
    let (backend, model) = match &assign {
        Some(a) if !a.backend.is_empty() || !a.model.is_empty() => {
            (a.backend.clone(), a.model.clone())
        }
        _ => inherit(&q, project_id),
    };
    let mut q = JobQueue::load();
    let job = q.enqueue_full(kind, project_id, "", &backend, &model);
    if let Some(sp) = sprint {
        q.set_sprint(&job.id, &sp);
    }
    if kind == JobKind::Steps {
        let defs = crate::bindings::load_step_defs(&root);
        if let Some(j) = crate::bindings::pick_steps_journey(&features, &defs, &board.focus_journey)
        {
            q.set_work(&job.id, &j, "");
            q.append(&job.id, &format!("tests for journey {j}"));
        }
    } else if kind == JobKind::Design {
        if let Some(j) =
            crate::mockups::pick_design_journey(&root, &features, &board.focus_journey)
        {
            q.set_work(&job.id, &j, "");
            q.append(&job.id, &format!("drawing journey {j}"));
        }
    } else if !matches!(kind, JobKind::Run | JobKind::Author | JobKind::Design) {
        if let Some(a) = assign {
            q.set_work(&job.id, &a.epic, &a.rid);
            if !a.backend.is_empty() || !a.model.is_empty() {
                let who = crate::tokens::agent_label(&a.backend, &a.model);
                q.append(&job.id, &format!("assigned {who} · {}", a.rid));
            }
        }
    }
    if kind == JobKind::Author {
        let onboard = q
            .jobs
            .iter()
            .rev()
            .any(|j| j.project_id == project_id && j.onboard);
        if onboard {
            if let Some(j) = q.jobs.iter_mut().find(|j| j.id == job.id) {
                j.onboard = true;
            }
        }
        if onboard || led.entries.is_empty() {
            let plan = crate::talk::load_plan(&root, "");
            if !plan.trim().is_empty() {
                q.set_prompt(&job.id, &plan);
            }
        } else {
            q.set_prompt(&job.id, &refine_spec_prompt(&led));
        }
    }
    let live = q.get(&job.id).cloned().unwrap_or(job.clone());
    let cap = crate::parallel::Capacity::load();
    if !crate::parallel::can_admit(&live, &q.jobs, &cap).ok() {
        q.jobs.retain(|j| j.id != job.id);
        q.save().map_err(|e| e.to_string())?;
        return Ok(None);
    }
    q.append(
        &job.id,
        match kind {
            JobKind::Author => "play — tests showed the spec needs a pass; rewriting the spec",
            JobKind::Design => "play — drawing storyboards for each journey",
            JobKind::Ux => "play — UX pass now that the look is past wireframes",
            JobKind::Plan => "play — sprint retro, then plan the next slice",
            JobKind::Steps => "play — writing tests",
            JobKind::Run => "play — run every test first so we can see what depends on what",
            JobKind::Build => "play — build this ticket until it passes",
            _ => "play",
        },
    );
    q.save().map_err(|e| e.to_string())?;
    Ok(Some(q.get(&job.id).cloned().unwrap_or(job)))
}

fn play_kind_after_survey(
    root: &Path,
    led: &Ledger,
    features: &[crate::spec::Feature],
    focus_journey: &str,
) -> JobKind {
    let remaining = led
        .entries
        .values()
        .filter(|e| {
            e.status != GREEN
                && e.status != ORPHAN
                && (focus_journey.trim().is_empty() || e.epic == focus_journey)
        })
        .count();
    if led.spec_shaped_failures() > 0 {
        return JobKind::Author;
    }
    if harness_points_at_steps(led) {
        return JobKind::Steps;
    }
    let defs = crate::bindings::load_step_defs(root);
    let unbound = crate::bindings::unbound_scenarios(features, &defs, focus_journey).len();
    if remaining > 0 && unbound * 2 >= remaining {
        return JobKind::Steps;
    }
    JobKind::Build
}

fn harness_points_at_steps(led: &Ledger) -> bool {
    let dump = led
        .display_harness_error()
        .or_else(|| led.harness_error().map(|s| s.to_string()))
        .unwrap_or_default();
    if dump.is_empty() {
        return false;
    }
    let t = dump.to_ascii_lowercase();
    t.contains("tests/shalt.rs")
        || t.contains("tests/cucumber.rs")
        || t.contains("(test \"shalt\")")
        || t.contains("(test \"cucumber\")")
        || (t.contains("steps/")
            && (t.contains("syntaxerror") || t.contains("referenceerror")))
}

fn refine_spec_prompt(led: &Ledger) -> String {
    let mut lines = vec![
        "The spec is living. Tests (and implementation) showed it needs to change.".into(),
        "Rewrite spec/*.feature so scenarios are concrete, testable, and consistent.".into(),
        "Do not freeze old spec. Do not write tests or src/.".into(),
        String::new(),
        "What the last suite said:".into(),
    ];
    let mut n = 0;
    for e in led.entries.values() {
        if e.status == GREEN || e.status == ORPHAN {
            continue;
        }
        n += 1;
        if n > 12 {
            lines.push("…".into());
            break;
        }
        let fail = e.failure.as_deref().unwrap_or("no result yet");
        lines.push(format!("- {} ({}) · {} · {}", e.rid, e.status, e.name, fail));
    }
    lines.join("\n")
}

pub fn execute_job(job_id: &str) -> Result<String, String> {
    let q = JobQueue::load();
    let job = q
        .get(job_id)
        .cloned()
        .ok_or_else(|| format!("no job {job_id}"))?;
    match job.kind {
        JobKind::Author => execute_author(job_id),
        JobKind::Design => crate::compose::execute_design(job_id),
        JobKind::Steps => execute_steps(job_id),
        JobKind::Build => execute_build(job_id, 6),
        JobKind::Run => execute_run(job_id),
        JobKind::Verify => execute_verify(job_id),
        JobKind::Plan => execute_plan(job_id),
        JobKind::Ux => execute_ux(job_id),
        other => {
            let msg = format!("job kind {other:?} is not runnable yet");
            let _ = finish_err(job_id, &msg);
            Err(msg)
        }
    }
}

fn start_running(job_id: &str) -> Result<(Job, PathBuf), String> {
    let mut q = JobQueue::load();
    let job = q
        .get(job_id)
        .cloned()
        .ok_or_else(|| format!("no job {job_id}"))?;
    if matches!(
        job.status,
        JobStatus::Paused | JobStatus::Interrupted | JobStatus::Done | JobStatus::Failed
    ) {
        return Err(format!("stopped ({:?})", job.status));
    }
    q.set_status(job_id, JobStatus::Running);
    crate::parallel::mark_start(&mut q, job_id);
    let _ = q.save();
    let job = q
        .get(job_id)
        .cloned()
        .ok_or_else(|| format!("no job {job_id}"))?;
    let root = project_root(&job.project_id)?;
    Ok((job, root))
}

fn finish_ok(job_id: &str, summary: &str) -> Result<String, String> {
    finish_ok_with(job_id, summary, "")
}

fn finish_ok_with(job_id: &str, summary: &str, transcript: &str) -> Result<String, String> {
    let mut q = JobQueue::load();
    if q.keep_parked(
        job_id,
        "model returned after Pause — discarded. Play to continue.",
    ) {
        let _ = q.save();
        return Err("stopped (Paused)".into());
    }
    q.append(job_id, summary);
    q.set_status(job_id, JobStatus::Done);
    let _ = q.save();
    if let Some(j) = q.get(job_id).cloned() {
        if let Ok(root) = project_root(&j.project_id) {
            let _ = crate::journal::publish(&root, &j, transcript);
        }
        crate::parallel::mark_finish(&j);
    }
    Ok(summary.to_string())
}

fn finish_err(job_id: &str, stopped: &str) -> Result<String, String> {
    let mut q = JobQueue::load();
    let short = crate::jobs::human_error(stopped);
    if q.keep_parked(
        job_id,
        &format!("parked while waiting on the model ({short})"),
    ) {
        let _ = q.save();
        return Err(stopped.to_string());
    }
    if stopped.contains("model switched") {
        q.append(
            job_id,
            "old model call dropped — Play continues on the new one",
        );
        let _ = q.save();
        return Err(stopped.to_string());
    }
    q.set_error(job_id, &short);
    q.append(job_id, &format!("failed: {short}"));
    let status = if stopped.contains("stopped") {
        JobStatus::Interrupted
    } else {
        JobStatus::Failed
    };
    q.set_status(job_id, status);
    let _ = q.save();
    Err(stopped.to_string())
}

fn wait_if_parked(job_id: &str) -> Result<(), String> {
    loop {
        let q = JobQueue::load();
        let Some(j) = q.get(job_id) else {
            return Err("job disappeared".into());
        };
        match j.status {
            JobStatus::Waiting => {
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
            JobStatus::Paused => return Err("stopped (Paused)".into()),
            JobStatus::Running | JobStatus::Pending => return Ok(()),
            other => return Err(format!("stopped ({other:?})")),
        }
    }
}

fn steps_target_rel(cfg: &Config, journey: &str) -> String {
    if cfg.stack == "javascript" {
        format!("{}/{journey}.steps.js", cfg.steps.trim_end_matches('/'))
    } else {
        "tests/shalt.rs".into()
    }
}

fn stepwright_brief(root: &Path, journey: &str, model: &str, extra: &str) -> String {
    let cfg = Config::load(root).unwrap_or_default();
    let rel = steps_target_rel(&cfg, journey);
    let file_text = std::fs::read_to_string(root.join(&rel)).unwrap_or_else(|_| "(missing)\n".into());
    let features = load_specs(&root.join("spec"), false).unwrap_or_default();
    let unbound = crate::brief::unbound_lines(&features, journey);
    let defs = crate::bindings::load_step_defs(root);
    let bound_phrases: Vec<String> = defs
        .iter()
        .filter(|d| !d.stub)
        .map(|d| format!("{}|{}", d.kw, d.pattern))
        .collect();
    let contract_names: Vec<String> = std::fs::read_to_string(root.join("contract/interface.md"))
        .ok()
        .map(|md| {
            crate::scaffold::parse_js_contract(&md)
                .into_iter()
                .flat_map(|m| {
                    let path = m.path;
                    m.fns
                        .into_iter()
                        .map(move |f| format!("{}::{}", path, f.name))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut user = crate::brief::stepwright_user(&crate::brief::StepBrief {
        stack: cfg.stack.clone(),
        model: model.to_string(),
        journey: journey.to_string(),
        file_rel: rel,
        file_text,
        unbound,
        contract_names,
        bound_phrases,
    });
    if !extra.trim().is_empty() {
        user.push_str("\n\n");
        user.push_str(extra.trim());
    }
    crate::journal::with_pending(root, &user)
}

fn execute_steps(job_id: &str) -> Result<String, String> {
    let (mut job, root) = match start_running(job_id) {
        Ok(v) => v,
        Err(e) => return finish_err(job_id, &e),
    };
    pin_inner_loop(&mut job);
    if let Err(e) = ensure_play_lock(&root) {
        return finish_err(job_id, &e);
    }
    let features = load_specs(&root.join("spec"), false).unwrap_or_default();
    let board = Board::load(&root.join(".shalt/board.json"));
    let mut q = JobQueue::load();
    let defs_before = crate::bindings::load_step_defs(&root);
    let journey = if !job.epic.trim().is_empty() {
        job.epic.clone()
    } else {
        crate::bindings::pick_steps_journey(&features, &defs_before, &board.focus_journey)
            .unwrap_or_default()
    };
    if journey.is_empty() {
        return finish_err(job_id, "no journey still needs tests");
    }
    if job.epic.trim().is_empty() {
        q.set_work(job_id, &journey, "");
        job.epic = journey.clone();
    }
    let moved = crate::scaffold::quarantine_duplicate_step_files(&root).unwrap_or_default();
    if !moved.is_empty() {
        q.append(
            job_id,
            &format!("quarantined duplicate steps → .shalt/dup-steps ({})", moved.join(", ")),
        );
    }
    let _ = crate::scaffold::write_js_world_if_missing(&root);
    let stubbed = crate::scaffold::apply_step_stubs(&root, &journey).unwrap_or_default();
    if !stubbed.is_empty() {
        q.append(job_id, &format!("stubbed {}", stubbed.join(", ")));
    }
    let defs_before = crate::bindings::load_step_defs(&root);
    let bound_before = crate::bindings::bound_count(&features, &defs_before, &journey);
    q.append(
        job_id,
        &format!("writing real tests for journey {journey} ({bound_before} already bound)"),
    );
    let _ = q.save();
    let cfg = Config::load(&root).unwrap_or_default();
    let target = steps_target_rel(&cfg, &journey);
    let mut prompt = stepwright_brief(&root, &journey, &job.model, "");
    #[allow(unused_assignments)]
    let mut wrote: Vec<String> = Vec::new();
    #[allow(unused_assignments)]
    let mut transcript = String::new();
    #[allow(unused_assignments)]
    let mut bound_after = bound_before;
    let mut tries = 0usize;
    loop {
        if let Err(e) = wait_if_parked(job_id) {
            return finish_err(job_id, &e);
        }
        tries += 1;
        let backup = std::fs::read_to_string(root.join(&target)).ok();
        let t0 = Instant::now();
        match run_role_or_failover(&job, &root, "stepwright", &prompt, false) {
            Ok(res) => {
                let secs = t0.elapsed().as_secs_f64();
                let body = std::fs::read_to_string(root.join(&target)).unwrap_or_default();
                if !crate::scaffold::steps_source_ok(&target, &body) {
                    if let Some(prev) = &backup {
                        let _ = std::fs::write(root.join(&target), prev);
                    }
                    let mut q = JobQueue::load();
                    q.append(job_id, &format!("lane · write · {} · parse", job.model));
                    let _ = q.save();
                    if continue_write_chain(&mut job, tries, secs, res.completion_tokens) {
                        prompt = stepwright_brief(
                            &root,
                            &journey,
                            &job.model,
                            "Previous write did not parse. Fill pending bodies. Do not paste the prompt.",
                        );
                        continue;
                    }
                    return finish_err(job_id, "stepwright write did not parse");
                }
                let defs_after = crate::bindings::load_step_defs(&root);
                bound_after = crate::bindings::bound_count(&features, &defs_after, &journey);
                if bound_after <= bound_before {
                    let mut q = JobQueue::load();
                    q.append(job_id, &format!("lane · write · {} · bind", job.model));
                    let _ = q.save();
                    if continue_write_chain(&mut job, tries, secs, res.completion_tokens) {
                        prompt = stepwright_brief(
                            &root,
                            &journey,
                            &job.model,
                            &format!(
                                "Pending bodies are not a bind. Fill the Given/When/Then for `{journey}` so each scenario has a real oracle."
                            ),
                        );
                        continue;
                    }
                    return finish_err(
                        job_id,
                        &format!(
                            "stepwright wrote {} but bound no new scenarios in `{journey}` — stubs do not count.",
                            if res.wrote.is_empty() {
                                target.clone()
                            } else {
                                res.wrote.join(", ")
                            }
                        ),
                    );
                }
                wrote = if res.wrote.is_empty() {
                    vec![target.clone()]
                } else {
                    res.wrote
                };
                transcript = res.transcript;
                break;
            }
            Err(RoleError::Integrity(e)) => {
                let secs = t0.elapsed().as_secs_f64();
                if let Some(prev) = &backup {
                    let _ = std::fs::write(root.join(&target), prev);
                }
                let mut q = JobQueue::load();
                q.append(job_id, &format!("lane · write · {} · parse", job.model));
                let _ = q.save();
                if continue_write_chain(&mut job, tries, secs, 0) {
                    prompt = stepwright_brief(
                        &root,
                        &journey,
                        &job.model,
                        &format!("Previous pass was rejected: {e}. Fill bodies in `{target}` only."),
                    );
                    continue;
                }
                return finish_err(job_id, &e.to_string());
            }
            Err(e) => return finish_err(job_id, &e.to_string()),
        }
    }
    let mut audit_round = 0usize;
    loop {
        audit_round += 1;
        match run_audit(
            &mut job,
            &root,
            crate::audit::Subject::Tests,
            &journey,
            "",
        ) {
            Err(e) => return finish_err(job_id, &e),
            Ok(None) => {
                return finish_err(
                    job_id,
                    "auditor did not respond — fail closed; gate stays tests",
                );
            }
            Ok(Some(crate::audit::Verdict { pass: true, .. })) => break,
            Ok(Some(v)) => {
                if audit_round >= 2 {
                    return finish_err(
                        job_id,
                        &format!("auditor rejected the tests:\n{}", v.findings),
                    );
                }
                let installed = installed_local();
                if let Some((b, m)) = crate::alloc::pick_escalate_model(&installed, &job.model) {
                    set_job_lane(
                        &mut job,
                        &b,
                        &m,
                        &format!("lane · write · {m} · audit"),
                    );
                } else if !job.model.contains("8b") {
                    if let Some((b, m)) = crate::alloc::pick_fast_model(&installed) {
                        if m.contains("8b") {
                            set_job_lane(&mut job, &b, &m, &format!("lane · write · {m} · audit"));
                        }
                    }
                }
                prompt = stepwright_brief(
                    &root,
                    &journey,
                    &job.model,
                    &format!(
                        "The auditor rejected the tests. Fix these findings. Do not rewrite unrelated files.\n{}",
                        v.findings
                    ),
                );
                if let Err(e) = wait_if_parked(job_id) {
                    return finish_err(job_id, &e);
                }
                match run_role_or_failover(&job, &root, "stepwright", &prompt, false) {
                    Ok(res) => {
                        let body = std::fs::read_to_string(root.join(&target)).unwrap_or_default();
                        if !crate::scaffold::steps_source_ok(&target, &body) {
                            return finish_err(job_id, "auditor fix did not parse");
                        }
                        let defs_after = crate::bindings::load_step_defs(&root);
                        bound_after = crate::bindings::bound_count(&features, &defs_after, &journey);
                        if bound_after <= bound_before {
                            return finish_err(
                                job_id,
                                "auditor asked for a fix but the tests still bind no new scenarios",
                            );
                        }
                        wrote = res.wrote;
                        transcript = res.transcript;
                    }
                    Err(e) => return finish_err(job_id, &e.to_string()),
                }
            }
        }
    }
    let stubs = crate::scaffold::apply_js_contract_stubs(&root).unwrap_or_default();
    let extra = if stubs.is_empty() {
        String::new()
    } else {
        format!("; stubbed {}", stubs.join(", "))
    };
    finish_ok_with(
        job_id,
        &format!(
            "bound {bound_after} scenarios in `{journey}` (was {bound_before}); wrote {}{extra}",
            wrote.join(", ")
        ),
        &transcript,
    )
}

fn execute_build(job_id: &str, max_turns: usize) -> Result<String, String> {
    let (job, root) = match start_running(job_id) {
        Ok(v) => v,
        Err(e) => return finish_err(job_id, &e),
    };
    if let Err(e) = ensure_play_lock(&root) {
        return finish_err(job_id, &e);
    }
    let cfg = Config::load(&root).unwrap_or_default();
    let _ = crate::scaffold::apply_js_contract_stubs(&root);
    let features = match load_specs(&root.join("spec"), false) {
        Ok(f) => f,
        Err(e) => return finish_err(job_id, &e.to_string()),
    };
    let mut led = Ledger::load(&root.join(".shalt/ledger.json")).unwrap_or_default();
    led.sync_spec(&features);
    let held = holdout_rids(&features);
    let live: Vec<_> = led
        .entries
        .iter()
        .filter(|(_, e)| e.status != ORPHAN)
        .map(|(r, _)| r.clone())
        .collect();
    let visible: Vec<_> = live.iter().filter(|r| !held.contains(*r)).cloned().collect();
    if !held.is_empty() {
        let mut q = JobQueue::load();
        q.append(
            job_id,
            &format!("{} scenario(s) held out from the implementer", held.len()),
        );
        let _ = q.save();
    }
    let focus = job.rid.clone();
    let focus_name = features
        .iter()
        .flat_map(|f| &f.scenarios)
        .find(|s| s.rid.as_deref() == Some(focus.as_str()))
        .map(|s| s.name.clone())
        .unwrap_or_default();
    let mut job = job;
    pin_inner_loop(&mut job);
    let mut last_journal = String::new();
    for turn in 1..=max_turns {
        if let Err(e) = wait_if_parked(job_id) {
            return finish_err(job_id, &e);
        }
        let mut q = JobQueue::load();
        q.append(job_id, &format!("turn {turn}: running tests"));
        let _ = q.save();
        let run = run_suite(&root, &cfg);
        if run.harness_error {
            return finish_err(
                job_id,
                &format!("the test harness failed to run\n{}", harness_report(&run)),
            );
        }
        let before = led.clone();
        led.apply_run(&run.results, &format!("turn{turn}"), &run.collection_error);
        crate::mockups::promote_final_if_green(&root, &led);
        let _ = crate::journal::note_ledger(&root, &before, &led, &features);
        let _ = led.save(&root.join(".shalt/ledger.json"));
        let red_visible: Vec<_> = visible
            .iter()
            .filter(|r| {
                led.entries
                    .get(*r)
                    .map(|e| matches!(e.status.as_str(), RED | PENDING | STALE))
                    .unwrap_or(true)
            })
            .cloned()
            .collect();
        let red_this: Vec<_> = if focus.is_empty() {
            red_visible.clone()
        } else {
            red_visible.iter().filter(|r| *r == &focus).cloned().collect()
        };
        let mut q = JobQueue::load();
        if !focus.is_empty() {
            q.append(
                job_id,
                &format!(
                    "turn {turn}: ticket {focus} · {}/{} visible green",
                    visible.len() - red_visible.len(),
                    visible.len()
                ),
            );
        } else {
            q.append(
                job_id,
                &format!(
                    "turn {turn}: {}/{} visible green",
                    visible.len() - red_visible.len(),
                    visible.len()
                ),
            );
        }
        let _ = q.save();
        if red_this.is_empty() {
            break;
        }
        if turn >= 3 {
            let _ = escalate_writer(&mut job);
        }
        if let Err(e) = wait_if_parked(job_id) {
            return finish_err(job_id, &e);
        }
        let mut allow = HashSet::new();
        if !focus.is_empty() {
            allow.insert(focus.clone());
        }
        let mut dump = failure_digest(
            &run,
            if allow.is_empty() { None } else { Some(&allow) },
            8,
        );
        if !run.collection_error.is_empty() {
            dump = format!(
                "{dump}\n\nSUITE:\n{}",
                run.collection_error.chars().take(2000).collect::<String>()
            );
        }
        let mut prompt = crate::journal::with_pending(
            &root,
            &if !focus.is_empty() {
                format!(
                    "Make scenario {focus}{} pass. That ticket is this job. Other failing scenarios are other tickets — do not try to finish the whole spec in this turn. If the spec is wrong or incomplete, call ask_human; do not invent the missing behaviour.\n\nTEST OUTPUT:\n{dump}",
                    if focus_name.is_empty() {
                        String::new()
                    } else {
                        format!(" ({focus_name})")
                    }
                )
            } else {
                format!("Make the failing scenarios pass.\n\nTEST OUTPUT:\n{dump}")
            },
        );
        prompt.push_str(
            "\n\nYou are the FAST coder. Fill the stub the failing test needs. Do not redesign. Do not rewrite every file.",
        );
        let mut q = JobQueue::load();
        q.append(job_id, &format!("turn {turn}: writing code"));
        let _ = q.save();
        if let Some(latest) = JobQueue::load().get(job_id).cloned() {
            job = latest;
        }
        match run_role_or_failover(&job, &root, "implementer", &prompt, true) {
            Ok(res) => {
                let mut q = JobQueue::load();
                q.append(
                    job_id,
                    &format!("implementer wrote: {}", res.wrote.join(", ")),
                );
                let _ = q.save();
                if crate::journal::parse_dispatch(&res.transcript).is_some() {
                    last_journal = res.transcript;
                }
            }
            Err(RoleError::Integrity(e)) => {
                let mut q = JobQueue::load();
                q.append(
                    job_id,
                    &format!("turn {turn} REJECTED — {e} (nothing from this turn was kept)"),
                );
                let _ = q.save();
            }
            Err(e) => return finish_err(job_id, &e.to_string()),
        }
    }
    let run = run_suite(&root, &cfg);
    let before = led.clone();
    led.apply_run(&run.results, "final", &run.collection_error);
    crate::mockups::promote_final_if_green(&root, &led);
    let _ = crate::journal::note_ledger(&root, &before, &led, &features);
    let _ = led.save(&root.join(".shalt/ledger.json"));
    let vis_green = visible
        .iter()
        .all(|r| led.entries.get(r).map(|e| e.status == GREEN).unwrap_or(false));
    let overfit: Vec<_> = held
        .iter()
        .filter(|r| led.entries.get(*r).map(|e| e.status != GREEN).unwrap_or(false))
        .cloned()
        .collect();
    if vis_green && !overfit.is_empty() {
        let mut q = JobQueue::load();
        q.append(
            job_id,
            "OVERFIT: every visible scenario is green but held-out scenarios fail",
        );
        let _ = q.save();
    }
    let this_green = if focus.is_empty() {
        vis_green
    } else {
        led.entries
            .get(&focus)
            .map(|e| e.status == GREEN)
            .unwrap_or(false)
    };
    if this_green {
        match run_audit(
            &mut job,
            &root,
            crate::audit::Subject::Code,
            &if focus.is_empty() {
                String::new()
            } else {
                focus.clone()
            },
            "",
        ) {
            Err(e) => return finish_err(job_id, &e),
            Ok(None) | Ok(Some(crate::audit::Verdict { pass: true, .. })) => {}
            Ok(Some(v)) => {
                let mut prompt = crate::journal::with_pending(
                    &root,
                    &format!(
                        "Tests went green but the auditor rejected the code. Fix these findings. Do not hard-code expected outputs. Do not rewrite every file.\n{}",
                        v.findings
                    ),
                );
                prompt.push_str(
                    "\n\nYou are the FAST coder. Fill the stub the failing test needs. Do not redesign. Do not rewrite every file.",
                );
                if let Err(e) = wait_if_parked(job_id) {
                    return finish_err(job_id, &e);
                }
                match run_role_or_failover(&job, &root, "implementer", &prompt, true) {
                    Ok(res) => {
                        let mut q = JobQueue::load();
                        q.append(
                            job_id,
                            &format!("implementer wrote: {}", res.wrote.join(", ")),
                        );
                        let _ = q.save();
                    }
                    Err(RoleError::Integrity(e)) => {
                        return finish_err(job_id, &e.to_string());
                    }
                    Err(e) => return finish_err(job_id, &e.to_string()),
                }
                let run = run_suite(&root, &cfg);
                let before = led.clone();
                led.apply_run(&run.results, "audit-fix", &run.collection_error);
                crate::mockups::promote_final_if_green(&root, &led);
                let _ = crate::journal::note_ledger(&root, &before, &led, &features);
                let _ = led.save(&root.join(".shalt/ledger.json"));
                let still_green = if focus.is_empty() {
                    visible.iter().all(|r| {
                        led.entries.get(r).map(|e| e.status == GREEN).unwrap_or(false)
                    })
                } else {
                    led.entries
                        .get(&focus)
                        .map(|e| e.status == GREEN)
                        .unwrap_or(false)
                };
                if still_green {
                    match run_audit(
                        &mut job,
                        &root,
                        crate::audit::Subject::Code,
                        &focus,
                        "",
                    ) {
                        Err(e) => return finish_err(job_id, &e),
                        Ok(Some(v)) if !v.pass => {
                            return finish_err(
                                job_id,
                                &format!("auditor rejected the code:\n{}", v.findings),
                            );
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    let this_green = if focus.is_empty() {
        visible
            .iter()
            .all(|r| led.entries.get(r).map(|e| e.status == GREEN).unwrap_or(false))
    } else {
        led.entries
            .get(&focus)
            .map(|e| e.status == GREEN)
            .unwrap_or(false)
    };
    let vis_green = visible
        .iter()
        .all(|r| led.entries.get(r).map(|e| e.status == GREEN).unwrap_or(false));
    let summary = if this_green && vis_green {
        "visible scenarios are green".to_string()
    } else if this_green {
        format!("{focus} is green — next ticket until the spec passes")
    } else {
        "build finished with remaining red/pending work — play continues the next ticket".into()
    };
    finish_ok_with(job_id, &summary, &last_journal)
}

fn installed_local() -> Vec<String> {
    crate::api::list_models()
        .into_iter()
        .filter(|m| m.kind == "local" || m.backend == "qwen" || m.backend == "ollama")
        .map(|m| m.id)
        .collect()
}

fn set_job_lane(job: &mut Job, backend: &str, model: &str, line: &str) {
    if job.backend == backend && job.model == model {
        crate::api::keep_local_model(model);
        return;
    }
    let mut q = JobQueue::load();
    q.set_agent(&job.id, backend, model);
    q.append(&job.id, line);
    let _ = q.save();
    job.backend = backend.to_string();
    job.model = model.to_string();
    crate::api::keep_local_model(model);
}

/// Writers stay on the tiny local model. 27B is loaded only for an audit pass.
fn pin_inner_loop(job: &mut Job) {
    let installed = installed_local();
    let Some((backend, model)) = crate::alloc::pick_fast_model(&installed) else {
        crate::api::keep_local_model(&job.model);
        return;
    };
    if crate::alloc::is_write_model(&job.model) {
        crate::api::keep_local_model(&job.model);
        return;
    }
    set_job_lane(
        job,
        &backend,
        &model,
        &format!("lane · write · {model} · keep-alive"),
    );
}

fn auditor_model() -> String {
    crate::alloc::pick_audit_model(&installed_local())
        .map(|(_, m)| m)
        .unwrap_or_else(|| crate::api::DEFAULT_QWEN_MODEL.to_string())
}

/// On failure hop up the write chain. N is measured. No auditor sample yet:
/// walk the remaining writers once so a missing 27B sample cannot strand us on 1.7B.
fn continue_write_chain(job: &mut Job, fills: usize, secs: f64, completion: i64) -> bool {
    crate::speed::record_fill(&job.model, secs, completion);
    let auditor = auditor_model();
    let have_audit = crate::speed::sample(&auditor).is_some();
    if have_audit && !crate::speed::another_fill_fits(fills, secs, &job.model, &auditor) {
        return false;
    }
    if escalate_writer(job) {
        return true;
    }
    have_audit && crate::speed::another_fill_fits(fills, secs, &job.model, &auditor)
}

fn escalate_writer(job: &mut Job) -> bool {
    let installed = installed_local();
    let Some((backend, model)) = crate::alloc::pick_escalate_model(&installed, &job.model) else {
        return false;
    };
    set_job_lane(
        job,
        &backend,
        &model,
        &format!("lane · write · {model}"),
    );
    true
}

fn run_audit(
    job: &mut Job,
    root: &Path,
    subject: crate::audit::Subject,
    focus: &str,
    extra: &str,
) -> Result<Option<crate::audit::Verdict>, String> {
    let installed = installed_local();
    let Some((backend, model)) = crate::alloc::pick_audit_model(&installed) else {
        let mut q = JobQueue::load();
        q.append(&job.id, "lane · audit skipped — no local review model");
        let _ = q.save();
        return Ok(None);
    };
    let writer_b = job.backend.clone();
    let writer_m = job.model.clone();
    set_job_lane(job, &backend, &model, &format!("lane · audit · {model}"));
    if let Err(e) = wait_if_parked(&job.id) {
        set_job_lane(
            job,
            &writer_b,
            &writer_m,
            &format!("lane · write · {writer_m} · keep-alive"),
        );
        return Err(e);
    }
    let role = crate::audit::role_for(subject);
    let prompt = crate::audit::prompt(subject, focus, extra);
    let mut q = JobQueue::load();
    q.append(&job.id, &format!("audit · {role}"));
    let _ = q.save();
    let t0 = Instant::now();
    let result = match backend_for_job(job) {
        Ok(mut backend) => run_role(root, role, &prompt, &mut backend, false),
        Err(e) => {
            set_job_lane(
                job,
                &writer_b,
                &writer_m,
                &format!("lane · write · {writer_m} · keep-alive"),
            );
            return Err(e);
        }
    };
    let audit_secs = t0.elapsed().as_secs_f64();
    if let Ok(res) = &result {
        crate::speed::record_fill(&model, audit_secs, res.completion_tokens);
    } else if audit_secs > 0.0 {
        crate::speed::record(&model, 0.0, audit_secs);
    }
    set_job_lane(
        job,
        &writer_b,
        &writer_m,
        &format!("lane · write · {writer_m} · keep-alive"),
    );
    match result {
        Ok(res) => {
            let v = crate::audit::parse_verdict(&res.transcript);
            let mut q = JobQueue::load();
            if v.pass {
                q.append(&job.id, "audit · PASS");
            } else {
                let head = v.findings.lines().next().unwrap_or("findings");
                q.append(&job.id, &format!("audit · FAIL — {head}"));
            }
            let _ = q.save();
            Ok(Some(v))
        }
        Err(RoleError::Integrity(e)) => Ok(Some(crate::audit::Verdict {
            pass: false,
            findings: e.to_string(),
        })),
        Err(e) => {
            let msg = e.to_string();
            if crate::compose::looks_like_local_stall(&msg) {
                let mut q = JobQueue::load();
                q.append(
                    &job.id,
                    "audit · skipped — review model did not respond in time",
                );
                let _ = q.save();
                Ok(None)
            } else {
                Err(msg)
            }
        }
    }
}

fn execute_run(job_id: &str) -> Result<String, String> {
    let (_job, root) = match start_running(job_id) {
        Ok(v) => v,
        Err(e) => return finish_err(job_id, &e),
    };
    let cfg = Config::load(&root).unwrap_or_default();
    let mut led = Ledger::load(&root.join(".shalt/ledger.json")).unwrap_or_default();
    if let Ok(features) = load_specs(&root.join("spec"), false) {
        led.sync_spec(&features);
    }
    let mut q = JobQueue::load();
    q.append(job_id, "running the test suite");
    let _ = q.save();
    let run = run_suite(&root, &cfg);
    let blocked = if run.harness_error {
        harness_report(&run)
    } else {
        run.collection_error.clone()
    };
    let before = led.clone();
    led.apply_run(&run.results, &run.run_id, &blocked);
    crate::mockups::promote_final_if_green(&root, &led);
    if let Ok(features) = load_specs(&root.join("spec"), false) {
        let _ = crate::journal::note_ledger(&root, &before, &led, &features);
    }
    let _ = led.save(&root.join(".shalt/ledger.json"));
    let s = led.summary();
    let summary = if run.harness_error {
        format!(
            "survey: harness failed — shared dependency ({} red). Spec can still change; next Play builds or rewrites the spec.",
            s.get("red").and_then(|v| v.as_i64()).unwrap_or(0)
        )
    } else {
        format!(
            "survey · {} green · {} red · {} pending — then build tickets, or rewrite spec if the spec is wrong",
            s.get("green").and_then(|v| v.as_i64()).unwrap_or(0),
            s.get("red").and_then(|v| v.as_i64()).unwrap_or(0),
            s.get("pending").and_then(|v| v.as_i64()).unwrap_or(0)
        )
    };
    finish_ok(job_id, &summary)
}

fn pin_review(job: &mut Job) {
    let installed = installed_local();
    let Some((backend, model)) = crate::alloc::pick_audit_model(&installed) else {
        return;
    };
    set_job_lane(
        job,
        &backend,
        &model,
        &format!("lane · review · {model} · unencumbered"),
    );
}

fn execute_plan(job_id: &str) -> Result<String, String> {
    let (mut job, root) = match start_running(job_id) {
        Ok(v) => v,
        Err(e) => return finish_err(job_id, &e),
    };
    pin_review(&mut job);
    let retro_md = crate::sprint::last_retro_md(&root);
    if !retro_md.trim().is_empty() {
        let spec = crate::compose::spec_snapshot(&root);
        let prompt = crate::sprint::review_prompt(&retro_md, &spec);
        let mut q = JobQueue::load();
        q.append(&job.id, &format!("sprint review · {}", job.model));
        let _ = q.save();
        match backend_for_job(&job) {
            Ok(mut backend) => match backend.complete(
                "You review sprints. Bullet learnings only. No tools. No spec rewrites.",
                &[("user".into(), prompt)],
            ) {
                Ok(text) => {
                    let sid = retro_md
                        .lines()
                        .next()
                        .unwrap_or("")
                        .trim_start_matches("# ")
                        .split(' ')
                        .next()
                        .unwrap_or("C-1")
                        .to_string();
                    let sid = {
                        let board = Board::load(&root.join(".shalt/board.json"));
                        board
                            .sprints
                            .iter()
                            .rev()
                            .find(|s| s.closed_at.is_some())
                            .map(|s| s.id.clone())
                            .unwrap_or(sid)
                    };
                    let _ = crate::sprint::append_review(&root, &sid, &text);
                    let mut q = JobQueue::load();
                    q.append(&job.id, "sprint review · wrote 27B learnings");
                    let _ = q.save();
                }
                Err(e) => {
                    let mut q = JobQueue::load();
                    q.append(&job.id, &format!("sprint review skipped — {e}"));
                    let _ = q.save();
                }
            },
            Err(e) => {
                let mut q = JobQueue::load();
                q.append(&job.id, &format!("sprint review skipped — {e}"));
                let _ = q.save();
            }
        }
    }
    let board_path = root.join(".shalt/board.json");
    let mut board = Board::load(&board_path);
    let led = Ledger::load(&root.join(".shalt/ledger.json")).unwrap_or_default();
    let q = JobQueue::load();
    let last = board
        .sprints
        .iter()
        .rev()
        .find(|s| s.closed_at.is_some())
        .map(|s| s.id.clone());
    let Some(sid) = last else {
        return finish_err(job_id, "no closed sprint to plan from");
    };
    let Some(retro) = crate::tokens::retro(&board, &q.jobs, &job.project_id, &sid) else {
        return finish_err(job_id, "no retro");
    };
    let yolo = Org::yolo_plan(&job.project_id);
    let guests = if yolo {
        Vec::new()
    } else {
        let (question, guess) = crate::sprint::plan_question(&board, &led, &retro);
        let answered = job
            .turns
            .iter()
            .rev()
            .find(|t| !t.answer.trim().is_empty())
            .map(|t| t.answer.clone());
        if let Some(a) = answered {
            crate::sprint::parse_guests(&a)
        } else {
            let mut q = JobQueue::load();
            q.ask(&job.id, &question, &guess);
            q.append(&job.id, "waiting on you: sprint planning");
            let _ = q.save();
            return Err("waiting on you: sprint planning".into());
        }
    };
    match crate::sprint::apply_next_sprint(&root, &mut board, &led, &retro, &guests) {
        Ok(s) => {
            let _ = board.save(&board_path);
            let who = if guests.is_empty() {
                "shalt loop".into()
            } else {
                guests.join(", ")
            };
            finish_ok(
                job_id,
                &format!("planned {} · {} tickets seated · {}", s.title, crate::sprint::SPRINT_TICKET_CAP, who),
            )
        }
        Err(e) => finish_err(job_id, &e),
    }
}

fn execute_ux(job_id: &str) -> Result<String, String> {
    let (job, root) = match start_running(job_id) {
        Ok(v) => v,
        Err(e) => return finish_err(job_id, &e),
    };
    let features = load_specs(&root.join("spec"), false).unwrap_or_default();
    let q = JobQueue::load();
    let st = crate::ux::run_ux_pass(&root, &features, &q.jobs, &job.project_id);
    let summary = if st.pass {
        format!("UX pass · {} finding(s)", st.findings.len().saturating_sub(0))
    } else {
        format!("UX needs work · {}", st.findings.iter().take(3).cloned().collect::<Vec<_>>().join("; "))
    };
    if st.pass {
        finish_ok(job_id, &summary)
    } else {
        finish_ok(job_id, &summary)
    }
}

fn execute_verify(job_id: &str) -> Result<String, String> {
    let (_job, root) = match start_running(job_id) {
        Ok(v) => v,
        Err(e) => return finish_err(job_id, &e),
    };
    let led = Ledger::load(&root.join(".shalt/ledger.json")).unwrap_or_default();
    let features = load_specs(&root.join("spec"), false).unwrap_or_default();
    let mut problems = audit(&led, &features);
    let board = Board::load(&root.join(".shalt/board.json"));
    problems.extend(verify_drift(&board, &features));
    problems.extend(crate::mockups::verify_mockups(&root, &features));
    if problems.is_empty() {
        finish_ok(job_id, "verify: ok")
    } else {
        finish_err(job_id, &format!("verify failed: {}", problems.join("; ")))
    }
}

#[derive(Debug, Clone)]
pub struct PlayTick {
    pub job_id: String,
    pub kind: JobKind,
    pub summary: String,
}

pub enum PlayOutcome {
    Tick(PlayTick),
    Idle,
}

/// Why Play should stop instead of enqueueing the same failed stage again.
pub fn play_stop_reason(job: &Job) -> Option<String> {
    let blob = format!("{}\n{}", job.log, job.error).to_ascii_lowercase();
    if blob.contains("out of credits")
        || blob.contains("spending limit")
        || blob.contains("insufficient_quota")
        || blob.contains("insufficient_funds")
    {
        if crate::api::ollama_reachable() {
            // Next job will skip Grok and use local. Do not idle Play.
            return None;
        }
        return Some("Play stopped — the model is out of credits.".into());
    }
    if blob.contains("invalid api key")
        || blob.contains("incorrect api key")
        || blob.contains("authentication") && blob.contains("api")
    {
        return Some("Play stopped — the API key was refused.".into());
    }
    None
}

fn failure_key(job: &Job) -> String {
    let e = format!("{}\n{}", job.error, job.log).to_ascii_lowercase();
    if e.contains("does not support tools") {
        return "no-tools".into();
    }
    if e.contains("bound no new") {
        return "no-bind".into();
    }
    if e.contains("did not parse") {
        return "parse".into();
    }
    if e.contains("out of credits") || e.contains("spending limit") {
        return "credits".into();
    }
    e.chars().filter(|c| c.is_ascii_alphanumeric() || *c == ' ').take(80).collect()
}

/// Same stage, same failure, three times. Mixed errors (credits then no-bind) are not a spin.
fn spinning_failures(job: &Job) -> bool {
    let key = failure_key(job);
    if key.is_empty() {
        return false;
    }
    let q = JobQueue::load();
    let n = q
        .jobs
        .iter()
        .rev()
        .filter(|j| j.project_id == job.project_id && j.kind == job.kind)
        .filter(|j| j.status == JobStatus::Failed)
        .take(3)
        .filter(|j| failure_key(j) == key)
        .count();
    n >= 3
}

/// Desk Play stays on after this job. Failed builds used to idle the loop
/// while the suite was still red. Credits / key refusals, and three identical
/// failed stages in a row, stop the chain.
pub fn play_chains_after(job: &Job) -> bool {
    if !matches!(
        job.kind,
        JobKind::Author
            | JobKind::Design
            | JobKind::Steps
            | JobKind::Build
            | JobKind::Run
            | JobKind::Plan
            | JobKind::Ux
    ) {
        return false;
    }
    match job.status {
        JobStatus::Done => true,
        JobStatus::Failed => play_stop_reason(job).is_none() && !spinning_failures(job),
        _ => false,
    }
}

/// Unpause this project. Park others only when they hold a backend slot we need.
pub fn exclusive_unpause(project_id: &str) -> Result<(), String> {
    crate::parallel::claim_play(project_id)
}

/// Point the live job (and its ticket) at a different model. Play stays on:
/// the old worker drops, the next worker picks up the same job.
pub fn switch_play_model(
    project_id: &str,
    backend: &str,
    model: &str,
    job_id: Option<&str>,
    rid: Option<&str>,
) -> Result<Job, String> {
    let backend = backend.trim();
    let model = model.trim();
    if backend.is_empty() || model.is_empty() {
        return Err("pick an agent and a model".into());
    }
    if Org::load().get(project_id).is_none() {
        return Err(format!("no such project {project_id}"));
    }
    let mut q = JobQueue::load();
    let jid = job_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| {
            q.jobs
                .iter()
                .rev()
                .find(|j| {
                    j.project_id == project_id
                        && matches!(
                            j.status,
                            JobStatus::Running | JobStatus::Pending | JobStatus::Waiting
                        )
                })
                .map(|j| j.id.clone())
        })
        .ok_or_else(|| "no live job to switch".to_string())?;
    if !q.set_agent(&jid, backend, model) {
        return Err(format!("no job {jid}"));
    }
    if matches!(
        q.get(&jid).map(|j| j.status),
        Some(JobStatus::Paused | JobStatus::Interrupted | JobStatus::Failed)
    ) {
        q.set_status(&jid, JobStatus::Running);
    }
    {
        let mut org = Org::load();
        if org.pause(project_id, false, None) {
            let _ = org.save();
        }
    }
    let _ = crate::parallel::claim_play(project_id);
    let who = crate::tokens::agent_label(backend, model);
    q.append(
        &jid,
        &format!("switched to {who} — old call dropped, Play continues"),
    );
    let job = q
        .get(&jid)
        .cloned()
        .ok_or_else(|| format!("no job {jid}"))?;
    q.save().map_err(|e| e.to_string())?;
    let ticket = rid
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or(if job.rid.is_empty() {
            None
        } else {
            Some(job.rid.as_str())
        });
    if let Some(ticket) = ticket {
        if let Some(pref) = Org::load().get(project_id) {
            let board_path = PathBuf::from(&pref.path).join(".shalt/board.json");
            let mut board = Board::load(&board_path);
            if board.set_item_agent(ticket, backend, model) {
                let _ = board.save(&board_path);
            }
        }
    }
    Ok(job)
}

/// One stage of the thin loop: resume or enqueue, then execute.
pub fn play_step(project_id: &str) -> Result<PlayOutcome, String> {
    let mut q = JobQueue::load();
    let ids = q.resumable_for_project(project_id);
    let job = if let Some(id) = ids.into_iter().next() {
        q.set_status(&id, JobStatus::Running);
        q.append(&id, "playing — model is on this project");
        q.save().map_err(|e| e.to_string())?;
        q.get(&id).cloned().ok_or_else(|| format!("no job {id}"))?
    } else {
        match continue_project(project_id)? {
            Some(j) => j,
            None => return Ok(PlayOutcome::Idle),
        }
    };
    println!(
        "PLAY start stage={} job={}",
        kind_phase(job.kind),
        job.id
    );
    let kind = job.kind;
    let id = job.id.clone();
    let summary = match execute_job(&job.id) {
        Ok(s) => {
            println!("PLAY ok stage={} job={} {s}", kind_phase(kind), id);
            s
        }
        Err(e) => {
            println!("PLAY failed stage={} job={} {e}", kind_phase(kind), id);
            e
        }
    };
    Ok(PlayOutcome::Tick(PlayTick {
        job_id: id,
        kind,
        summary,
    }))
}

/// Spec → tests → run → code → run again, until idle. A failed stage still
/// continues if tickets remain.
pub fn play_loop(project_id: &str, max_steps: usize) -> Result<Vec<PlayTick>, String> {
    exclusive_unpause(project_id)?;
    let mut ticks = Vec::new();
    for _ in 0..max_steps.max(1) {
        match play_step(project_id)? {
            PlayOutcome::Idle => {
                println!("PLAY idle");
                return Ok(ticks);
            }
            PlayOutcome::Tick(t) => ticks.push(t),
        }
    }
    println!("PLAY done steps={}", ticks.len());
    Ok(ticks)
}
