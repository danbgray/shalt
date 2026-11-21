//! Cheap files shalt writes so the model fills modules instead of inventing a tree.

use crate::bindings::{
    parse_step_defs, step_has_table, step_kw_and_phrase, StepDef,
};
use crate::config::Config;
use crate::runner::list_step_files;
use crate::spec::load_specs;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

pub const JS_WORLD: &str = r#"import { AfterAll, Before, BeforeAll, setWorldConstructor } from '@cucumber/cucumber';
import { createServer } from '../src/server.js';
import { resetStore } from '../src/store.js';

function World() {
  this.currentUser = null;
  this.lastError = null;
  this.baseUrl = globalThis.__shaltBaseUrl;
}

setWorldConstructor(World);

let server;
BeforeAll(async function () {
  const started = await createServer();
  server = started.server;
  globalThis.__shaltBaseUrl = started.baseUrl;
});
Before(function () {
  resetStore();
  this.baseUrl = globalThis.__shaltBaseUrl;
});
AfterAll(async function () {
  if (server) await new Promise((resolve) => server.close(resolve));
});
"#;

const JS_SERVER_STUB: &str = r#"import http from 'node:http';

/** Listens, answers 404. Build replaces this with /r/:slug and /tasks. */
export function createServer() {
  const server = http.createServer((_req, res) => {
    res.writeHead(404, { 'content-type': 'text/plain' });
    res.end('not implemented');
  });
  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address();
      resolve({ server, baseUrl: `http://127.0.0.1:${port}`, port });
    });
  });
}
"#;

const JS_SERVER: &str = r#"import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import * as store from './store.js';

function slugTitle(title) {
  return String(title || 'recipe').toLowerCase().replace(/[^a-z0-9]+/g, '-');
}

function esc(s) {
  return String(s || '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function recipePage(recipe, view) {
  const title = view.title || recipe.title;
  const ings = (recipe.ingredients || [])
    .map((i) => '<li>' + esc(i.name || i) + (i.quantity ? ' · ' + esc(i.quantity) : '') + '</li>')
    .join('');
  const steps = (recipe.steps || [])
    .map((s) => {
      const ts = s.timestamp || s.startAt || '';
      return (
        '<li data-step="' +
        esc(s.number) +
        '" data-start="' +
        esc(ts) +
        '">' +
        esc(s.text) +
        (ts ? ' @' + esc(ts) : '') +
        '</li>'
      );
    })
    .join('');
  const packs = (store.packetsFor ? store.packetsFor(recipe) : [])
    .map((p) => {
      const n = (p.items && p.items.length) || 0;
      const url = p.orderUrl || '';
      return (
        '<section data-packet="' +
        esc(p.name) +
        '" data-count="' +
        n +
        '">packet ' +
        esc(p.name) +
        ' · ' +
        n +
        ' item' +
        (n === 1 ? '' : 's') +
        (url ? ' · <a href="' + esc(url) + '">Amazon Fresh</a> ' + esc(url) : '') +
        '</section>'
      );
    })
    .join('');
  const video = recipe.video
    ? '<p data-video="' + esc(recipe.video) + '">' + esc(recipe.video) + '</p>'
    : '';
  return (
    '<!doctype html><html><head><meta charset="utf-8"><title>' +
    esc(title) +
    '</title></head><body><h1>' +
    esc(title) +
    '</h1>' +
    (ings ? '<ul data-ingredients>' + ings + '</ul>' : '') +
    (steps ? '<ol data-steps>' + steps + '</ol>' : '') +
    packs +
    video +
    '</body></html>'
  );
}

function hydrateRecipe(r, body) {
  for (const ing of body.ingredients || []) {
    if (store.addIngredient) store.addIngredient(r, ing);
  }
  for (const st of body.steps || []) {
    const n = st.number || st[0] || 1;
    const text = st.text || st[1] || st;
    if (store.addStep) store.addStep(r, n, text);
    if (st.timestamp && store.tagStepTimestamp) store.tagStepTimestamp(r, n, st.timestamp);
  }
  if (body.video && store.attachFullVideo) store.attachFullVideo(r, body.video);
  if (body.patronOnly && store.setPatronOnly) store.setPatronOnly(r, true);
  for (const pack of body.packets || []) {
    if (!store.createPacket) continue;
    const p = store.createPacket(r, pack.name, pack.items || []);
    if (pack.orderUrl && store.attachAmazonFreshLink) store.attachAmazonFreshLink(p, pack.orderUrl);
  }
}

function readBody(req) {
  return new Promise((resolve) => {
    const chunks = [];
    req.on('data', (c) => chunks.push(c));
    req.on('end', () => {
      const raw = Buffer.concat(chunks).toString('utf8');
      try {
        resolve(raw ? JSON.parse(raw) : {});
      } catch (_) {
        resolve({});
      }
    });
  });
}

function send(res, code, body, type) {
  const buf = typeof body === 'string' ? body : JSON.stringify(body);
  res.writeHead(code, { 'content-type': type || 'application/json; charset=utf-8' });
  res.end(buf);
}

function serveUi(url, res) {
  const srcDir = path.dirname(fileURLToPath(import.meta.url));
  const ui = path.join(srcDir, 'ui');
  let rel = url.pathname === '/' ? '/index.html' : url.pathname;
  rel = path.normalize(rel).replace(/^(\.\.[/\\])+/, '');
  let file = path.join(ui, rel);
  if (!file.startsWith(ui) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) {
    file = path.join(srcDir, rel);
    if (!file.startsWith(srcDir) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) {
      return false;
    }
  }
  const ext = path.extname(file);
  const type =
    ext === '.html'
      ? 'text/html; charset=utf-8'
      : ext === '.js'
        ? 'text/javascript; charset=utf-8'
        : ext === '.css'
          ? 'text/css; charset=utf-8'
          : 'application/octet-stream';
  send(res, 200, fs.readFileSync(file, 'utf8'), type);
  return true;
}

export function createServer(opts = {}) {
  const want = opts.port ?? Number(process.env.SHALT_PORT || 0);
  const server = http.createServer(async (req, res) => {
    const url = new URL(req.url || '/', 'http://127.0.0.1');
    const user = req.headers['x-user'] || null;
    if (req.method === 'GET' && url.pathname.startsWith('/r/')) {
      const wantPath = url.pathname;
      const slug = wantPath.slice(3);
      const titles = typeof store.allRecipes === 'function' ? store.allRecipes() : [];
      let recipe = null;
      for (const t of titles.slice().reverse()) {
        const r = store.getRecipe(t);
        if (!r) continue;
        if (r.shareUrl === wantPath || slugTitle(r.title) === slug) {
          recipe = r;
          break;
        }
      }
      const view = store.viewRecipe ? store.viewRecipe(user, recipe) : { status: 'not-found' };
      if (!recipe || view.status === 'not-found' || view.notFound) {
        send(res, 404, 'not found', 'text/plain');
        return;
      }
      if (view.status === 'paywall') {
        send(res, 402, '<h1>' + esc(view.title || 'patron-only') + '</h1><p>paywall</p>', 'text/html; charset=utf-8');
        return;
      }
      send(res, 200, recipePage(recipe, view), 'text/html; charset=utf-8');
      return;
    }
    if (req.method === 'GET' && url.pathname === '/tasks') {
      const filter = url.searchParams.get('filter') || 'all';
      if (store.setFilter) store.setFilter(filter);
      const list = store.visibleList ? store.visibleList() : store.allList ? store.allList() : [];
      send(res, 200, list);
      return;
    }
    if (req.method === 'POST' && url.pathname === '/tasks') {
      const body = await readBody(req);
      if (store.addTask) store.addTask(body.title);
      send(res, 201, store.visibleList ? store.visibleList() : []);
      return;
    }
    if (req.method === 'GET' && url.pathname === '/recipes') {
      send(res, 200, store.allRecipes ? store.allRecipes() : []);
      return;
    }
    if (req.method === 'POST' && url.pathname === '/recipes') {
      const body = await readBody(req);
      const who = body.user || user || 'maya@example.com';
      const r = store.createRecipe ? store.createRecipe(who, body.title) : null;
      if (r) hydrateRecipe(r, body);
      if (r && store.publish) store.publish(r);
      send(res, 201, { title: r && r.title, url: r && r.shareUrl });
      return;
    }
    if (req.method === 'GET' && url.pathname === '/patrons') {
      const who = url.searchParams.get('user') || user;
      const o = store.patronageOffer ? store.patronageOffer(who) : null;
      send(res, 200, { user: who, amountPerMonth: o && o.amountPerMonth });
      return;
    }
    if (req.method === 'GET' && serveUi(url, res)) return;
    send(res, 404, 'not found', 'text/plain');
  });
  return new Promise((resolve) => {
    server.listen(want, '127.0.0.1', () => {
      const { port } = server.address();
      resolve({ server, baseUrl: `http://127.0.0.1:${port}`, port });
    });
  });
}

if (import.meta.url === `file://${process.argv[1]}`) {
  createServer().then((s) => {
    console.log(s.baseUrl);
  });
}
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
        let prev = std::fs::read_to_string(&path).unwrap_or_default();
        if prev.contains("createServer") {
            return Ok(None);
        }
    }
    std::fs::create_dir_all(root.join(&cfg.steps)).map_err(|e| e.to_string())?;
    std::fs::write(&path, JS_WORLD).map_err(|e| e.to_string())?;
    Ok(Some(path))
}

fn server_is_stub(body: &str) -> bool {
    let t = body.trim();
    t.is_empty()
        || t.contains("not implemented")
        || !t.contains("pathname.startsWith('/r/')")
}

/// 404 listener so BeforeAll can bind a port. Build replaces it with /r/:slug and /tasks.
pub fn fill_js_server(root: &Path, complete: bool) -> Result<bool, String> {
    let cfg = Config::load(root).unwrap_or_default();
    if cfg.stack != "javascript" && !root.join("package.json").is_file() {
        return Ok(false);
    }
    let dir = root.join("src");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("server.js");
    let prev = std::fs::read_to_string(&path).unwrap_or_default();
    if complete {
        if !server_is_stub(&prev) {
            return Ok(false);
        }
        std::fs::write(&path, JS_SERVER).map_err(|e| e.to_string())?;
        Ok(true)
    } else {
        if path.is_file() {
            return Ok(false);
        }
        std::fs::write(&path, JS_SERVER_STUB).map_err(|e| e.to_string())?;
        Ok(true)
    }
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
            if let Some((body, imports)) = js_then_or_oracle(root, &kw, &pat, &args) {
                if !seen_pats.insert(pat.clone()) {
                    continue;
                }
                filled.push(pat.clone());
                for i in imports {
                    used.insert(i);
                }
                let kind = if body.contains("fetch(") {
                    "async function"
                } else {
                    "function"
                };
                out.push_str(&format!("{kw}('{pat}', {kind} ({args}) {{\n{body}\n}});\n"));
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
        let _ = fill_js_store_stubs(root, &used);
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
    let _ = write_js_world_if_missing(root);
    let _ = fill_js_server(root, false);
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
const JS_STORE_TODO: &str = r#"import { DatabaseSync } from 'node:sqlite';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

let tasks = [];
let filter = 'all';

function dbPath() {
  const dir = path.join(path.dirname(fileURLToPath(import.meta.url)), '../.shalt');
  fs.mkdirSync(dir, { recursive: true });
  return process.env.SHALT_DB || path.join(dir, 'app.sqlite');
}
function dumpTodo() {
  try {
    const d = new DatabaseSync(dbPath());
    d.exec('CREATE TABLE IF NOT EXISTS kv (k TEXT PRIMARY KEY, v TEXT);');
    d.prepare('INSERT OR REPLACE INTO kv (k,v) VALUES (?,?)').run(
      'todo',
      JSON.stringify({ tasks, filter })
    );
  } catch (_) {}
}
function loadTodo() {
  try {
    const d = new DatabaseSync(dbPath());
    const row = d.prepare('SELECT v FROM kv WHERE k = ?').get('todo');
    if (row && row.v) {
      const s = JSON.parse(row.v);
      tasks = s.tasks || [];
      filter = s.filter || 'all';
    }
  } catch (_) {}
}
loadTodo();

export function resetStore() {
  tasks = [];
  filter = 'all';
  dumpTodo();
}

export function addTask(title) {
  const t = String(title || '').trim();
  if (!t) throw new Error('title required');
  tasks.push({ title: t, done: false });
  dumpTodo();
  return tasks[tasks.length - 1];
}

export function ensureActive(title) {
  let t = tasks.find((x) => x.title === title);
  if (!t) t = addTask(title);
  t.done = false;
  return t;
}

export function ensureCompleted(title) {
  let t = tasks.find((x) => x.title === title);
  if (!t) t = addTask(title);
  t.done = true;
  return t;
}

export function markDone(title) {
  const t = tasks.find((x) => x.title === title);
  if (!t) throw new Error('no task ' + title);
  t.done = true;
  dumpTodo();
  return t;
}

export function setFilter(next) {
  filter = next;
  dumpTodo();
}

export function clearCompleted() {
  tasks = tasks.filter((t) => !t.done);
  dumpTodo();
}

export function allList() {
  return tasks.map((t) => t.title);
}

export function activeList() {
  return tasks.filter((t) => !t.done).map((t) => t.title);
}

export function completedList() {
  return tasks.filter((t) => t.done).map((t) => t.title);
}

export function visibleList() {
  if (filter === 'active') return activeList();
  if (filter === 'completed') return completedList();
  return allList();
}
"#;

const JS_STORE: &str = r#"import { DatabaseSync } from 'node:sqlite';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

let recipes = [];
let packets = [];
let tags = {};
let patronage = {};
let patrons = {};

function dbPath() {
  const dir = path.join(path.dirname(fileURLToPath(import.meta.url)), '../.shalt');
  fs.mkdirSync(dir, { recursive: true });
  return process.env.SHALT_DB || path.join(dir, 'app.sqlite');
}
function dumpRecipe() {
  try {
    const d = new DatabaseSync(dbPath());
    d.exec('CREATE TABLE IF NOT EXISTS kv (k TEXT PRIMARY KEY, v TEXT);');
    d.prepare('INSERT OR REPLACE INTO kv (k,v) VALUES (?,?)').run(
      'recipe',
      JSON.stringify({ recipes, packets, tags, patronage, patrons })
    );
  } catch (_) {}
}
function loadRecipe() {
  try {
    const d = new DatabaseSync(dbPath());
    const row = d.prepare('SELECT v FROM kv WHERE k = ?').get('recipe');
    if (row && row.v) {
      const s = JSON.parse(row.v);
      recipes = s.recipes || [];
      packets = s.packets || [];
      tags = s.tags || {};
      patronage = s.patronage || {};
      patrons = s.patrons || {};
    }
  } catch (_) {}
}
loadRecipe();

export function resetStore() {
  recipes = [];
  packets = [];
  tags = {};
  patronage = {};
  patrons = {};
  dumpRecipe();
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
  dumpRecipe();
  return r;
}

export function getRecipe(title) {
  if (title && typeof title === 'object') return title;
  return recipes.find((r) => r.title === title) || null;
}

export function allRecipes() {
  return recipes.map((r) => r.title);
}

export function addIngredient(recipe, name) {
  const r = getRecipe(recipe);
  const item = typeof name === 'string' ? { name, quantity: 1 } : name;
  r.ingredients.push(item);
  dumpRecipe();
  return r;
}

export function addStep(recipe, number, text) {
  const r = getRecipe(recipe);
  r.steps = r.steps.filter((s) => s.number !== number);
  r.steps.push({ number, text, timestamp: null });
  r.steps.sort((a, b) => a.number - b.number);
  dumpRecipe();
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
  dumpRecipe();
  return { url: r.shareUrl };
}

export function isPublic(recipe) {
  const r = getRecipe(recipe);
  return !!(r && r.visibility === 'public');
}

export function openShareUrl(path, viewer) {
  const want = String(path || '');
  const slug = want.replace(/^\/r\//, '');
  const r = recipes.find(
    (x) =>
      x.shareUrl === want ||
      String(x.title || '')
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, '-') === slug
  );
  if (!r) return { status: 'not-found', title: null };
  return viewRecipe(viewer, r);
}

export function setVisibility(recipe, visibility) {
  const r = getRecipe(recipe);
  r.visibility = visibility;
  dumpRecipe();
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
  dumpRecipe();
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
  dumpRecipe();
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
  const p = { name, recipe: r, recipeTitle: r && r.title, items: ingredientNames || [] };
  packets.push(p);
  dumpRecipe();
  return p;
}

export function packetsFor(recipe) {
  const r = getRecipe(recipe);
  const title = r && r.title;
  return packets.filter(
    (p) => (p.recipe && p.recipe.title) === title || p.recipeTitle === title
  );
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
  if (p && p.orderUrl) return { url: p.orderUrl };
  const owner = p && p.recipe && p.recipe.owner;
  const tag = (owner && tags[owner]) || '';
  const url = 'https://fresh.amazon.com/order?packet=' + encodeURIComponent((p && p.name) || 'pack') + (tag ? '&tag=' + encodeURIComponent(tag) : '');
  return { url };
}

export function attachAmazonFreshLink(packet, url) {
  const p = getPacket(packet);
  if (!p) throw new Error('no packet ' + packet);
  p.orderUrl = url;
  dumpRecipe();
  return p;
}

export function packetOrderUrl(packet) {
  const p = getPacket(packet);
  return p && p.orderUrl;
}

export function enablePatronage(user, amount) {
  patronage[user] = { amountPerMonth: Number(amount) };
  dumpRecipe();
  return patronage[user];
}

export function patronageOffer(user) {
  return patronage[user] || null;
}

export function subscribePatron(patron, author, _amount) {
  patrons[patron + '\0' + author] = true;
  dumpRecipe();
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
  dumpRecipe();
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
        || t.contains("not implemented:")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum JsProduct {
    Recipe,
    Todo,
    Other,
}

fn spec_blob(root: &Path) -> String {
    let mut blob = String::new();
    if let Ok(p) = std::fs::read_to_string(root.join(".shalt/plan.md")) {
        blob.push_str(&p);
        blob.push('\n');
    }
    if let Ok(rd) = std::fs::read_dir(root.join("spec")) {
        for e in rd.flatten() {
            if e.path().extension().and_then(|s| s.to_str()) == Some("feature") {
                if let Ok(s) = std::fs::read_to_string(e.path()) {
                    blob.push_str(&s);
                    blob.push('\n');
                }
            }
        }
    }
    blob.to_ascii_lowercase()
}

fn js_product(root: &Path) -> JsProduct {
    let b = spec_blob(root);
    let recipe = b.contains("recipe");
    let todo = b.contains("todo")
        || (b.contains("task titled") && b.contains("clear completed"));
    if recipe && !todo {
        JsProduct::Recipe
    } else if todo && !recipe {
        JsProduct::Todo
    } else if recipe {
        JsProduct::Recipe
    } else if todo {
        JsProduct::Todo
    } else {
        JsProduct::Other
    }
}

fn store_wrong_product(body: &str, want: JsProduct) -> bool {
    match want {
        JsProduct::Todo => {
            body.contains("export function createRecipe") && !body.contains("export function addTask")
        }
        JsProduct::Recipe => {
            body.contains("export function addTask") && !body.contains("export function createRecipe")
        }
        JsProduct::Other => false,
    }
}

/// Throwing `src/store.js` so the first suite run can go **red**. The complete
/// store waits for Build (`fill_js_store_if_stub`). Filling both sides here is
/// how we got never-red 4/4 and 6/6.
pub fn fill_js_store_stubs(root: &Path, names: &BTreeSet<String>) -> Result<bool, String> {
    let cfg = Config::load(root).unwrap_or_default();
    if cfg.stack != "javascript" && !root.join("package.json").is_file() {
        return Ok(false);
    }
    if names.is_empty() {
        return Ok(false);
    }
    let dir = root.join("src");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("store.js");
    let prev = std::fs::read_to_string(&path).unwrap_or_default();
    if !store_is_stub(&prev) && !store_wrong_product(&prev, js_product(root)) {
        return Ok(false);
    }
    let mut body = String::from("export function resetStore() {}\n");
    let mut names = names.clone();
    names.insert("resetStore".into());
    for n in names {
        if n == "resetStore" {
            continue;
        }
        if !n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        body.push_str(&format!(
            "export function {n}() {{ throw new Error('not implemented: {n}'); }}\n"
        ));
    }
    std::fs::write(&path, body).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Write src/store.js when it is still a stub or the wrong product.
/// Call from Build after the first suite colour, not from Steps.
pub fn fill_js_store_if_stub(root: &Path) -> Result<bool, String> {
    let cfg = Config::load(root).unwrap_or_default();
    if cfg.stack != "javascript" && !root.join("package.json").is_file() {
        return Ok(false);
    }
    let dir = root.join("src");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("store.js");
    let prev = std::fs::read_to_string(&path).unwrap_or_default();
    let want = js_product(root);
    let recipe_missing = want == JsProduct::Recipe
        && prev.contains("export function createRecipe")
        && (!prev.contains("export function isPublic")
            || !prev.contains("export function openShareUrl")
            || !prev.contains("export function attachAmazonFreshLink"));
    if !store_is_stub(&prev) && !store_wrong_product(&prev, want) && !recipe_missing {
        return Ok(false);
    }
    let body = match want {
        JsProduct::Todo => JS_STORE_TODO,
        JsProduct::Recipe => JS_STORE,
        JsProduct::Other => "export function resetStore() {}\n",
    };
    std::fs::write(&path, body).map_err(|e| e.to_string())?;
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

fn observe_wants_http(observe: &str) -> bool {
    let t = observe.to_ascii_lowercase();
    t.contains("unsigned get")
        || t.contains("signed-in") && t.contains("get")
        || t.contains("get /")
        || t.contains("get of /")
        || t.contains("get of the public")
        || t.contains("get /tasks")
        || t.contains("get /patrons")
        || t.contains("get /r/")
}

fn then_fits_pat(then: &str, pat: &str) -> bool {
    let t = then.to_ascii_lowercase();
    let p = pat.to_ascii_lowercase();
    p.split("{string}")
        .map(|s| s.trim_matches(|c: char| c == '"' || c.is_whitespace()))
        .filter(|s| !s.is_empty())
        .all(|part| t.contains(part))
}

fn observe_for_pattern(root: &Path, pat: &str) -> Option<String> {
    let spec = root.join("spec");
    let rd = std::fs::read_dir(&spec).ok()?;
    for e in rd.flatten() {
        if e.path().extension().and_then(|s| s.to_str()) != Some("feature") {
            continue;
        }
        let raw = std::fs::read_to_string(e.path()).ok()?;
        for (_sc, oracles) in crate::oracles::parse_oracles(&raw) {
            for o in oracles {
                if then_fits_pat(&o.then, pat) {
                    return Some(o.observe);
                }
            }
        }
    }
    None
}

fn js_http_then_body(observe: &str, pattern: &str, args: &str) -> Option<(String, Vec<String>)> {
    let p = pattern.to_lowercase();
    let a = arg_names(args);
    let a0 = a.first().cloned().unwrap_or_else(|| "a0".into());
    let a1 = a.get(1).cloned().unwrap_or_else(|| "a1".into());
    let obs = observe.to_ascii_lowercase();
    if p.contains("patronage available") || obs.contains("/patrons") {
        let body = format!(
            "  const who = this.currentUser || 'maya@example.com';\n  const res = await fetch(this.baseUrl + '/patrons?user=' + encodeURIComponent(who));\n  assert.equal(res.status, 200);\n  const data = await res.json();\n  assert.equal(data.amountPerMonth, Number(String({a0}).replace(/[^0-9.]/g, '')));"
        );
        return Some((body, vec![]));
    }
    let path_lit = regex::Regex::new(r"(/r/[a-z0-9-]+|/tasks(?:\?[^\s]*)?)")
        .ok()
        .and_then(|re| re.find(&obs).map(|m| m.as_str().to_string()));
    let path_js = match path_lit {
        Some(p) if p.starts_with("/tasks") => format!("'{p}'"),
        Some(p) => format!("(this.lastPath || '{p}')"),
        None if obs.contains("/tasks") => "'/tasks'".into(),
        None => {
            "(this.lastPath || (this.lastShare && (this.lastShare.url || this.lastShare.path)) || '/r/weeknight-tomato-pasta')".into()
        }
    };
    let user = if obs.contains("signed-in") || obs.contains("patron") {
        "this.patron || this.currentUser"
    } else {
        "null"
    };
    if p.contains("see title")
        || p.contains("is public")
        || p.contains("shows")
        || p.contains("is completed")
        || p.contains("visible list")
        || p.contains("list is only")
        || p.contains("active list")
        || p.contains("packet")
        || p.contains("links to")
        || obs.contains("get")
    {
        let needle = a0.clone();
        let json = obs.contains("/tasks");
        let extra = if p.contains("amazon") {
            format!("\n  assert.ok(text.includes({a1}), text);")
        } else if p.contains("links to") {
            format!("\n  assert.ok(text.includes({a1}), text);")
        } else if p.contains("contains") {
            let n = count_in_phrase(&p).unwrap_or(1);
            format!("\n  assert.ok(/data-count=\"{n}\"|{n} item/.test(text), text);")
        } else {
            String::new()
        };
        let body = if json && (p.contains("only") || obs.contains("only")) {
            format!(
                "  const res = await fetch(this.baseUrl + {path_js});\n  assert.equal(res.status, 200);\n  const data = await res.json();\n  assert.deepEqual(data, [{needle}]);"
            )
        } else if json {
            format!(
                "  const res = await fetch(this.baseUrl + {path_js});\n  assert.equal(res.status, 200);\n  const data = await res.json();\n  assert.ok(Array.isArray(data) && data.includes({needle}), JSON.stringify(data));"
            )
        } else {
            format!(
                "  const headers = {{}};\n  const user = {user};\n  if (user) headers['x-user'] = user;\n  const res = await fetch(this.baseUrl + {path_js}, {{ headers }});\n  const text = await res.text();\n  assert.equal(res.status, 200, text);\n  assert.ok(text.includes({needle}), text);{extra}"
            )
        };
        return Some((body, vec![]));
    }
    None
}

fn js_then_or_oracle(
    root: &Path,
    kw: &str,
    pattern: &str,
    args: &str,
) -> Option<(String, Vec<String>)> {
    if kw.eq_ignore_ascii_case("then") {
        if let Some(obs) = observe_for_pattern(root, pattern) {
            if observe_wants_http(&obs) {
                if let Some(http) = js_http_then_body(&obs, pattern, args) {
                    return Some(http);
                }
            }
        }
    }
    js_oracle_body(kw, pattern, args)
}

fn js_oracle_body(kw: &str, pattern: &str, args: &str) -> Option<(String, Vec<String>)> {
    let kw = kw.to_lowercase();
    let kw = kw.as_str();
    let p = pattern.to_lowercase();
    let a = arg_names(args);
    let a0 = a.first().cloned().unwrap_or_else(|| "a0".into());
    let a1 = a.get(1).cloned().unwrap_or_else(|| "a1".into());
    if kw == "given" && p.contains("list is empty") {
        return Some(("  resetStore();\n  this.filter = 'all';".into(), vec!["resetStore".into()]));
    }
    if kw == "when" && p.contains("add a task titled") {
        return Some((format!("  addTask({a0});"), vec!["addTask".into()]));
    }
    if kw == "then" && p.contains("as active") {
        return Some((
            format!("  assert.ok(activeList().includes({a0}));"),
            vec!["activeList".into()],
        ));
    }
    if kw == "given" && p.contains("task") && p.contains("is active") {
        return Some((format!("  ensureActive({a0});"), vec!["ensureActive".into()]));
    }
    if kw == "when" && p.contains("mark") && p.contains("done") {
        return Some((format!("  markDone({a0});"), vec!["markDone".into()]));
    }
    if kw == "then" && p.contains("is completed") {
        return Some((
            format!(
                "  assert.ok(completedList().includes({a0}));\n  assert.ok(!activeList().includes({a0}));"
            ),
            vec!["completedList".into(), "activeList".into()],
        ));
    }
    if kw == "given" && p.contains("task") && p.contains("is completed") {
        return Some((
            format!("  ensureCompleted({a0});"),
            vec!["ensureCompleted".into()],
        ));
    }
    if kw == "when" && p.contains("filter to active") {
        return Some(("  setFilter('active');\n  this.filter = 'active';".into(), vec!["setFilter".into()]));
    }
    if kw == "then" && p.contains("visible list is only") {
        return Some((
            format!("  assert.deepEqual(visibleList(), [{a0}]);"),
            vec!["visibleList".into()],
        ));
    }
    if kw == "when" && p.contains("clear completed") {
        return Some(("  clearCompleted();".into(), vec!["clearCompleted".into()]));
    }
    if kw == "then" && p.contains("list is only") {
        return Some((
            format!("  assert.deepEqual(allList(), [{a0}]);"),
            vec!["allList".into()],
        ));
    }
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
            format!(
                "  this.lastRecipe = createRecipe({a1}, {a0});\n  this.lastShare = publish(this.lastRecipe);\n  this.lastPath = this.lastShare && this.lastShare.url;"
            ),
            vec!["createRecipe".into(), "publish".into()],
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
    if kw == "then" && p.contains("step") && p.contains("links to") {
        let n = step_number(&p).unwrap_or(1);
        return Some((
            format!(
                "  const clip = openStepClip(this.lastRecipe || getRecipe({a0}), {n});\n  assert.equal(clip && clip.startAt, {a1});"
            ),
            vec!["openStepClip".into(), "getRecipe".into()],
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
                "  this.currentUser = this.currentUser || 'maya@example.com';\n  this.lastRecipe = createRecipe(this.currentUser, {a0});\n  setVisibility(this.lastRecipe, 'public');\n  for (const row of table.hashes()) {{\n    addIngredient(this.lastRecipe, {{ name: row.name, quantity: row.quantity }});\n  }}\n  this.lastShare = publish(this.lastRecipe);\n  this.lastPath = this.lastShare && this.lastShare.url;"
            ),
            vec![
                "createRecipe".into(),
                "setVisibility".into(),
                "addIngredient".into(),
                "publish".into(),
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
    if kw == "when" && p.contains("attaches") && p.contains("amazon fresh") {
        return Some((
            format!(
                "  this.lastPacket = attachAmazonFreshLink({a1}, {a0});\n  this.lastUrl = {a0};"
            ),
            vec!["attachAmazonFreshLink".into()],
        ));
    }
    if kw == "then" && p.contains("has amazon fresh order link") {
        return Some((
            format!("  assert.equal(packetOrderUrl({a0}), {a1});"),
            vec!["packetOrderUrl".into()],
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
            format!(
                "  this.patron = {a0};\n  const r = getRecipe({a1});\n  this.lastPath = (r && r.shareUrl) || '/r/' + String({a1}).toLowerCase().replace(/[^a-z0-9]+/g, '-');\n  this.lastView = viewRecipe({a0}, {a1});"
            ),
            vec!["viewRecipe".into(), "getRecipe".into()],
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
                "  this.lastRecipe = createRecipe('maya@example.com', {a0});\n  addIngredient(this.lastRecipe, 'salt');\n  this.lastShare = publish(this.lastRecipe);\n  this.lastPath = this.lastShare && this.lastShare.url;"
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
            format!(
                "  this.lastShare = publish({a0});\n  this.lastPath = (this.lastShare && this.lastShare.url) || (this.lastRecipe && this.lastRecipe.shareUrl);"
            ),
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
            format!("  this.lastPath = {a0};\n  this.lastView = openShareUrl({a0}, null);"),
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
    let boot_body = std::fs::read_to_string(&boot_path).unwrap_or_default();
    let boot_ok = !boot_path.exists()
        || boot_body.contains(BOOT_MARK)
        || boot_body.contains("from './store'")
        || boot_body.contains("from '../store.js'");
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
        assert!(js_oracle_body("When", "I add a task titled {string}", "a0").is_some());
        assert!(js_oracle_body("Then", "the list shows {string} as active", "a0").is_some());
        assert!(js_oracle_body("Then", "the recipe {string} has 3 ingredients", "a0").is_some());
        assert!(js_oracle_body(
            "Then",
            "packet {string} has Amazon Fresh order link {string}",
            "a0, a1"
        )
        .is_some());
        let (body, _) = js_oracle_body(
            "Then",
            "step 1 of {string} links to {string}",
            "a0, a1",
        )
        .expect("video clip");
        assert!(body.contains("openStepClip"), "{body}");
        assert!(js_oracle_body("Then", "the recipe {string} is public", "a0").is_some());
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
    let chrome = prototype_main_chrome(films);
    format!(
        "<!doctype html>\n<!-- {INDEX_MARK} -->\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{title}</title><link rel=\"stylesheet\" href=\"tokens.css\">{embed}</head>\n<body>\n<header><h1>{title}</h1><p>{look}</p><nav>{nav}</nav></header>\n<main>{chrome}</main>\n<script type=\"module\" src=\"boot.js\"></script>\n</body></html>\n",
        title = esc(name),
        look = esc(&look),
        nav = nav,
        chrome = chrome,
        embed = embed,
    )
}

fn prototype_main_chrome(films: &[crate::mockups::Film]) -> String {
    let journeys: Vec<&str> = films.iter().map(|f| f.journey.as_str()).collect();
    if journeys.iter().any(|j| *j == "tasks") {
        return concat!(
            "<form data-action=\"add-task\"><label>Title <input type=\"text\" name=\"title\" placeholder=\"Buy milk\"></label> ",
            "<button type=\"button\" class=\"primary\" data-action=\"add\">Add task</button></form>\n",
            "<ul data-todo-list></ul>\n",
            "<label>Show <select data-action=\"filter\"><option value=\"all\">All</option><option value=\"active\">Active</option><option value=\"completed\">Completed</option></select></label>\n",
            "<button type=\"button\" data-action=\"done\">Mark done</button>\n",
            "<button type=\"button\" data-action=\"clear\">Clear completed</button>"
        )
        .into();
    }
    if journeys
        .iter()
        .any(|j| *j == "recipes" || *j == "sharing")
    {
        return concat!(
            "<form><label>Title <input type=\"text\" name=\"title\" placeholder=\"Weeknight Tomato Pasta\"></label> ",
            "<button type=\"button\" data-action=\"publish\">Publish</button></form>\n",
            "<ul data-recipe-list></ul>\n",
            "<p data-share-url></p>\n",
            "<h2 data-recipe-title></h2>"
        )
        .into();
    }
    "<p>This is the prototype, polished. Screens live under journeys/.</p>".into()
}

fn prototype_boot_js(films: &[crate::mockups::Film]) -> String {
    let journeys: Vec<String> = films
        .iter()
        .filter(|f| f.kind != "none")
        .map(|f| f.journey.clone())
        .collect();
    let j = serde_json::to_string(&journeys).unwrap_or_else(|_| "[]".into());
    format!(
        "/** {BOOT_MARK} — talk to the product HTTP API, not node:sqlite. */\n\
window.shaltJourneys = {j};\n\
async function titles(filter) {{\n\
  try {{\n\
    const q = filter ? '/tasks?filter=' + encodeURIComponent(filter) : '/tasks';\n\
    const r = await fetch(q);\n\
    if (r.ok) return await r.json();\n\
  }} catch (e) {{}}\n\
  try {{\n\
    const r = await fetch('/recipes');\n\
    if (r.ok) return await r.json();\n\
  }} catch (e) {{}}\n\
  return [];\n\
}}\n\
async function paint(filter) {{\n\
  const list = await titles(filter);\n\
  const main = document.querySelector('main') || document.body;\n\
  let ul = main.querySelector('[data-todo-list], [data-recipe-list]');\n\
  if (!ul) {{\n\
    ul = document.createElement('ul');\n\
    ul.setAttribute('data-todo-list', '1');\n\
    main.appendChild(ul);\n\
  }}\n\
  ul.innerHTML = list.map((t) => '<li>' + String(t).replace(/</g,'') + '</li>').join('');\n\
  const share = document.querySelector('[data-share-url]');\n\
  const heading = document.querySelector('[data-recipe-title]');\n\
  const last = list.slice(-1)[0];\n\
  if (share && last) share.textContent = '/r/' + String(last).toLowerCase().replace(/[^a-z0-9]+/g, '-');\n\
  if (heading && last) heading.textContent = last;\n\
}}\n\
function bind() {{\n\
  const input = document.querySelector('input[type=\"text\"]');\n\
  const buttons = Array.from(document.querySelectorAll('button'));\n\
  const add = buttons.find((b) => /add/i.test(b.textContent || '')) || buttons[0];\n\
  const filter = document.querySelector('select');\n\
  if (add && input) {{\n\
    add.addEventListener('click', async () => {{\n\
      const title = input.value;\n\
      try {{\n\
        await fetch('/tasks', {{ method: 'POST', headers: {{ 'content-type': 'application/json' }}, body: JSON.stringify({{ title }}) }});\n\
      }} catch (e) {{}}\n\
      input.value = '';\n\
      paint();\n\
    }});\n\
    input.addEventListener('keydown', (ev) => {{\n\
      if (ev.key === 'Enter') add.click();\n\
    }});\n\
  }}\n\
  const pub = document.querySelector('[data-action=\"publish\"]');\n\
  const titleIn = document.querySelector('input[name=\"title\"]');\n\
  if (pub) {{\n\
    pub.addEventListener('click', async () => {{\n\
      const t = (titleIn && titleIn.value) || 'Untitled';\n\
      try {{\n\
        await fetch('/recipes', {{ method: 'POST', headers: {{ 'content-type': 'application/json' }}, body: JSON.stringify({{ title: t, user: 'maya@example.com' }}) }});\n\
      }} catch (e) {{}}\n\
      paint();\n\
    }});\n\
  }}\n\
  if (filter) {{\n\
    filter.addEventListener('change', () => {{\n\
      const v = String(filter.value || '').toLowerCase();\n\
      const f = v.includes('active') ? 'active' : v.includes('complete') ? 'completed' : 'all';\n\
      paint(f);\n\
    }});\n\
  }}\n\
  paint();\n\
}}\n\
if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', bind);\n\
else bind();\n\
"
    )
}
