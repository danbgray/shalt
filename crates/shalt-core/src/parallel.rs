//! Overlap and backend capacity.
//!
//! Overlap is **epoch** (a shared workspace of work), not project.
//! Spec is one epoch. Tests are one epoch. Then each ticket is a build epoch
//! until the spec passes. Local (Qwen) holds 1–2 GPU slots. Cloud (Grok) holds
//! N slots, measured from finished jobs.

use crate::alloc::is_local;
use crate::jobs::{epoch_of, Job, JobQueue, JobStatus};
use crate::org::Org;
use crate::tokens::job_secs;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

pub const SCHEMA: &str = "shalt.parallel/1";
pub const LOCAL_MAX: u32 = 2;
pub const CLOUD_MAX: u32 = 8;
pub const DEFAULT_LOCAL: u32 = 1;
pub const DEFAULT_CLOUD: u32 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    Local,
    Cloud,
}

impl SlotKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SlotKind::Local => "local",
            SlotKind::Cloud => "cloud",
        }
    }
}

pub fn slot_kind(backend: &str) -> SlotKind {
    if is_local(backend) {
        SlotKind::Local
    } else {
        SlotKind::Cloud
    }
}

/// Two jobs collide if they share an epoch (the same workspace of work).
pub fn jobs_overlap(a: &Job, b: &Job) -> bool {
    if a.id == b.id {
        return false;
    }
    let ea = epoch_of(a);
    let eb = epoch_of(b);
    !ea.is_empty() && ea == eb
}

pub fn holds_slot(j: &Job) -> bool {
    matches!(j.status, JobStatus::Running | JobStatus::Pending)
}

pub fn used_slots(jobs: &[Job], kind: SlotKind) -> u32 {
    jobs.iter()
        .filter(|j| holds_slot(j) && slot_kind(&j.backend) == kind)
        .count() as u32
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Admit {
    Yes,
    Overlap { epoch: String },
    Full { kind: SlotKind, used: u32, cap: u32 },
}

impl Admit {
    pub fn ok(&self) -> bool {
        matches!(self, Admit::Yes)
    }
}

/// Whether `job` may start given the other live work and the measured caps.
pub fn can_admit(job: &Job, jobs: &[Job], cap: &Capacity) -> Admit {
    if jobs
        .iter()
        .any(|o| holds_slot(o) && jobs_overlap(job, o))
    {
        return Admit::Overlap {
            epoch: epoch_of(job),
        };
    }
    let kind = slot_kind(&job.backend);
    let used = used_slots(jobs, kind);
    let already = jobs.iter().any(|j| j.id == job.id && holds_slot(j));
    let used_others = if already { used.saturating_sub(1) } else { used };
    let n = cap.get(kind);
    if used_others >= n {
        Admit::Full {
            kind,
            used: used_others,
            cap: n,
        }
    } else {
        Admit::Yes
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capacity {
    #[serde(default = "schema")]
    pub schema: String,
    #[serde(default = "default_local")]
    pub local_cap: u32,
    #[serde(default = "default_cloud")]
    pub cloud_cap: u32,
    #[serde(default)]
    pub samples: Vec<Sample>,
}

fn schema() -> String {
    SCHEMA.into()
}
fn default_local() -> u32 {
    DEFAULT_LOCAL
}
fn default_cloud() -> u32 {
    DEFAULT_CLOUD
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sample {
    pub kind: String,
    pub backend: String,
    pub concurrent: u32,
    pub secs: i64,
    pub tokens: i64,
    #[serde(default)]
    pub at: String,
}

impl Default for Capacity {
    fn default() -> Self {
        Self {
            schema: SCHEMA.into(),
            local_cap: DEFAULT_LOCAL,
            cloud_cap: DEFAULT_CLOUD,
            samples: Vec::new(),
        }
    }
}

impl Capacity {
    pub fn path() -> PathBuf {
        Org::home_dir().join("parallel.json")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &std::path::Path) -> Self {
        if !path.exists() {
            return Self::default();
        }
        serde_json::from_str(&fs::read_to_string(path).unwrap_or_default()).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(p) = path.parent() {
            fs::create_dir_all(p)?;
        }
        fs::write(path, serde_json::to_string_pretty(self)? + "\n")
    }

    pub fn get(&self, kind: SlotKind) -> u32 {
        match kind {
            SlotKind::Local => self.local_cap.clamp(1, LOCAL_MAX),
            SlotKind::Cloud => self.cloud_cap.clamp(1, CLOUD_MAX),
        }
    }

    pub fn record(&mut self, job: &Job) {
        let secs = job_secs(job);
        if secs < 5 {
            return;
        }
        let concurrent = job.concurrent.max(1);
        self.samples.push(Sample {
            kind: slot_kind(&job.backend).as_str().into(),
            backend: if job.backend.is_empty() {
                "qwen".into()
            } else {
                job.backend.clone()
            },
            concurrent,
            secs,
            tokens: job.prompt_tokens + job.completion_tokens,
            at: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        });
        if self.samples.len() > 80 {
            let extra = self.samples.len() - 80;
            self.samples.drain(0..extra);
        }
        self.retune();
    }

    /// Throughput ≈ concurrent / avg_secs. Raise cloud N while that still climbs;
    /// local stays 1 unless two-at-once is not much slower than one.
    pub fn retune(&mut self) {
        self.local_cap = tune_local(&self.samples);
        self.cloud_cap = tune_cloud(&self.samples);
    }
}

fn avg_at(samples: &[Sample], kind: &str, n: u32) -> Option<f64> {
    let hit: Vec<i64> = samples
        .iter()
        .filter(|s| s.kind == kind && s.concurrent == n && s.secs > 0)
        .map(|s| s.secs)
        .collect();
    if hit.len() < 3 {
        return None;
    }
    Some(hit.iter().sum::<i64>() as f64 / hit.len() as f64)
}

fn tune_local(samples: &[Sample]) -> u32 {
    match (avg_at(samples, "local", 1), avg_at(samples, "local", 2)) {
        (Some(one), Some(two)) if two <= one * 1.8 => 2,
        (Some(_), Some(_)) => 1,
        _ => DEFAULT_LOCAL,
    }
}

fn tune_cloud(samples: &[Sample]) -> u32 {
    let one = match avg_at(samples, "cloud", 1) {
        Some(v) => v,
        None => return DEFAULT_CLOUD,
    };
    let mut best = 1u32;
    let mut best_tp = 1.0 / one;
    for n in 2..=CLOUD_MAX {
        let Some(avg) = avg_at(samples, "cloud", n) else {
            break;
        };
        if avg > one * 2.2 {
            break;
        }
        let tp = n as f64 / avg;
        if tp >= best_tp * 0.85 {
            best = n;
            if tp > best_tp {
                best_tp = tp;
            }
        } else {
            break;
        }
    }
    best.clamp(CLOUD_MIN_HINT, CLOUD_MAX)
}

const CLOUD_MIN_HINT: u32 = 1;

pub fn mark_start(q: &mut JobQueue, job_id: &str) {
    let Some(j) = q.get(job_id) else {
        return;
    };
    let kind = slot_kind(&j.backend);
    let n = used_slots(&q.jobs, kind).max(1);
    q.set_concurrent(job_id, n);
}

pub fn mark_finish(job: &Job) {
    if !matches!(job.status, JobStatus::Done) {
        return;
    }
    let mut cap = Capacity::load();
    cap.record(job);
    let _ = cap.save();
}

pub fn intended_backend(project_id: &str, jobs: &[Job]) -> String {
    jobs.iter()
        .rev()
        .find(|j| {
            j.project_id == project_id
                && (!j.backend.is_empty())
                && matches!(
                    j.status,
                    JobStatus::Running
                        | JobStatus::Pending
                        | JobStatus::Paused
                        | JobStatus::Waiting
                        | JobStatus::Interrupted
                )
        })
        .map(|j| j.backend.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "qwen".into())
}

/// Unpause this project. Park others only when they hold a slot we need.
pub fn claim_play(project_id: &str) -> Result<(), String> {
    let mut org = Org::load();
    if org.get(project_id).is_none() {
        return Err(format!("unknown project {project_id}"));
    }
    let taker = org
        .get(project_id)
        .map(|p| p.name.clone())
        .unwrap_or_else(|| project_id.to_string());
    org.set_paused(project_id, false);
    org.clear_notice(project_id);
    org.save().map_err(|e| e.to_string())?;

    let q = JobQueue::load();
    let backend = intended_backend(project_id, &q.jobs);
    let kind = slot_kind(&backend);
    let cap = Capacity::load();
    let used_others = q
        .jobs
        .iter()
        .filter(|j| {
            holds_slot(j) && j.project_id != project_id && slot_kind(&j.backend) == kind
        })
        .count() as u32;
    let need = 1u32;
    let room = cap.get(kind);
    if used_others + need <= room {
        return Ok(());
    }
    let steal = used_others + need - room;
    let mut holders: Vec<(String, String)> = Vec::new();
    for j in &q.jobs {
        if !holds_slot(j) || j.project_id == project_id || slot_kind(&j.backend) != kind {
            continue;
        }
        if holders.iter().any(|(id, _)| id == &j.project_id) {
            continue;
        }
        holders.push((j.project_id.clone(), j.created_at.clone()));
    }
    holders.sort_by(|a, b| a.1.cmp(&b.1));
    let reason = crate::org::capacity_taken_by(&taker, kind.as_str());
    let mut org = Org::load();
    let mut q = JobQueue::load();
    for (oid, _) in holders.into_iter().take(steal as usize) {
        org.pause(&oid, true, Some(&reason));
        q.pause_project(&oid);
    }
    let _ = q.save();
    org.save().map_err(|e| e.to_string())?;
    Ok(())
}

/// A parked-for-capacity (not You paused) or already-unpaused job that fits a free slot.
pub fn next_fillable(jobs: &[Job], org: &Org, cap: &Capacity) -> Option<String> {
    let mut candidates: Vec<&Job> = jobs
        .iter()
        .filter(|j| {
            matches!(j.status, JobStatus::Paused | JobStatus::Interrupted)
        })
        .collect();
    candidates.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    for j in candidates {
        let Some(pref) = org.get(&j.project_id) else {
            continue;
        };
        if pref.paused && !crate::org::is_capacity_pause(&pref.pause_reason) {
            continue;
        }
        if pref.pause_reason == crate::org::YOU_PAUSED {
            continue;
        }
        let mut probe = (*j).clone();
        probe.status = JobStatus::Pending;
        if can_admit(&probe, jobs, cap).ok() {
            return Some(j.id.clone());
        }
    }
    None
}

#[derive(Debug, Clone, Serialize)]
pub struct ParallelView {
    pub local_cap: u32,
    pub cloud_cap: u32,
    pub local_live: u32,
    pub cloud_live: u32,
    pub note: String,
}

pub fn view(jobs: &[Job]) -> ParallelView {
    let cap = Capacity::load();
    let local_live = used_slots(jobs, SlotKind::Local);
    let cloud_live = used_slots(jobs, SlotKind::Cloud);
    ParallelView {
        local_cap: cap.get(SlotKind::Local),
        cloud_cap: cap.get(SlotKind::Cloud),
        local_live,
        cloud_live,
        note: format!(
            "Qwen {local_live}/{} local · Grok {cloud_live}/{} cloud. Same epoch never overlaps.",
            cap.get(SlotKind::Local),
            cap.get(SlotKind::Cloud)
        ),
    }
}

/// Throughput helper for tests.
pub fn retune_caps(samples: Vec<Sample>) -> (u32, u32) {
    let mut c = Capacity {
        samples,
        ..Capacity::default()
    };
    c.retune();
    (c.local_cap, c.cloud_cap)
}

pub fn counts_by_concurrent(samples: &[Sample], kind: &str) -> BTreeMap<u32, usize> {
    let mut m = BTreeMap::new();
    for s in samples {
        if s.kind == kind {
            *m.entry(s.concurrent).or_insert(0) += 1;
        }
    }
    m
}
