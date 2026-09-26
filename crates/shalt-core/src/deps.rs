//! What can start now vs what waits. Built from the survey run and spec order.

use crate::board::Board;
use crate::jobs::{Job, JobStatus};
use crate::ledger::{Ledger, GREEN, ORPHAN};
use crate::tokens::{epic_name, ticket_name, ticket_status};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

#[derive(Debug, Clone, Serialize)]
pub struct TicketNode {
    pub rid: String,
    pub name: String,
    pub epic: String,
    pub file: String,
    pub status: String,
    pub blocked_by: Vec<String>,
    pub why: String,
    pub ready: bool,
    pub wave: u32,
    #[serde(default)]
    pub running: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct WorkMap {
    pub ready: Vec<TicketNode>,
    pub blocked: Vec<TicketNode>,
    pub waves: Vec<Vec<String>>,
    pub note: String,
}

impl WorkMap {
    pub fn is_ready(&self, rid: &str) -> bool {
        if self.ready.iter().any(|n| n.rid == rid) {
            return true;
        }
        if self.blocked.iter().any(|n| n.rid == rid) {
            return false;
        }
        true
    }
}

fn fail_fp(s: &str) -> String {
    let s = s.to_lowercase();
    let line = s.lines().next().unwrap_or("").trim();
    line.chars().filter(|c| !c.is_control()).take(96).collect()
}

fn surveyed(e: &crate::ledger::Entry) -> bool {
    e.last_run_at.is_some()
}

/// Map unfinished tickets into parallel waves.
/// Shared identical failures wait on the lowest-rank ticket in that cluster.
/// After a survey, later scenarios in the same feature wait on earlier unfinished ones.
pub fn work_map(
    board: &Board,
    ledger: &Ledger,
    jobs: &[Job],
    project_id: &str,
    sprint_id: Option<&str>,
) -> WorkMap {
    let busy: HashSet<&str> = jobs
        .iter()
        .filter(|j| {
            j.project_id == project_id
                && !j.rid.is_empty()
                && matches!(
                    j.status,
                    JobStatus::Running | JobStatus::Pending | JobStatus::Waiting
                )
        })
        .map(|j| j.rid.as_str())
        .collect();
    let mut items: Vec<&crate::board::BoardItem> = board.items.iter().collect();
    items.sort_by_key(|i| (i.rank, i.rid.as_str()));
    if let Some(sid) = sprint_id {
        let in_sprint: Vec<_> = items
            .iter()
            .copied()
            .filter(|i| i.sprint_id.as_deref() == Some(sid))
            .collect();
        if !in_sprint.is_empty() {
            items = in_sprint;
        }
    }
    let mut nodes: Vec<TicketNode> = Vec::new();
    for it in &items {
        let status = ticket_status(ledger, &it.rid);
        if status == GREEN || status == ORPHAN {
            continue;
        }
        let e = ledger.entries.get(&it.rid);
        nodes.push(TicketNode {
            rid: it.rid.clone(),
            name: ticket_name(ledger, &it.rid),
            epic: epic_name(ledger, &it.rid),
            file: e.map(|e| e.feature_file.clone()).unwrap_or_default(),
            status,
            blocked_by: Vec::new(),
            why: String::new(),
            ready: true,
            wave: 0,
            running: busy.contains(it.rid.as_str()),
        });
    }

    let mut blockers: HashMap<String, (Vec<String>, String)> = HashMap::new();

    // Shared failure cluster: one ticket unblocks the rest.
    let any_survey = nodes.iter().any(|n| {
        ledger
            .entries
            .get(&n.rid)
            .map(surveyed)
            .unwrap_or(false)
    });
    if any_survey {
        let mut clusters: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for n in &nodes {
            let Some(e) = ledger.entries.get(&n.rid) else {
                continue;
            };
            if !surveyed(e) {
                continue;
            }
            let fp = fail_fp(e.failure.as_deref().unwrap_or(""));
            if fp.len() < 12 || fp.contains("no test bound") {
                continue;
            }
            clusters.entry(fp).or_default().push(n.rid.clone());
        }
        for (fp, rids) in clusters {
            if rids.len() < 2 {
                continue;
            }
            let head = rids[0].clone();
            for rid in rids.iter().skip(1) {
                let why = format!(
                    "same failure as {} ({})",
                    ticket_name(ledger, &head),
                    fp.chars().take(48).collect::<String>()
                );
                blockers.insert(rid.clone(), (vec![head.clone()], why));
            }
        }

        // Same feature file: later unfinished waits on the previous unfinished in that file.
        let mut by_file: BTreeMap<(String, usize), String> = BTreeMap::new();
        for n in &nodes {
            let line = ledger.entries.get(&n.rid).map(|e| e.line).unwrap_or(0);
            let file = if n.file.is_empty() {
                "_".into()
            } else {
                n.file.clone()
            };
            by_file.insert((file, line), n.rid.clone());
        }
        let mut prev: HashMap<String, String> = HashMap::new();
        for ((file, _), rid) in &by_file {
            if let Some(p) = prev.get(file) {
                if !blockers.contains_key(rid) {
                    let why = format!(
                        "earlier scenario in {file} ({})",
                        ticket_name(ledger, p)
                    );
                    blockers.insert(rid.clone(), (vec![p.clone()], why));
                }
            }
            prev.insert(file.clone(), rid.clone());
        }
    }

    for n in &mut nodes {
        if let Some((bys, why)) = blockers.get(&n.rid) {
            n.blocked_by = bys.clone();
            n.why = why.clone();
        }
    }

    // Waves: Kahn
    let mut remaining: BTreeSet<String> = nodes.iter().map(|n| n.rid.clone()).collect();
    let mut waves: Vec<Vec<String>> = Vec::new();
    let mut wave_of: HashMap<String, u32> = HashMap::new();
    let mut guard = 0;
    while !remaining.is_empty() && guard < 64 {
        guard += 1;
        let mut wave = Vec::new();
        for n in &nodes {
            if !remaining.contains(&n.rid) {
                continue;
            }
            let blocked = n.blocked_by.iter().any(|b| remaining.contains(b));
            if !blocked {
                wave.push(n.rid.clone());
            }
        }
        if wave.is_empty() {
            // cycle: dump the rest
            wave.extend(remaining.iter().cloned());
        }
        for rid in &wave {
            remaining.remove(rid);
            wave_of.insert(rid.clone(), waves.len() as u32);
        }
        waves.push(wave);
    }
    for n in &mut nodes {
        n.wave = *wave_of.get(&n.rid).unwrap_or(&0);
        n.ready = n.wave == 0;
    }
    let ready: Vec<_> = nodes.iter().filter(|n| n.ready).cloned().collect();
    let blocked: Vec<_> = nodes.iter().filter(|n| !n.ready).cloned().collect();
    let note = if !any_survey {
        format!(
            "{} ticket(s) ready to start. Run the suite first to see what depends on what.",
            ready.len()
        )
    } else if blocked.is_empty() {
        format!(
            "{} ticket(s) independent — fan out across Grok / Claude / Codex.",
            ready.len()
        )
    } else {
        format!(
            "Wave 1: {} ready now. {} wait on shared failures or earlier scenarios.",
            ready.len(),
            blocked.len()
        )
    };
    WorkMap {
        ready,
        blocked,
        waves,
        note,
    }
}
