//! Project blog: progress posts as work moves, longer features when something notable lands.

use crate::jobs::{kind_phase, Job, JobKind};
use crate::ledger::{Ledger, GREEN, ORPHAN};
use crate::mockups::journey_slug;
use crate::spec::Feature;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub const SCHEMA: &str = "shalt.journal/1";
pub const FORM_PROGRESS: &str = "progress";
pub const FORM_FEATURE: &str = "feature";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Journal {
    #[serde(default = "schema")]
    pub schema: String,
    #[serde(default = "one")]
    pub volume: u32,
    #[serde(default)]
    pub issues: Vec<Issue>,
    #[serde(default)]
    pub pending: Vec<Pending>,
}

fn schema() -> String {
    SCHEMA.into()
}
fn one() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Issue {
    pub date: String,
    pub number: u32,
    #[serde(default)]
    pub dispatches: Vec<Dispatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Dispatch {
    pub at: String,
    pub job_id: String,
    pub kind: String,
    pub desk: String,
    pub backend: String,
    pub model: String,
    pub rid: String,
    pub title: String,
    pub body: String,
    #[serde(default = "progress_form")]
    pub form: String,
    #[serde(default)]
    pub event: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub comments: Vec<Comment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Comment {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub at: String,
    #[serde(default)]
    pub by: String,
    #[serde(default)]
    pub body: String,
    /// Empty = top-level comment on the post; otherwise another comment's id.
    #[serde(default)]
    pub parent: String,
}

fn progress_form() -> String {
    FORM_PROGRESS.into()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Pending {
    pub event: String,
    pub why: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub form: &'static str,
    pub title: String,
    pub body: String,
}

impl Journal {
    pub fn path(root: &Path) -> PathBuf {
        root.join(".shalt/journal.json")
    }

    pub fn load(root: &Path) -> Self {
        let mut j: Self = fs::read_to_string(Self::path(root))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        j.ensure_ids();
        j
    }

    pub fn save(&self, root: &Path) -> std::io::Result<()> {
        let p = Self::path(root);
        if let Some(dir) = p.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(p, serde_json::to_string_pretty(self)? + "\n")
    }

    fn covers(&self, event: &str) -> bool {
        if event.is_empty() {
            return false;
        }
        self.pending.iter().any(|p| p.event == event)
            || self.issues.iter().any(|i| {
                i.dispatches
                    .iter()
                    .any(|d| d.event == event || d.job_id == event)
            })
    }

    fn post_key(d: &Dispatch) -> String {
        format!("{}:{}", d.form, d.job_id)
    }

    fn mint_post_id(d: &Dispatch) -> String {
        let raw = format!("{}|{}|{}|{}", d.form, d.job_id, d.at, d.title);
        format!("p-{}", &hex::encode(Sha256::digest(raw.as_bytes()))[..12])
    }

    fn mint_comment_id(c: &Comment) -> String {
        let raw = format!("{}|{}|{}|{}", c.at, c.by, c.parent, c.body);
        format!("c-{}", &hex::encode(Sha256::digest(raw.as_bytes()))[..12])
    }

    pub fn post_id(d: &Dispatch) -> String {
        if d.id.is_empty() {
            Self::mint_post_id(d)
        } else {
            d.id.clone()
        }
    }

    pub fn ensure_ids(&mut self) {
        for issue in &mut self.issues {
            for d in &mut issue.dispatches {
                if d.id.is_empty() {
                    d.id = Self::mint_post_id(d);
                }
                for c in &mut d.comments {
                    if c.id.is_empty() {
                        c.id = Self::mint_comment_id(c);
                    }
                }
            }
        }
    }

    /// Add a comment or a reply on a filed post. `parent` is empty for a top-level comment.
    pub fn add_comment(
        &mut self,
        post: &str,
        parent: &str,
        by: &str,
        body: &str,
    ) -> Result<Comment, String> {
        let body = body.trim();
        if body.is_empty() {
            return Err("write a comment".into());
        }
        if body.chars().count() > 8000 {
            return Err("comment is too long".into());
        }
        let post = post.trim();
        if post.is_empty() {
            return Err("which post?".into());
        }
        self.ensure_ids();
        let d = self
            .issues
            .iter_mut()
            .flat_map(|i| i.dispatches.iter_mut())
            .find(|d| d.id == post)
            .ok_or_else(|| format!("no post {post}"))?;
        let parent = parent.trim();
        if !parent.is_empty() && !d.comments.iter().any(|c| c.id == parent) {
            return Err("no such comment to reply to".into());
        }
        let by = {
            let b = by.trim();
            if b.is_empty() {
                "You"
            } else {
                b
            }
        };
        let mut c = Comment {
            id: String::new(),
            at: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            by: by.into(),
            body: body.into(),
            parent: parent.into(),
        };
        c.id = Self::mint_comment_id(&c);
        d.comments.push(c.clone());
        Ok(c)
    }

    /// File a post on `date` (YYYY-MM-DD). Newest issue first. Same form+job_id is ignored.
    pub fn file_on_date(&mut self, root: &Path, date: &str, d: Dispatch) -> std::io::Result<bool> {
        let min = if d.form == FORM_FEATURE { 180 } else { 40 };
        if d.job_id.is_empty() || d.body.chars().filter(|c| c.is_alphabetic()).count() < min {
            return Ok(false);
        }
        let key = Self::post_key(&d);
        if self
            .issues
            .iter()
            .any(|i| i.dispatches.iter().any(|x| Self::post_key(x) == key))
        {
            return Ok(false);
        }
        if !d.event.is_empty() {
            self.pending.retain(|p| p.event != d.event);
        }
        let mut d = d;
        if d.id.is_empty() {
            d.id = Self::mint_post_id(&d);
        }
        if let Some(issue) = self.issues.iter_mut().find(|i| i.date == date) {
            issue.dispatches.push(d);
            issue.dispatches.sort_by(|a, b| match (a.form.as_str(), b.form.as_str()) {
                ("feature", "progress") => std::cmp::Ordering::Less,
                ("progress", "feature") => std::cmp::Ordering::Greater,
                _ => b.at.cmp(&a.at),
            });
        } else {
            let number = self.issues.iter().map(|i| i.number).max().unwrap_or(0) + 1;
            self.issues.insert(
                0,
                Issue {
                    date: date.to_string(),
                    number,
                    dispatches: vec![d],
                },
            );
            self.issues.sort_by(|a, b| b.date.cmp(&a.date));
        }
        if self.volume == 0 {
            self.volume = 1;
        }
        self.save(root)?;
        Ok(true)
    }
}

pub fn desk_name(kind: JobKind) -> &'static str {
    match kind {
        JobKind::Author => "Specifying",
        JobKind::Design => "Designing",
        JobKind::Steps => "Tests",
        JobKind::Build => "Code",
        JobKind::Run => "Survey",
        JobKind::Verify => "Verify",
        JobKind::Mutate => "Mutation",
        JobKind::Diagrams => "Diagrams",
        JobKind::Plan => "Planning",
        JobKind::Ux => "UX",
    }
}

fn last_block(text: &str, marker: &str) -> Option<(String, String)> {
    let lower = text.to_ascii_lowercase();
    let idx = lower.rfind(marker)?;
    let rest = text[idx + marker.len()..].trim();
    if rest.is_empty() {
        return None;
    }
    let cut = rest
        .find("\nJOURNAL:")
        .or_else(|| rest.find("\njournal:"))
        .or_else(|| rest.find("\nFEATURE:"))
        .or_else(|| rest.find("\nfeature:"))
        .unwrap_or(rest.len());
    let rest = rest[..cut].trim();
    let mut lines = rest.lines();
    let first = lines.next().unwrap_or("").trim();
    let rest_body = lines.collect::<Vec<_>>().join("\n");
    let rest_body = rest_body.trim();
    let (title, body) = if rest_body.is_empty() {
        (first.to_string(), first.to_string())
    } else if first.is_empty() {
        let mut paras = rest_body.splitn(2, "\n\n");
        let t = paras.next().unwrap_or(rest_body).trim();
        let b = paras.next().unwrap_or(rest_body).trim();
        (t.to_string(), b.to_string())
    } else {
        (first.to_string(), rest_body.to_string())
    };
    let title = title.trim().trim_matches('"').to_string();
    let body = body.trim().to_string();
    if title.eq_ignore_ascii_case("ok") {
        return None;
    }
    Some((title, body))
}

pub fn parse_posts(text: &str) -> Vec<Parsed> {
    let mut out = Vec::new();
    if let Some((title, body)) = last_block(text, "feature:") {
        if body.chars().filter(|c| c.is_alphabetic()).count() >= 180 {
            out.push(Parsed {
                form: FORM_FEATURE,
                title,
                body,
            });
        }
    }
    if let Some((title, body)) = last_block(text, "journal:") {
        if body.chars().filter(|c| c.is_alphabetic()).count() >= 40 {
            out.push(Parsed {
                form: FORM_PROGRESS,
                title,
                body,
            });
        }
    }
    out
}

/// Last progress block only (tests and old call sites).
pub fn parse_dispatch(text: &str) -> Option<(String, String)> {
    parse_posts(text)
        .into_iter()
        .find(|p| p.form == FORM_PROGRESS)
        .map(|p| (p.title, p.body))
}

pub fn dispatch_from(job: &Job, text: &str) -> Dispatch {
    posts_from(job, text)
        .into_iter()
        .find(|d| d.form == FORM_PROGRESS)
        .unwrap_or_default()
}

pub fn posts_from(job: &Job, text: &str) -> Vec<Dispatch> {
    parse_posts(text)
        .into_iter()
        .map(|p| Dispatch {
            at: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            job_id: if p.form == FORM_FEATURE {
                String::new()
            } else {
                job.id.clone()
            },
            kind: kind_phase(job.kind).into(),
            desk: desk_name(job.kind).into(),
            backend: job.backend.clone(),
            model: job.model.clone(),
            rid: job.rid.clone(),
            title: p.title,
            body: p.body,
            form: p.form.into(),
            event: String::new(),
            id: String::new(),
            comments: Vec::new(),
        })
        .collect()
}

/// Human feedback on a filed post. Replies set `parent` to a comment id.
pub fn comment(
    root: &Path,
    post: &str,
    parent: &str,
    by: &str,
    body: &str,
) -> Result<Comment, String> {
    let mut j = Journal::load(root);
    let c = j.add_comment(post, parent, by, body)?;
    j.save(root).map_err(|e| e.to_string())?;
    Ok(c)
}

/// Harvest JOURNAL: (progress) and FEATURE: (notable) blocks and file them.
pub fn publish(root: &Path, job: &Job, transcript: &str) -> std::io::Result<bool> {
    match job.kind {
        JobKind::Author | JobKind::Design | JobKind::Steps | JobKind::Build => {}
        _ => return Ok(false),
    }
    let blob = {
        let t = transcript.to_ascii_lowercase();
        if t.contains("journal:") || t.contains("feature:") {
            transcript
        } else {
            &job.log
        }
    };
    let mut posts = posts_from(job, blob);
    if posts.is_empty() {
        return Ok(false);
    }
    let date = Utc::now().format("%Y-%m-%d").to_string();
    let mut j = Journal::load(root);
    let pending = j.pending.first().cloned();
    let mut any = false;
    for mut d in posts.drain(..) {
        if d.form == FORM_FEATURE {
            if let Some(p) = pending.as_ref() {
                d.event = p.event.clone();
                d.job_id = p.event.clone();
            } else if d.job_id.is_empty() {
                d.job_id = format!("{}:feature", job.id);
            }
        }
        if j.file_on_date(root, &date, d)? {
            any = true;
        }
    }
    Ok(any)
}

pub fn note_event(root: &Path, event: &str, why: &str) -> std::io::Result<bool> {
    let event = event.trim();
    let why = why.trim();
    if event.is_empty() || why.is_empty() {
        return Ok(false);
    }
    let mut j = Journal::load(root);
    if j.covers(event) {
        return Ok(false);
    }
    j.pending.push(Pending {
        event: event.into(),
        why: why.into(),
    });
    j.save(root)?;
    Ok(true)
}

pub fn pending_brief(root: &Path) -> String {
    let j = Journal::load(root);
    let Some(p) = j.pending.first() else {
        return String::new();
    };
    format!(
        "NOTABLE — this project's blog needs a longer FEATURE article, not a short JOURNAL progress note.\n{}\nEnd with exactly:\nFEATURE: <headline>\n<body of 6–12 short paragraphs, magazine voice>\n",
        p.why.trim()
    )
}

pub fn with_pending(root: &Path, prompt: &str) -> String {
    let extra = pending_brief(root);
    if extra.is_empty() {
        prompt.to_string()
    } else {
        format!("{prompt}\n\n{extra}")
    }
}

fn green_rids(led: &Ledger) -> HashSet<String> {
    led.entries
        .values()
        .filter(|e| e.status == GREEN)
        .map(|e| e.rid.clone())
        .collect()
}

pub fn fully_green_journeys(features: &[Feature], led: &Ledger) -> Vec<String> {
    let mut slugs: Vec<String> = Vec::new();
    for f in features {
        let j = journey_slug(f);
        if !slugs.contains(&j) {
            slugs.push(j);
        }
    }
    slugs
        .into_iter()
        .filter(|j| {
            let rids: Vec<String> = features
                .iter()
                .filter(|f| journey_slug(f) == *j)
                .flat_map(|f| f.scenarios.iter())
                .filter_map(|s| s.rid.clone())
                .collect();
            !rids.is_empty()
                && rids.iter().all(|r| {
                    led.entries
                        .get(r)
                        .map(|e| e.status == GREEN)
                        .unwrap_or(false)
                })
        })
        .collect()
}

/// Queue a FEATURE article when the ledger crosses a notable line.
pub fn note_ledger(
    root: &Path,
    before: &Ledger,
    after: &Ledger,
    features: &[Feature],
) -> std::io::Result<()> {
    let g0 = green_rids(before);
    let g1 = green_rids(after);
    if g0.is_empty() && !g1.is_empty() {
        let _ = note_event(
            root,
            "first-green",
            "The first scenario just went green — a bound test passed against the spec. Write the feature about what that actually means for this product, not a status ping.",
        );
    }
    let before_j: HashSet<String> = fully_green_journeys(features, before).into_iter().collect();
    for j in fully_green_journeys(features, after) {
        if before_j.contains(&j) {
            continue;
        }
        let _ = note_event(
            root,
            &format!("journey-green:{j}"),
            &format!("The whole “{j}” journey is green. Write the feature as the story of that journey — who it is for, what just became true, what is still pencil."),
        );
    }
    let live: Vec<_> = after
        .entries
        .values()
        .filter(|e| e.status != ORPHAN)
        .collect();
    if !live.is_empty() && live.iter().all(|e| e.status == GREEN) && (g0.len() < g1.len()) {
        let _ = note_event(
            root,
            "spec-green",
            "Every live scenario is green. Write the feature as a dispatch from the end of the loop — what the product does now, what the spec still owes.",
        );
    }
    Ok(())
}

pub fn note_spec_born(root: &Path, features: &[Feature]) -> std::io::Result<bool> {
    let n: usize = features.iter().map(|f| f.scenarios.len()).sum();
    if n == 0 {
        return Ok(false);
    }
    note_event(
        root,
        "spec-born",
        "The spec just grew its first journeys. Write the feature as the opening essay: what this system shall be, who it is for, and the shape of the work ahead.",
    )
}

pub fn note_design_up(root: &Path) -> std::io::Result<bool> {
    let dir = root.join("mockups/journeys");
    let has_html = dir.exists()
        && walkdir::WalkDir::new(&dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .any(|e| {
                e.path()
                    .extension()
                    .and_then(|s| s.to_str())
                    == Some("html")
            });
    if !has_html {
        return Ok(false);
    }
    note_event(
        root,
        "design-up",
        "Storyboards exist — sketched HTML for the journeys. Write the feature about the pictures: what the product looks like in pencil, which screens matter, what is still a named empty board. Name each journey as it appears on the plan so the blog can link to its film.",
    )
}

pub const SIGN_OFF: &str = " This project has a blog humans read. Do not write a blog file. After meaningful work, put a short progress note in done()'s summary, first person as this role, journeys named as on the plan. End the summary with:\nJOURNAL: <headline>\n<body>\nIf the user prompt asks for a FEATURE article, put that longer essay in the same done() summary:\nFEATURE: <headline>\n<body>";
