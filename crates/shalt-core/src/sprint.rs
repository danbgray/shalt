//! Small sprints: cap the slice, write a retro, plan the next one from learnings.

use crate::board::{Board, BoardItem, Sprint, SprintRetro};
use crate::jobs::{Job, JobKind, JobQueue};
use crate::ledger::{Ledger, GREEN, ORPHAN};
use crate::tokens::{self, job_wall_secs};
use std::fs;
use std::path::Path;

pub const SPRINT_TICKET_CAP: usize = 5;
pub const SPRINT_TOKEN_CAP: i64 = 400_000;

pub fn token_cap(board: &Board) -> i64 {
    board
        .sprints
        .iter()
        .rev()
        .find(|s| s.closed_at.is_some() && s.spent > 0)
        .map(|s| s.spent.clamp(80_000, 800_000))
        .unwrap_or(SPRINT_TOKEN_CAP)
}

fn is_open(it: &BoardItem, ledger: &Ledger) -> bool {
    let st = ledger
        .entries
        .get(&it.rid)
        .map(|e| e.status.as_str())
        .unwrap_or("");
    st != GREEN && st != ORPHAN
}

/// Unfinished tickets on this sprint, rank order.
pub fn sprint_open_tickets<'a>(
    board: &'a Board,
    ledger: &Ledger,
    sid: &str,
) -> Vec<&'a BoardItem> {
    let mut v: Vec<_> = board
        .items
        .iter()
        .filter(|it| it.sprint_id.as_deref() == Some(sid) && is_open(it, ledger))
        .collect();
    v.sort_by_key(|it| it.rank);
    v
}

pub fn sprint_complete(board: &Board, ledger: &Ledger) -> bool {
    let Some(s) = board.active_sprint() else {
        return false;
    };
    let seated = board
        .items
        .iter()
        .filter(|it| it.sprint_id.as_deref() == Some(s.id.as_str()))
        .count();
    seated > 0 && sprint_open_tickets(board, ledger, &s.id).is_empty()
}

pub fn backlog<'a>(board: &'a Board, ledger: &Ledger) -> Vec<&'a BoardItem> {
    let mut v: Vec<_> = board
        .items
        .iter()
        .filter(|it| it.sprint_id.is_none() && is_open(it, ledger))
        .collect();
    v.sort_by_key(|it| it.rank);
    v
}

/// Seat a small slice onto the open sprint. Does not dump the whole backlog.
pub fn seat_sprint_slice(board: &mut Board, ledger: &Ledger) -> usize {
    let cap_n = SPRINT_TICKET_CAP;
    let cap_tok = token_cap(board);
    let Some(sid) = board.ensure_open_sprint() else {
        return 0;
    };
    let already: usize = board
        .items
        .iter()
        .filter(|it| it.sprint_id.as_deref() == Some(sid.as_str()) && is_open(it, ledger))
        .count();
    if already >= cap_n {
        return 0;
    }
    let mut used_tok: i64 = board
        .items
        .iter()
        .filter(|it| it.sprint_id.as_deref() == Some(sid.as_str()) && is_open(it, ledger))
        .map(|it| it.token_estimate.max(0))
        .sum();
    let mut seated = already;
    let mut n = 0;
    let mut picks: Vec<String> = Vec::new();
    for it in board.items.iter() {
        if it.sprint_id.is_some() || !is_open(it, ledger) {
            continue;
        }
        if seated >= cap_n {
            break;
        }
        if it.token_estimate > 0 && used_tok + it.token_estimate > cap_tok && seated > 0 {
            break;
        }
        picks.push(it.rid.clone());
        used_tok += it.token_estimate.max(0);
        seated += 1;
    }
    for rid in picks {
        if let Some(it) = board.items.iter_mut().find(|i| i.rid == rid) {
            it.sprint_id = Some(sid.clone());
            n += 1;
        }
    }
    n
}

/// Unfinished tickets on a closing sprint go back to the backlog for the next plan.
pub fn return_unfinished(board: &mut Board, ledger: &Ledger, sid: &str) -> usize {
    let mut n = 0;
    for it in &mut board.items {
        if it.sprint_id.as_deref() == Some(sid) && is_open(it, ledger) {
            it.sprint_id = None;
            n += 1;
        }
    }
    n
}

pub fn retro_dir(root: &Path) -> std::path::PathBuf {
    root.join(".shalt/retro")
}

pub fn write_retro(
    root: &Path,
    board: &Board,
    jobs: &[Job],
    ledger: &Ledger,
    project_id: &str,
    sid: &str,
) -> Result<String, String> {
    let r = tokens::retro(board, jobs, project_id, sid).ok_or_else(|| "no sprint".to_string())?;
    let secs_est: i64 = board
        .items
        .iter()
        .filter(|it| it.sprint_id.as_deref() == Some(sid))
        .map(|it| it.time_estimate_secs)
        .sum();
    let secs_spent: i64 = jobs
        .iter()
        .filter(|j| j.project_id == project_id && j.sprint_id == sid)
        .map(job_wall_secs)
        .sum();
    let acc = r
        .accuracy
        .map(|a| format!("{a:.1}×"))
        .unwrap_or_else(|| "—".into());
    let time_acc = if secs_est > 0 {
        format!("{:.1}×", secs_spent as f64 / secs_est as f64)
    } else {
        "—".into()
    };
    let learn = learnings(&r, secs_est, secs_spent);
    let next_n = SPRINT_TICKET_CAP;
    let next_tok = token_cap_from_retro(&r);
    let md = format!(
        "# {} retro\n\nClosed: {}\nTickets: {}\nTokens: {} spent / {} forecast ({acc})\nTime: {} / {} ({time_acc})\n\n## Learnings\n{}\n\n## Next sprint\nCapacity: {} tokens, {next_n} tickets.\nScale the next forecasts by {acc} (tokens) and {time_acc} (time).\n",
        r.title,
        if r.closed { "yes" } else { "closing" },
        r.tickets,
        r.spent,
        r.estimated,
        secs_spent,
        secs_est,
        learn,
        next_tok,
    );
    let dir = retro_dir(root);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{sid}.md"));
    fs::write(&path, &md).map_err(|e| e.to_string())?;
    let _ = ledger;
    Ok(md)
}

fn token_cap_from_retro(r: &SprintRetro) -> i64 {
    if r.spent > 0 {
        r.spent.clamp(80_000, 800_000)
    } else {
        SPRINT_TOKEN_CAP
    }
}

fn learnings(r: &SprintRetro, secs_est: i64, secs_spent: i64) -> String {
    let mut lines = Vec::new();
    if let Some(a) = r.accuracy {
        if a > 1.3 {
            lines.push(format!(
                "- Tokens ran hot ({a:.1}×). Next sprint forecasts go up, and slices stay at {} tickets.",
                SPRINT_TICKET_CAP
            ));
        } else if a < 0.7 {
            lines.push(format!(
                "- Tokens came in under ({a:.1}×). Next forecasts come down; still cap the sprint at {} tickets so work stays small.",
                SPRINT_TICKET_CAP
            ));
        } else {
            lines.push(format!(
                "- Token forecast was close ({a:.1}×). Keep the {}-ticket slice.",
                SPRINT_TICKET_CAP
            ));
        }
    }
    if secs_est > 0 {
        let t = secs_spent as f64 / secs_est as f64;
        if t > 1.5 {
            lines.push("- Time ran long vs forecast. Prefer a faster local model for mechanical tickets; keep the large model for spec and gnarly tests.".into());
        } else if t < 0.6 {
            lines.push("- Time was quicker than forecast. The next sprint can take the same ticket count.".into());
        }
    }
    if lines.is_empty() {
        lines.push("- First sprint. Next one stays small so we get another retro soon.".into());
    }
    lines.join("\n")
}

pub fn review_prompt(retro_md: &str, spec: &str) -> String {
    format!(
        "You are reviewing a shalt sprint. Small models wrote tests and code; you are the large local reviewer (27B).\n\
         Do not rewrite the spec. Do not invent tickets. Write 8–12 short bullets:\n\
         what the slice got right, where the small models were sloppy (stubs, hard-coded answers, missed binds),\n\
         and what the next 5-ticket slice must not repeat.\n\n\
         RETRO NUMBERS:\n{}\n\nSPEC (trim):\n{}\n",
        retro_md.chars().take(2500).collect::<String>(),
        spec.chars().take(2500).collect::<String>()
    )
}

pub fn append_review(root: &Path, sid: &str, review: &str) -> Result<(), String> {
    let path = retro_dir(root).join(format!("{sid}.md"));
    let mut md = fs::read_to_string(&path).unwrap_or_default();
    if !md.contains("## 27B review") {
        md.push_str("\n\n## 27B review\n");
        md.push_str(review.trim());
        md.push('\n');
        fs::write(&path, md).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn last_retro_md(root: &Path) -> String {
    let dir = retro_dir(root);
    let mut files: Vec<_> = fs::read_dir(&dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("md"))
        .collect();
    files.sort();
    files
        .last()
        .and_then(|p| fs::read_to_string(p).ok())
        .unwrap_or_default()
}

pub fn sprint_brief(root: &Path) -> String {
    let retro = last_retro_md(root);
    let plan = fs::read_to_string(root.join(".shalt/sprint-plan.md")).unwrap_or_default();
    let mut bits = Vec::new();
    if !retro.trim().is_empty() {
        bits.push(format!(
            "LAST SPRINT RETRO (honor this; do not re-litigate):\n{}",
            retro.chars().take(1800).collect::<String>()
        ));
    }
    if !plan.trim().is_empty() {
        bits.push(format!(
            "THIS SPRINT PLAN:\n{}",
            plan.chars().take(1200).collect::<String>()
        ));
    }
    bits.join("\n\n")
}

pub fn plan_question(board: &Board, ledger: &Ledger, retro: &SprintRetro) -> (String, String) {
    let next: Vec<String> = backlog(board, ledger)
        .into_iter()
        .take(SPRINT_TICKET_CAP)
        .map(|it| {
            if it.rid.is_empty() {
                it.rid.clone()
            } else {
                format!("{} (~{} tok)", it.rid, it.token_estimate)
            }
        })
        .collect();
    let list = if next.is_empty() {
        "(backlog is empty)".into()
    } else {
        next.join(", ")
    };
    let acc = retro
        .accuracy
        .map(|a| format!("{a:.1}×"))
        .unwrap_or_else(|| "—".into());
    let q = format!(
        "{} closed at {acc} forecast ({} spent / {} estimated). Next sprint is {} tickets: {list}. Invite anyone to planning? List names, or skip.",
        retro.title, retro.spent, retro.estimated, SPRINT_TICKET_CAP
    );
    let guess = if next.is_empty() {
        "skip — no backlog".into()
    } else {
        format!("skip — take the proposed slice of {}", SPRINT_TICKET_CAP)
    };
    (q, guess)
}

pub fn parse_guests(answer: &str) -> Vec<String> {
    let t = answer.trim();
    if t.is_empty() {
        return Vec::new();
    }
    let lower = t.to_ascii_lowercase();
    if lower == "skip"
        || lower.starts_with("skip")
        || lower.starts_with("no")
        || lower.contains("no one")
        || lower.contains("nobody")
    {
        return Vec::new();
    }
    t.split(|c| c == ',' || c == ';' || c == '\n')
        .map(|s| s.trim().trim_matches('.').to_string())
        .filter(|s| !s.is_empty() && s.len() < 80 && !s.eq_ignore_ascii_case("skip"))
        .take(8)
        .collect()
}

/// Open the next sprint, seat a small slice, write the plan. Uses last retro for capacity.
pub fn apply_next_sprint(
    root: &Path,
    board: &mut Board,
    ledger: &Ledger,
    retro: &SprintRetro,
    guests: &[String],
) -> Result<Sprint, String> {
    let n = board.sprints.len() + 1;
    let title = format!("Sprint {n}");
    let s = board.open_sprint(&title);
    let _ = seat_sprint_slice(board, ledger);
    if let Some(open) = board.sprints.iter_mut().find(|x| x.id == s.id) {
        open.notes = format!(
            "From {}: tokens {} spent / {} forecast. Slice {} tickets.",
            retro.title, retro.spent, retro.estimated, SPRINT_TICKET_CAP
        );
        open.guests = guests.to_vec();
    }
    let seated: Vec<_> = board
        .items
        .iter()
        .filter(|it| it.sprint_id.as_deref() == Some(s.id.as_str()))
        .map(|it| it.rid.as_str())
        .collect();
    let guest_line = if guests.is_empty() {
        "Planning was the shalt loop (no extra humans).".into()
    } else {
        format!("Invited: {}.", guests.join(", "))
    };
    let plan = format!(
        "# {}\n\n{}\n\nTickets: {}\n\nLearnings from {} applied: next forecasts scale by {}.\n",
        s.title,
        guest_line,
        if seated.is_empty() {
            "(none — backlog empty)".into()
        } else {
            seated.join(", ")
        },
        retro.title,
        retro
            .accuracy
            .map(|a| format!("{a:.1}×"))
            .unwrap_or_else(|| "1.0×".into())
    );
    fs::create_dir_all(root.join(".shalt")).map_err(|e| e.to_string())?;
    fs::write(root.join(".shalt/sprint-plan.md"), plan).map_err(|e| e.to_string())?;
    Ok(board
        .sprints
        .iter()
        .find(|x| x.id == s.id)
        .cloned()
        .unwrap_or(s))
}

/// Close the open sprint, write the retro, optionally plan the next one.
pub fn close_and_learn(
    root: &Path,
    board: &mut Board,
    jobs: &[Job],
    ledger: &Ledger,
    project_id: &str,
) -> Result<SprintRetro, String> {
    let sid = board
        .active_sprint()
        .map(|s| s.id.clone())
        .ok_or_else(|| "no open sprint".to_string())?;
    let r = tokens::retro(board, jobs, project_id, &sid)
        .ok_or_else(|| "no sprint".to_string())?;
    board.close_sprint(&sid, r.estimated, r.spent);
    let _ = return_unfinished(board, ledger, &sid);
    let md = write_retro(root, board, jobs, ledger, project_id, &sid)?;
    if let Some(s) = board.sprints.iter_mut().find(|s| s.id == sid) {
        s.notes = md.lines().take(8).collect::<Vec<_>>().join("\n");
    }
    tokens::retro(board, jobs, project_id, &sid).ok_or_else(|| "no sprint".into())
}

pub fn cadence_due(board: &Board, ledger: &Ledger) -> bool {
    sprint_complete(board, ledger) && !backlog(board, ledger).is_empty()
}

pub fn maybe_enqueue_plan(
    project_id: &str,
    root: &Path,
    board: &mut Board,
    jobs: &[Job],
    ledger: &Ledger,
    _yolo: bool,
) -> Result<Option<Job>, String> {
    if !cadence_due(board, ledger) {
        return Ok(None);
    }
    let retro = close_and_learn(root, board, jobs, ledger, project_id)?;
    // Always a Plan job so 27B reviews the sprint. Yolo only skips the invite.
    let mut q = JobQueue::load();
    let job = q.enqueue_full(
        JobKind::Plan,
        project_id,
        &format!("Plan the next sprint from {}.", retro.title),
        "",
        "",
    );
    q.append(
        &job.id,
        &format!("sprint closed · {} · planning the next slice", retro.title),
    );
    q.save().map_err(|e| e.to_string())?;
    Ok(q.get(&job.id).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::Entry;

    fn entry(rid: &str, status: &str) -> (String, Entry) {
        let e = serde_json::from_value(serde_json::json!({
            "rid": rid,
            "name": rid,
            "feature": "F",
            "feature_file": "f.feature",
            "status": status
        }))
        .unwrap();
        (rid.into(), e)
    }

    #[test]
    fn review_prompt_asks_for_bullets() {
        let p = review_prompt("# C-1 retro\nTickets: 5\n", "Feature: Share");
        assert!(p.contains("27B"));
        assert!(p.contains("8–12") || p.contains("8-12") || p.contains("bullets"));
    }

    #[test]
    fn slice_caps_the_open_sprint() {
        let mut board = Board::default();
        for i in 1..=12 {
            board.items.push(BoardItem {
                rid: format!("S-{i}"),
                rank: i,
                token_estimate: 50_000,
                ..Default::default()
            });
        }
        let led = Ledger::default();
        let n = seat_sprint_slice(&mut board, &led);
        assert_eq!(n, SPRINT_TICKET_CAP);
        let sid = board.active_sprint().unwrap().id.clone();
        let on = board
            .items
            .iter()
            .filter(|i| i.sprint_id.as_deref() == Some(sid.as_str()))
            .count();
        assert_eq!(on, SPRINT_TICKET_CAP);
        assert_eq!(backlog(&board, &led).len(), 12 - SPRINT_TICKET_CAP);
    }

    #[test]
    fn complete_sprint_returns_nothing_when_seated_work_is_green() {
        let mut board = Board::default();
        board.open_sprint("Sprint 1");
        let sid = board.active_sprint().unwrap().id.clone();
        board.items.push(BoardItem {
            rid: "S-1".into(),
            rank: 1,
            sprint_id: Some(sid.clone()),
            ..Default::default()
        });
        board.items.push(BoardItem {
            rid: "S-2".into(),
            rank: 2,
            ..Default::default()
        });
        let mut led = Ledger::default();
        led.entries.insert(entry("S-1", GREEN).0, entry("S-1", GREEN).1);
        assert!(sprint_complete(&board, &led));
        assert_eq!(backlog(&board, &led).len(), 1);
    }

    #[test]
    fn planning_invite_skip_is_empty_and_names_parse() {
        assert!(parse_guests("skip").is_empty());
        assert!(parse_guests("skip — take the proposed slice").is_empty());
        assert_eq!(parse_guests("Ada, Linus").len(), 2);
    }
}
