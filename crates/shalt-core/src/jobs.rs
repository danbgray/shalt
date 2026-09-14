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
        let job = Job {
            id: format!("J-{:08x}", rand::random::<u32>()),
            kind,
            project_id: project_id.into(),
            status: JobStatus::Pending,
            created_at: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            log: String::new(),
        };
        self.jobs.push(job.clone());
        job
    }

    pub fn interrupt_running(&mut self) {
        for j in &mut self.jobs {
            if j.status == JobStatus::Running {
                j.status = JobStatus::Interrupted;
            }
        }
    }
}
