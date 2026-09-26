//! Portable plan pack: English plan + Gherkin + model pool. Import starts a new project to implement.

use crate::board::{Board, PoolSlot};
use crate::compose::resolve_new_project_dir;
use crate::config::init_plan_workspace;
use crate::ledger::Ledger;
use crate::org::{Org, ProjectRef};
use crate::spec::{load_specs, put_spec_file, stamp_rids};
use crate::talk::{load_plan, save_plan};
use crate::tokens::stamp_forecasts;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub const SCHEMA: &str = "shalt.plan/1";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlanPack {
    #[serde(default = "schema")]
    pub schema: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub plan: String,
    /// Feature path → body. Keys may be `foo.feature` or `spec/foo.feature`.
    #[serde(default)]
    pub features: BTreeMap<String, String>,
    #[serde(default)]
    pub pool: Vec<PoolSlot>,
    #[serde(default)]
    pub prefer: String,
}

fn schema() -> String {
    SCHEMA.into()
}

/// Snapshot a project's plan + spec so another machine can start implementing it.
pub fn export_pack(root: &Path, name: &str) -> Result<PlanPack, String> {
    let spec = root.join("spec");
    let features = load_specs(&spec, false).map_err(|e| e.to_string())?;
    let mut map = BTreeMap::new();
    for f in features {
        let path = spec.join(&f.file);
        let body = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let key = if f.file.starts_with("spec/") {
            f.file
        } else {
            format!("spec/{}", f.file)
        };
        map.insert(key, body);
    }
    let board = Board::load(&root.join(".shalt/board.json"));
    Ok(PlanPack {
        schema: SCHEMA.into(),
        name: name.trim().to_string(),
        plan: load_plan(root, ""),
        features: map,
        pool: if board.pool.is_empty() {
            crate::alloc::default_pool()
        } else {
            board.pool
        },
        prefer: if board.prefer.is_empty() {
            "balanced".into()
        } else {
            board.prefer
        },
    })
}

/// Create a new project from a pack. Writes spec + plan. Does not Play.
pub fn import_pack(
    pack: &PlanPack,
    dir: Option<&Path>,
    name: Option<&str>,
) -> Result<ProjectRef, String> {
    if pack.features.is_empty() && pack.plan.trim().is_empty() {
        return Err("plan pack has no spec and no plan".into());
    }
    let name = name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| {
            let n = pack.name.trim();
            if n.is_empty() {
                None
            } else {
                Some(n.to_string())
            }
        })
        .unwrap_or_else(|| "imported".into());
    let dest = resolve_new_project_dir(dir, &name);
    fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    init_plan_workspace(&dest, &name)?;
    if !pack.plan.trim().is_empty() {
        save_plan(&dest, &pack.plan).map_err(|e| e.to_string())?;
    }
    let spec_dir = dest.join("spec");
    for (file, body) in &pack.features {
        put_spec_file(&spec_dir, file, body)?;
    }
    let _ = stamp_rids(&spec_dir);
    let features = load_specs(&spec_dir, false).map_err(|e| e.to_string())?;
    let mut led = Ledger::load(&dest.join(".shalt/ledger.json")).unwrap_or_default();
    led.sync_spec(&features);
    led.save(&dest.join(".shalt/ledger.json"))
        .map_err(|e| e.to_string())?;
    let mut board = Board::load(&dest.join(".shalt/board.json"));
    board.sync_new_rids(&features);
    board.sync_epics(&features);
    if !pack.pool.is_empty() {
        board.pool = pack.pool.clone();
    }
    if !pack.prefer.trim().is_empty() {
        board.prefer = pack.prefer.clone();
    } else if board.prefer.is_empty() {
        board.prefer = "balanced".into();
    }
    stamp_forecasts(&mut board, &led, &[], "", &features);
    board
        .save(&dest.join(".shalt/board.json"))
        .map_err(|e| e.to_string())?;
    let mut org = Org::load();
    let project = org.add(&dest)?;
    org.save().map_err(|e| e.to_string())?;
    Ok(project)
}
