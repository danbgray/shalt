use crate::ledger::Ledger;
use crate::spec::{duplicate_rids, file_hash, Feature};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;


pub const LEDGER_FILE: &str = ".shalt/ledger.json";
pub const BOARD_FILE: &str = ".shalt/board.json";

pub static ZONES: &[(&str, &[&str])] = &[
    ("author", &["spec"]),
    ("stepwright", &["steps", "contract"]),
    ("implementer", &["src"]),
    ("human", &["spec", "steps", "contract", "src"]),
];

pub static READS: &[(&str, &[&str])] = &[
    ("author", &["spec"]),
    ("stepwright", &["spec", "steps", "contract"]),
    ("implementer", &["spec", "contract", "src"]),
];

pub const ALL_ZONES: &[&str] = &["spec", "steps", "contract", "src"];

pub fn writes_for(role: &str) -> &'static [&'static str] {
    ZONES.iter().find(|(r, _)| *r == role).map(|(_, z)| *z).unwrap_or(&[])
}
pub fn reads_for(role: &str) -> &'static [&'static str] {
    READS.iter().find(|(r, _)| *r == role).map(|(_, z)| *z).unwrap_or(&[])
}

#[derive(Debug)]
pub struct IntegrityViolation {
    pub role: String,
    pub offences: HashMap<String, Vec<String>>,
}

impl std::fmt::Display for IntegrityViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let detail: Vec<String> = self
            .offences
            .iter()
            .filter(|(_, p)| !p.is_empty())
            .map(|(z, p)| {
                let mut s = p.clone();
                s.sort();
                s.truncate(6);
                format!("{z}: {}", s.join(", "))
            })
            .collect();
        write!(
            f,
            "role '{}' wrote outside its zone -> {}",
            self.role,
            detail.join("; ")
        )
    }
}
impl std::error::Error for IntegrityViolation {}

pub fn iter_files(zdir: &Path) -> Vec<(PathBuf, bool)> {
    let mut out = Vec::new();
    if !zdir.exists() {
        return out;
    }
    let mut stack = vec![zdir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        let mut ents: Vec<_> = rd.filter_map(|e| e.ok()).collect();
        ents.sort_by_key(|e| e.file_name());
        for e in ents {
            let p = e.path();
            if p.components().any(|c| c.as_os_str() == "__pycache__") {
                continue;
            }
            let Ok(meta) = fs::symlink_metadata(&p) else { continue };
            if meta.file_type().is_symlink() {
                out.push((p, true));
            } else if meta.is_dir() {
                stack.push(p);
            } else if meta.is_file() {
                out.push((p, false));
            }
        }
    }
    out
}

pub type Snap = HashMap<String, HashMap<String, String>>;

pub fn snapshot(root: &Path, zones: &[&str]) -> Snap {
    let mut snap = Snap::new();
    for z in zones {
        let mut files = HashMap::new();
        for (p, is_link) in iter_files(&root.join(z)) {
            let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            let val = if is_link {
                format!(
                    "symlink:{}",
                    fs::read_link(&p).map(|t| t.display().to_string()).unwrap_or_default()
                )
            } else {
                file_hash(&p).unwrap_or_default()
            };
            files.insert(rel, val);
        }
        snap.insert((*z).to_string(), files);
    }
    snap
}

pub fn diff_snap(before: &Snap, after: &Snap) -> HashMap<String, Vec<String>> {
    let mut out = HashMap::new();
    let mut zones: Vec<_> = before.keys().chain(after.keys()).cloned().collect();
    zones.sort();
    zones.dedup();
    for z in zones {
        let empty = HashMap::new();
        let b = before.get(&z).unwrap_or(&empty);
        let a = after.get(&z).unwrap_or(&empty);
        let mut keys: Vec<_> = b.keys().chain(a.keys()).cloned().collect();
        keys.sort();
        keys.dedup();
        let changed: Vec<String> = keys.into_iter().filter(|p| b.get(p) != a.get(p)).collect();
        out.insert(z, changed);
    }
    out
}

pub struct GuardedTurn {
    root: PathBuf,
    role: String,
    backup_dir: PathBuf,
    _allowed: Vec<String>,
    protected: Vec<String>,
    before: Snap,
    ledger_before: Option<String>,
    ledger_bytes: Option<Vec<u8>>,
    board_before: Option<String>,
    board_bytes: Option<Vec<u8>>,
    committed: bool,
}

impl GuardedTurn {
    pub fn enter(root: &Path, role: &str, backup_dir: &Path) -> std::io::Result<Self> {
        let allowed: Vec<String> = writes_for(role).iter().map(|s| s.to_string()).collect();
        let protected: Vec<String> = ALL_ZONES
            .iter()
            .filter(|z| !allowed.iter().any(|a| a == **z))
            .map(|s| s.to_string())
            .collect();
        let before = snapshot(root, ALL_ZONES);
        let led = root.join(LEDGER_FILE);
        let ledger_before = if led.exists() { file_hash(&led).ok() } else { None };
        let ledger_bytes = if led.exists() { fs::read(&led).ok() } else { None };
        let board = root.join(BOARD_FILE);
        let board_before = if board.exists() { file_hash(&board).ok() } else { None };
        let board_bytes = if board.exists() { fs::read(&board).ok() } else { None };
        if backup_dir.exists() {
            fs::remove_dir_all(backup_dir)?;
        }
        fs::create_dir_all(backup_dir)?;
        for z in &protected {
            let src = root.join(z);
            if src.exists() {
                copy_tree(&src, &backup_dir.join(z))?;
            }
        }
        Ok(Self {
            root: root.to_path_buf(),
            role: role.to_string(),
            backup_dir: backup_dir.to_path_buf(),
            _allowed: allowed,
            protected,
            before,
            ledger_before,
            ledger_bytes,
            board_before,
            board_bytes,
            committed: false,
        })
    }

    pub fn commit(mut self) -> Result<(), IntegrityViolation> {
        let after = snapshot(&self.root, ALL_ZONES);
        let d = diff_snap(&self.before, &after);
        let mut offences: HashMap<String, Vec<String>> = HashMap::new();
        for z in &self.protected {
            if let Some(ch) = d.get(z) {
                if !ch.is_empty() {
                    offences.insert(z.clone(), ch.clone());
                }
            }
        }
        let led = self.root.join(LEDGER_FILE);
        let now_hash = if led.exists() { file_hash(&led).ok() } else { None };
        if now_hash != self.ledger_before {
            offences.entry("ledger".into()).or_default().push(LEDGER_FILE.into());
        }
        let board = self.root.join(BOARD_FILE);
        let now_board = if board.exists() { file_hash(&board).ok() } else { None };
        if now_board != self.board_before {
            offences.entry("board".into()).or_default().push(BOARD_FILE.into());
        }
        if !offences.is_empty() {
            self.restore();
            return Err(IntegrityViolation {
                role: self.role.clone(),
                offences,
            });
        }
        self.committed = true;
        Ok(())
    }

    fn restore(&self) {
        if let Some(bytes) = &self.ledger_bytes {
            let led = self.root.join(LEDGER_FILE);
            let _ = fs::create_dir_all(led.parent().unwrap());
            let _ = fs::write(&led, bytes);
        }
        if let Some(bytes) = &self.board_bytes {
            let p = self.root.join(BOARD_FILE);
            let _ = fs::create_dir_all(p.parent().unwrap());
            let _ = fs::write(&p, bytes);
        }
        for z in &self.protected {
            let tgt = self.root.join(z);
            let bak = self.backup_dir.join(z);
            if tgt.exists() {
                let _ = fs::remove_dir_all(&tgt);
            }
            if bak.exists() {
                let _ = copy_tree(&bak, &tgt);
                bump_mtimes(&tgt);
            }
        }
    }
}

impl Drop for GuardedTurn {
    fn drop(&mut self) {
        if !self.committed {
            self.restore();
        }
    }
}

fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for (p, is_link) in iter_files(src) {
        if is_link {
            continue;
        }
        let rel = p.strip_prefix(src).unwrap();
        let t = dst.join(rel);
        if let Some(parent) = t.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&p, &t)?;
    }
    Ok(())
}

fn bump_mtimes(dir: &Path) {
    let now = filetime_now();
    for (p, is_link) in iter_files(dir) {
        if !is_link {
            let _ = filetime_set(&p, now);
        }
    }
}

fn filetime_now() -> SystemTime {
    SystemTime::now()
}

fn filetime_set(p: &Path, t: SystemTime) -> std::io::Result<()> {
    let f = fs::File::options().write(true).open(p)?;
    f.set_modified(t)
}

pub fn audit(ledger: &Ledger, features: &[Feature]) -> Vec<String> {
    let mut problems = Vec::new();
    for (file, name) in duplicate_rids(features) {
        problems.push(format!("duplicate scenario id: {file} :: {name}"));
    }
    for f in features {
        for s in &f.scenarios {
            if s.rid.is_none() {
                problems.push(format!("unstamped scenario (run `shalt approve`): {} :: {}", f.file, s.name));
            }
        }
    }
    for e in ledger.by_status(crate::ledger::ORPHAN) {
        problems.push(format!("orphan: ledger has '{}' ({}) but the spec no longer does", e.name, e.rid));
    }
    for e in ledger.entries.values() {
        if e.status == crate::ledger::GREEN && e.verified_spec_hash != e.spec_hash {
            problems.push(format!("green but unverified against current spec: {} {}", e.rid, e.name));
        }
        if e.status == crate::ledger::GREEN && e.mutants_killed == Some(0) {
            problems.push(format!(
                "vacuous: {} '{}' is green but detected no mutation — its step definitions may not assert what the scenario says",
                e.rid, e.name
            ));
        } else if e.status == crate::ledger::GREEN {
            if let Some(n) = e.blind_spots {
                if n > 0 {
                    problems.push(format!(
                        "weak oracle: {} '{}' ran {n} mutated version(s) of code it executes without noticing",
                        e.rid, e.name
                    ));
                }
            }
        }
    }
    let lock = &ledger.spec_lock;
    if lock.is_null() || lock.as_object().map(|o| o.is_empty()).unwrap_or(true) {
        problems.push("spec is not approved: no human sign-off recorded".into());
        return problems;
    }
    let approved = lock.get("scenario_hashes").and_then(|h| h.as_object());
    if let Some(approved) = approved {
        if approved.is_empty() {
            problems.push("spec lock predates content hashing; re-run `shalt approve`".into());
            return problems;
        }
        let mut current = HashMap::new();
        for f in features {
            for s in &f.scenarios {
                if let Some(rid) = &s.rid {
                    current.insert(rid.clone(), s.spec_hash(&f.background));
                }
            }
        }
        for (rid, h) in approved {
            let h = h.as_str().unwrap_or("");
            match current.get(rid) {
                None => problems.push(format!("approved scenario {rid} is no longer in the spec")),
                Some(ch) if ch != h => {
                    let name = features
                        .iter()
                        .flat_map(|f| f.scenarios.iter())
                        .find(|s| s.rid.as_deref() == Some(rid))
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| rid.clone());
                    problems.push(format!("scenario changed since approval, unapproved: {rid} {name}"));
                }
                _ => {}
            }
        }
        for rid in current.keys() {
            if !approved.contains_key(rid) {
                problems.push(format!("scenario added since approval, unapproved: {rid}"));
            }
        }
    } else {
        problems.push("spec lock predates content hashing; re-run `shalt approve`".into());
    }
    problems
}
