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
    #[serde(default)]
    pub closed_at: Option<String>,
    /// Snapshot at close: sum of ticket estimates in this sprint.
    #[serde(default)]
    pub estimated: i64,
    /// Snapshot at close: tokens jobs billed to this sprint.
    #[serde(default)]
    pub spent: i64,
    /// Retro notes frozen at close / planning.
    #[serde(default)]
    pub notes: String,
    /// Optional humans invited to the planning meeting.
    #[serde(default)]
    pub guests: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BoardEpic {
    pub name: String,
    /// Forecast tokens for the epic (sum of tickets when 0). Not a cap.
    #[serde(default)]
    pub token_estimate: i64,
    /// Agent assigned to this epic (tickets inherit unless they override).
    #[serde(default)]
    pub backend: String,
    #[serde(default)]
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BoardItem {
    pub rid: String,
    pub rank: i64,
    #[serde(default)]
    pub goal_id: Option<String>,
    #[serde(default)]
    pub milestone_id: Option<String>,
    #[serde(default)]
    pub sprint_id: Option<String>,
    /// System forecast in tokens. Not a cap. 0 means unset — shalt fills it.
    #[serde(default)]
    pub token_estimate: i64,
    /// System forecast in seconds. Not a cap.
    #[serde(default)]
    pub time_estimate_secs: i64,
    /// Agent override for this ticket. Empty inherits the epic’s agent.
    #[serde(default)]
    pub backend: String,
    #[serde(default)]
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SprintRetro {
    pub sprint_id: String,
    pub title: String,
    pub tickets: usize,
    pub estimated: i64,
    pub spent: i64,
    /// spent / estimated. 1.0 is exact. None if nothing was estimated.
    pub accuracy: Option<f64>,
    /// spent - estimated. Positive means we underestimated.
    pub bias: i64,
    pub closed: bool,
    pub suggest: i64,
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
    #[serde(default)]
    pub epics: Vec<BoardEpic>,
    /// Models this project may assign. Empty means the default Qwen+Grok pool.
    #[serde(default)]
    pub pool: Vec<PoolSlot>,
    /// cheap | balanced | fast — how to weigh local cost vs cloud speed.
    #[serde(default)]
    pub prefer: String,
    /// Empty = breadth (one unbound journey per tests job). A slug locks Play to that journey.
    #[serde(default)]
    pub focus_journey: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PoolSlot {
    pub backend: String,
    pub model: String,
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
        let sprint = self.active_sprint().map(|s| s.id.clone());
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
                            sprint_id: sprint.clone(),
                            ..Default::default()
                        });
                    }
                }
            }
        }
    }

    /// Open a sprint if work exists and none is open.
    pub fn ensure_open_sprint(&mut self) -> Option<String> {
        if let Some(s) = self.active_sprint() {
            return Some(s.id.clone());
        }
        if self.items.is_empty() {
            return None;
        }
        Some(self.open_sprint("Sprint 1").id)
    }

    /// Unfinished tickets with no sprint sit on the open sprint, not the whole backlog.
    pub fn seat_on_open_sprint(&mut self) -> usize {
        crate::sprint::seat_sprint_slice(self, &crate::ledger::Ledger::default())
    }

    pub fn unschedule(&mut self, rid: &str) -> bool {
        let n = self.items.len();
        self.items.retain(|i| i.rid != rid);
        n != self.items.len()
    }

    pub fn assign(&mut self, rid: &str, goal: Option<String>, milestone: Option<String>, sprint: Option<String>) -> bool {
        if let Some(it) = self.items.iter_mut().find(|i| i.rid == rid) {
            if let Some(g) = goal {
                it.goal_id = if g.is_empty() { None } else { Some(g) };
            }
            if let Some(m) = milestone {
                it.milestone_id = if m.is_empty() { None } else { Some(m) };
            }
            if let Some(s) = sprint {
                it.sprint_id = if s.is_empty() { None } else { Some(s) };
            }
            true
        } else {
            false
        }
    }

    /// Put this rid at the front of the board (lowest rank). Creates the item if missing.
    pub fn promote(&mut self, rid: &str) -> bool {
        if rid.is_empty() {
            return false;
        }
        let min = self.items.iter().map(|i| i.rank).min().unwrap_or(1);
        let next = min - 1;
        if let Some(it) = self.items.iter_mut().find(|i| i.rid == rid) {
            it.rank = next;
            true
        } else {
            self.items.push(BoardItem {
                rid: rid.into(),
                rank: next,
                ..Default::default()
            });
            true
        }
    }

    pub fn set_estimate(&mut self, rid: &str, tokens: i64) -> bool {
        if let Some(it) = self.items.iter_mut().find(|i| i.rid == rid) {
            it.token_estimate = tokens.max(0);
            true
        } else {
            false
        }
    }

    pub fn set_time_estimate(&mut self, rid: &str, secs: i64) -> bool {
        if let Some(it) = self.items.iter_mut().find(|i| i.rid == rid) {
            it.time_estimate_secs = secs.max(0);
            true
        } else {
            false
        }
    }

    pub fn set_item_agent(&mut self, rid: &str, backend: &str, model: &str) -> bool {
        if let Some(it) = self.items.iter_mut().find(|i| i.rid == rid) {
            it.backend = backend.trim().to_string();
            it.model = model.trim().to_string();
            true
        } else {
            false
        }
    }

    fn epic_mut(&mut self, name: &str) -> Option<&mut BoardEpic> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        if !self.epics.iter().any(|e| e.name == name) {
            self.epics.push(BoardEpic {
                name: name.into(),
                ..Default::default()
            });
        }
        self.epics.iter_mut().find(|e| e.name == name)
    }

    pub fn set_epic_estimate(&mut self, name: &str, tokens: i64) -> bool {
        let Some(e) = self.epic_mut(name) else {
            return false;
        };
        e.token_estimate = tokens.max(0);
        true
    }

    pub fn set_epic_agent(&mut self, name: &str, backend: &str, model: &str) -> bool {
        let Some(e) = self.epic_mut(name) else {
            return false;
        };
        e.backend = backend.trim().to_string();
        e.model = model.trim().to_string();
        true
    }

    pub fn sync_epics(&mut self, features: &[Feature]) {
        for f in features {
            let name = f.epic();
            if name.is_empty() {
                continue;
            }
            if !self.epics.iter().any(|e| e.name == name) {
                self.epics.push(BoardEpic {
                    name,
                    ..Default::default()
                });
            }
        }
    }

    pub fn active_sprint(&self) -> Option<&Sprint> {
        self.sprints
            .iter()
            .find(|s| s.enabled && s.closed_at.is_none())
    }

    pub fn open_sprint(&mut self, title: &str) -> Sprint {
        for s in &mut self.sprints {
            if s.closed_at.is_none() {
                s.enabled = false;
            }
        }
        let n = self.sprints.len() + 1;
        let sprint = Sprint {
            id: format!("C-{n}"),
            title: if title.trim().is_empty() {
                format!("Sprint {n}")
            } else {
                title.trim().to_string()
            },
            start: Some(chrono::Utc::now().format("%Y-%m-%d").to_string()),
            end: None,
            enabled: true,
            closed_at: None,
            estimated: 0,
            spent: 0,
            notes: String::new(),
            guests: Vec::new(),
        };
        self.sprints.push(sprint.clone());
        sprint
    }

    pub fn close_sprint(&mut self, id: &str, estimated: i64, spent: i64) -> bool {
        if let Some(s) = self.sprints.iter_mut().find(|s| s.id == id) {
            s.enabled = false;
            s.closed_at = Some(chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string());
            s.end = Some(chrono::Utc::now().format("%Y-%m-%d").to_string());
            s.estimated = estimated;
            s.spent = spent;
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

    pub fn upsert_goal(&mut self, id: Option<&str>, title: &str) -> Goal {
        let title = title.trim();
        if let Some(id) = id.map(str::trim).filter(|s| !s.is_empty()) {
            if let Some(g) = self.goals.iter_mut().find(|g| g.id == id) {
                if !title.is_empty() {
                    g.title = title.into();
                }
                return g.clone();
            }
            let g = Goal {
                id: id.into(),
                title: if title.is_empty() { id.into() } else { title.into() },
                project_ids: vec![],
            };
            self.goals.push(g.clone());
            return g;
        }
        if let Some(g) = self.goals.iter().find(|g| g.title == title) {
            return g.clone();
        }
        let id = slug_id("g", title, self.goals.len() + 1);
        let g = Goal {
            id,
            title: if title.is_empty() {
                "Goal".into()
            } else {
                title.into()
            },
            project_ids: vec![],
        };
        self.goals.push(g.clone());
        g
    }

    pub fn upsert_milestone(
        &mut self,
        id: Option<&str>,
        title: &str,
        target: Option<&str>,
    ) -> Milestone {
        let title = title.trim();
        let target = target
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        if let Some(id) = id.map(str::trim).filter(|s| !s.is_empty()) {
            if let Some(m) = self.milestones.iter_mut().find(|m| m.id == id) {
                if !title.is_empty() {
                    m.title = title.into();
                }
                if target.is_some() {
                    m.target = target;
                }
                return m.clone();
            }
            let m = Milestone {
                id: id.into(),
                title: if title.is_empty() { id.into() } else { title.into() },
                target,
            };
            self.milestones.push(m.clone());
            return m;
        }
        if let Some(m) = self.milestones.iter().find(|m| m.title == title) {
            return m.clone();
        }
        let id = slug_id("m", title, self.milestones.len() + 1);
        let m = Milestone {
            id,
            title: if title.is_empty() {
                "Milestone".into()
            } else {
                title.into()
            },
            target,
        };
        self.milestones.push(m.clone());
        m
    }
}

fn slug_id(prefix: &str, title: &str, n: usize) -> String {
    let mut s = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if matches!(c, ' ' | '-' | '_') && !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
        if s.len() > 32 {
            break;
        }
    }
    let s = s.trim_matches('-');
    if s.is_empty() {
        format!("{prefix}-{n}")
    } else {
        format!("{prefix}-{s}")
    }
}

pub fn verify_drift(board: &Board, features: &[Feature]) -> Vec<String> {
    board
        .dangling_rids(features)
        .into_iter()
        .map(|rid| format!("overlay drift: board points at {rid} which is not in the spec"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_goal_and_place() {
        let mut b = Board::default();
        let g = b.upsert_goal(Some("g-envelope"), "Shops exchange a packet");
        assert_eq!(g.id, "g-envelope");
        let again = b.upsert_goal(Some("g-envelope"), "Shops exchange a hash-bound packet");
        assert_eq!(b.goals.len(), 1);
        assert_eq!(again.title, "Shops exchange a hash-bound packet");
        let m = b.upsert_milestone(Some("m-format"), "0.0.1 format locked", None);
        assert_eq!(m.id, "m-format");
        b.items.push(BoardItem {
            rid: "S-1".into(),
            rank: 1,
            ..Default::default()
        });
        assert!(b.assign(
            "S-1",
            Some("g-envelope".into()),
            Some("m-format".into()),
            None
        ));
        assert_eq!(b.items[0].goal_id.as_deref(), Some("g-envelope"));
        assert_eq!(b.items[0].milestone_id.as_deref(), Some("m-format"));
        assert!(b.assign("S-1", Some(String::new()), Some(String::new()), None));
        assert_eq!(b.items[0].goal_id, None);
        assert_eq!(b.items[0].milestone_id, None);
    }

    #[test]
    fn slug_from_title_when_id_omitted() {
        let mut b = Board::default();
        let g = b.upsert_goal(None, "Envelope");
        assert_eq!(g.id, "g-envelope");
    }
}
