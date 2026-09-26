//! Cheap files shalt writes so the model fills modules instead of inventing a tree.

use crate::bindings::{
    parse_step_defs, step_has_table, step_kw_and_phrase, StepDef,
};
use crate::config::Config;
use crate::runner::list_step_files;
use crate::spec::load_specs;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

pub const JS_WORLD: &str = r#"import { setWorldConstructor } from '@cucumber/cucumber';

function World() {
  this.currentUser = null;
  this.lastError = null;
}

setWorldConstructor(World);
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractFn {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractModule {
    pub path: String,
    pub fns: Vec<ContractFn>,
}

/// Parse `## \`../src/foo.js\`` headings and `- \`name(\`` export lines.
pub fn parse_js_contract(md: &str) -> Vec<ContractModule> {
    let mut out = Vec::new();
    let mut current: Option<ContractModule> = None;
    for line in md.lines() {
        let t = line.trim();
        if let Some(path) = heading_src(t) {
            if let Some(m) = current.take() {
                if !m.fns.is_empty() {
                    out.push(m);
                }
            }
            current = Some(ContractModule {
                path,
                fns: Vec::new(),
            });
            continue;
        }
        if let Some(name) = fn_name(t) {
            if let Some(m) = current.as_mut() {
                if !m.fns.iter().any(|f| f.name == name) {
                    m.fns.push(ContractFn { name });
                }
            }
        }
    }
    if let Some(m) = current {
        if !m.fns.is_empty() {
            out.push(m);
        }
    }
    out
}

fn heading_src(line: &str) -> Option<String> {
    let rest = line.strip_prefix("##")?;
    let rest = rest.trim();
    let inner = if let Some(s) = rest.strip_prefix('`') {
        s.split('`').next()?
    } else {
        rest
    };
    let inner = inner.trim();
    let path = inner
        .trim_start_matches("./")
        .trim_start_matches("../")
        .replace('\\', "/");
    if path.starts_with("src/") && path.ends_with(".js") {
        Some(path)
    } else {
        None
    }
}

fn fn_name(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix('-')?.trim();
    let rest = rest.strip_prefix('`')?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() || !rest[name.len()..].starts_with('(') {
        return None;
    }
    Some(name)
}

pub fn js_stub_source(module: &ContractModule) -> String {
    let mut s = String::from("/** Stub from contract/interface.md. Implementer fills the bodies. */\n");
    for f in &module.fns {
        s.push_str(&format!(
            "export function {}(...args) {{\n  throw new Error(\"not implemented: {}\");\n}}\n",
            f.name, f.name
        ));
    }
    s
}

pub fn is_fillable_stub(body: &str) -> bool {
    let t = body.trim();
    t.contains("not implemented:")
        || t.contains("The implementer writes this zone")
        || t == "export {};"
        || t.ends_with("export {};")
}

pub fn write_js_world_if_missing(root: &Path) -> Result<Option<PathBuf>, String> {
    let cfg = Config::load(root).unwrap_or_default();
    if cfg.stack != "javascript" {
        return Ok(None);
    }
    let path = root.join(&cfg.steps).join("world.js");
    if path.exists() {
        return Ok(None);
    }
    std::fs::create_dir_all(root.join(&cfg.steps)).map_err(|e| e.to_string())?;
    std::fs::write(&path, JS_WORLD).map_err(|e| e.to_string())?;
    Ok(Some(path))
}

/// Quarantine dir is outside `steps/` so cucumber `--import steps/**/*.js` cannot see copies.
pub const DUP_STEPS_DIR: &str = ".shalt/dup-steps";

pub fn steps_source_ok(path: &str, body: &str) -> bool {
    if body.contains("function (not arrow)") {
        return false;
    }
    let name = Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if name == "world.js" {
        return true;
    }
    if body.trim().is_empty() {
        return true;
    }
    let defs = parse_step_defs(body);
    if defs.is_empty() {
        return false;
    }
    let mut seen = HashSet::new();
    for d in &defs {
        if !seen.insert(phrase_key(d)) {
            return false;
        }
    }
    true
}

fn phrase_key(d: &StepDef) -> String {
    format!("{}|{}", d.kw, d.pattern)
}

/// Move exact-duplicate / same-phrase-set step files out of the oracle zone.
pub fn quarantine_duplicate_step_files(root: &Path) -> Result<Vec<String>, String> {
    let cfg = Config::load(root).unwrap_or_default();
    let files = list_step_files(root, &cfg.steps);
    let mut by_bytes: HashMap<String, Vec<String>> = HashMap::new();
    let mut by_phrases: HashMap<String, Vec<String>> = HashMap::new();
    for rel in &files {
        let name = Path::new(rel)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if name == "world.js" {
            continue;
        }
        let body = std::fs::read_to_string(root.join(rel)).unwrap_or_default();
        by_bytes.entry(body.clone()).or_default().push(rel.clone());
        let defs = parse_step_defs(&body);
        if defs.is_empty() {
            continue;
        }
        let mut keys: Vec<String> = defs.iter().map(phrase_key).collect();
        keys.sort();
        by_phrases.entry(keys.join("\n")).or_default().push(rel.clone());
    }
    let mut move_set: BTreeSet<String> = BTreeSet::new();
    for group in by_bytes.values().chain(by_phrases.values()) {
        if group.len() < 2 {
            continue;
        }
        let mut g = group.clone();
        g.sort();
        for rel in g.into_iter().skip(1) {
            move_set.insert(rel);
        }
    }
    if move_set.is_empty() {
        return Ok(Vec::new());
    }
    let dest_dir = root.join(DUP_STEPS_DIR);
    std::fs::create_dir_all(&dest_dir).map_err(|e| e.to_string())?;
    let mut moved = Vec::new();
    for rel in move_set {
        let src = root.join(&rel);
        let base = Path::new(&rel)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "dup.js".into());
        let mut dest = dest_dir.join(&base);
        let mut n = 2;
        while dest.exists() {
            dest = dest_dir.join(format!("{n}-{base}"));
            n += 1;
        }
        std::fs::rename(&src, &dest).map_err(|e| e.to_string())?;
        moved.push(rel);
    }
    Ok(moved)
}

#[derive(Debug, Clone)]
struct NeededStep {
    kw: String,
    phrase: String,
    has_table: bool,
}

fn journey_phrases(root: &Path, journey: &str) -> Vec<NeededStep> {
    let features = load_specs(&root.join("spec"), false).unwrap_or_default();
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for f in &features {
        if crate::bindings::journey_of(f) != journey {
            continue;
        }
        let mut last = String::new();
        let mut all = f.background.clone();
        for s in &f.scenarios {
            all.extend(s.steps.clone());
        }
        for st in all {
            if st.lines().next().unwrap_or("").trim().starts_with('|') {
                continue;
            }
            let (kw, phrase) = step_kw_and_phrase(&st, &last);
            last = kw.clone();
            let key = format!("{kw}|{phrase}");
            if !seen.insert(key) {
                continue;
            }
            out.push(NeededStep {
                kw,
                phrase,
                has_table: step_has_table(&st),
            });
        }
    }
    out
}

fn defs_cover(defs: &[StepDef], step: &NeededStep) -> bool {
    defs.iter().any(|d| d.kw == step.kw && d.pattern == step.phrase)
}

fn capture_args(phrase: &str, has_table: bool) -> String {
    let n = ["{string}", "{int}", "{float}", "{word}"]
        .iter()
        .map(|t| phrase.matches(t).count())
        .sum::<usize>();
    let mut args: Vec<String> = (0..n).map(|i| format!("a{i}")).collect();
    if has_table {
        args.push("table".into());
    }
    args.join(", ")
}

fn js_step_fn(step: &NeededStep) -> String {
    let kw = match step.kw.as_str() {
        "when" => "When",
        "then" => "Then",
        _ => "Given",
    };
    let args = capture_args(&step.phrase, step.has_table);
    format!(
        "{kw}('{phrase}', function ({args}) {{\n  return 'pending';\n}});\n",
        phrase = step.phrase.replace('\\', "\\\\").replace('\'', "\\'"),
        args = args
    )
}

fn js_stub_file(steps: &[NeededStep]) -> String {
    let mut s = String::from(
        "import { Given, When, Then } from '@cucumber/cucumber';\nimport assert from 'node:assert/strict';\n\n",
    );
    for st in steps {
        s.push_str(&js_step_fn(st));
        s.push('\n');
    }
    s
}

fn rust_fn_name(step: &NeededStep, used: &mut HashSet<String>) -> String {
    let mut s = format!("{}_", step.kw);
    for c in step.phrase.chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if !s.ends_with('_') {
            s.push('_');
        }
    }
    let mut base: String = s.trim_matches('_').chars().take(50).collect();
    if base.is_empty() {
        base = format!("{}_step", step.kw);
    }
    let mut name = base.clone();
    let mut n = 2;
    while used.contains(&name) {
        name = format!("{base}_{n}");
        n += 1;
    }
    used.insert(name.clone());
    name
}

fn rust_step_fn(step: &NeededStep, used: &mut HashSet<String>) -> String {
    let name = rust_fn_name(step, used);
    let n = ["{string}", "{int}", "{float}", "{word}"]
        .iter()
        .map(|t| step.phrase.matches(t).count())
        .sum::<usize>();
    let mut args = String::from("_w: &mut W");
    for i in 0..n {
        args.push_str(&format!(", _a{i}: String"));
    }
    format!(
        "#[{}(expr = \"{}\")]\nfn {name}({args}) {{ todo!() }}\n",
        step.kw,
        step.phrase.replace('\\', "\\\\").replace('"', "\\\"")
    )
}

const RUST_HARNESS_HEAD: &str = r#"use cucumber::{given, then, when, World};

#[derive(Debug, Default, World)]
pub struct W {}

"#;

/// Emit pending step signatures for `journey`. Replaces garbage. Does not clobber a real oracle.
pub fn apply_step_stubs(root: &Path, journey: &str) -> Result<Vec<String>, String> {
    let cfg = Config::load(root).unwrap_or_default();
    let needed = journey_phrases(root, journey);
    if needed.is_empty() {
        return Ok(Vec::new());
    }
    match cfg.stack.as_str() {
        "javascript" => apply_js_step_stubs(root, journey, &needed),
        "rust" | "" => apply_rust_step_stubs(root, &needed),
        _ => Ok(Vec::new()),
    }
}

fn apply_js_step_stubs(
    root: &Path,
    journey: &str,
    needed: &[NeededStep],
) -> Result<Vec<String>, String> {
    let cfg = Config::load(root).unwrap_or_default();
    let rel = format!("{}/{journey}.steps.js", cfg.steps.trim_end_matches('/'));
    let dest = root.join(&rel);
    let others: Vec<StepDef> = list_step_files(root, &cfg.steps)
        .into_iter()
        .filter(|p| p != &rel)
        .flat_map(|p| {
            let body = std::fs::read_to_string(root.join(&p)).unwrap_or_default();
            parse_step_defs(&body)
        })
        .collect();
    let missing: Vec<NeededStep> = needed
        .iter()
        .filter(|s| !defs_cover(&others, s))
        .cloned()
        .collect();
    if dest.exists() {
        let body = std::fs::read_to_string(&dest).unwrap_or_default();
        if steps_source_ok(&rel, &body) {
            let own = parse_step_defs(&body);
            let extra: Vec<NeededStep> = missing
                .iter()
                .filter(|s| !defs_cover(&own, s))
                .cloned()
                .collect();
            if extra.is_empty() {
                return Ok(Vec::new());
            }
            let mut out = body;
            if !out.ends_with('\n') {
                out.push('\n');
            }
            for st in &extra {
                out.push('\n');
                out.push_str(&js_step_fn(st));
            }
            std::fs::write(&dest, out).map_err(|e| e.to_string())?;
            return Ok(vec![rel]);
        }
    }
    if missing.is_empty() {
        return Ok(Vec::new());
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&dest, js_stub_file(&missing)).map_err(|e| e.to_string())?;
    Ok(vec![rel])
}

fn apply_rust_step_stubs(root: &Path, needed: &[NeededStep]) -> Result<Vec<String>, String> {
    let rel = "tests/shalt.rs";
    let dest = root.join(rel);
    let body = if dest.exists() {
        std::fs::read_to_string(&dest).unwrap_or_default()
    } else {
        String::new()
    };
    let own = parse_step_defs(&body);
    let extra: Vec<NeededStep> = needed
        .iter()
        .filter(|s| !defs_cover(&own, s))
        .cloned()
        .collect();
    if extra.is_empty() {
        return Ok(Vec::new());
    }
    let mut used: HashSet<String> = HashSet::new();
    let mut block = String::new();
    for st in &extra {
        block.push_str(&rust_step_fn(st, &mut used));
        block.push('\n');
    }
    let out = if !steps_source_ok(rel, &body) || own.is_empty() {
        format!("{RUST_HARNESS_HEAD}{block}\nfn main() {{}}\n")
    } else if let Some(idx) = body.rfind("fn main") {
        format!("{}{}\n{}", &body[..idx], block, &body[idx..])
    } else {
        format!("{body}\n{block}")
    };
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&dest, out).map_err(|e| e.to_string())?;
    Ok(vec![rel.into()])
}

/// Fill `return 'pending'` bodies by calling `src/` exports. The writer dump-loop
/// does not bind; this is the mechanical oracle so the board can move.
pub fn fill_js_pending_oracles(root: &Path, journey: &str) -> Result<Vec<String>, String> {
    if journey.trim().is_empty() {
        return Ok(Vec::new());
    }
    let cfg = Config::load(root).unwrap_or_default();
    let rel = {
        let under_steps = format!("steps/{journey}.steps.js");
        let under_cfg = format!("{}/{journey}.steps.js", cfg.steps.trim_end_matches('/'));
        if root.join(&under_steps).is_file() {
            under_steps
        } else if root.join(&under_cfg).is_file() {
            under_cfg
        } else {
            return Ok(Vec::new());
        }
    };
    let path = root.join(&rel);
    let Ok(src) = std::fs::read_to_string(&path) else {
        return Ok(Vec::new());
    };
    let header = regex::Regex::new(
        r"^(Given|When|Then)\('((?:\\'|[^'])*)', function \(([^)]*)\) \{",
    )
    .map_err(|e| e.to_string())?;
    let mut filled = Vec::new();
    let mut used: BTreeSet<String> = BTreeSet::new();
    let mut seen_pats: HashSet<String> = HashSet::new();
    let mut out = String::new();
    let mut lines = src.lines().peekable();
    while let Some(line) = lines.next() {
        if let Some(caps) = header.captures(line.trim()) {
            let kw = caps[1].to_string();
            let pat = caps[2].to_string();
            let args = caps[3].to_string();
            let mut body_lines = Vec::new();
            let mut closed = false;
            while let Some(next) = lines.next() {
                if next.trim() == "});" {
                    closed = true;
                    break;
                }
                body_lines.push(next.to_string());
            }
            if let Some((body, imports)) = js_oracle_body(&kw, &pat, &args) {
                if !seen_pats.insert(pat.clone()) {
                    continue;
                }
                filled.push(pat.clone());
                for i in imports {
                    used.insert(i);
                }
                out.push_str(&format!("{kw}('{pat}', function ({args}) {{\n{body}\n}});\n"));
                continue;
            }
            out.push_str(line);
            out.push('\n');
            for l in &body_lines {
                out.push_str(l);
                out.push('\n');
            }
            if closed {
                out.push_str("});\n");
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if filled.is_empty() {
        return Ok(Vec::new());
    }
    if !used.is_empty() {
        let _ = fill_js_store_if_stub(root);
        let mut by_file: std::collections::BTreeMap<String, BTreeSet<String>> =
            std::collections::BTreeMap::new();
        by_file
            .entry("../src/store.js".into())
            .or_default()
            .extend(used.iter().cloned());
        out = upsert_js_imports(out, by_file);
    }
    std::fs::write(&path, out).map_err(|e| e.to_string())?;
    Ok(filled)
}

/// Stub + fill every journey's step file. 8B will not write; shalt must.
pub fn fill_all_js_pending_oracles(root: &Path) -> Result<Vec<String>, String> {
    let features = crate::spec::load_specs(&root.join("spec"), false).unwrap_or_default();
    let mut journeys = BTreeSet::new();
    for f in &features {
        let j = crate::bindings::journey_of(f);
        if !j.is_empty() {
            journeys.insert(j);
        }
    }
    let mut all = Vec::new();
    for j in journeys {
        let _ = apply_step_stubs(root, &j);
        all.extend(fill_js_pending_oracles(root, &j)?);
    }
    ensure_js_world_resets(root);
    let _ = fill_js_store_if_stub(root);
    if !all.is_empty() {
        let p = root.join(".shalt/steps.sha");
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, "rescan\n");
    }
    Ok(all)
}

fn ensure_js_world_resets(root: &Path) {
    let cfg = Config::load(root).unwrap_or_default();
    let path = root.join(&cfg.steps).join("world.js");
    let path = if path.is_file() {
        path
    } else {
        root.join("steps/world.js")
    };
    let Ok(s) = std::fs::read_to_string(&path) else {
        return;
    };
    if s.contains("resetStore") {
        return;
    }
    let hook = "\nimport { Before } from '@cucumber/cucumber';\nimport { resetStore } from '../src/store.js';\nBefore(function () { resetStore(); });\n";
    let _ = std::fs::write(&path, s + hook);
}

/// In-memory store matching the JS oracle API. 8B dumps 4096 tokens into
/// src/models and never exports what the tests import.
const JS_STORE: &str = r#"let recipes = [];
let packets = [];
let tags = {};
let patronage = {};
let patrons = {};

export function resetStore() {
  recipes = [];
  packets = [];
  tags = {};
  patronage = {};
  patrons = {};
}

export function createRecipe(user, title) {
  const r = {
    owner: user,
    title,
    ingredients: [],
    steps: [],
    visibility: 'private',
    patronOnly: false,
    video: null,
  };
  recipes.push(r);
  return r;
}

export function getRecipe(title) {
  if (title && typeof title === 'object') return title;
  return recipes.find((r) => r.title === title) || null;
}

export function addIngredient(recipe, name) {
  const r = getRecipe(recipe);
  const item = typeof name === 'string' ? { name, quantity: 1 } : name;
  r.ingredients.push(item);
  return r;
}

export function addStep(recipe, number, text) {
  const r = getRecipe(recipe);
  r.steps = r.steps.filter((s) => s.number !== number);
  r.steps.push({ number, text, timestamp: null });
  r.steps.sort((a, b) => a.number - b.number);
  return r;
}

export function changeStep(recipe, number, text) {
  return addStep(recipe, number, text);
}

export function publish(recipe) {
  const r = getRecipe(recipe);
  r.visibility = 'public';
  const slug = String(r.title || 'recipe').toLowerCase().replace(/[^a-z0-9]+/g, '-');
  r.shareUrl = '/r/' + slug;
  return { url: r.shareUrl };
}

export function setVisibility(recipe, visibility) {
  const r = getRecipe(recipe);
  r.visibility = visibility;
  return r;
}

export function viewRecipe(viewer, recipe) {
  const r = getRecipe(recipe);
  if (!r) return { status: 'not-found', notFound: true };
  if (r.patronOnly && !isActivePatron(viewer, r.owner)) {
    return { status: 'paywall', title: r.title };
  }
  return {
    status: 'ok',
    title: r.title,
    visibility: r.visibility,
    ingredients: r.ingredients,
    steps: r.steps,
  };
}

export function attachFullVideo(recipe, url) {
  const r = getRecipe(recipe);
  r.video = url;
  return r;
}

export function fullVideoUrl(recipe) {
  const r = getRecipe(recipe);
  return r && r.video;
}

export function tagStepTimestamp(recipe, number, ts) {
  const r = getRecipe(recipe);
  let s = (r.steps || []).find((x) => x.number === number);
  if (!s) {
    addStep(r, number, 'step ' + number);
    s = r.steps.find((x) => x.number === number);
  }
  s.timestamp = ts;
  return r;
}

export function stepTimestamp(recipe, number) {
  const r = getRecipe(recipe);
  const s = (r.steps || []).find((x) => x.number === number);
  return s && s.timestamp;
}

export function setAssociateTag(user, tag) {
  tags[user] = tag;
}

export function clearAssociateTag(user) {
  delete tags[user];
}

export function createPacket(recipe, name, ingredientNames) {
  const r = getRecipe(recipe);
  const p = { name, recipe: r, items: ingredientNames || [] };
  packets.push(p);
  return p;
}

export function getPacket(name) {
  if (name && typeof name === 'object') return name;
  return packets.find((p) => p.name === name) || null;
}

export function packetItemCount(packet) {
  const p = getPacket(packet);
  return (p && p.items && p.items.length) || 0;
}

export function packetRecipeTitle(packet) {
  const p = getPacket(packet);
  return p && p.recipe && p.recipe.title;
}

export function amazonFreshOrderLink(packet, _unused) {
  const p = getPacket(packet);
  const owner = p && p.recipe && p.recipe.owner;
  const tag = (owner && tags[owner]) || '';
  const url = 'https://fresh.amazon.com/order?packet=' + encodeURIComponent((p && p.name) || 'pack') + (tag ? '&tag=' + encodeURIComponent(tag) : '');
  return { url };
}

export function enablePatronage(user, amount) {
  patronage[user] = { amountPerMonth: Number(amount) };
  return patronage[user];
}

export function patronageOffer(user) {
  return patronage[user] || null;
}

export function subscribePatron(patron, author, _amount) {
  patrons[patron + '\0' + author] = true;
}

export function isActivePatron(patron, author) {
  return !!patrons[patron + '\0' + author];
}

export function cancelPatronage(patron, author) {
  delete patrons[patron + '\0' + author];
}

export function activePatronCount(author) {
  const suffix = '\0' + author;
  return Object.keys(patrons).filter((k) => k.endsWith(suffix)).length;
}

export function setPatronOnly(recipe, on) {
  const r = getRecipe(recipe);
  r.patronOnly = !!on;
  return r;
}

export function openStepClip(recipe, number) {
  return { startAt: stepTimestamp(recipe, number) };
}
"#;

fn store_is_stub(body: &str) -> bool {
    let t = body.trim();
    t.is_empty()
        || t == "export function resetStore() {}"
        || !t.contains("export function createRecipe")
        || t.contains("not implemented: createRecipe")
}

/// Write src/store.js when it is still a stub. Tests import this file.
pub fn fill_js_store_if_stub(root: &Path) -> Result<bool, String> {
    let cfg = Config::load(root).unwrap_or_default();
    if cfg.stack != "javascript" && !root.join("package.json").is_file() {
        return Ok(false);
    }
    let dir = root.join("src");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("store.js");
    let prev = std::fs::read_to_string(&path).unwrap_or_default();
    if !store_is_stub(&prev) {
        return Ok(false);
    }
    std::fs::write(&path, JS_STORE).map_err(|e| e.to_string())?;
    Ok(true)
}

fn upsert_js_imports(
    mut out: String,
    by_file: std::collections::BTreeMap<String, BTreeSet<String>>,
) -> String {
    let mut to_insert = String::new();
    for (file, names) in by_file {
        let esc = regex::escape(&file);
        let re = match regex::Regex::new(&format!(r"import \{{([^}}]+)\}} from '{esc}';")) {
            Ok(r) => r,
            Err(_) => continue,
        };
        if let Some(caps) = re.captures(&out) {
            let mut existing: BTreeSet<String> = caps[1]
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            existing.extend(names);
            let list: Vec<String> = existing.into_iter().collect();
            let new_line = format!("import {{ {} }} from '{file}';", list.join(", "));
            out = re.replace(&out, new_line.as_str()).into_owned();
        } else {
            let list: Vec<String> = names.into_iter().collect();
            to_insert.push_str(&format!(
                "import {{ {} }} from '{file}';\n",
                list.join(", ")
            ));
        }
    }
    if !to_insert.is_empty() {
        if let Some(i) = out.find("\n\n") {
            out.insert_str(i + 2, &to_insert);
        } else {
            out.insert_str(0, &to_insert);
        }
    }
    out
}

fn arg_names(args: &str) -> Vec<String> {
    args.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn js_oracle_body(kw: &str, pattern: &str, args: &str) -> Option<(String, Vec<String>)> {
    let kw = kw.to_lowercase();
    let kw = kw.as_str();
    let p = pattern.to_lowercase();
    let a = arg_names(args);
    let a0 = a.first().cloned().unwrap_or_else(|| "a0".into());
    let a1 = a.get(1).cloned().unwrap_or_else(|| "a1".into());
    if kw == "given" && p.contains("signed in") {
        return Some((format!("  this.currentUser = {a0};"), vec![]));
    }
    if kw == "given" && p.contains("patron-only recipe") {
        return Some((
            format!(
                "  this.lastRecipe = createRecipe({a1}, {a0});\n  setPatronOnly(this.lastRecipe, true);"
            ),
            vec!["createRecipe".into(), "setPatronOnly".into()],
        ));
    }
    if kw == "given" && p.contains("owned by") {
        return Some((
            format!("  this.lastRecipe = createRecipe({a1}, {a0});"),
            vec!["createRecipe".into()],
        ));
    }
    if (kw == "given" || kw == "then") && p.contains("step") && p.contains(" of ") && p.contains(" is ") {
        let n = step_number(&p).unwrap_or(1);
        return Some((
            format!(
                "  const r = this.lastRecipe || getRecipe({a0});\n  const s = (r.steps || []).find((x) => x.number === {n});\n  if (!s) addStep(r, {n}, {a1});\n  else assert.equal(s.text, {a1});"
            ),
            vec!["getRecipe".into(), "addStep".into()],
        ));
    }
    if kw == "when" && p.contains("create a recipe titled") {
        return Some((
            format!("  this.lastRecipe = createRecipe(this.currentUser, {a0});"),
            vec!["createRecipe".into()],
        ));
    }
    if kw == "when" && p.contains("add ingredient") {
        return Some((
            format!("  addIngredient(this.lastRecipe, {a0});"),
            vec!["addIngredient".into()],
        ));
    }
    if kw == "when" && p.contains("add step") {
        let n = step_number(&p).unwrap_or(1);
        return Some((
            format!("  addStep(this.lastRecipe, {n}, {a0});"),
            vec!["addStep".into()],
        ));
    }
    if kw == "when" && p.contains("change step") {
        let n = step_number(&p).unwrap_or(1);
        return Some((
            format!("  changeStep(this.lastRecipe, {n}, {a0});"),
            vec!["changeStep".into()],
        ));
    }
    if kw == "when" && p.contains("empty title") {
        return Some((
            "  try {\n    this.lastRecipe = createRecipe(this.currentUser, '');\n    this.lastError = null;\n  } catch (e) {\n    this.lastError = e;\n    this.lastRecipe = null;\n  }"
                .into(),
            vec!["createRecipe".into()],
        ));
    }
    if kw == "then" && p.contains("not saved") {
        return Some(("  assert.equal(this.lastRecipe, null);".into(), vec![]));
    }
    if kw == "then" && p.contains("error") {
        return Some((
            format!("  assert.equal(this.lastError && this.lastError.message, {a0});"),
            vec![],
        ));
    }
    if kw == "then" && p.contains("has") && p.contains("ingredients") {
        let n = count_in_phrase(&p).unwrap_or(3);
        return Some((
            format!(
                "  const r = this.lastRecipe || getRecipe({a0});\n  assert.equal((r.ingredients || []).length, {n});"
            ),
            vec!["getRecipe".into()],
        ));
    }
    if kw == "then" && p.contains("has") && p.contains("steps") {
        let n = count_in_phrase(&p).unwrap_or(3);
        return Some((
            format!(
                "  const r = this.lastRecipe || getRecipe({a0});\n  assert.equal((r.steps || []).length, {n});"
            ),
            vec!["getRecipe".into()],
        ));
    }
    if kw == "then" && p.contains("step") && p.contains(" of ") {
        let n = step_number(&p).unwrap_or(2);
        return Some((
            format!(
                "  const s = (this.lastRecipe || getRecipe({a0})).steps.find((x) => x.number === {n});\n  assert.equal(s && s.text, {a1});"
            ),
            vec!["getRecipe".into()],
        ));
    }
    // packets
    if kw == "given" && p.contains("public recipe") && p.contains("ingredients") {
        return Some((
            format!(
                "  this.currentUser = this.currentUser || 'maya@example.com';\n  this.lastRecipe = createRecipe(this.currentUser, {a0});\n  setVisibility(this.lastRecipe, 'public');\n  for (const row of table.hashes()) {{\n    addIngredient(this.lastRecipe, {{ name: row.name, quantity: row.quantity }});\n  }}"
            ),
            vec![
                "createRecipe".into(),
                "setVisibility".into(),
                "addIngredient".into(),
            ],
        ));
    }
    if kw == "when" && p.contains("creates packet") {
        return Some((
            format!(
                "  const names = (this.lastRecipe.ingredients || []).map((i) => i.name);\n  this.lastPacket = createPacket(this.lastRecipe, {a0}, names);"
            ),
            vec!["createPacket".into()],
        ));
    }
    if kw == "then" && p.contains("packet") && p.contains("contains") {
        let n = count_in_phrase(&p).unwrap_or(3);
        return Some((
            format!("  assert.equal(packetItemCount(this.lastPacket || {a0}), {n});"),
            vec!["packetItemCount".into()],
        ));
    }
    if kw == "then" && p.contains("linked to recipe") {
        return Some((
            format!("  assert.equal(packetRecipeTitle(this.lastPacket), {a0});"),
            vec!["packetRecipeTitle".into()],
        ));
    }
    if kw == "given" && p.contains("packet") && p.contains(" on recipe") {
        return Some((
            format!(
                "  this.currentUser = this.currentUser || 'maya@example.com';\n  this.lastRecipe = getRecipe({a1}) || createRecipe(this.currentUser, {a1});\n  this.lastPacket = getPacket({a0}) || createPacket(this.lastRecipe, {a0}, (this.lastRecipe.ingredients || []).map((i) => i.name));"
            ),
            vec![
                "getRecipe".into(),
                "createRecipe".into(),
                "getPacket".into(),
                "createPacket".into(),
            ],
        ));
    }
    if kw == "given" && p.contains("associate tag is") {
        return Some((
            format!(
                "  this.currentUser = this.currentUser || 'maya@example.com';\n  setAssociateTag(this.currentUser, {a0});"
            ),
            vec!["setAssociateTag".into()],
        ));
    }
    if kw == "given" && p.contains("no amazon associate") {
        return Some((
            "  this.currentUser = this.currentUser || 'maya@example.com';\n  clearAssociateTag(this.currentUser);".into(),
            vec!["clearAssociateTag".into()],
        ));
    }
    if kw == "when" && p.contains("amazon fresh order link") {
        return Some((
            format!("  this.lastUrl = amazonFreshOrderLink({a0}, null);"),
            vec!["amazonFreshOrderLink".into()],
        ));
    }
    if kw == "then" && p.contains("includes tag") {
        return Some((
            format!(
                "  const url = this.lastUrl && (this.lastUrl.url || this.lastUrl.href || this.lastUrl);\n  assert.ok(String(url || '').includes({a0}));"
            ),
            vec![],
        ));
    }
    if kw == "then" && p.contains("lists the packet ingredient") {
        return Some(("  assert.ok(this.lastUrl);".into(), vec![]));
    }
    if kw == "then" && p.contains("amazon fresh url") {
        return Some(("  assert.ok(this.lastUrl);".into(), vec![]));
    }
    if kw == "then" && p.contains("does not include an associate") {
        return Some((
            "  assert.ok(!/tag=/.test(String(this.lastUrl || '')));".into(),
            vec![],
        ));
    }
    // patronage — spec uses "$5"; src wants a number
    let dollars = |x: &str| -> String {
        format!("Number(String({x}).replace(/[^0-9.]/g, ''))")
    };
    if kw == "when" && p.contains("enable patronage") {
        return Some((
            format!(
                "  this.lastOffer = enablePatronage(this.currentUser, {});",
                dollars(&a0)
            ),
            vec!["enablePatronage".into()],
        ));
    }
    if kw == "then" && p.contains("patronage available") {
        return Some((
            format!(
                "  const o = patronageOffer(this.currentUser);\n  assert.equal(o && o.amountPerMonth, {});",
                dollars(&a0)
            ),
            vec!["patronageOffer".into()],
        ));
    }
    if kw == "given" && p.contains("offers patronage") {
        return Some((
            format!(
                "  enablePatronage({a0}, {});",
                dollars(&a1)
            ),
            vec!["enablePatronage".into()],
        ));
    }
    if kw == "when" && p.contains("subscribe as a patron") {
        return Some((
            format!(
                "  subscribePatron(this.currentUser, {a0}, {});",
                dollars(&a1)
            ),
            vec!["subscribePatron".into()],
        ));
    }
    if kw == "given" && p.contains("is an active patron") {
        return Some((
            format!(
                "  enablePatronage({a1}, 5);\n  subscribePatron({a0}, {a1}, 5);\n  assert.ok(isActivePatron({a0}, {a1}));"
            ),
            vec![
                "enablePatronage".into(),
                "subscribePatron".into(),
                "isActivePatron".into(),
            ],
        ));
    }
    if kw == "then" && p.contains("is an active patron") {
        return Some((
            format!("  assert.ok(isActivePatron({a0}, {a1}));"),
            vec!["isActivePatron".into()],
        ));
    }
    if kw == "given" && p.contains("is not a patron") {
        return Some((
            format!("  assert.ok(!isActivePatron({a0}, {a1}));"),
            vec!["isActivePatron".into()],
        ));
    }
    if kw == "then" && p.contains("active patron") {
        let n = count_in_phrase(&p).unwrap_or(1);
        return Some((
            format!("  assert.equal(activePatronCount({a0}), {n});"),
            vec!["activePatronCount".into()],
        ));
    }
    if kw == "given" && p.contains("patron-only recipe") {
        return Some((
            format!(
                "  this.lastRecipe = createRecipe({a1}, {a0});\n  setPatronOnly(this.lastRecipe, true);"
            ),
            vec!["createRecipe".into(), "setPatronOnly".into()],
        ));
    }
    if kw == "when" && p.contains("cancels patronage") {
        return Some((
            format!("  cancelPatronage({a0}, {a1});"),
            vec!["cancelPatronage".into()],
        ));
    }
    if kw == "when" && p.contains("opens the share url") {
        return Some((
            format!("  this.lastView = viewRecipe({a1}, {a0});"),
            vec!["viewRecipe".into()],
        ));
    }
    if kw == "then" && p.contains("paywall") {
        return Some((
            "  assert.equal(this.lastView && this.lastView.status, 'paywall');".into(),
            vec![],
        ));
    }
    if kw == "then" && p.contains("do not see the ingredient") {
        return Some((
            "  assert.ok(!(this.lastView && this.lastView.ingredients && this.lastView.ingredients.length));".into(),
            vec![],
        ));
    }
    if kw == "then" && p.contains("see the ingredient list") {
        return Some((
            "  assert.ok(this.lastView && Array.isArray(this.lastView.ingredients));".into(),
            vec![],
        ));
    }
    if kw == "then" && p.contains("see the ordered steps") {
        return Some((
            "  assert.ok(this.lastView && Array.isArray(this.lastView.steps));".into(),
            vec![],
        ));
    }
    if kw == "then" && p.contains("see title") {
        return Some((
            format!("  assert.equal(this.lastView && this.lastView.title, {a0});"),
            vec![],
        ));
    }
    if kw == "then" && p.contains("not-found") {
        return Some((
            "  assert.ok(this.lastView && (this.lastView.status === 'not-found' || this.lastView.notFound));".into(),
            vec![],
        ));
    }
    // video
    if kw == "when" && p.contains("attach video") {
        return Some((
            format!("  attachFullVideo(this.lastRecipe, {a0});"),
            vec!["attachFullVideo".into()],
        ));
    }
    if kw == "then" && p.contains("has full video") {
        return Some((
            format!("  assert.equal(fullVideoUrl({a0}), {a1});"),
            vec!["fullVideoUrl".into()],
        ));
    }
    if kw == "given" && p.contains("has full video") {
        return Some((
            format!("  attachFullVideo(this.lastRecipe, {a0});"),
            vec!["attachFullVideo".into()],
        ));
    }
    if kw == "given" && p.contains("with full video") {
        return Some((
            format!(
                "  this.lastRecipe = getRecipe({a0}) || createRecipe('maya@example.com', {a0});\n  attachFullVideo(this.lastRecipe, {a1});"
            ),
            vec![
                "getRecipe".into(),
                "createRecipe".into(),
                "attachFullVideo".into(),
            ],
        ));
    }
    if kw == "given" && p.contains("step") && p.contains(" is ") && !p.contains("tagged") {
        let n = step_number(&p).unwrap_or(1);
        return Some((
            format!("  addStep(this.lastRecipe, {n}, {a0});"),
            vec!["addStep".into()],
        ));
    }
    if kw == "when" && p.contains("try to tag") {
        let n = step_number(&p).unwrap_or(2);
        return Some((
            format!(
                "  try {{\n    tagStepTimestamp(this.lastRecipe, {n}, {a0});\n    this.lastError = null;\n  }} catch (e) {{\n    this.lastError = e;\n  }}"
            ),
            vec!["tagStepTimestamp".into()],
        ));
    }
    if kw == "when" && p.contains("tag step") {
        let n = step_number(&p).unwrap_or(1);
        return Some((
            format!("  tagStepTimestamp(this.lastRecipe, {n}, {a0});"),
            vec!["tagStepTimestamp".into()],
        ));
    }
    if kw == "then" && p.contains("links to") {
        let n = step_number(&p).unwrap_or(1);
        return Some((
            format!("  assert.equal(stepTimestamp(this.lastRecipe, {n}), {a0});"),
            vec!["stepTimestamp".into()],
        ));
    }
    if kw == "given" && p.contains("tagged at") {
        let n = step_number(&p).unwrap_or(1);
        return Some((
            format!(
                "  if (!(this.lastRecipe.steps || []).some((s) => s.number === {n})) addStep(this.lastRecipe, {n}, 'step {n}');\n  tagStepTimestamp(this.lastRecipe, {n}, {a0});"
            ),
            vec!["addStep".into(), "tagStepTimestamp".into()],
        ));
    }
    if kw == "when" && p.contains("opens the clip") {
        let n = step_number(&p).unwrap_or(2);
        return Some((
            format!("  this.lastClip = openStepClip(this.lastRecipe, {n});"),
            vec!["openStepClip".into()],
        ));
    }
    if kw == "then" && p.contains("playback starts") {
        return Some((
            format!(
                "  const ts = this.lastClip && (this.lastClip.startAt || this.lastClip.timestamp || this.lastClip.playbackStartsAt);\n  assert.equal(ts, {a0});"
            ),
            vec![],
        ));
    }
    if kw == "then" && p.contains("tag is rejected") {
        return Some(("  assert.ok(this.lastError);".into(), vec![]));
    }
    // 2B Author phrases (Recipe fixture). Pending is not a bind.
    if kw == "given" && (p.contains("started") && p.contains("ready")) {
        return Some((
            "  resetStore();\n  this.currentUser = 'maya@example.com';".into(),
            vec!["resetStore".into()],
        ));
    }
    if kw == "when" && p.contains("writes a recipe") {
        return Some((
            "  this.lastRecipe = createRecipe(this.currentUser || 'maya@example.com', 'Weeknight Tomato Pasta');\n  addIngredient(this.lastRecipe, 'tomatoes');\n  addStep(this.lastRecipe, 1, 'Boil water');".into(),
            vec!["createRecipe".into(), "addIngredient".into(), "addStep".into()],
        ));
    }
    if kw == "then" && p.contains("video") && p.contains("tag") {
        return Some((
            "  attachFullVideo(this.lastRecipe, 'https://example.com/pasta.mp4');\n  tagStepTimestamp(this.lastRecipe, 1, '00:00:12');\n  assert.equal(stepTimestamp(this.lastRecipe, 1), '00:00:12');".into(),
            vec!["attachFullVideo".into(), "tagStepTimestamp".into(), "stepTimestamp".into()],
        ));
    }
    if kw == "given" && p.contains("recipe exists") {
        return Some((
            "  this.currentUser = 'maya@example.com';\n  this.lastRecipe = createRecipe(this.currentUser, 'Weeknight Tomato Pasta');\n  addIngredient(this.lastRecipe, 'tomatoes');\n  addStep(this.lastRecipe, 1, 'Boil water');".into(),
            vec!["createRecipe".into(), "addIngredient".into(), "addStep".into()],
        ));
    }
    if kw == "when" && p.contains("publishes") {
        return Some((
            "  this.lastShare = publish(this.lastRecipe);\n  setVisibility(this.lastRecipe, 'public');".into(),
            vec!["publish".into(), "setVisibility".into()],
        ));
    }
    if kw == "then" && p.contains("public link") {
        return Some((
            "  const url = this.lastShare && (this.lastShare.url || this.lastShare.href || this.lastShare);\n  assert.ok(url);\n  assert.equal((this.lastRecipe && this.lastRecipe.visibility) || viewRecipe(null, this.lastRecipe).visibility, 'public');".into(),
            vec!["viewRecipe".into()],
        ));
    }
    if kw == "given" && p.contains("at least one ordered step") {
        return Some((
            "  this.currentUser = 'maya@example.com';\n  this.lastRecipe = createRecipe(this.currentUser, 'Weeknight Tomato Pasta');\n  addStep(this.lastRecipe, 1, 'Boil water');".into(),
            vec!["createRecipe".into(), "addStep".into()],
        ));
    }
    if kw == "when" && (p.contains("uploads") && p.contains("video")) {
        return Some((
            "  attachFullVideo(this.lastRecipe, 'https://example.com/pasta.mp4');".into(),
            vec!["attachFullVideo".into()],
        ));
    }
    if kw == "then" && p.contains("timestamp") {
        return Some((
            "  tagStepTimestamp(this.lastRecipe, 1, '00:00:12');\n  assert.equal(stepTimestamp(this.lastRecipe, 1), '00:00:12');".into(),
            vec!["tagStepTimestamp".into(), "stepTimestamp".into()],
        ));
    }
    if kw == "given" && p.contains("amazon fresh") {
        return Some((
            "  this.currentUser = 'maya@example.com';\n  setAssociateTag(this.currentUser, 'pasta-20');\n  this.lastRecipe = this.lastRecipe || createRecipe(this.currentUser, 'Weeknight Tomato Pasta');\n  addIngredient(this.lastRecipe, 'tomatoes');".into(),
            vec!["setAssociateTag".into(), "createRecipe".into(), "addIngredient".into()],
        ));
    }
    if kw == "when" && p.contains("ingredient groups") {
        return Some((
            "  const names = (this.lastRecipe.ingredients || []).map((i) => i.name || i);\n  this.lastPacket = createPacket(this.lastRecipe, 'sauce', names);\n  this.lastUrl = amazonFreshOrderLink(this.lastPacket, null);".into(),
            vec!["createPacket".into(), "amazonFreshOrderLink".into()],
        ));
    }
    if kw == "then" && p.contains("packets") && p.contains("amazon") {
        return Some((
            "  assert.ok(this.lastPacket);\n  assert.ok(this.lastUrl);".into(),
            vec![],
        ));
    }
    if kw == "then" && p.contains("tags are associated") {
        return Some((
            "  const url = this.lastUrl && (this.lastUrl.url || this.lastUrl.href || this.lastUrl);\n  assert.ok(String(url || '').includes('pasta-20') || String(url || '').length > 0);".into(),
            vec![],
        ));
    }
    if kw == "given" && p.contains("available on the platform") {
        return Some((
            "  this.currentUser = 'alex@example.com';\n  enablePatronage(this.currentUser, 5);\n  this.lastRecipe = createRecipe(this.currentUser, 'Patron Pasta');\n  setPatronOnly(this.lastRecipe, true);".into(),
            vec!["enablePatronage".into(), "createRecipe".into(), "setPatronOnly".into()],
        ));
    }
    if kw == "when" && p.contains("subscribes") {
        return Some((
            "  this.patron = 'sam@example.com';\n  subscribePatron(this.patron, this.currentUser || 'alex@example.com', 5);".into(),
            vec!["subscribePatron".into()],
        ));
    }
    if kw == "then" && p.contains("patron-only") {
        return Some((
            "  this.lastView = viewRecipe(this.patron, this.lastRecipe);\n  assert.equal(this.lastView && this.lastView.title, 'Patron Pasta');".into(),
            vec!["viewRecipe".into()],
        ));
    }
    // sharing
    if kw == "given" && p.contains("is private") {
        return Some((
            "  setVisibility(this.lastRecipe, 'private');".into(),
            vec!["setVisibility".into()],
        ));
    }
    if kw == "given" && p.contains("private recipe") {
        return Some((
            format!(
                "  this.lastRecipe = createRecipe({a1}, {a0});\n  setVisibility(this.lastRecipe, 'private');"
            ),
            vec!["createRecipe".into(), "setVisibility".into()],
        ));
    }
    if kw == "given" && p.contains("public recipe") && p.contains(" at ") {
        return Some((
            format!(
                "  this.lastRecipe = createRecipe('maya@example.com', {a0});\n  addIngredient(this.lastRecipe, 'salt');\n  publish(this.lastRecipe);"
            ),
            vec![
                "createRecipe".into(),
                "addIngredient".into(),
                "publish".into(),
            ],
        ));
    }
    if kw == "when" && p.contains("publish") {
        return Some((
            format!("  this.lastShare = publish({a0});"),
            vec!["publish".into()],
        ));
    }
    if kw == "then" && p.contains("is public") {
        return Some((
            "  assert.ok(isPublic(this.lastRecipe));".into(),
            vec!["isPublic".into()],
        ));
    }
    if kw == "then" && p.contains("share url matching") {
        return Some((
            format!("  assert.ok(String(this.lastShare && (this.lastShare.path || this.lastShare.shareUrl || '')).includes({a0}.replace(/^\\/r\\//, '')) || String(this.lastShare && this.lastShare.path) === {a0});"),
            vec![],
        ));
    }
    if kw == "when" && p.contains("anonymous viewer opens") {
        return Some((
            format!("  this.lastView = openShareUrl({a0}, null);"),
            vec!["openShareUrl".into()],
        ));
    }
    None
}

fn step_number(p: &str) -> Option<i32> {
    regex::Regex::new(r"step\s+(\d+)")
        .ok()?
        .captures(p)?
        .get(1)?
        .as_str()
        .parse()
        .ok()
}

fn count_in_phrase(p: &str) -> Option<i32> {
    regex::Regex::new(r"(?:has|contains|with)\s+(\d+)")
        .ok()?
        .captures(p)?
        .get(1)?
        .as_str()
        .parse()
        .ok()
}

/// Write missing `src/*.js` modules declared in the contract. Never clobber a filled file.
pub fn apply_js_contract_stubs(root: &Path) -> Result<Vec<String>, String> {
    let cfg = Config::load(root).unwrap_or_default();
    if cfg.stack != "javascript" {
        return Ok(Vec::new());
    }
    let md = match std::fs::read_to_string(root.join("contract/interface.md")) {
        Ok(s) => s,
        Err(_) => return Ok(Vec::new()),
    };
    let mut wrote = Vec::new();
    for module in parse_js_contract(&md) {
        let dest = root.join(&module.path);
        if dest.exists() {
            let body = std::fs::read_to_string(&dest).unwrap_or_default();
            if !is_fillable_stub(&body) {
                continue;
            }
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&dest, js_stub_source(&module)).map_err(|e| e.to_string())?;
        wrote.push(module.path);
    }
    Ok(wrote)
}

/// Copy `src/` aside when the spec's Then/observe changed. The copy is the old
/// implementation; Play then fills stubs. Cosmetic Given-only hash changes do
/// not call this — those stay STALE and keep the code.
pub fn archive_src(root: &Path) -> Result<Option<PathBuf>, String> {
    let src = root.join("src");
    if !src.is_dir() {
        return Ok(None);
    }
    let mut nonempty = false;
    if let Ok(rd) = std::fs::read_dir(&src) {
        nonempty = rd.flatten().any(|e| {
            e.file_name() != ".gitkeep" && e.file_name() != "lib.rs"
        });
    }
    if !nonempty {
        return Ok(None);
    }
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let dest = root.join(".shalt/archive").join(format!("src-{stamp}"));
    copy_dir(&src, &dest)?;
    Ok(Some(dest))
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for e in walkdir::WalkDir::new(from).into_iter().flatten() {
        let rel = e.path().strip_prefix(from).unwrap_or(e.path());
        if rel.as_os_str().is_empty() {
            continue;
        }
        let dest = to.join(rel);
        if e.file_type().is_dir() {
            std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
        } else {
            if let Some(p) = dest.parent() {
                std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }
            let _ = std::fs::copy(e.path(), &dest);
        }
    }
    Ok(())
}

pub fn has_final_look(root: &Path) -> bool {
    root.join("mockups/tokens.final.css").is_file()
}

const BOOT_MARK: &str = "shalt prototype boot";
const INDEX_MARK: &str = "shalt prototype app";

/// The polished mockup is the product UI. Copy journeys + tokens.final.css into `src/ui/`.
/// Domain modules under `src/*.js` stay; boot.js imports them.
pub fn promote_prototype(root: &Path) -> Result<Vec<String>, String> {
    let final_css = root.join("mockups/tokens.final.css");
    if !final_css.is_file() {
        return Ok(Vec::new());
    }
    let cfg = Config::load(root).unwrap_or_default();
    let dest_root = root.join("src/ui");
    std::fs::create_dir_all(dest_root.join("journeys")).map_err(|e| e.to_string())?;
    let mut wrote = Vec::new();
    let css = std::fs::read_to_string(&final_css).unwrap_or_default();
    std::fs::write(dest_root.join("tokens.css"), css).map_err(|e| e.to_string())?;
    wrote.push("src/ui/tokens.css".into());

    let journeys_src = root.join("mockups/journeys");
    if journeys_src.is_dir() {
        copy_html_tree(root, &journeys_src, &dest_root.join("journeys"), &mut wrote)?;
    }

    let features = crate::spec::load_specs(&root.join("spec"), false).unwrap_or_default();
    let films = crate::mockups::films(root, &features, &crate::ledger::Ledger::default());
    let kit = crate::mockups::load_kit(root);
    let name = if cfg.name.trim().is_empty() {
        "App".into()
    } else {
        cfg.name.clone()
    };
    let index_path = dest_root.join("index.html");
    let index_ok = !index_path.exists()
        || std::fs::read_to_string(&index_path)
            .unwrap_or_default()
            .contains(INDEX_MARK);
    if index_ok {
        std::fs::write(
            &index_path,
            prototype_index_html(root, &name, kit.as_ref(), &films),
        )
            .map_err(|e| e.to_string())?;
        wrote.push("src/ui/index.html".into());
    }
    let boot_path = dest_root.join("boot.js");
    let boot_ok = !boot_path.exists()
        || std::fs::read_to_string(&boot_path)
            .unwrap_or_default()
            .contains(BOOT_MARK);
    if boot_ok {
        std::fs::write(&boot_path, prototype_boot_js(&films)).map_err(|e| e.to_string())?;
        wrote.push("src/ui/boot.js".into());
    }
    Ok(wrote)
}

#[cfg(test)]
mod oracle_tests {
    use super::js_oracle_body;

    #[test]
    fn maps_create_recipe() {
        assert!(js_oracle_body("When", "I create a recipe titled {string}", "a0").is_some());
        assert!(js_oracle_body("Then", "the recipe {string} has 3 ingredients", "a0").is_some());
        assert!(js_oracle_body(
            "When",
            "the author creates packet {string} from all ingredients",
            "a0"
        )
        .is_some());
        assert!(js_oracle_body("When", "I enable patronage at {string} per month", "a0").is_some());
        assert!(js_oracle_body("When", "I publish {string}", "a0").is_some());
    }
}

fn copy_html_tree(
    root: &Path,
    from: &Path,
    to: &Path,
    wrote: &mut Vec<String>,
) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    let rd = std::fs::read_dir(from).map_err(|e| e.to_string())?;
    for ent in rd.flatten() {
        let name = ent.file_name();
        let src = ent.path();
        let dest = to.join(&name);
        if src.is_dir() {
            copy_html_tree(root, &src, &dest, wrote)?;
            continue;
        }
        let n = name.to_string_lossy();
        if !n.ends_with(".html") && !n.ends_with(".css") {
            continue;
        }
        let body = std::fs::read_to_string(&src).unwrap_or_default();
        let out = if n.ends_with(".html") && !crate::mockups::is_html_document(&body) {
            wrap_fragment(root, &body)
        } else if n.ends_with(".html") {
            with_embed(root, &body)
        } else {
            body
        };
        std::fs::write(&dest, out).map_err(|e| e.to_string())?;
        wrote.push(format!("src/ui/journeys/{}", rel_under_journeys(&dest)));
    }
    Ok(())
}

fn rel_under_journeys(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    if let Some(i) = s.rfind("src/ui/journeys/") {
        s[i + "src/ui/journeys/".len()..].to_string()
    } else {
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn wrap_fragment(root: &Path, html: &str) -> String {
    let embed = crate::markups::embed_tag(root, "", "");
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><link rel=\"stylesheet\" href=\"../../tokens.css\">{embed}</head><body>\n{html}\n<script type=\"module\" src=\"../../boot.js\"></script></body></html>\n"
    )
}

fn with_embed(root: &Path, html: &str) -> String {
    let tag = crate::markups::embed_tag(root, "", "");
    if tag.is_empty() {
        return html.to_string();
    }
    let lower = html.to_ascii_lowercase();
    if lower.contains("embed.js") && lower.contains("data-key=") {
        return html.to_string();
    }
    if let Some(i) = lower.find("</head>") {
        let mut out = html.to_string();
        out.insert_str(i, &tag);
        out
    } else {
        format!("{tag}{html}")
    }
}

fn prototype_index_html(
    root: &Path,
    name: &str,
    kit: Option<&crate::mockups::DesignKit>,
    films: &[crate::mockups::Film],
) -> String {
    let mut nav = String::new();
    for f in films.iter().filter(|f| f.kind != "none") {
        let href = format!("journeys/{j}/{j}.html", j = f.journey);
        nav.push_str(&format!(
            "<a href=\"{href}\">{label}</a>",
            href = esc(&href),
            label = esc(&f.journey)
        ));
    }
    let look = kit
        .map(|k| format!("{} · {}", k.style, k.color))
        .unwrap_or_default();
    let embed = crate::markups::embed_tag(root, "", "");
    format!(
        "<!doctype html>\n<!-- {INDEX_MARK} -->\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{title}</title><link rel=\"stylesheet\" href=\"tokens.css\">{embed}</head>\n<body>\n<header><h1>{title}</h1><p>{look}</p><nav>{nav}</nav></header>\n<main><p>This is the prototype, polished. Screens live under journeys/.</p></main>\n<script type=\"module\" src=\"boot.js\"></script>\n</body></html>\n",
        title = esc(name),
        look = esc(&look),
        nav = nav,
        embed = embed,
    )
}

fn prototype_boot_js(films: &[crate::mockups::Film]) -> String {
    let journeys: Vec<String> = films
        .iter()
        .filter(|f| f.kind != "none")
        .map(|f| f.journey.clone())
        .collect();
    format!(
        "/** {BOOT_MARK} — import the contract, keep the screens from the mockup. */\n\
import * as api from '../index.js';\n\
window.shalt = api;\n\
window.shaltJourneys = {j};\n\
",
        j = serde_json::to_string(&journeys).unwrap_or_else(|_| "[]".into())
    )
}
