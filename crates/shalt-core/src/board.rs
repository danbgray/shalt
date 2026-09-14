use crate::spec::Feature;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

pub const SCHEMA: &str = "shalt.board/1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub project_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Milestone {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub target: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sprint {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub start: Option<String>,
    #[serde(default)]
    pub end: Option<String>,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardItem {
    pub rid: String,
    pub rank: i64,
    #[serde(default)]
    pub goal_id: Option<String>,
    #[serde(default)]
    pub milestone_id: Option<String>,
    #[serde(default)]
    pub sprint_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Board {
    #[serde(default = "schema")]
    pub schema: String,
    #[serde(default)]
    pub goals: Vec<Goal>,
    #[serde(default)]
    pub milestones: Vec<Milestone>,
    #[serde(default)]
    pub sprints: Vec<Sprint>,
    #[serde(default)]
    pub items: Vec<BoardItem>,
}

fn schema() -> String {
    SCHEMA.into()
}

impl Board {
    pub fn load(path: &Path) -> Self {
        if !path.exists() {
            return Self {
                schema: SCHEMA.into(),
                ..Default::default()
            };
        }
        serde_json::from_str(&fs::read_to_string(path).unwrap_or_default()).unwrap_or_else(|_| Self {
            schema: SCHEMA.into(),
            ..Default::default()
        })
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(p) = path.parent() {
            fs::create_dir_all(p)?;
        }
        let mut v = self.clone();
        v.schema = SCHEMA.into();
        fs::write(path, serde_json::to_string_pretty(&v)? + "\n")
    }

    pub fn sync_new_rids(&mut self, features: &[Feature]) {
        let mut max_rank = self.items.iter().map(|i| i.rank).max().unwrap_or(0);
        let existing: std::collections::HashSet<_> = self.items.iter().map(|i| i.rid.clone()).collect();
        for f in features {
            for s in &f.scenarios {
                if let Some(rid) = &s.rid {
                    if !existing.contains(rid) {
                        max_rank += 1;
                        self.items.push(BoardItem {
                            rid: rid.clone(),
                            rank: max_rank,
                            goal_id: None,
                            milestone_id: None,
                            sprint_id: None,
                        });
                    }
                }
            }
        }
    }

    pub fn unschedule(&mut self, rid: &str) -> bool {
        let n = self.items.len();
        self.items.retain(|i| i.rid != rid);
        n != self.items.len()
    }

    pub fn assign(&mut self, rid: &str, goal: Option<String>, milestone: Option<String>, sprint: Option<String>) -> bool {
        if let Some(it) = self.items.iter_mut().find(|i| i.rid == rid) {
            if goal.is_some() {
                it.goal_id = goal;
            }
            if milestone.is_some() {
                it.milestone_id = milestone;
            }
            if sprint.is_some() {
                it.sprint_id = sprint;
            }
            true
        } else {
            false
        }
    }

    pub fn set_rank(&mut self, rid: &str, rank: i64) -> bool {
        if let Some(it) = self.items.iter_mut().find(|i| i.rid == rid) {
            it.rank = rank;
            true
        } else {
            false
        }
    }

    pub fn dangling_rids(&self, features: &[Feature]) -> Vec<String> {
        let known: std::collections::HashSet<_> = features
            .iter()
            .flat_map(|f| f.scenarios.iter())
            .filter_map(|s| s.rid.clone())
            .collect();
        self.items
            .iter()
            .filter(|i| !known.contains(&i.rid))
            .map(|i| i.rid.clone())
            .collect()
    }
}

pub fn verify_drift(board: &Board, features: &[Feature]) -> Vec<String> {
    board
        .dangling_rids(features)
        .into_iter()
        .map(|rid| format!("overlay drift: board points at {rid} which is not in the spec"))
        .collect()
}
