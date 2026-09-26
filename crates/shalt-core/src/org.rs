use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectRef {
    pub id: String,
    pub name: String,
    pub path: String,
    /// When true, running jobs for this project park so another project can use the model.
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub pause_reason: String,
    #[serde(default)]
    pub notice: String,
    /// When true, ask_human takes the model's guess (legacy; same as yolo_mode=all).
    #[serde(default)]
    pub yolo: bool,
    /// off | plan | all. Empty falls back to `yolo`.
    #[serde(default)]
    pub yolo_mode: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum YoloMode {
    #[default]
    Off,
    /// Take the guess only for sprint planning. Spec/tests/code still wait.
    Plan,
    /// Take every ask_human guess.
    All,
}

impl YoloMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" | "false" | "0" | "none" => Some(Self::Off),
            "plan" | "planning" => Some(Self::Plan),
            "all" | "on" | "true" | "1" => Some(Self::All),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Plan => "plan",
            Self::All => "all",
        }
    }

    pub fn asks_all(self) -> bool {
        matches!(self, Self::All)
    }

    pub fn plans(self) -> bool {
        matches!(self, Self::Plan | Self::All)
    }

    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Plan,
            Self::Plan => Self::All,
            Self::All => Self::Off,
        }
    }
}

impl ProjectRef {
    pub fn yolo_mode_enum(&self) -> YoloMode {
        if let Some(m) = YoloMode::parse(&self.yolo_mode) {
            return m;
        }
        if self.yolo {
            YoloMode::All
        } else {
            YoloMode::Off
        }
    }
}

/// Manual Pause from the desk or `shalt org pause`.
pub const YOU_PAUSED: &str = "You paused this project.";

/// Play finished: every scenario is green. The loop stops.
pub fn play_done_reason(green: usize, total: usize) -> String {
    if total == 0 {
        "Done — all scenarios are green.".into()
    } else {
        format!("Done — {green}/{total} scenarios are green.")
    }
}

/// `shalt stop` parked every project so a later Play is a deliberate start.
pub const YOU_STOPPED: &str = "Stopped. Play here to start again.";

/// Fallback when a project was parked before we recorded who took the slot.
pub const SLOT_TAKEN: &str =
    "Paused because another project took the model slot. Play here to take it back.";

pub fn slot_taken_by(taker: &str) -> String {
    format!(
        "Paused because Play started on “{taker}”. One model slot — Play here to take it back."
    )
}

pub fn capacity_taken_by(taker: &str, kind: &str) -> String {
    format!(
        "Paused because “{taker}” needed a {kind} slot. Play here to take one back."
    )
}

pub fn is_capacity_pause(reason: &str) -> bool {
    reason.contains("needed a ") && reason.contains(" slot")
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Org {
    pub name: String,
    #[serde(default)]
    pub projects: Vec<ProjectRef>,
}

impl Org {
    pub fn home_dir() -> PathBuf {
        if let Ok(h) = std::env::var("SHALT_HOME") {
            return PathBuf::from(h);
        }
        dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".shalt")
    }

    pub fn path() -> PathBuf {
        Self::home_dir().join("org.toml")
    }

    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    pub fn load_from(path: &Path) -> Self {
        if !path.exists() {
            return Self {
                name: "local".into(),
                projects: vec![],
            };
        }
        toml::from_str(&fs::read_to_string(path).unwrap_or_default()).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(p) = path.parent() {
            fs::create_dir_all(p)?;
        }
        fs::write(path, toml::to_string_pretty(self).unwrap_or_default())
    }

    pub fn add(&mut self, path: &Path) -> Result<ProjectRef, String> {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let id = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "project".into());
        if self.projects.iter().any(|p| p.id == id || p.path == path.display().to_string()) {
            return Err(format!("project {id} already in org"));
        }
        let pref = ProjectRef {
            id: id.clone(),
            name: id.clone(),
            path: path.display().to_string(),
            paused: false,
            pause_reason: String::new(),
            notice: String::new(),
            yolo: false,
            yolo_mode: String::new(),
        };
        self.projects.push(pref.clone());
        Ok(pref)
    }

    pub fn rename(&mut self, id: &str, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() {
            return false;
        }
        if let Some(p) = self.projects.iter_mut().find(|p| p.id == id) {
            p.name = name.to_string();
            true
        } else {
            false
        }
    }

    /// Drop from the org catalog. Does not delete files on disk.
    pub fn remove(&mut self, id: &str) -> bool {
        let n = self.projects.len();
        self.projects.retain(|p| p.id != id);
        self.projects.len() != n
    }

    pub fn get(&self, id: &str) -> Option<&ProjectRef> {
        self.projects.iter().find(|p| p.id == id)
    }

    pub fn find_by_path(&self, path: &Path) -> Option<&ProjectRef> {
        let s = path
            .canonicalize()
            .unwrap_or_else(|_| path.to_path_buf())
            .display()
            .to_string();
        self.projects
            .iter()
            .find(|p| p.path == s || Path::new(&p.path) == path)
    }

    /// Register `path` in the org catalog if it is not already there.
    pub fn ensure_registered(path: &Path) -> Result<ProjectRef, String> {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let mut org = Self::load();
        if let Some(p) = org.find_by_path(&path).cloned() {
            return Ok(p);
        }
        let p = org.add(&path)?;
        org.save().map_err(|e| e.to_string())?;
        Ok(p)
    }

    pub fn set_paused(&mut self, id: &str, paused: bool) -> bool {
        self.pause(id, paused, None)
    }

    pub fn pause(&mut self, id: &str, paused: bool, reason: Option<&str>) -> bool {
        if let Some(p) = self.projects.iter_mut().find(|p| p.id == id) {
            p.paused = paused;
            if paused {
                if let Some(r) = reason.filter(|s| !s.is_empty()) {
                    p.pause_reason = r.to_string();
                    p.notice = r.to_string();
                }
            } else {
                p.pause_reason.clear();
                p.notice.clear();
            }
            true
        } else {
            false
        }
    }

    /// Pause every project. Already-paused rows keep their reason.
    pub fn pause_all(&mut self, reason: &str) -> Vec<String> {
        let mut ids = Vec::new();
        for p in &mut self.projects {
            if p.paused {
                continue;
            }
            p.paused = true;
            p.pause_reason = reason.to_string();
            p.notice = reason.to_string();
            ids.push(p.id.clone());
        }
        ids
    }

    pub fn set_yolo(&mut self, id: &str, yolo: bool) -> bool {
        self.set_yolo_mode(id, if yolo { YoloMode::All } else { YoloMode::Off })
    }

    pub fn set_yolo_mode(&mut self, id: &str, mode: YoloMode) -> bool {
        if let Some(p) = self.projects.iter_mut().find(|p| p.id == id) {
            p.yolo = mode.asks_all();
            p.yolo_mode = mode.as_str().into();
            true
        } else {
            false
        }
    }

    pub fn yolo_mode(id: &str) -> YoloMode {
        if id.is_empty() {
            return YoloMode::Off;
        }
        Self::load()
            .get(id)
            .map(|p| p.yolo_mode_enum())
            .unwrap_or(YoloMode::Off)
    }

    /// True when every ask_human takes the guess.
    pub fn yolo(id: &str) -> bool {
        Self::yolo_mode(id).asks_all()
    }

    /// True when sprint planning takes the guess (plan or all).
    pub fn yolo_plan(id: &str) -> bool {
        Self::yolo_mode(id).plans()
    }

    pub fn clear_notice(&mut self, id: &str) -> bool {
        if let Some(p) = self.projects.iter_mut().find(|p| p.id == id) {
            p.notice.clear();
            true
        } else {
            false
        }
    }

    /// Older pauses had no reason. Fill one so the desk can flash why.
    pub fn explain_pauses(&mut self) -> bool {
        let mut changed = false;
        for p in &mut self.projects {
            if p.paused && p.pause_reason.is_empty() {
                p.pause_reason = SLOT_TAKEN.into();
                if p.notice.is_empty() {
                    p.notice = SLOT_TAKEN.into();
                }
                changed = true;
            }
        }
        changed
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressPoint {
    pub day: String,
    pub remaining: i64,
    pub green: i64,
    pub total: i64,
}

/// One sample per UTC day per project, used for remaining-work sparkline.
pub fn record_progress(id: &str, remaining: i64, green: i64, total: i64) -> Vec<ProgressPoint> {
    let path = Org::home_dir().join("progress.json");
    let mut all: std::collections::BTreeMap<String, Vec<ProgressPoint>> =
        fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
    let day = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let point = ProgressPoint {
        day: day.clone(),
        remaining,
        green,
        total,
    };
    let series = all.entry(id.to_string()).or_default();
    if let Some(last) = series.last_mut() {
        if last.day == day {
            *last = point.clone();
        } else {
            series.push(point);
        }
    } else {
        series.push(point);
    }
    if series.len() > 60 {
        let drop = series.len() - 60;
        series.drain(0..drop);
    }
    let out = series.clone();
    let _ = fs::create_dir_all(Org::home_dir());
    let _ = fs::write(
        path,
        serde_json::to_string_pretty(&all).unwrap_or_default() + "\n",
    );
    out
}

pub fn progress_for(id: &str) -> Vec<ProgressPoint> {
    let path = Org::home_dir().join("progress.json");
    let all: std::collections::BTreeMap<String, Vec<ProgressPoint>> = fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    all.get(id).cloned().unwrap_or_default()
}

fn file_stamp(path: &Path) -> u64 {
    let Ok(meta) = path.metadata() else {
        return 0;
    };
    let t = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    t ^ meta.len().wrapping_mul(1_000_003)
}

fn dir_mark(dir: &Path) -> (u64, u32) {
    dir_mark_depth(dir, 0)
}

fn dir_mark_depth(dir: &Path, depth: u32) -> (u64, u32) {
    let mut max = file_stamp(dir);
    let mut n = 0u32;
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            n += 1;
            let p = e.path();
            max = max.max(file_stamp(&p));
            if depth > 0 && p.is_dir() {
                let inner = dir_mark_depth(&p, depth - 1);
                max = max.max(inner.0);
                n += inner.1;
            }
        }
    }
    (max, n)
}

/// Cheap fingerprint of the on-disk magazine (spec, drawings, journal, plan).
/// Directory listings only — not file bodies — so the live strip can skip a full reload.
pub fn document_stamp(root: &Path) -> String {
    let spec = dir_mark(&root.join("spec"));
    let mock = dir_mark(&root.join("mockups"));
    let journeys = dir_mark_depth(&root.join("mockups/journeys"), 1);
    format!(
        "s{}-{}:m{}-{}:j{}-{}:n{}:p{}:l{}",
        spec.0,
        spec.1,
        mock.0,
        mock.1,
        journeys.0,
        journeys.1,
        file_stamp(&root.join(".shalt/journal.json")),
        file_stamp(&root.join(".shalt/plan.md")),
        file_stamp(&root.join(".shalt/ledger.json")),
    )
}

#[cfg(test)]
mod tests {
    use super::document_stamp;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn document_stamp_moves_when_spec_lands() {
        let t = TempDir::new().unwrap();
        let a = document_stamp(t.path());
        fs::create_dir_all(t.path().join("spec")).unwrap();
        fs::write(t.path().join("spec/a.feature"), "Feature: A\n").unwrap();
        let b = document_stamp(t.path());
        assert_ne!(a, b);
    }

    #[test]
    fn document_stamp_moves_when_a_journey_dir_appears() {
        let t = TempDir::new().unwrap();
        fs::create_dir_all(t.path().join("mockups/journeys")).unwrap();
        let a = document_stamp(t.path());
        fs::create_dir_all(t.path().join("mockups/journeys/award")).unwrap();
        fs::write(
            t.path().join("mockups/journeys/award/storyboard.json"),
            "{}\n",
        )
        .unwrap();
        let b = document_stamp(t.path());
        assert_ne!(a, b);
        fs::write(
            t.path().join("mockups/journeys/award/inbox.html"),
            "<p>inbox</p>\n",
        )
        .unwrap();
        let c = document_stamp(t.path());
        assert_ne!(b, c);
        fs::write(
            t.path().join("mockups/journeys/award/inbox.html"),
            "<p>inbox redrawn</p>\n",
        )
        .unwrap();
        let d = document_stamp(t.path());
        assert_ne!(c, d);
    }
}
