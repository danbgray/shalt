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
    !parse_step_defs(body).is_empty()
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
