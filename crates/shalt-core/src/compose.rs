use crate::api::system_for;
use crate::config::init_workspace;
use crate::jobs::{Job, JobKind, JobQueue, JobStatus};
use crate::ledger::Ledger;
use crate::narrative::slug;
use crate::org::{Org, ProjectRef};
use crate::roles::run_role;
use crate::spec::load_specs;
use crate::OpenAICompatBackend;
use std::path::PathBuf;
use std::time::Duration;

pub fn author_user_prompt(request: &str) -> String {
    format!("Translate this request into Gherkin feature files under spec/.\n\nREQUEST:\n{request}\n")
}

pub fn author_system_prompt() -> &'static str {
    system_for("author")
}

pub struct ComposeRequest {
    pub prompt: String,
    pub backend: String,
    pub model: String,
    pub name: Option<String>,
}

pub fn start_project(req: ComposeRequest) -> Result<(ProjectRef, Job), String> {
    let prompt = req.prompt.trim();
    if prompt.is_empty() {
        return Err("describe the project first".into());
    }
    let backend = match req.backend.as_str() {
        "grok" | "openai" | "qwen" | "ollama" => req.backend.clone(),
        "" => "qwen".into(),
        other => return Err(format!("unknown backend {other:?}")),
    };
    let name = req
        .name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| {
            let first = prompt.lines().next().unwrap_or(prompt);
            let s = slug(&first.chars().take(48).collect::<String>(), "project");
            if s.is_empty() { "project".into() } else { s }
        });
    let mut dir = Org::home_dir().join("projects").join(&name);
    if dir.exists() {
        dir = Org::home_dir().join("projects").join(format!("{name}-{:04x}", rand::random::<u16>()));
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    init_workspace(&dir, "python", &name)?;
    let mut org = Org::load();
    let project = org.add(&dir)?;
    org.save().map_err(|e| e.to_string())?;
    let mut q = JobQueue::load();
    let job = q.enqueue_full(JobKind::Author, &project.id, prompt, &backend, &req.model);
    q.save().map_err(|e| e.to_string())?;
    Ok((project, job))
}

pub fn execute_author(job_id: &str) -> Result<String, String> {
    let mut q = JobQueue::load();
    let job = q
        .jobs
        .iter()
        .find(|j| j.id == job_id)
        .cloned()
        .ok_or_else(|| format!("no job {job_id}"))?;
    q.set_status(job_id, JobStatus::Running);
    q.append(job_id, &format!("authoring spec with {}…", if job.model.is_empty() { job.backend.as_str() } else { job.model.as_str() }));
    let _ = q.save();
    let org = Org::load();
    let project = org
        .get(&job.project_id)
        .ok_or_else(|| format!("unknown project {}", job.project_id))?;
    let root = PathBuf::from(&project.path);
    let mut backend = OpenAICompatBackend::from_preset(
        if job.backend.is_empty() { "qwen" } else { &job.backend },
        if job.model.is_empty() { None } else { Some(job.model.as_str()) },
        None,
    )?;
    let jid = job_id.to_string();
    let jid_prog = jid.clone();
    backend.on_progress = Some(Box::new(move |line: &str| {
        let mut q = JobQueue::load();
        q.append(&jid_prog, line);
        let _ = q.save();
    }));
    let mut last_prompt = job.prompt.clone();
    let mut announced_pause = false;
    let jid_gate = jid.clone();
    backend.on_gate = Some(Box::new(move || {
        loop {
            let q = JobQueue::load();
            let Some(j) = q.get(&jid_gate) else {
                return Err("job disappeared".into());
            };
            match j.status {
                JobStatus::Paused => {
                    if !announced_pause {
                        let mut q = JobQueue::load();
                        q.append(&jid_gate, "paused — waiting for resume");
                        let _ = q.save();
                        announced_pause = true;
                    }
                    std::thread::sleep(Duration::from_millis(250));
                }
                JobStatus::Interrupted | JobStatus::Failed | JobStatus::Done => {
                    return Err(format!("stopped ({:?})", j.status));
                }
                JobStatus::Running | JobStatus::Pending => {
                    announced_pause = false;
                    if j.prompt != last_prompt {
                        last_prompt = j.prompt.clone();
                        return Ok(Some(format!(
                            "The human revised the request. Follow this version now:\n\n{}",
                            last_prompt
                        )));
                    }
                    return Ok(None);
                }
            }
        }
    }));
    let prompt = author_user_prompt(&job.prompt);
    let result = run_role(&root, "author", &prompt, &mut backend, false);
    let mut q = JobQueue::load();
    match result {
        Ok(res) => {
            if let Ok(features) = load_specs(&root.join("spec"), false) {
                let mut led = Ledger::load(&root.join(".shalt/ledger.json")).unwrap_or_default();
                led.sync_spec(&features);
                let _ = led.save(&root.join(".shalt/ledger.json"));
                let mut board = crate::board::Board::load(&root.join(".shalt/board.json"));
                board.sync_new_rids(&features);
                let _ = board.save(&root.join(".shalt/board.json"));
            }
            let summary = format!("wrote {}: {}", res.wrote.len(), res.wrote.join(", "));
            q.append(job_id, &summary);
            q.set_status(job_id, JobStatus::Done);
            let _ = q.save();
            Ok(summary)
        }
        Err(e) => {
            let stopped = e.to_string();
            q.append(job_id, &format!("failed: {stopped}"));
            let status = if stopped.contains("stopped") {
                JobStatus::Interrupted
            } else {
                JobStatus::Failed
            };
            q.set_status(job_id, status);
            let _ = q.save();
            Err(stopped)
        }
    }
}
