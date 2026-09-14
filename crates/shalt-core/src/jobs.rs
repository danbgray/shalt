use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const SCHEMA: &str = "shalt.jobs/1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum JobKind {
    Author,
    Steps,
    Build,
    Run,
    Mutate,
    Diagrams,
    Verify,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Pending,
    Running,
    Paused,
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
        serde_json::from_str(&fs::read_to_string(path).unwrap_or_default()).unwrap_or_default()
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
            j.status = status;
            true
        } else {
            false
        }
    }

    pub fn append(&mut self, id: &str, line: &str) -> bool {
        if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
            if !j.log.is_empty() && !j.log.ends_with('\n') {
                j.log.push('\n');
            }
            j.log.push_str(line.trim_end());
            j.log.push('\n');
            true
        } else {
            false
        }
    }

    pub fn get(&self, id: &str) -> Option<&Job> {
        self.jobs.iter().find(|j| j.id == id)
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
            }
        }
    }

    pub fn interrupt_running(&mut self) {
        self.interrupt_orphans();
    }
}
