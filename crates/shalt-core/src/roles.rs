use crate::integrity::{
    diff_snap, iter_files, reads_for, snapshot, writes_for, GuardedTurn, IntegrityViolation, ALL_ZONES,
};
use crate::spec::strip_holdouts;
use crate::Backend;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum RoleError {
    Integrity(IntegrityViolation),
    Other(String),
}

impl std::fmt::Display for RoleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RoleError::Integrity(e) => write!(f, "{e}"),
            RoleError::Other(s) => write!(f, "{s}"),
        }
    }
}
impl std::error::Error for RoleError {}
impl From<IntegrityViolation> for RoleError {
    fn from(e: IntegrityViolation) -> Self {
        RoleError::Integrity(e)
    }
}
impl From<std::io::Error> for RoleError {
    fn from(e: std::io::Error) -> Self {
        RoleError::Other(e.to_string())
    }
}

#[derive(Debug)]
pub struct RoleResult {
    pub role: String,
    pub transcript: String,
    pub wrote: Vec<String>,
    pub removed: Vec<String>,
}

fn stage_for(root: &Path, role: &str, stage: &Path, hide_holdouts: bool) -> std::io::Result<()> {
    fs::create_dir_all(stage)?;
    let mut zones: Vec<&str> = reads_for(role).iter().chain(writes_for(role).iter()).copied().collect();
    zones.sort();
    zones.dedup();
    for z in zones {
        let src = root.join(z);
        fs::create_dir_all(stage.join(z))?;
        if !src.exists() {
            continue;
        }
        for (p, is_link) in iter_files(&src) {
            if is_link {
                continue;
            }
            let rel = p.strip_prefix(&src).unwrap();
            let tgt = stage.join(z).join(rel);
            if let Some(parent) = tgt.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&p, &tgt)?;
        }
    }
    if hide_holdouts {
        let spec = stage.join("spec");
        if spec.exists() {
            for (p, is_link) in iter_files(&spec) {
                if is_link {
                    continue;
                }
                if p.extension().and_then(|s| s.to_str()) == Some("feature") {
                    let raw = fs::read_to_string(&p)?;
                    fs::write(&p, strip_holdouts(&raw))?;
                }
            }
        }
    }
    Ok(())
}

fn stage_offences(stage: &Path, role: &str) -> HashMap<String, Vec<String>> {
    let allowed: Vec<&str> = writes_for(role).to_vec();
    let readable: Vec<&str> = reads_for(role).to_vec();
    let mut offences: HashMap<String, Vec<String>> = HashMap::new();
    for (p, is_link) in iter_files(stage) {
        let rel = p.strip_prefix(stage).unwrap();
        let top = rel
            .components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .unwrap_or_else(|| "<stage root>".into());
        if is_link {
            let target = fs::read_link(&p)
                .map(|t| t.display().to_string())
                .unwrap_or_default();
            offences
                .entry("symlink".into())
                .or_default()
                .push(format!("{} -> {target}", rel.display()));
            continue;
        }
        if allowed.iter().any(|z| *z == top) || readable.iter().any(|z| *z == top) {
            continue;
        }
        offences.entry(top).or_default().push(rel.display().to_string());
    }
    offences
}

fn mirror_back(stage: &Path, root: &Path, zones: &[&str]) -> std::io::Result<(Vec<String>, Vec<String>)> {
    let mut wrote = Vec::new();
    let mut removed = Vec::new();
    for z in zones {
        let src = stage.join(z);
        let dest = root.join(z);
        fs::create_dir_all(&dest)?;
        let staged: HashMap<String, PathBuf> = iter_files(&src)
            .into_iter()
            .filter(|(_, l)| !*l)
            .map(|(p, _)| {
                (
                    p.strip_prefix(&src).unwrap().to_string_lossy().replace('\\', "/"),
                    p,
                )
            })
            .collect();
        let present: HashMap<String, PathBuf> = iter_files(&dest)
            .into_iter()
            .filter(|(_, l)| !*l)
            .map(|(p, _)| {
                (
                    p.strip_prefix(&dest).unwrap().to_string_lossy().replace('\\', "/"),
                    p,
                )
            })
            .collect();
        let mut staged_keys: Vec<_> = staged.keys().cloned().collect();
        staged_keys.sort();
        for rel in staged_keys {
            let p = &staged[&rel];
            let tgt = dest.join(&rel);
            if let Some(parent) = tgt.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(p, &tgt)?;
            wrote.push(format!("{z}/{rel}"));
        }
        let mut present_keys: Vec<_> = present.keys().cloned().collect();
        present_keys.sort();
        for rel in present_keys {
            if !staged.contains_key(&rel) {
                fs::remove_file(&present[&rel])?;
                removed.push(format!("{z}/{rel}"));
            }
        }
    }
    Ok((wrote, removed))
}

pub fn run_role(
    root: &Path,
    role: &str,
    prompt: &str,
    backend: &mut dyn Backend,
    hide_holdouts: bool,
) -> Result<RoleResult, RoleError> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let parent = tempfile::Builder::new().prefix("shalt-stage-").tempdir()?;
    let stage = parent.path().join(role);
    stage_for(&root, role, &stage, hide_holdouts)?;
    let before = snapshot(&stage, ALL_ZONES);
    let backup = tempfile::Builder::new().prefix("shalt-backup-").tempdir()?;
    let guard = GuardedTurn::enter(&root, role, backup.path())?;
    let transcript = backend.run(role, prompt, &stage).map_err(RoleError::Other)?;

    let mut offences = stage_offences(&stage, role);
    let d = diff_snap(&before, &snapshot(&stage, ALL_ZONES));
    for z in reads_for(role) {
        if !writes_for(role).contains(z) {
            if let Some(ch) = d.get(*z) {
                if !ch.is_empty() {
                    offences.entry((*z).to_string()).or_default().extend(ch.clone());
                }
            }
        }
    }
    if !offences.is_empty() {
        drop(guard);
        return Err(RoleError::Integrity(IntegrityViolation {
            role: role.to_string(),
            offences,
        }));
    }
    let (wrote, removed) = mirror_back(&stage, &root, writes_for(role))?;
    guard.commit()?;
    Ok(RoleResult {
        role: role.to_string(),
        transcript,
        wrote,
        removed,
    })
}
