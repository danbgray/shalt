use crate::config::Config;
use crate::integrity::{
    diff_snap, iter_files, read_zones, snapshot, write_zones, GuardedTurn, IntegrityViolation,
};
use crate::spec::{load_specs, strip_holdouts};
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
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
}

fn copy_meta(root: &Path, stage: &Path, name: &str) -> std::io::Result<()> {
    let src = root.join(name);
    if src.is_file() {
        fs::copy(&src, stage.join(name))?;
    }
    Ok(())
}

fn copy_rel(root: &Path, stage: &Path, rel: &str) -> std::io::Result<()> {
    let src = root.join(rel);
    if !src.is_file() {
        return Ok(());
    }
    let tgt = stage.join(rel);
    if let Some(parent) = tgt.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(&src, &tgt)?;
    Ok(())
}

/// Files a focused stepwright turn may see. Other step files stay off the stage.
pub fn focused_stepwright_rels(root: &Path, focus: &str) -> Vec<String> {
    let cfg = Config::load(root).unwrap_or_default();
    let mut out = Vec::new();
    if root.join("shalt.toml").is_file() {
        out.push("shalt.toml".into());
    }
    if root.join("Cargo.toml").is_file() {
        out.push("Cargo.toml".into());
    }
    if let Ok(features) = load_specs(&root.join("spec"), false) {
        for f in features {
            if crate::bindings::journey_of(&f) == focus {
                let file = f.file.replace('\\', "/");
                let rel = if file.starts_with("spec/") {
                    file
                } else {
                    format!("spec/{file}")
                };
                if root.join(&rel).is_file() {
                    out.push(rel);
                }
            }
        }
    }
    if cfg.stack == "javascript" {
        let world = format!("{}/world.js", cfg.steps.trim_end_matches('/'));
        if root.join(&world).is_file() {
            out.push(world);
        }
        let steps = format!(
            "{}/{focus}.steps.js",
            cfg.steps.trim_end_matches('/')
        );
        if root.join(&steps).is_file() {
            out.push(steps);
        }
    } else {
        if root.join("tests/shalt.rs").is_file() {
            out.push("tests/shalt.rs".into());
        }
    }
    if root.join("contract/interface.md").is_file() {
        out.push("contract/interface.md".into());
    }
    out
}

fn stage_for(
    root: &Path,
    role: &str,
    stage: &Path,
    hide_holdouts: bool,
    cfg: &Config,
    focus_journey: &str,
) -> std::io::Result<()> {
    fs::create_dir_all(stage)?;
    copy_meta(root, stage, "shalt.toml")?;
    copy_meta(root, stage, "Cargo.toml")?;
    if role == "stepwright" && !focus_journey.trim().is_empty() {
        for rel in focused_stepwright_rels(root, focus_journey) {
            if rel == "shalt.toml" || rel == "Cargo.toml" {
                continue;
            }
            copy_rel(root, stage, &rel)?;
        }
        let ft = root.join(".shalt/fill-target.json");
        if ft.is_file() {
            let dest = stage.join(".shalt/fill-target.json");
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            let _ = fs::copy(&ft, &dest);
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
        return Ok(());
    }
    let mut zones = read_zones(role, &cfg.steps, &cfg.src);
    for z in write_zones(role, &cfg.steps, &cfg.src) {
        if !zones.contains(&z) {
            zones.push(z);
        }
    }
    zones.sort();
    zones.dedup();
    for z in &zones {
        let src = root.join(z);
        fs::create_dir_all(stage.join(z))?;
        if !src.exists() {
            continue;
        }
        for (p, is_link) in iter_files(&src) {
            if is_link {
                continue;
            }
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if name.contains(".thumb.") || name.ends_with(".thumb.svg") || name == ".DS_Store" {
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

fn stage_offences(stage: &Path, role: &str, cfg: &Config) -> HashMap<String, Vec<String>> {
    let allowed = write_zones(role, &cfg.steps, &cfg.src);
    let readable = read_zones(role, &cfg.steps, &cfg.src);
    let mut offences: HashMap<String, Vec<String>> = HashMap::new();
    for (p, is_link) in iter_files(stage) {
        let rel = p.strip_prefix(stage).unwrap();
        let top = rel
            .components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .unwrap_or_else(|| "<stage root>".into());
        if top == "shalt.toml" || top == "Cargo.toml" {
            continue;
        }
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
        if allowed.iter().any(|z| z == &top) || readable.iter().any(|z| z == &top) {
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

fn allowed_focused_writes(cfg: &Config, focus: &str) -> Vec<String> {
    let mut v = Vec::new();
    if cfg.stack == "javascript" {
        v.push(format!(
            "{}/{focus}.steps.js",
            cfg.steps.trim_end_matches('/')
        ));
    } else {
        v.push("tests/shalt.rs".into());
    }
    v
}

pub fn run_role(
    root: &Path,
    role: &str,
    prompt: &str,
    backend: &mut dyn Backend,
    hide_holdouts: bool,
) -> Result<RoleResult, RoleError> {
    run_role_focused(root, role, prompt, backend, hide_holdouts, "")
}

pub fn run_role_focused(
    root: &Path,
    role: &str,
    prompt: &str,
    backend: &mut dyn Backend,
    hide_holdouts: bool,
    focus_journey: &str,
) -> Result<RoleResult, RoleError> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let cfg = Config::load(&root).unwrap_or_default();
    let parent = tempfile::Builder::new().prefix("shalt-stage-").tempdir()?;
    let stage = parent.path().join(role);
    stage_for(&root, role, &stage, hide_holdouts, &cfg, focus_journey)?;
    let writes = write_zones(role, &cfg.steps, &cfg.src);
    let reads = read_zones(role, &cfg.steps, &cfg.src);
    let mut all = reads.clone();
    for z in &writes {
        if !all.contains(z) {
            all.push(z.clone());
        }
    }
    let zone_refs: Vec<&str> = all.iter().map(|s| s.as_str()).collect();
    let before = snapshot(&stage, &zone_refs);
    let backup = tempfile::Builder::new().prefix("shalt-backup-").tempdir()?;
    let guard = GuardedTurn::enter(&root, role, backup.path())?;
    let transcript = backend.run(role, prompt, &stage).map_err(RoleError::Other)?;
    let (prompt_tokens, completion_tokens) = backend.usage();

    let mut offences = stage_offences(&stage, role, &cfg);
    let d = diff_snap(&before, &snapshot(&stage, &zone_refs));
    for z in &reads {
        if !writes.iter().any(|w| w == z) {
            if let Some(ch) = d.get(z) {
                if !ch.is_empty() {
                    offences.entry(z.clone()).or_default().extend(ch.clone());
                }
            }
        }
    }
    if role == "stepwright" && !focus_journey.trim().is_empty() {
        let allow = allowed_focused_writes(&cfg, focus_journey);
        for z in &writes {
            if let Some(ch) = d.get(z) {
                for rel in ch {
                    let norm = rel.replace('\\', "/");
                    if !allow.iter().any(|a| a == &norm) {
                        offences.entry("focus".into()).or_default().push(norm);
                    }
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
    let write_refs: Vec<&str> = writes.iter().map(|s| s.as_str()).collect();
    let (wrote, removed) = mirror_back(&stage, &root, &write_refs)?;
    guard.commit()?;
    Ok(RoleResult {
        role: role.to_string(),
        transcript,
        wrote,
        removed,
        prompt_tokens,
        completion_tokens,
    })
}
