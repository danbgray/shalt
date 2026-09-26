//! Chat aimed at the spec, used before Play starts tests and code.

use crate::config::Config;
use crate::jobs::{ChatMsg, JobQueue, JobStatus};
use crate::ledger::Ledger;
use crate::org::Org;
use crate::roles::{run_role, RoleError};
use crate::runner::steps_has_tests;
use crate::spec::load_specs;
use crate::OpenAICompatBackend;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SpecTalk {
    #[serde(default)]
    pub messages: Vec<ChatMsg>,
}

impl SpecTalk {
    pub fn path(root: &Path) -> PathBuf {
        root.join(".shalt/spec-chat.json")
    }

    pub fn load(root: &Path) -> Self {
        fs::read_to_string(Self::path(root))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, root: &Path) -> std::io::Result<()> {
        let p = Self::path(root);
        if let Some(dir) = p.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(p, serde_json::to_string_pretty(self)? + "\n")
    }

    pub fn append(&mut self, role: &str, content: &str) {
        self.messages.push(ChatMsg {
            role: role.into(),
            content: content.into(),
            ..Default::default()
        });
        if self.messages.len() > 80 {
            let drop = self.messages.len() - 80;
            self.messages.drain(0..drop);
        }
    }
}

fn project_root(project_id: &str) -> Result<PathBuf, String> {
    let org = Org::load();
    let p = org
        .get(project_id)
        .ok_or_else(|| format!("unknown project {project_id}"))?;
    Ok(PathBuf::from(&p.path))
}

fn inherit_model(project_id: &str) -> (String, String) {
    let q = JobQueue::load();
    q.jobs
        .iter()
        .rev()
        .find(|j| j.project_id == project_id && (!j.backend.is_empty() || !j.model.is_empty()))
        .map(|j| (j.backend.clone(), j.model.clone()))
        .unwrap_or_else(|| ("qwen".into(), String::new()))
}

fn project_busy(project_id: &str) -> bool {
    let q = JobQueue::load();
    q.jobs.iter().any(|j| {
        j.project_id == project_id
            && matches!(
                j.status,
                JobStatus::Running | JobStatus::Pending | JobStatus::Waiting
            )
    })
}

/// One conversational turn that may rewrite spec/ only. Does not start tests or code.
pub fn revise_spec(project_id: &str, message: &str) -> Result<SpecTalk, String> {
    let message = message.trim();
    if message.is_empty() {
        return Err("message is empty".into());
    }
    if project_busy(project_id) {
        return Err("pause this project first so the model is free to talk about the spec".into());
    }
    let root = project_root(project_id)?;
    let mut talk = SpecTalk::load(&root);
    talk.append("user", message);
    let _ = talk.save(&root);

    let mut history = String::new();
    for m in talk.messages.iter().rev().take(12).collect::<Vec<_>>().into_iter().rev() {
        history.push_str(&format!("{}: {}\n", m.role, m.content.trim()));
    }
    let spec = crate::compose::spec_snapshot(&root);
    let prompt = format!(
        "The spec is living under spec/. Edit only spec/*.feature files. Do not write tests, src/, or blog files. Do not call list_files. If the current spec already matches their message, call done() immediately.\n\n\
         CURRENT SPEC:\n{spec}\n\n\
         CONVERSATION:\n{history}\n\
         Apply their latest message by editing the feature files."
    );

    let (backend_name, model) = inherit_model(project_id);
    let preset = if backend_name.is_empty() {
        "qwen"
    } else {
        backend_name.as_str()
    };
    let mut backend = OpenAICompatBackend::from_preset(
        preset,
        if model.is_empty() { None } else { Some(model.as_str()) },
        None,
    )?;
    let result = match run_role(&root, "author", &prompt, &mut backend, false) {
        Ok(res) => Ok(res),
        Err(RoleError::Integrity(e)) => Err(RoleError::Integrity(e)),
        Err(e) => {
            let msg = e.to_string();
            if crate::compose::looks_like_local_stall(&msg)
                && crate::alloc::is_local(preset)
                && crate::api::xai_api_key().is_some()
            {
                match OpenAICompatBackend::from_preset(
                    "grok",
                    Some(crate::api::DEFAULT_GROK_MODEL),
                    None,
                ) {
                    Ok(mut grok) => run_role(&root, "author", &prompt, &mut grok, false),
                    Err(e2) => Err(RoleError::Other(e2)),
                }
            } else {
                Err(e)
            }
        }
    };
    match result {
        Ok(res) => {
            let _ = crate::spec::stamp_rids(&root.join("spec"));
            if let Ok(features) = load_specs(&root.join("spec"), false) {
                let mut led = Ledger::load(&root.join(".shalt/ledger.json")).unwrap_or_default();
                led.sync_spec(&features);
                let _ = led.save(&root.join(".shalt/ledger.json"));
                let mut board = crate::board::Board::load(&root.join(".shalt/board.json"));
                board.sync_new_rids(&features);
                board.sync_epics(&features);
                let qnow = crate::jobs::JobQueue::load();
                crate::alloc::allocate_unassigned_now(
                    &mut board,
                    &led,
                    &qnow.jobs,
                    project_id,
                );
                let _ = board.save(&root.join(".shalt/board.json"));
            }
            let reply = if res.wrote.is_empty() {
                "I didn't change any files. Say what should be different in the spec.".to_string()
            } else {
                format!("Updated {}", res.wrote.join(", "))
            };
            talk.append("assistant", &reply);
            let _ = talk.save(&root);
            Ok(talk)
        }
        Err(e) => {
            talk.append("assistant", &format!("couldn't revise the spec: {e}"));
            let _ = talk.save(&root);
            Err(e.to_string())
        }
    }
}

pub fn work_started(root: &Path) -> bool {
    let cfg = Config::load(root).unwrap_or_default();
    steps_has_tests(&root.join(&cfg.steps))
}

/// True when tests exist and a job is live — manager must Pause, edit, then Play.
pub fn edits_need_pause(project_id: &str) -> bool {
    let Ok(root) = project_root(project_id) else {
        return false;
    };
    work_started(&root) && project_busy(project_id)
}

pub fn plan_path(root: &Path) -> PathBuf {
    root.join(".shalt/plan.md")
}

pub fn load_plan(root: &Path, fallback: &str) -> String {
    let p = plan_path(root);
    if p.exists() {
        fs::read_to_string(p).unwrap_or_else(|_| fallback.to_string())
    } else if !fallback.trim().is_empty() {
        fallback.to_string()
    } else {
        String::new()
    }
}

pub fn save_plan(root: &Path, text: &str) -> std::io::Result<()> {
    let p = plan_path(root);
    if let Some(dir) = p.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(p, text)
}

/// True when the plan already has a brief. A wipe stub or a bare heading
/// does not count — a token rerun must not leave the magazine page empty.
pub fn plan_is_keepable(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return false;
    }
    let lower = t.to_ascii_lowercase();
    if lower.contains("wiped for a clean") {
        return false;
    }
    t.lines().any(|l| {
        let l = l.trim();
        !l.is_empty() && !l.starts_with('#')
    })
}

/// First English brief. Does not clobber a plan a human already wrote.
pub fn seed_plan(root: &Path, text: &str) {
    if plan_path(root).exists() {
        let existing = fs::read_to_string(plan_path(root)).unwrap_or_default();
        if plan_is_keepable(&existing) {
            return;
        }
    }
    let t = text.trim();
    if t.is_empty() {
        return;
    }
    let body = if t.starts_with('#') {
        t.to_string()
    } else {
        format!(
            "{t}\n\n## How the journeys hang together\n\n{{{{journeys}}}}\n"
        )
    };
    let _ = save_plan(root, &body);
}

/// Fold interview answers into the free-form plan once, so agent + human share one document.
pub fn fold_answers_into_plan(root: &Path, prompt: &str, turns: &[crate::jobs::TalkTurn]) {
    let answers: Vec<&str> = turns
        .iter()
        .map(|t| t.answer.trim())
        .filter(|a| !a.is_empty())
        .collect();
    if answers.is_empty() {
        return;
    }
    let current = load_plan(root, prompt);
    if current.contains("## What we know") {
        return;
    }
    let mut body = current.trim().to_string();
    if body.is_empty() {
        body = prompt.trim().to_string();
    }
    body.push_str("\n\n## What we know\n");
    for a in answers {
        body.push_str("\n- ");
        body.push_str(a);
        body.push('\n');
    }
    let _ = save_plan(root, &body);
}
