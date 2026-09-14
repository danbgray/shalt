use std::fs;
use std::path::{Path, PathBuf};

pub trait Backend {
    fn name(&self) -> &str;
    fn run(&mut self, role: &str, prompt: &str, stage: &Path) -> Result<String, String>;
}

pub struct FixtureBackend {
    pub fixtures: PathBuf,
    turns: std::collections::HashMap<String, usize>,
}

impl FixtureBackend {
    pub fn new(fixtures: impl Into<PathBuf>) -> Self {
        Self {
            fixtures: fixtures.into(),
            turns: Default::default(),
        }
    }
}

impl Backend for FixtureBackend {
    fn name(&self) -> &str {
        "fixture"
    }

    fn run(&mut self, role: &str, _prompt: &str, stage: &Path) -> Result<String, String> {
        let n = self.turns.get(role).copied().unwrap_or(0);
        self.turns.insert(role.to_string(), n + 1);
        let dir = self.fixtures.join(role);
        let mut candidates: Vec<PathBuf> = if dir.exists() {
            fs::read_dir(&dir)
                .map_err(|e| e.to_string())?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|s| s.to_str())
                        .map(|s| s.starts_with("turn"))
                        .unwrap_or(false)
                })
                .collect()
        } else {
            vec![]
        };
        candidates.sort();
        if candidates.is_empty() {
            return Err(format!("no fixtures for role {role:?} in {}", self.fixtures.display()));
        }
        let turn = &candidates[n.min(candidates.len() - 1)];
        let mut note = String::new();
        let mut items: Vec<_> = fs::read_dir(turn)
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok())
            .collect();
        items.sort_by_key(|e| e.file_name());
        for item in items {
            let p = item.path();
            if item.file_name() == "_note.txt" {
                note = fs::read_to_string(&p).unwrap_or_default();
                continue;
            }
            let dest = stage.join(item.file_name());
            if p.is_dir() {
                copy_dir(&p, &dest).map_err(|e| e.to_string())?;
            } else {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                fs::copy(&p, &dest).map_err(|e| e.to_string())?;
            }
        }
        Ok(format!("[fixture {role} {}] {note}", turn.file_name().unwrap().to_string_lossy()).trim().into())
    }
}

fn copy_dir(src: &Path, dest: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dest)?;
    for e in fs::read_dir(src)? {
        let e = e?;
        let p = e.path();
        let d = dest.join(e.file_name());
        if p.is_dir() {
            copy_dir(&p, &d)?;
        } else {
            if let Some(parent) = d.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&p, &d)?;
        }
    }
    Ok(())
}

/// Test helper: a backend that runs an arbitrary closure against the stage.
pub struct FnBackend<F> {
    pub f: F,
}
impl<F> Backend for FnBackend<F>
where
    F: FnMut(&Path),
{
    fn name(&self) -> &str {
        "fn"
    }
    fn run(&mut self, _role: &str, _prompt: &str, stage: &Path) -> Result<String, String> {
        (self.f)(stage);
        Ok("done".into())
    }
}
