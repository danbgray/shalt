use crate::draft::Draft;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

pub const SCHEMA: &str = "shalt.jobs/1";

/// One line (or status change) for the UI event stream.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LiveEvent {
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub job_id: String,
    #[serde(default)]
    pub rid: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub line: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub backend: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub tokens: i64,
    #[serde(default)]
    pub turn: u32,
    #[serde(default)]
    pub secs: i64,
    #[serde(default)]
    pub tick: bool,
}

static LIVE_HOOK: OnceLock<Mutex<Option<Arc<dyn Fn(&LiveEvent) + Send + Sync>>>> = OnceLock::new();

pub fn set_live_hook(f: impl Fn(&LiveEvent) + Send + Sync + 'static) {
    let slot = LIVE_HOOK.get_or_init(|| Mutex::new(None));
    *slot.lock().unwrap() = Some(Arc::new(f));
}

pub fn emit_live(ev: LiveEvent) {
    let Some(slot) = LIVE_HOOK.get() else {
        return;
    };
    let Ok(g) = slot.lock() else {
        return;
    };
    if let Some(f) = g.as_ref() {
        f(&ev);
    }
}

fn growl(title: &str, body: &str) {
    let title = title.chars().take(80).collect::<String>();
    let body = body.chars().take(180).collect::<String>();
    let esc = |s: &str| {
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', " ")
    };
    let script = format!(
        "display notification \"{}\" with title \"{}\" sound name \"Purr\"",
        esc(&body),
        esc(&title)
    );
    let _ = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn emit_job(j: &Job, line: &str) {
    emit_live(live_from_job(j, line));
}

fn prompt_title(prompt: &str) -> String {
    prompt
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .chars()
        .take(72)
        .collect()
}

pub fn job_turn(log: &str) -> u32 {
    log.lines()
        .rev()
        .find_map(|l| {
            let t = l.trim();
            let rest = t.strip_prefix("turn ")?;
            rest.split(|c: char| c == ':' || c.is_whitespace())
                .next()?
                .parse()
                .ok()
        })
        .unwrap_or(0)
}

fn elapsed_secs(j: &Job) -> i64 {
    let a = chrono::NaiveDateTime::parse_from_str(&j.created_at, "%Y-%m-%dT%H:%M:%SZ").ok();
    if !j.finished_at.is_empty() {
        let b = chrono::NaiveDateTime::parse_from_str(&j.finished_at, "%Y-%m-%dT%H:%M:%SZ").ok();
        return match (a, b) {
            (Some(a), Some(b)) => (b - a).num_seconds().max(0),
            _ => 0,
        };
    }
    a.map(|a| (Utc::now().naive_utc() - a).num_seconds().max(0))
        .unwrap_or(0)
}

/// Snapshot of a job for the UI event stream.
pub fn live_from_job(j: &Job, line: &str) -> LiveEvent {
    let line = if line.is_empty() {
        status_line(j)
    } else {
        line.to_string()
    };
    LiveEvent {
        project_id: j.project_id.clone(),
        job_id: j.id.clone(),
        rid: j.rid.clone(),
        status: format!("{:?}", j.status).to_lowercase(),
        kind: kind_phase(j.kind).into(),
        line,
        name: prompt_title(&j.prompt),
        backend: j.backend.clone(),
        model: j.model.clone(),
        created_at: j.created_at.clone(),
        tokens: j.prompt_tokens + j.completion_tokens,
        turn: job_turn(&j.log),
        secs: elapsed_secs(j),
        tick: false,
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum JobKind {
    Author,
    Design,
    Steps,
    Build,
    Run,
    Mutate,
    Diagrams,
    Verify,
    /// Close-the-loop meeting: retro of the last sprint, plan the next slice.
    Plan,
    /// Playwright (when installed) plus static UX after the look is past wireframes.
    Ux,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Pending,
    Running,
    Paused,
    Waiting,
    Done,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub kind: JobKind,
    pub project_id: String,
    pub status: JobStatus,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub log: String,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub backend: String,
    #[serde(default)]
    pub model: String,
    /// Who drafts answers to ask_human. Empty = same as Play (`backend`/`model`).
    #[serde(default)]
    pub answer_backend: String,
    #[serde(default)]
    pub answer_model: String,
    #[serde(default)]
    pub question: String,
    #[serde(default)]
    pub answer: String,
    #[serde(default)]
    pub turns: Vec<TalkTurn>,
    /// Bad Yolo guesses on the current question before parking for a human.
    #[serde(default)]
    pub ask_retries: u32,
    #[serde(default)]
    pub draft: Draft,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub onboard: bool,
    #[serde(default)]
    pub prompt_tokens: i64,
    #[serde(default)]
    pub completion_tokens: i64,
    #[serde(default)]
    pub sprint_id: String,
    /// Epic this job was billed against (from the next unfinished ticket at enqueue).
    #[serde(default)]
    pub epic: String,
    /// Ticket this job was aimed at, when known.
    #[serde(default)]
    pub rid: String,
    #[serde(default)]
    pub finished_at: String,
    /// How many same-kind slots were live when this job started (1 = alone).
    #[serde(default)]
    pub concurrent: u32,
    /// Shared-workspace id. Same epoch overlaps; same project does not.
    #[serde(default)]
    pub epoch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TalkTurn {
    #[serde(default)]
    pub question: String,
    #[serde(default)]
    pub answer: String,
    #[serde(default)]
    pub guess: String,
    /// True when Yolo (or adopt_guess) filled the answer, not the human.
    #[serde(default)]
    pub yolo: bool,
    #[serde(default)]
    pub chat: Vec<ChatMsg>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AskRecord {
    pub question: String,
    pub guess: String,
    pub answer: String,
    pub yolo: bool,
    pub job_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatMsg {
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub backend: String,
    #[serde(default)]
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JobQueue {
    #[serde(default = "schema")]
    pub schema: String,
    #[serde(default)]
    pub jobs: Vec<Job>,
}

fn schema() -> String {
    SCHEMA.into()
}

impl JobQueue {
    pub fn path() -> PathBuf {
        crate::org::Org::home_dir().join("jobs.json")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Self {
        if !path.exists() {
            return Self {
                schema: SCHEMA.into(),
                jobs: vec![],
            };
        }
        let mut q: Self =
            serde_json::from_str(&fs::read_to_string(path).unwrap_or_default()).unwrap_or_default();
        for j in &mut q.jobs {
            if j.turns.iter().all(|t| t.answer.is_empty()) {
                let recovered = turns_from_log(&j.log);
                if !recovered.is_empty() {
                    j.turns = recovered;
                }
            }
        }
        q
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(p) = path.parent() {
            fs::create_dir_all(p)?;
        }
        fs::write(path, serde_json::to_string_pretty(self)? + "\n")
    }

    pub fn enqueue(&mut self, kind: JobKind, project_id: &str) -> Job {
        self.enqueue_full(kind, project_id, "", "", "")
    }

    pub fn enqueue_full(
        &mut self,
        kind: JobKind,
        project_id: &str,
        prompt: &str,
        backend: &str,
        model: &str,
    ) -> Job {
        let job = Job {
            id: format!("J-{:08x}", rand::random::<u32>()),
            kind,
            project_id: project_id.into(),
            status: JobStatus::Pending,
            created_at: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            log: String::new(),
            prompt: prompt.into(),
            backend: backend.into(),
            model: model.into(),
            answer_backend: String::new(),
            answer_model: String::new(),
            question: String::new(),
            answer: String::new(),
            turns: Vec::new(),
            ask_retries: 0,
            draft: Draft::default(),
            error: String::new(),
            onboard: false,
            prompt_tokens: 0,
            completion_tokens: 0,
            sprint_id: String::new(),
            epic: String::new(),
            rid: String::new(),
            finished_at: String::new(),
            concurrent: 0,
            epoch: epoch_id(project_id, kind, "", ""),
        };
        self.jobs.push(job.clone());
        job
    }

    pub fn update(&mut self, id: &str, status: JobStatus, log: &str) -> bool {
        if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
            j.status = status;
            if !log.is_empty() {
                j.log = log.to_string();
            }
            true
        } else {
            false
        }
    }

    pub fn set_status(&mut self, id: &str, status: JobStatus) -> bool {
        if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
            // A late worker must not mark a paused job done or failed.
            // Play still sets Running on Paused; abandon writes Done directly.
            if matches!(j.status, JobStatus::Paused | JobStatus::Interrupted)
                && matches!(status, JobStatus::Done | JobStatus::Failed)
            {
                return false;
            }
            j.status = status;
            if matches!(status, JobStatus::Running | JobStatus::Pending) {
                j.error.clear();
                j.finished_at.clear();
            }
            if matches!(
                status,
                JobStatus::Done | JobStatus::Failed | JobStatus::Interrupted
            ) && j.finished_at.is_empty()
            {
                j.finished_at = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
            }
            emit_job(j, "");
            true
        } else {
            false
        }
    }

    pub fn set_concurrent(&mut self, id: &str, n: u32) -> bool {
        if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
            j.concurrent = n;
            true
        } else {
            false
        }
    }

    pub fn set_agent(&mut self, id: &str, backend: &str, model: &str) -> bool {
        if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
            j.backend = backend.to_string();
            j.model = model.to_string();
            true
        } else {
            false
        }
    }

    /// Who drafts ask_human answers. Does not change the Play worker.
    pub fn set_answer_agent(&mut self, id: &str, backend: &str, model: &str) -> bool {
        if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
            j.answer_backend = backend.trim().to_string();
            j.answer_model = model.trim().to_string();
            true
        } else {
            false
        }
    }

    pub fn is_parked(&self, id: &str) -> bool {
        matches!(
            self.get(id).map(|j| j.status),
            Some(JobStatus::Paused | JobStatus::Interrupted)
        )
    }

    pub fn wants_worker(&self, id: &str) -> bool {
        matches!(
            self.get(id).map(|j| j.status),
            Some(JobStatus::Running | JobStatus::Pending)
        )
    }

    /// Keep Pause/Interrupted when a late worker result arrives.
    /// Do not append the late note — it becomes the Now bar and looks like a hang.
    pub fn keep_parked(&mut self, id: &str, _note: &str) -> bool {
        self.is_parked(id)
    }

    /// Park running/pending jobs so another project can use the model. Waiting stays waiting.
    pub fn pause_project(&mut self, project_id: &str) -> Vec<String> {
        let mut ids = Vec::new();
        for j in &mut self.jobs {
            if j.project_id != project_id {
                continue;
            }
            if matches!(j.status, JobStatus::Running | JobStatus::Pending) {
                j.status = JobStatus::Paused;
                if !j.log.is_empty() && !j.log.ends_with('\n') {
                    j.log.push('\n');
                }
                j.log.push_str(
                    "paused — another project took the model slot. Play here to take it back.\n",
                );
                ids.push(j.id.clone());
            }
        }
        ids
    }

    /// Park every running/pending job. Waiting stays waiting.
    pub fn pause_all(&mut self, note: &str) -> Vec<String> {
        let mut ids = Vec::new();
        for j in &mut self.jobs {
            if !matches!(j.status, JobStatus::Running | JobStatus::Pending) {
                continue;
            }
            j.status = JobStatus::Paused;
            if !j.log.is_empty() && !j.log.ends_with('\n') {
                j.log.push('\n');
            }
            j.log.push_str(note);
            if !j.log.ends_with('\n') {
                j.log.push('\n');
            }
            ids.push(j.id.clone());
        }
        ids
    }

    /// Drop parked work so the next Play starts from the current spec, not an old job.
    /// Author jobs stay parked — an interrupted spec must be resumable.
    pub fn abandon_for_restart(&mut self, project_id: &str) -> Vec<String> {
        let mut ids = Vec::new();
        for j in &mut self.jobs {
            if j.project_id != project_id {
                continue;
            }
            if j.kind == JobKind::Author {
                continue;
            }
            if matches!(
                j.status,
                JobStatus::Paused
                    | JobStatus::Interrupted
                    | JobStatus::Pending
                    | JobStatus::Waiting
            ) {
                j.status = JobStatus::Done;
                if !j.log.is_empty() && !j.log.ends_with('\n') {
                    j.log.push('\n');
                }
                j.log
                    .push_str("abandoned — Play restarts from the current spec and stack\n");
                ids.push(j.id.clone());
            }
        }
        ids
    }

    /// Parked work Play can attach a worker to. Failed is terminal — hop via
    /// `continue_project` instead of reopening the same stall.
    pub fn resumable_for_project(&self, project_id: &str) -> Vec<String> {
        self.jobs
            .iter()
            .filter(|j| {
                j.project_id == project_id
                    && matches!(
                        j.status,
                        JobStatus::Paused | JobStatus::Interrupted | JobStatus::Pending
                    )
            })
            .map(|j| j.id.clone())
            .collect()
    }

    pub fn latest_author(&self, project_id: &str) -> Option<&Job> {
        self.jobs
            .iter()
            .rev()
            .find(|j| j.project_id == project_id && j.kind == JobKind::Author)
    }

    /// Spec authoring is still open: live, parked, or cut short then marked done.
    /// Failed is terminal so a 0.6b stall cannot own Play after the spec exists.
    pub fn authoring_open(&self, project_id: &str) -> bool {
        self.unfinished_author(project_id).is_some()
    }

    pub fn unfinished_author(&self, project_id: &str) -> Option<&Job> {
        let j = self.latest_author(project_id)?;
        if matches!(
            j.status,
            JobStatus::Pending
                | JobStatus::Running
                | JobStatus::Paused
                | JobStatus::Waiting
                | JobStatus::Interrupted
        ) {
            return Some(j);
        }
        if j.status == JobStatus::Done && author_was_cut_short(j) {
            return Some(j);
        }
        None
    }

    /// Reopen an author job that was marked done after an interrupt/abandon.
    pub fn reopen_cut_short_author(&mut self, project_id: &str) -> Option<String> {
        let id = self
            .unfinished_author(project_id)
            .filter(|j| j.status == JobStatus::Done)
            .map(|j| j.id.clone())?;
        self.set_status(&id, JobStatus::Interrupted);
        self.append(
            &id,
            "reopened — spec was not finished. Play resumes from here.",
        );
        Some(id)
    }

    pub fn resume_target(&self, project_id: &str) -> Option<&Job> {
        if let Some(j) = self.unfinished_author(project_id) {
            return Some(j);
        }
        self.jobs.iter().rev().find(|j| {
            j.project_id == project_id
                && matches!(
                    j.status,
                    JobStatus::Paused | JobStatus::Interrupted | JobStatus::Pending
                )
        })
    }

    pub fn append(&mut self, id: &str, line: &str) -> bool {
        let line = line.trim_end();
        if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
            if j.log.lines().last() == Some(line) {
                return true;
            }
            if !j.log.is_empty() && !j.log.ends_with('\n') {
                j.log.push('\n');
            }
            j.log.push_str(line);
            j.log.push('\n');
            emit_job(j, line);
            true
        } else {
            false
        }
    }

    /// Heartbeat while the HTTP call is outstanding. Does not grow the job log.
    pub fn tick(&self, id: &str, line: &str) -> bool {
        if let Some(j) = self.get(id) {
            let mut ev = live_from_job(j, line);
            ev.tick = true;
            emit_live(ev);
            true
        } else {
            false
        }
    }

    pub fn put_file(&mut self, id: &str, path: &str, content: &str) -> bool {
        if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
            j.draft.put(path, content);
            true
        } else {
            false
        }
    }

    pub fn get(&self, id: &str) -> Option<&Job> {
        self.jobs.iter().find(|j| j.id == id)
    }

    pub fn remembered_answer(&self, id: &str, question: &str) -> Option<String> {
        self.remembered_answer_filtered(id, question, false)
    }

    /// Human answers only — Yolo guesses must not skip the wait when Yolo is off.
    pub fn remembered_human_answer(&self, id: &str, question: &str) -> Option<String> {
        self.remembered_answer_filtered(id, question, true)
    }

    fn remembered_answer_filtered(
        &self,
        id: &str,
        question: &str,
        human_only: bool,
    ) -> Option<String> {
        let j = self.get(id)?;
        if let Some(a) = remembered_on(j, question, human_only) {
            return Some(a);
        }
        let pid = j.project_id.as_str();
        if pid.is_empty() {
            return None;
        }
        self.jobs
            .iter()
            .rev()
            .filter(|o| o.id != id && o.project_id == pid)
            .find_map(|o| remembered_on(o, question, human_only))
    }

    pub fn ask(&mut self, id: &str, question: &str, guess: &str) -> bool {
        if self.get(id).is_none() {
            return false;
        }
        if self.remembered_human_answer(id, question).is_some() {
            return true;
        }
        let short = ask_short(question);
        let was_waiting = self.get(id).is_some_and(|j| j.status == JobStatus::Waiting);
        let ok = if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
            j.status = JobStatus::Waiting;
            j.question = question.to_string();
            j.answer.clear();
            if let Some(last) = j.turns.last_mut() {
                if last.answer.is_empty() && same_ask(&last.question, question) {
                    if !guess.is_empty() {
                        last.guess = guess.to_string();
                    }
                    true
                } else {
                    j.turns.push(TalkTurn {
                        question: question.to_string(),
                        answer: String::new(),
                        guess: guess.to_string(),
                        yolo: false,
                        chat: Vec::new(),
                    });
                    true
                }
            } else {
                j.turns.push(TalkTurn {
                    question: question.to_string(),
                    answer: String::new(),
                    guess: guess.to_string(),
                    yolo: false,
                    chat: Vec::new(),
                });
                true
            }
        } else {
            false
        };
        if ok && !short.is_empty() {
            self.append(id, &format!("waiting on you: {short}"));
            if !was_waiting {
                growl("shalt — waiting on you", &short);
                if let Some(j) = self.get(id) {
                    emit_job(j, &format!("Waiting for your answer — {short}"));
                }
            }
        }
        ok
    }

    pub fn append_chat(&mut self, id: &str, role: &str, content: &str) -> bool {
        let j = match self.jobs.iter_mut().find(|j| j.id == id) {
            Some(j) => j,
            None => return false,
        };
        let t = match j.turns.iter_mut().rev().find(|t| t.answer.is_empty()) {
            Some(t) => t,
            None => return false,
        };
        t.chat.push(ChatMsg {
            role: role.to_string(),
            content: content.to_string(),
            backend: String::new(),
            model: String::new(),
        });
        true
    }

    pub fn append_chat_from(
        &mut self,
        id: &str,
        role: &str,
        content: &str,
        backend: &str,
        model: &str,
    ) -> bool {
        let j = match self.jobs.iter_mut().find(|j| j.id == id) {
            Some(j) => j,
            None => return false,
        };
        let t = match j.turns.iter_mut().rev().find(|t| t.answer.is_empty()) {
            Some(t) => t,
            None => return false,
        };
        t.chat.push(ChatMsg {
            role: role.to_string(),
            content: content.to_string(),
            backend: backend.to_string(),
            model: model.to_string(),
        });
        true
    }

    pub fn open_turn(&self, id: &str) -> Option<&TalkTurn> {
        self.get(id)?
            .turns
            .iter()
            .rev()
            .find(|t| t.answer.is_empty())
    }

    pub fn take_answer(&mut self, id: &str) -> Option<String> {
        let j = self.jobs.iter_mut().find(|j| j.id == id)?;
        if j.answer.is_empty() {
            return None;
        }
        let a = std::mem::take(&mut j.answer);
        j.question.clear();
        j.status = JobStatus::Running;
        Some(a)
    }

    pub fn set_answer(&mut self, id: &str, answer: &str) -> bool {
        let answer = answer.trim();
        if answer.is_empty() {
            return false;
        }
        if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
            let question = j
                .turns
                .iter()
                .rev()
                .find(|t| t.answer.is_empty())
                .map(|t| t.question.clone())
                .filter(|q| !q.is_empty())
                .unwrap_or_else(|| j.question.clone());
            j.prompt = fold_into_spec(&j.prompt, &question, answer);
            j.answer = answer.to_string();
            if let Some(t) = j.turns.iter_mut().rev().find(|t| t.answer.is_empty()) {
                t.answer = answer.to_string();
                t.yolo = false;
            }
            j.status = JobStatus::Running;
            true
        } else {
            false
        }
    }

    /// Count a refused Yolo guess. Resets when the question changes.
    pub fn note_bad_guess(&mut self, id: &str, question: &str) -> u32 {
        let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) else {
            return 0;
        };
        if let Some(last) = j.turns.last() {
            if !last.question.is_empty() && !same_ask(&last.question, question) {
                j.ask_retries = 0;
            }
        }
        j.ask_retries = j.ask_retries.saturating_add(1);
        j.ask_retries
    }

    /// Record a decision without parking — used when Yolo takes the guess.
    pub fn settle_ask(&mut self, id: &str, question: &str, guess: &str) -> Option<String> {
        if self.get(id).is_none() {
            return None;
        }
        if let Some(a) = self.remembered_answer(id, question) {
            return Some(a);
        }
        let answer = yolo_reply(guess);
        if answer.is_empty() {
            return None;
        }
        if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
            j.ask_retries = 0;
            j.prompt = fold_into_spec(&j.prompt, question, &answer);
            if let Some(last) = j.turns.last_mut() {
                if last.answer.is_empty() && same_ask(&last.question, question) {
                    last.answer = answer.clone();
                    last.yolo = true;
                    if !guess.is_empty() {
                        last.guess = guess.to_string();
                    }
                } else {
                    j.turns.push(TalkTurn {
                        question: question.to_string(),
                        answer: answer.clone(),
                        guess: guess.to_string(),
                        yolo: true,
                        chat: Vec::new(),
                    });
                }
            } else {
                j.turns.push(TalkTurn {
                    question: question.to_string(),
                    answer: answer.clone(),
                    guess: guess.to_string(),
                    yolo: true,
                    chat: Vec::new(),
                });
            }
            j.question.clear();
            j.answer.clear();
            if j.status == JobStatus::Waiting {
                j.status = JobStatus::Running;
            }
        }
        Some(answer)
    }

    /// Close an open wait with a concrete stored guess. Empty/filler guesses stay parked.
    pub fn adopt_guess(&mut self, id: &str) -> bool {
        let waiting = self.get(id).is_some_and(|j| j.status == JobStatus::Waiting);
        if !waiting && self.open_turn(id).is_none() {
            return false;
        }
        let guess = self
            .open_turn(id)
            .map(|t| t.guess.clone())
            .unwrap_or_default();
        let answer = yolo_reply(&guess);
        if answer.is_empty() {
            return false;
        }
        let ok = self.set_answer(id, &answer);
        if ok {
            if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
                if let Some(t) = j.turns.iter_mut().rev().find(|t| !t.answer.is_empty()) {
                    t.yolo = true;
                    if t.guess.is_empty() {
                        t.guess = guess;
                    }
                }
            }
        }
        ok
    }

    /// Unpark Author waits that asked about calendars, not product behaviour.
    pub fn skip_non_product_waits(&mut self, project_id: &str) -> Vec<String> {
        let ids: Vec<String> = self
            .jobs
            .iter()
            .filter(|j| {
                j.project_id == project_id
                    && j.status == JobStatus::Waiting
                    && j.kind == JobKind::Author
                    && !ask_is_product_behavior(&j.question)
            })
            .map(|j| j.id.clone())
            .collect();
        let mut out = Vec::new();
        for id in ids {
            if self.set_answer(
                &id,
                "No calendar. Write Feature/Scenario/When/Then for product behaviour.",
            ) {
                self.append(
                    &id,
                    "skipped schedule question — specify product behaviour",
                );
                out.push(id);
            }
        }
        out
    }

    pub fn adopt_guesses_for_project(&mut self, project_id: &str) -> Vec<String> {
        let ids: Vec<String> = self
            .jobs
            .iter()
            .filter(|j| j.project_id == project_id && j.status == JobStatus::Waiting)
            .map(|j| j.id.clone())
            .collect();
        let mut out = Vec::new();
        for id in ids {
            if self.adopt_guess(&id) {
                out.push(id);
            }
        }
        out
    }

    pub fn asks_for_project(&self, project_id: &str) -> Vec<AskRecord> {
        let mut out = Vec::new();
        for j in self.jobs.iter().rev() {
            if j.project_id != project_id {
                continue;
            }
            for t in j.turns.iter().rev() {
                if t.question.trim().is_empty() {
                    continue;
                }
                if t.answer.trim().is_empty() && t.guess.trim().is_empty() {
                    continue;
                }
                out.push(AskRecord {
                    question: t.question.clone(),
                    guess: t.guess.clone(),
                    answer: t.answer.clone(),
                    yolo: t.yolo,
                    job_id: j.id.clone(),
                });
                if out.len() >= 48 {
                    out.reverse();
                    return out;
                }
            }
        }
        out.reverse();
        out
    }

    /// Record a human correction for a past Yolo (or human) answer.
    pub fn correct_ask(&mut self, project_id: &str, question: &str, answer: &str) -> bool {
        let answer = answer.trim();
        if question.trim().is_empty() || answer.is_empty() {
            return false;
        }
        for j in self.jobs.iter_mut().rev() {
            if j.project_id != project_id {
                continue;
            }
            if let Some(t) = j
                .turns
                .iter_mut()
                .rev()
                .find(|t| same_ask(&t.question, question))
            {
                t.answer = answer.to_string();
                t.yolo = false;
                return true;
            }
        }
        if let Some(j) = self
            .jobs
            .iter_mut()
            .rev()
            .find(|j| j.project_id == project_id)
        {
            j.turns.push(TalkTurn {
                question: question.to_string(),
                answer: answer.to_string(),
                guess: String::new(),
                yolo: false,
                chat: Vec::new(),
            });
            return true;
        }
        false
    }

    /// Take the chat's ANSWER (or `answer`) into the plain-language spec and close the turn.
    pub fn adopt_chat(&mut self, id: &str, answer: Option<&str>) -> bool {
        let text = match answer.map(str::trim).filter(|s| !s.is_empty()) {
            Some(a) => a.to_string(),
            None => {
                let Some(j) = self.get(id) else {
                    return false;
                };
                let Some(turn) = j.turns.iter().rev().find(|t| t.answer.is_empty()) else {
                    return false;
                };
                let Some(last) = turn
                    .chat
                    .iter()
                    .rev()
                    .find(|m| m.role == "assistant" && !m.content.trim().is_empty())
                else {
                    return false;
                };
                let parsed = parse_chat_answer(&last.content);
                if parsed.is_empty() {
                    return false;
                }
                parsed
            }
        };
        self.set_answer(id, &text)
    }

    pub fn set_error(&mut self, id: &str, error: &str) -> bool {
        if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
            j.error = error.to_string();
            true
        } else {
            false
        }
    }

    pub fn add_tokens(&mut self, id: &str, prompt: i64, completion: i64) -> bool {
        if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
            j.prompt_tokens += prompt.max(0);
            j.completion_tokens += completion.max(0);
            emit_job(j, "");
            true
        } else {
            false
        }
    }

    pub fn set_sprint(&mut self, id: &str, sprint_id: &str) -> bool {
        if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
            j.sprint_id = sprint_id.to_string();
            true
        } else {
            false
        }
    }

    pub fn set_work(&mut self, id: &str, epic: &str, rid: &str) -> bool {
        if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
            j.epic = epic.to_string();
            j.rid = rid.to_string();
            j.epoch = epoch_id(&j.project_id, j.kind, rid, epic);
            emit_job(j, "");
            true
        } else {
            false
        }
    }

    pub fn set_prompt(&mut self, id: &str, prompt: &str) -> bool {
        if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
            j.prompt = prompt.to_string();
            true
        } else {
            false
        }
    }

    pub fn interrupt_orphans(&mut self) {
        for j in &mut self.jobs {
            if j.status == JobStatus::Running {
                j.status = JobStatus::Interrupted;
                if !j.log.is_empty() && !j.log.ends_with('\n') {
                    j.log.push('\n');
                }
                j.log.push_str("desk restarted — Play continues this job\n");
            }
        }
    }

    pub fn interrupt_running(&mut self) {
        self.interrupt_orphans();
    }
}

fn remembered_on(j: &Job, question: &str, human_only: bool) -> Option<String> {
    j.turns.iter().rev().find_map(|t| {
        if t.answer.trim().is_empty() {
            None
        } else if human_only && t.yolo {
            None
        } else if same_ask(&t.question, question) {
            Some(t.answer.clone())
        } else {
            None
        }
    })
}

fn author_was_cut_short(j: &Job) -> bool {
    let log = j.log.to_ascii_lowercase();
    // A later successful write means the pause was recovered; do not reopen.
    if log.contains("wrote ") && log.contains("spec/") {
        return false;
    }
    log.contains("abandoned")
        || log.contains("stopped (interrupted)")
        || log.contains("parked while waiting")
}

fn norm_ask(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .take(80)
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn scenario_ids(s: &str) -> Vec<String> {
    let lower = s.to_ascii_lowercase();
    let b = lower.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 2 < b.len() {
        if b[i] == b's' && b[i + 1] == b'-' {
            let mut j = i + 2;
            while j < b.len() && b[j].is_ascii_hexdigit() {
                j += 1;
            }
            if j - (i + 2) >= 4 {
                out.push(lower[i..j].to_string());
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn quoted_asks(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find('"') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('"') else {
            break;
        };
        let inner: String = rest[..end]
            .chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(|c| c.to_lowercase())
            .collect();
        if inner.len() >= 8 {
            out.push(inner);
        }
        rest = &rest[end + 1..];
    }
    out
}

fn ask_stop(w: &str) -> bool {
    matches!(
        w,
        "the"
            | "and"
            | "or"
            | "but"
            | "to"
            | "of"
            | "in"
            | "on"
            | "for"
            | "with"
            | "from"
            | "that"
            | "this"
            | "these"
            | "those"
            | "are"
            | "was"
            | "were"
            | "be"
            | "been"
            | "being"
            | "it"
            | "its"
            | "as"
            | "at"
            | "by"
            | "if"
            | "then"
            | "than"
            | "so"
            | "not"
            | "do"
            | "does"
            | "did"
            | "should"
            | "would"
            | "could"
            | "can"
            | "may"
            | "will"
            | "we"
            | "you"
            | "they"
            | "what"
            | "which"
            | "who"
            | "how"
            | "when"
            | "where"
            | "why"
            | "has"
            | "have"
            | "had"
            | "there"
            | "their"
            | "our"
            | "your"
            | "any"
            | "all"
            | "one"
            | "two"
            | "first"
            | "into"
            | "about"
            | "over"
            | "after"
            | "before"
            | "while"
            | "also"
            | "just"
            | "only"
            | "more"
            | "scenario"
            | "gherkin"
            | "step"
            | "steps"
            | "given"
            | "feature"
            | "uses"
            | "using"
            | "used"
            | "references"
            | "reference"
            | "appears"
            | "appear"
            | "please"
            | "add"
            | "added"
            | "adding"
    )
}

fn ask_tokens(s: &str) -> HashSet<String> {
    s.split(|c: char| !c.is_alphanumeric() && c != '_')
        .map(|w| w.to_ascii_lowercase())
        .filter(|w| w.len() >= 4 && !ask_stop(w))
        .collect()
}

fn quote_hit(a: &[String], b: &[String]) -> bool {
    a.iter().any(|x| {
        b.iter()
            .any(|y| x == y || x.contains(y.as_str()) || y.contains(x.as_str()))
    })
}

pub fn same_ask(a: &str, b: &str) -> bool {
    let na = norm_ask(a);
    let nb = norm_ask(b);
    if !na.is_empty() && (na == nb || na.starts_with(&nb) || nb.starts_with(&na)) {
        return true;
    }
    let ids_a = scenario_ids(a);
    let ids_b = scenario_ids(b);
    let quotes_a = quoted_asks(a);
    let quotes_b = quoted_asks(b);
    let ta = ask_tokens(a);
    let tb = ask_tokens(b);
    if ta.is_empty() || tb.is_empty() {
        return false;
    }
    let inter = ta.intersection(&tb).count();
    let union = ta.union(&tb).count();
    let quoted = quote_hit(&quotes_a, &quotes_b);
    let same_id = !ids_a.is_empty() && ids_a.iter().any(|id| ids_b.contains(id));
    if same_id && (quoted || inter >= 4) {
        return true;
    }
    if quoted && inter >= 3 {
        return true;
    }
    inter >= 6 && union > 0 && inter * 2 >= union
}

/// Tool-result text when we replay a decision instead of parking again.
pub fn replay_ask(kind: Option<JobKind>, answer: &str) -> String {
    let zone = match kind {
        Some(JobKind::Build) => {
            " You cannot edit spec/ or contract/ in this job — implement under src/, or stop."
        }
        Some(JobKind::Author) => " Write spec/ only.",
        Some(JobKind::Design) => " Write mockups/ only.",
        Some(JobKind::Steps) => " Write steps/ and contract/ only.",
        _ => "",
    };
    format!(
        "Already decided — do not ask this again, including rephrased.{zone}\n{}",
        answer.trim()
    )
}

pub fn parse_chat_answer(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    if let Some(idx) = lower.rfind("answer:") {
        text[idx + "answer:".len()..].trim().to_string()
    } else {
        text.trim().to_string()
    }
}

/// Who drafts the ask_human answer. `backend`/`model` overrides win unless they
/// are the CLI default (`fixture`), which means "use the job".
pub fn resolve_answer_agent(
    job: &Job,
    backend: Option<&str>,
    model: Option<&str>,
) -> (String, String) {
    let override_b = backend
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "fixture");
    let b = override_b
        .or(Some(job.answer_backend.as_str()).filter(|s| !s.is_empty()))
        .or(Some(job.backend.as_str()).filter(|s| !s.is_empty()))
        .unwrap_or("qwen");
    let m = model
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or(Some(job.answer_model.as_str()).filter(|s| !s.is_empty()))
        .or(Some(job.model.as_str()).filter(|s| !s.is_empty()))
        .unwrap_or("");
    (b.to_string(), m.to_string())
}

fn heading_for(prompt: &str, question: &str) -> String {
    prompt
        .lines()
        .find_map(|line| {
            let h = line.strip_prefix("## ")?;
            if h.is_empty() {
                return None;
            }
            if same_ask(h, question) {
                Some(h.to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| ask_short(question))
}

/// Merge one interview decision into the English spec. Same heading replaces in place.
pub fn fold_into_spec(prompt: &str, question: &str, answer: &str) -> String {
    let answer = answer.trim();
    if answer.is_empty() {
        return prompt.to_string();
    }
    let heading = heading_for(prompt, question);
    if heading.is_empty() {
        return prompt.to_string();
    }
    let marker = format!("## {heading}");
    let section = format!("\n\n{marker}\n{answer}");
    let body = prompt.trim_end();
    if let Some(start) = body.find(&marker) {
        let after = start + marker.len();
        let next = body[after..].find("\n## ").map(|i| after + i);
        match next {
            Some(n) => format!("{}{}{}", body[..start].trim_end(), section, &body[n..]),
            None => format!("{}{}", body[..start].trim_end(), section),
        }
    } else {
        format!("{body}{section}")
    }
}

pub fn ask_mode_note(mode: crate::org::YoloMode) -> &'static str {
    match mode {
        crate::org::YoloMode::All => {
            "YOLO is on for every question. Call ask_human with a concrete guess that answers a product fact (who, amount, rule). Never ask for ship dates or implementation calendars. Empty guesses and 'make an assumption' are refused."
        }
        crate::org::YoloMode::Plan => {
            "YOLO is planning-only. Sprint planning takes the guess. For spec, tests, and code you MUST call ask_human and wait. Do not invent the missing fact."
        }
        crate::org::YoloMode::Off => {
            "YOLO is off. If anything is ambiguous you MUST call ask_human and wait. Do not invent the missing fact. Do not write files instead of asking."
        }
    }
}

/// Fillers and auditors do not interview. The yolo note would make them hallucinate ask_human.
pub fn ask_mode_note_for(role: &str, mode: crate::org::YoloMode) -> &'static str {
    if matches!(role, "stepwright" | "auditor" | "code_auditor") {
        ""
    } else {
        ask_mode_note(mode)
    }
}

/// Yolo may take a guess only when it actually answers the question.
pub fn guess_is_concrete(guess: &str) -> bool {
    let g = guess.trim();
    if g.is_empty() {
        return false;
    }
    let l = g.to_ascii_lowercase();
    if l.contains("reasonable assumption") {
        return false;
    }
    if l.contains("write it down as a concrete") {
        return false;
    }
    if l.starts_with("proceed with") {
        return false;
    }
    if g.ends_with('?') {
        return false;
    }
    if l == "idk" || l == "unknown" || l == "n/a" || l == "tbd" {
        return false;
    }
    true
}

/// Author may ask about product behaviour (who, amount, rule), not calendars or delivery.
pub fn ask_is_product_behavior(question: &str) -> bool {
    let l = question.to_ascii_lowercase();
    if l.contains("implementation") {
        return false;
    }
    if l.contains("expected date") || l.contains("ship date") || l.contains("deadline") {
        return false;
    }
    if l.contains("due date") || l.contains("eta") {
        return false;
    }
    if l.contains("when will")
        && (l.contains("done") || l.contains("ready") || l.contains("ship") || l.contains("release"))
    {
        return false;
    }
    if l.contains("feature implementation") {
        return false;
    }
    if l.contains("spec_hash") || l.contains("spec hash") {
        return false;
    }
    true
}

pub fn yolo_reply(guess: &str) -> String {
    let g = guess.trim();
    if guess_is_concrete(g) {
        g.to_string()
    } else {
        String::new()
    }
}

pub(crate) fn ask_short(question: &str) -> String {
    let piece = question
        .split(|c| c == '?' || c == '\n')
        .next()
        .unwrap_or(question)
        .trim();
    piece.chars().take(160).collect()
}

/// Timer while the model is generating. Live ticks only — not a log line.
/// Running/waiting beats a parked leftover so the Now bar is not an old Pause line.
pub fn pick_live_job<'a, I>(jobs: I) -> Option<&'a Job>
where
    I: IntoIterator<Item = &'a Job>,
{
    let list: Vec<&'a Job> = jobs.into_iter().collect();
    list.iter()
        .rev()
        .copied()
        .find(|j| {
            matches!(
                j.status,
                JobStatus::Running | JobStatus::Pending | JobStatus::Waiting
            )
        })
        .or_else(|| {
            list.iter()
                .rev()
                .copied()
                .find(|j| j.status == JobStatus::Paused)
        })
}

pub fn heartbeat_line(line: &str) -> bool {
    let t = line.trim();
    t.starts_with("still waiting")
        || t.starts_with("thinking ·")
        || t.starts_with("still generating")
}

/// Last N activity lines, skipping heartbeat spam so the desk shows real steps.
pub fn activity_lines(log: &str, n: usize) -> Vec<String> {
    log.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !heartbeat_line(l))
        .rev()
        .take(n)
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

/// Heartbeats and file writes belong in the job log. File dumps and model
/// essays do not — they bury the activity the UI should show.
pub fn progress_line(line: &str) -> bool {
    let t = line.trim();
    heartbeat_line(t)
        || t.starts_with("contacting ")
        || t.starts_with("step ")
        || t.starts_with("[write_file]")
        || t.starts_with("[ask_human]")
        || t.starts_with("[list_files]")
        || t.starts_with("[done]")
        || t.starts_with("wrote ")
        || t.starts_with("failed:")
        || t.starts_with("waiting on you")
        || t.starts_with("prompt revised")
        || t.starts_with("pause requested")
        || t.starts_with("resume")
        || t.starts_with("stopped")
        || t.starts_with("tokens +")
        || t.contains("failing over")
        || t.starts_with("HTTP ")
        || t.starts_with("lane · write · dump")
        || t.starts_with("lane · write · harvested")
        || t.starts_with("lane · write · filled")
        || t.starts_with("ERROR:")
}

/// Inbox card: latest job wins. An old failed author must not hide a later done spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectJobState {
    pub running: usize,
    pub waiting: usize,
    pub blocked: usize,
    pub phase: String,
    pub state: String,
    pub detail: String,
    pub issue: Option<Issue>,
}

/// What went wrong and what to do. Shown on the inbox card and the project page.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Issue {
    pub job_id: String,
    pub kind: String,
    pub status: String,
    pub title: String,
    pub log: Vec<String>,
    pub steps: Vec<String>,
    #[serde(default)]
    pub backend: String,
    #[serde(default)]
    pub model: String,
}

/// Workspace identity for overlap.
/// Spec is one epoch. Tests are one epoch. Then each ticket is its own build epoch.
pub fn epoch_id(project_id: &str, kind: JobKind, rid: &str, epic: &str) -> String {
    let p = project_id.trim();
    if p.is_empty() {
        return String::new();
    }
    match kind {
        JobKind::Author => format!("{p}/spec"),
        JobKind::Design => format!("{p}/design"),
        JobKind::Steps => format!("{p}/tests"),
        JobKind::Run => format!("{p}/run"),
        JobKind::Build | JobKind::Verify => {
            let rid = rid.trim();
            if !rid.is_empty() {
                format!("{p}/{rid}")
            } else if !epic.trim().is_empty() {
                format!("{p}/epic:{}", epic.trim())
            } else {
                format!("{p}/{}", kind_phase(kind))
            }
        }
        _ => {
            let rid = rid.trim();
            if !rid.is_empty() {
                format!("{p}/{rid}")
            } else {
                format!("{p}/{}", kind_phase(kind))
            }
        }
    }
}

pub fn epoch_of(j: &Job) -> String {
    if !j.epoch.trim().is_empty() {
        j.epoch.clone()
    } else {
        epoch_id(&j.project_id, j.kind, &j.rid, &j.epic)
    }
}

pub fn kind_phase(kind: JobKind) -> &'static str {
    match kind {
        JobKind::Author => "author",
        JobKind::Design => "design",
        JobKind::Steps => "steps",
        JobKind::Build => "build",
        JobKind::Run => "run",
        JobKind::Verify => "verify",
        JobKind::Mutate => "mutate",
        JobKind::Diagrams => "diagrams",
        JobKind::Plan => "plan",
        JobKind::Ux => "ux",
    }
}

fn dashboard_detail(j: &Job) -> String {
    let raw = status_line(j);
    let lines: Vec<&str> = raw
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    let pick = lines
        .iter()
        .rev()
        .find(|l| l.contains("pip install") || l.contains("needs "))
        .copied()
        .or_else(|| lines.first().copied())
        .unwrap_or("");
    pick.chars().take(140).collect()
}

pub fn summarize_jobs(jobs: &[&Job], paused: bool) -> ProjectJobState {
    let running = jobs
        .iter()
        .filter(|j| matches!(j.status, JobStatus::Running | JobStatus::Pending))
        .count();
    let waiting = jobs
        .iter()
        .filter(|j| j.status == JobStatus::Waiting)
        .count();
    let latest = jobs.last().copied();
    let phase = latest
        .filter(|j| j.status != JobStatus::Done)
        .or(latest)
        .map(|j| kind_phase(j.kind).to_string())
        .unwrap_or_default();
    let (state, detail): (String, String) = if paused {
        (
            "paused".into(),
            latest
                .map(dashboard_detail)
                .unwrap_or_else(|| "Paused".into()),
        )
    } else if running > 0 {
        (
            "playing".into(),
            latest
                .map(dashboard_detail)
                .unwrap_or_else(|| "Playing".into()),
        )
    } else if waiting > 0 {
        (
            "waiting".into(),
            jobs.iter()
                .rev()
                .find(|j| j.status == JobStatus::Waiting)
                .map(|j| dashboard_detail(j))
                .unwrap_or_else(|| "Waiting on you".into()),
        )
    } else {
        match latest.map(|j| j.status) {
            Some(JobStatus::Failed) => (
                "failed".into(),
                latest.map(dashboard_detail).unwrap_or_default(),
            ),
            Some(JobStatus::Interrupted) => (
                "interrupted".into(),
                latest
                    .map(dashboard_detail)
                    .unwrap_or_else(|| "Stopped. Play to resume.".into()),
            ),
            Some(JobStatus::Paused) => (
                "paused".into(),
                latest
                    .map(dashboard_detail)
                    .unwrap_or_else(|| "Paused".into()),
            ),
            _ => ("idle".into(), String::new()),
        }
    };
    let blocked = match state.as_str() {
        "failed" | "interrupted" => 1,
        _ => 0,
    };
    let issue = jobs
        .iter()
        .rev()
        .find(|j| j.status == JobStatus::Waiting)
        .copied()
        .or_else(|| {
            latest.filter(|j| matches!(j.status, JobStatus::Failed | JobStatus::Interrupted))
        })
        .map(diagnose);
    ProjectJobState {
        running,
        waiting,
        blocked,
        phase,
        state,
        detail,
        issue,
    }
}

pub fn diagnose(j: &Job) -> Issue {
    if j.status == JobStatus::Waiting {
        return Issue {
            job_id: j.id.clone(),
            kind: kind_phase(j.kind).into(),
            status: "waiting".into(),
            title: status_line(j),
            log: vec![],
            steps: vec![],
            backend: j.backend.clone(),
            model: j.model.clone(),
        };
    }
    let raw = if !j.error.is_empty() {
        j.error.clone()
    } else {
        status_line(j)
    };
    let title = human_error(&raw);
    let log: Vec<String> = j
        .log
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .rev()
        .take(8)
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    Issue {
        job_id: j.id.clone(),
        kind: kind_phase(j.kind).into(),
        status: format!("{:?}", j.status).to_lowercase(),
        title,
        log,
        steps: repair_steps(j, &raw),
        backend: j.backend.clone(),
        model: j.model.clone(),
    }
}

fn repair_steps(j: &Job, raw: &str) -> Vec<String> {
    let s = raw.to_lowercase();
    let model = if j.model.is_empty() {
        crate::api::DEFAULT_QWEN_MODEL.to_string()
    } else {
        j.model.clone()
    };
    if s.contains("credit") || s.contains("spending limit") || s.contains("permission-denied") {
        return vec![
            "Switch to Qwen below and retry — local Ollama does not need xAI credits.".into(),
            "Or add credits at https://console.x.ai and retry Grok.".into(),
        ];
    }
    if crate::compose::looks_like_missing_key(raw) {
        let key_line = if s.contains("claude") || s.contains("anthropic") {
            "Put the Anthropic key in ~/.shalt/config.toml under [keys] anthropic, or set ANTHROPIC_API_KEY."
        } else if s.contains("openai") || s.contains("codex") {
            "Put the OpenAI key in ~/.shalt/config.toml under [keys] openai, or set OPENAI_API_KEY."
        } else {
            "Put the xAI key in ~/.shalt/config.toml under [keys] xai, or set XAI_API_KEY."
        };
        return vec![
            key_line.into(),
            "Or assign the task to Qwen and Play — local Ollama does not need a key.".into(),
        ];
    }
    if s.contains("11434")
        || s.contains("connection refused")
        || (s.contains("ollama") && (s.contains("busy") || s.contains("not running")))
    {
        return vec![
            "Start Ollama (`ollama serve` or the Ollama app).".into(),
            format!("If the model is missing: `ollama pull {model}`."),
            "Pause other shalt projects so this one has the GPU/CPU.".into(),
            "Or assign the task to Grok and Play.".into(),
        ];
    }
    if s.contains("timed out") || s.contains("didn't respond") {
        return vec![
            "If this was a local model, Ollama may be busy — pause other projects.".into(),
            format!("Confirm the model is loaded: `ollama ps` (want {model})."),
            "If an xAI key is set, Play fails over to Grok when Qwen stalls.".into(),
            "Or assign Grok on the Now card and Play.".into(),
        ];
    }
    if s.contains("not found") && (s.contains("model") || s.contains("pull")) {
        return vec![
            format!("Pull the model: `ollama pull {model}`."),
            "On Command, pick a model that is actually installed, then Play.".into(),
        ];
    }
    if s.contains("npx")
        || s.contains("cucumber-js")
        || (s.contains("node") && (s.contains("not found") || s.contains("enoent")))
    {
        return vec![
            "The javascript stack needs Node.js (`npx cucumber-js`).".into(),
            "Install Node, then Play. shalt writes step definitions under steps/.".into(),
            "Do not patch src/ by hand; that skips the claim.".into(),
        ];
    }
    if s.contains("pytest") || s.contains("externally-managed") || s.contains("python") {
        return vec![
            "Shalt’s supported stacks are Rust and JavaScript. Python is later — don’t pip-install into Homebrew."
                .into(),
            "Pick rust or javascript on Command, then Play.".into(),
            "Play writes tests/shalt.rs on rust, or steps/ on javascript.".into(),
        ];
    }
    if s.contains("wrote no test") || s.contains("stepwright") {
        return vec![
            "Open the spec and check there are scenarios with steps.".into(),
            "Talk to the spec on the Plan tab if a scenario is incomplete.".into(),
            "Play again to rewrite the tests.".into(),
        ];
    }
    if s.contains("cargo") || s.contains("compile") || s.contains("harness") {
        return vec![
            "Open the job log and read the compiler/harness output.".into(),
            "If the test harness is missing, Play should write it (tests/shalt.rs on rust, steps/ on javascript) — retry Play.".into(),
            "Do not patch src/ by hand; that skips the claim.".into(),
        ];
    }
    vec![
        format!(
            "Open the job log ({}) and read the last failed: line.",
            j.id
        ),
        "Fix the cause, or change the agent/model on the task.".into(),
        "Play to retry.".into(),
    ]
}

/// One sentence for the job page / `shalt job show`. Never just the enum name.
pub fn status_line(j: &Job) -> String {
    let last = j
        .log
        .lines()
        .rev()
        .map(|l| l.trim())
        .find(|l| {
            !l.is_empty()
                && !heartbeat_line(l)
                && !(l.starts_with("step ") && l.contains("waiting on the model"))
        })
        .unwrap_or("");
    match j.status {
        JobStatus::Waiting => {
            let q = if !j.question.is_empty() {
                j.question.as_str()
            } else {
                j.turns
                    .iter()
                    .rev()
                    .find(|t| t.answer.is_empty())
                    .map(|t| t.question.as_str())
                    .unwrap_or("")
            };
            let short = ask_short(q);
            if short.is_empty() {
                "Waiting for your answer".into()
            } else {
                format!("Waiting for your answer — {short}")
            }
        }
        JobStatus::Running | JobStatus::Pending => {
            if last.is_empty() {
                let who = if j.model.is_empty() {
                    j.backend.as_str()
                } else {
                    j.model.as_str()
                };
                if who.is_empty() {
                    "Working…".into()
                } else {
                    format!("Working with {who}…")
                }
            } else {
                last.to_string()
            }
        }
        JobStatus::Paused => "Paused. Play to continue.".into(),
        JobStatus::Interrupted => {
            "Stopped. Resume to continue from the spec already written.".into()
        }
        JobStatus::Failed => {
            if j.error.is_empty() {
                human_error(last)
            } else {
                j.error.clone()
            }
        }
        JobStatus::Done => {
            if last.is_empty() {
                "Finished".into()
            } else {
                last.to_string()
            }
        }
    }
}

pub fn human_error(raw: &str) -> String {
    let s = raw.to_lowercase();
    if s.contains("credit") || s.contains("spending limit") || s.contains("permission-denied") {
        "xAI refused the request: this team is out of credits or hit its spending limit.".into()
    } else if s.contains("timed out") || s.contains("didn't respond in time") {
        "The model didn't respond in time. If this was a local model, Ollama may be busy.".into()
    } else if crate::compose::looks_like_missing_key(raw) {
        if s.contains("claude") || s.contains("anthropic") {
            "No Anthropic API key. Put it in ~/.shalt/config.toml or set ANTHROPIC_API_KEY.".into()
        } else if s.contains("openai") || s.contains("codex") {
            "No OpenAI API key. Put it in ~/.shalt/config.toml or set OPENAI_API_KEY.".into()
        } else if s.contains("grok") || s.contains("xai") {
            "No xAI API key. Put it in ~/.shalt/config.toml or set XAI_API_KEY.".into()
        } else {
            raw.chars().take(280).collect()
        }
    } else {
        raw.chars().take(280).collect()
    }
}

pub fn turns_from_log(log: &str) -> Vec<TalkTurn> {
    let mut turns = Vec::new();
    let mut pending: Option<String> = None;
    for line in log.lines() {
        let t = line.trim();
        if let Some(q) = t.strip_prefix('?') {
            let q = q.trim().trim_matches('"').trim();
            if q.is_empty() {
                continue;
            }
            if pending.as_deref() == Some(q) {
                continue;
            }
            if let Some(prev) = pending.take() {
                turns.push(TalkTurn {
                    question: prev,
                    answer: String::new(),
                    ..Default::default()
                });
            }
            pending = Some(q.to_string());
        } else if let Some(a) = t.strip_prefix("you:") {
            let a = a.trim();
            if a.is_empty() {
                continue;
            }
            if let Some(q) = pending.take() {
                if turns
                    .last()
                    .map(|t| t.answer == a && t.question == q)
                    .unwrap_or(false)
                {
                    continue;
                }
                turns.push(TalkTurn {
                    question: q,
                    answer: a.to_string(),
                    ..Default::default()
                });
            } else if let Some(last) = turns.last_mut() {
                if last.answer.is_empty() {
                    last.answer = a.to_string();
                }
            }
        }
    }
    if let Some(q) = pending {
        if !turns.iter().any(|t| t.question == q) {
            turns.push(TalkTurn {
                question: q,
                answer: String::new(),
                ..Default::default()
            });
        }
    }
    turns
}

#[cfg(test)]
mod live_event_tests {
    use super::*;

    #[test]
    fn append_emits_live_event() {
        let got = Arc::new(Mutex::new(None));
        let g2 = got.clone();
        set_live_hook(move |ev| {
            *g2.lock().unwrap() = Some(ev.clone());
        });
        let mut q = JobQueue {
            schema: SCHEMA.into(),
            jobs: vec![],
        };
        let j = q.enqueue(JobKind::Build, "p");
        assert!(q.append(&j.id, "step 1: waiting on the model…"));
        let ev = got.lock().unwrap().clone().expect("emitted");
        assert_eq!(ev.project_id, "p");
        assert_eq!(ev.job_id, j.id);
        assert_eq!(ev.kind, "build");
        assert_eq!(ev.line, "step 1: waiting on the model…");
        assert!(!ev.tick);

        let before = q.get(&j.id).unwrap().log.clone();
        assert!(q.tick(&j.id, "thinking · qwen3.8:27b-mlx · 16s"));
        let ev = got.lock().unwrap().clone().expect("tick");
        assert!(ev.tick);
        assert_eq!(ev.line, "thinking · qwen3.8:27b-mlx · 16s");
        assert_eq!(q.get(&j.id).unwrap().log, before);
    }

    #[test]
    fn fillers_do_not_get_a_yolo_ask_note() {
        assert!(ask_mode_note_for("stepwright", crate::org::YoloMode::All).is_empty());
        assert!(ask_mode_note_for("auditor", crate::org::YoloMode::All).is_empty());
        assert!(ask_mode_note_for("code_auditor", crate::org::YoloMode::Off).is_empty());
        assert!(ask_mode_note_for("author", crate::org::YoloMode::All).contains("ask_human"));
        assert!(ask_mode_note_for("implementer", crate::org::YoloMode::All).contains("ask_human"));
    }

    #[test]
    fn pick_live_job_prefers_running_over_paused() {
        let mut q = JobQueue {
            schema: SCHEMA.into(),
            jobs: vec![],
        };
        let paused = q.enqueue_full(JobKind::Steps, "p", "", "grok", "grok-4");
        q.set_status(&paused.id, JobStatus::Paused);
        q.append(
            &paused.id,
            "model returned after Pause — discarded. Play to continue.",
        );
        let run = q.enqueue_full(JobKind::Design, "p", "", "qwen", "qwen3.8:27b-mlx");
        q.set_status(&run.id, JobStatus::Running);
        let live = pick_live_job(q.jobs.iter()).expect("live");
        assert_eq!(live.id, run.id);
        assert_eq!(live.model, "qwen3.8:27b-mlx");
        let paused_job = q.get(&paused.id).unwrap();
        let line = status_line(paused_job);
        assert_eq!(line, "Paused. Play to continue.");
        assert!(!line.contains("discarded"), "{line}");
        assert!(!line.contains("grok"), "{line}");
    }

    #[test]
    fn live_event_carries_tokens_turn_ticket_and_elapsed() {
        let mut q = JobQueue {
            schema: SCHEMA.into(),
            jobs: vec![],
        };
        let j = q.enqueue_full(
            JobKind::Build,
            "p",
            "Make scenario S-abc pass. That ticket is this job.",
            "grok",
            "grok-4",
        );
        q.set_work(&j.id, "Billing", "S-abc");
        q.add_tokens(&j.id, 40_000, 8_000);
        q.append(&j.id, "turn 3: writing code");
        let job = q.get(&j.id).expect("job");
        let ev = live_from_job(job, "");
        assert_eq!(ev.rid, "S-abc");
        assert_eq!(ev.tokens, 48_000);
        assert_eq!(ev.turn, 3);
        assert_eq!(ev.kind, "build");
        assert_eq!(ev.backend, "grok");
        assert_eq!(ev.model, "grok-4");
        assert!(ev.name.contains("Make scenario S-abc"));
        assert!(!ev.created_at.is_empty());
        assert!(ev.secs >= 0);
        assert_eq!(ev.line, "turn 3: writing code");
    }
}

#[cfg(test)]
mod ask_match_tests {
    use super::*;

    const Q0: &str = r#"The Gherkin scenario S-e6c9cbf6 uses a step "product X is a manufacturable product" that has no matching method in the ErpSystem trait. The trait appears incomplete for this scenario."#;
    const Q1: &str = r#"The scenario S-e6c9cbf6 uses "product X is a manufacturable product" which requires declare_manufacturable, but that method is absent from the public contract in interface.md (while present in the internal src/contract.rs). Should I add it to the public trait?"#;
    const Q_REPHRASE: &str = r#"The first BOM scenario references a step "product X is a manufacturable product" that has no corresponding method in the ErpSystem trait. The contract appears incomplete for this scenario."#;
    const Q_OTHER: &str = "Which currency should invoices use?";

    #[test]
    fn same_ask_matches_rephrased_contract_gap() {
        assert!(same_ask(Q0, Q1), "same scenario + same quoted step");
        assert!(
            same_ask(Q0, Q_REPHRASE),
            "rephrase without scenario id still matches"
        );
        assert!(same_ask(Q1, Q_REPHRASE));
        assert!(
            same_ask("Which currency?", "Which currency should we use?"),
            "prefix match still works"
        );
        assert!(!same_ask(Q0, Q_OTHER));
        assert!(!same_ask(
            Q0,
            "The two instructions contradict on the exact signature and location for declare_manufacturable. Which one should I follow?"
        ));
    }

    #[test]
    fn remembered_answer_replays_a_rephrased_question() {
        let mut q = JobQueue::default();
        let job = q.enqueue(JobKind::Build, "erp");
        q.ask(&job.id, Q0, "add the method");
        q.set_answer(&job.id, "Add declare_manufacturable to the trait.");
        let got = q.remembered_answer(&job.id, Q_REPHRASE);
        assert_eq!(
            got.as_deref(),
            Some("Add declare_manufacturable to the trait.")
        );
        assert_eq!(q.get(&job.id).unwrap().status, JobStatus::Running);
        q.ask(&job.id, Q_REPHRASE, "add it again?");
        assert_eq!(
            q.get(&job.id).unwrap().status,
            JobStatus::Running,
            "already-answered rephrase must not park again"
        );
    }

    #[test]
    fn remembered_answer_is_shared_across_jobs_on_the_project() {
        let mut q = JobQueue::default();
        let a = q.enqueue(JobKind::Build, "erp");
        q.ask(&a.id, Q0, "add it");
        q.set_answer(&a.id, "Add declare_manufacturable.");
        let b = q.enqueue(JobKind::Build, "erp");
        let got = q.remembered_answer(&b.id, Q_REPHRASE);
        assert_eq!(got.as_deref(), Some("Add declare_manufacturable."));
        let other = q.enqueue(JobKind::Build, "other-project");
        assert!(q.remembered_answer(&other.id, Q_REPHRASE).is_none());
    }

    #[test]
    fn fold_into_spec_replaces_a_rephrased_heading() {
        let once = fold_into_spec("Build an ERP.", Q0, "Add declare_manufacturable.");
        let twice = fold_into_spec(&once, Q_REPHRASE, "Use product: &str.");
        assert!(twice.contains("Use product: &str."), "{twice}");
        assert!(
            !twice.contains("Add declare_manufacturable."),
            "old answer must be replaced, not stacked: {twice}"
        );
        assert_eq!(
            twice.matches("## ").count(),
            1,
            "one heading, not two: {twice}"
        );
    }

    #[test]
    fn yolo_settles_without_parking() {
        let mut q = JobQueue::default();
        let job = q.enqueue(JobKind::Build, "erp");
        let a = q
            .settle_ask(&job.id, "Which currency?", "USD")
            .expect("settled");
        assert_eq!(a, "USD");
        let j = q.get(&job.id).unwrap();
        assert_eq!(j.status, JobStatus::Pending);
        assert!(j.turns.iter().all(|t| !t.answer.is_empty()));
        assert!(j.prompt.contains("USD"), "{}", j.prompt);
        assert!(q.open_turn(&job.id).is_none());
    }

    #[test]
    fn yolo_adopt_guess_unparks_a_wait() {
        let mut q = JobQueue::default();
        let job = q.enqueue(JobKind::Build, "erp");
        q.ask(&job.id, "Which currency?", "USD");
        assert_eq!(q.get(&job.id).unwrap().status, JobStatus::Waiting);
        assert!(q.adopt_guess(&job.id));
        assert_eq!(q.take_answer(&job.id).as_deref(), Some("USD"));
        assert_eq!(q.get(&job.id).unwrap().status, JobStatus::Running);
    }

    #[test]
    fn yolo_guess_does_not_skip_the_wait_later() {
        let mut q = JobQueue::default();
        let job = q.enqueue(JobKind::Build, "erp");
        q.settle_ask(&job.id, "Which currency?", "USD");
        assert_eq!(q.get(&job.id).unwrap().turns[0].yolo, true);
        assert!(q.remembered_answer(&job.id, "Which currency?").is_some());
        assert!(
            q.remembered_human_answer(&job.id, "Which currency?").is_none(),
            "Yolo guesses must not count as a human answer"
        );
        q.ask(&job.id, "Which currency?", "EUR");
        assert_eq!(
            q.get(&job.id).unwrap().status,
            JobStatus::Waiting,
            "Yolo off must park even if a Yolo guess exists"
        );
    }

    #[test]
    fn schedule_questions_are_not_product() {
        assert!(!ask_is_product_behavior(
            "What is the expected date for the feature implementation?"
        ));
        assert!(ask_is_product_behavior(
            "When does the monthly patron subscription start?"
        ));
        assert!(!ask_is_product_behavior(
            "What is the actual spec_hash for tasks UI?"
        ));
        assert!(!guess_is_concrete("What is the correct spec_hash for tasks UI?"));
    }

    #[test]
    fn yolo_empty_guess_retries_then_waits() {
        assert!(yolo_reply("").is_empty());
        assert!(!guess_is_concrete(
            "Proceed with a reasonable assumption and write it down as a concrete example."
        ));
        let mut q = JobQueue::default();
        let job = q.enqueue(JobKind::Build, "erp");
        assert!(q.settle_ask(&job.id, "When does the monthly subscription start?", "").is_none());
        for i in 1..=4 {
            assert_eq!(q.note_bad_guess(&job.id, "When does the monthly subscription start?"), i);
        }
        assert_eq!(q.note_bad_guess(&job.id, "When does the monthly subscription start?"), 5);
        q.ask(&job.id, "When does the monthly subscription start?", "");
        assert!(!q.adopt_guess(&job.id));
        assert_eq!(q.get(&job.id).unwrap().status, JobStatus::Waiting);
    }

    #[test]
    fn answer_agent_is_not_the_play_worker() {
        let mut q = JobQueue::default();
        let job = q.enqueue_full(JobKind::Build, "erp", "", "qwen", "qwen3.8:27b-mlx");
        assert_eq!(
            resolve_answer_agent(&q.get(&job.id).unwrap(), None, None),
            ("qwen".into(), "qwen3.8:27b-mlx".into())
        );
        assert!(q.set_answer_agent(&job.id, "grok", "grok-4"));
        let j = q.get(&job.id).unwrap();
        assert_eq!(j.backend, "qwen");
        assert_eq!(j.model, "qwen3.8:27b-mlx");
        assert_eq!(
            resolve_answer_agent(j, None, None),
            ("grok".into(), "grok-4".into())
        );
        assert_eq!(
            resolve_answer_agent(j, Some("fixture"), None),
            ("grok".into(), "grok-4".into()),
            "CLI default fixture must not steal the answer agent"
        );
        assert_eq!(
            resolve_answer_agent(j, Some("openai"), Some("gpt-5")),
            ("openai".into(), "gpt-5".into())
        );
    }
}
