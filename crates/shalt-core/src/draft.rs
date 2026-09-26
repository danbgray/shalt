//! In-progress spec, as browsable entities rather than a markdown dump.

use crate::spec::{parse_text, Feature};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Draft {
    #[serde(default)]
    pub files: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftView {
    pub epics: Vec<EpicView>,
    pub stories: Vec<StoryView>,
    pub tasks: Vec<TaskView>,
    pub actors: Vec<ActorView>,
    #[serde(default)]
    pub files: Vec<FileView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileView {
    pub path: String,
    pub bytes: usize,
    #[serde(default)]
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpicView {
    pub id: String,
    pub name: String,
    pub story_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryView {
    pub id: String,
    pub name: String,
    pub file: String,
    pub epic: String,
    pub actor: String,
    pub capability: String,
    pub benefit: String,
    pub narrative: String,
    pub task_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskView {
    pub id: String,
    pub name: String,
    pub story_id: String,
    pub epic: String,
    pub file: String,
    pub rid: Option<String>,
    pub holdout: bool,
    pub line: usize,
    pub block_start: usize,
    pub steps: Vec<StepView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepView {
    pub keyword: String,
    pub text: String,
    pub table: Vec<Vec<String>>,
    pub docstring: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActorView {
    pub id: String,
    pub name: String,
    pub story_ids: Vec<String>,
}

impl Draft {
    pub fn put(&mut self, path: &str, content: &str) {
        let path = path.trim().trim_start_matches("./").to_string();
        if path.is_empty() {
            return;
        }
        self.files.insert(path, content.to_string());
    }

    pub fn view(&self) -> DraftView {
        view_from_files(&self.files)
    }

    pub fn restore_to(&self, root: &std::path::Path) -> std::io::Result<()> {
        for (rel, body) in &self.files {
            let p = root.join(rel);
            if let Some(dir) = p.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(p, body)?;
        }
        Ok(())
    }

    pub fn merge_dir(&mut self, spec_dir: &std::path::Path) {
        self.merge_dir_over(spec_dir, false);
    }

    /// Disk is the source of truth for stamped Gherkin. Job drafts only overlay
    /// files that are not on disk yet (in-flight author).
    pub fn merge_dir_over(&mut self, spec_dir: &std::path::Path, overwrite: bool) {
        let Ok(features) = crate::spec::load_specs(spec_dir, false) else {
            return;
        };
        for f in features {
            let path = format!("spec/{}", f.file);
            if !overwrite && self.files.contains_key(&path) {
                continue;
            }
            let p = spec_dir.join(&f.file);
            if let Ok(body) = std::fs::read_to_string(&p) {
                self.put(&path, &body);
            }
        }
    }

    /// Project view: stamped files on disk, plus a live job's in-flight writes.
    pub fn for_project(
        spec_dir: &std::path::Path,
        jobs: &[crate::jobs::Job],
        project_id: &str,
    ) -> Self {
        let mut d = Self::default();
        d.merge_dir_over(spec_dir, true);
        for j in jobs.iter().rev() {
            if j.project_id != project_id {
                continue;
            }
            if !matches!(
                j.status,
                crate::jobs::JobStatus::Running
                    | crate::jobs::JobStatus::Pending
                    | crate::jobs::JobStatus::Waiting
            ) {
                continue;
            }
            for (k, v) in &j.draft.files {
                d.put(k, v);
            }
        }
        d
    }
}

fn split_step(raw: &str) -> StepView {
    let mut lines = raw.split('\n');
    let first = lines.next().unwrap_or("").trim();
    let (keyword, text) = match first.split_once(' ') {
        Some((k, rest))
            if matches!(
                k,
                "Given" | "When" | "Then" | "And" | "But" | "given" | "when" | "then" | "and" | "but"
            ) =>
        {
            (k.to_string(), rest.to_string())
        }
        _ => (String::new(), first.to_string()),
    };
    let mut table = Vec::new();
    let mut docstring = None;
    for line in lines {
        let t = line.trim();
        if t.starts_with("<<<") && t.ends_with(">>>") {
            docstring = Some(t.trim_start_matches("<<<").trim_end_matches(">>>").to_string());
        } else if t.starts_with('|') {
            let cells: Vec<String> = t
                .trim_matches('|')
                .split('|')
                .map(|c| c.trim().to_string())
                .collect();
            table.push(cells);
        }
    }
    StepView {
        keyword,
        text,
        table,
        docstring,
    }
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_matches('-').chars().take(40).collect()
}

pub fn view_from_files(files: &BTreeMap<String, String>) -> DraftView {
    let mut stories = Vec::new();
    let mut tasks = Vec::new();
    let mut epic_map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut actor_map: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for (path, body) in files {
        if !path.ends_with(".feature") {
            continue;
        }
        let rel = path
            .strip_prefix("spec/")
            .unwrap_or(path.as_str())
            .to_string();
        let Ok(Some(f)) = parse_text(body, &rel) else {
            continue;
        };
        push_feature(f, &mut stories, &mut tasks, &mut epic_map, &mut actor_map);
    }

    let epics = epic_map
        .into_iter()
        .map(|(name, story_ids)| EpicView {
            id: format!("e:{}", slug(&name)),
            name,
            story_ids,
        })
        .collect();
    let actors = actor_map
        .into_iter()
        .map(|(name, story_ids)| ActorView {
            id: format!("a:{}", slug(&name)),
            name,
            story_ids,
        })
        .collect();
    let file_views = files
        .iter()
        .filter(|(p, _)| p.ends_with(".feature"))
        .map(|(p, b)| FileView {
            path: p.clone(),
            bytes: b.len(),
            body: b.clone(),
        })
        .collect();
    DraftView {
        epics,
        stories,
        tasks,
        actors,
        files: file_views,
    }
}

fn push_feature(
    f: Feature,
    stories: &mut Vec<StoryView>,
    tasks: &mut Vec<TaskView>,
    epic_map: &mut BTreeMap<String, Vec<String>>,
    actor_map: &mut BTreeMap<String, Vec<String>>,
) {
    let st = f.story();
    let actor = f.inferred_actor();
    let epic = f.epic();
    let story_id = format!("s:{}", slug(&f.file));
    let mut task_ids = Vec::new();
    for sc in &f.scenarios {
        let tid = format!("t:{}:{}", slug(&f.file), sc.line);
        task_ids.push(tid.clone());
        tasks.push(TaskView {
            id: tid,
            name: sc.name.clone(),
            story_id: story_id.clone(),
            epic: epic.clone(),
            file: f.file.clone(),
            rid: sc.rid.clone(),
            holdout: sc.is_holdout(),
            line: sc.line,
            block_start: sc.block_start(),
            steps: sc.steps.iter().map(|s| split_step(s)).collect(),
        });
    }
    if !actor.is_empty() {
        actor_map.entry(actor.clone()).or_default().push(story_id.clone());
    }
    if !epic.is_empty() {
        epic_map.entry(epic.clone()).or_default().push(story_id.clone());
    }
    let narrative = st.one_line();
    stories.push(StoryView {
        id: story_id,
        name: f.name,
        file: f.file,
        epic,
        actor,
        capability: st.capability,
        benefit: st.benefit,
        narrative,
        task_ids,
    });
}

pub fn view_from_features(features: &[Feature]) -> DraftView {
    let mut stories = Vec::new();
    let mut tasks = Vec::new();
    let mut epic_map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut actor_map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for f in features {
        push_feature(f.clone(), &mut stories, &mut tasks, &mut epic_map, &mut actor_map);
    }
    let epics = epic_map
        .into_iter()
        .map(|(name, story_ids)| EpicView {
            id: format!("e:{}", slug(&name)),
            name,
            story_ids,
        })
        .collect();
    let actors = actor_map
        .into_iter()
        .map(|(name, story_ids)| ActorView {
            id: format!("a:{}", slug(&name)),
            name,
            story_ids,
        })
        .collect();
    let file_views = features
        .iter()
        .map(|f| FileView {
            path: format!("spec/{}", f.file),
            bytes: 0,
            body: String::new(),
        })
        .collect();
    DraftView {
        epics,
        stories,
        tasks,
        actors,
        files: file_views,
    }
}
