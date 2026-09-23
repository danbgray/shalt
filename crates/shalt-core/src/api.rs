//! OpenAI-compatible chat-completions backend (Grok / OpenAI).
//!
//! Four scoped tools. Path refusals return `REFUSED:` to the model. The workspace
//! guard around `run_role` is still the guarantee.

use crate::backends::Backend;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const MAX_READ: usize = 20_000;
const MAX_WRITE: usize = 400_000;
/// Keep this many latest tool results verbatim; older ones are stubbed so
/// prompt tokens do not grow with every read.
const KEEP_TOOL_RESULTS: usize = 2;
const COMPACT_TOOL_AFTER: usize = 2_000;
pub const ASSUME_REPLY: &str = "The human is not taking questions this turn. Make a reasonable assumption, write it into the scenario as a concrete example, and continue.";

pub const ROLE_SYSTEM: &[(&str, &str)] = &[
    ("author", "You are the SPEC AUTHOR in a BDD pipeline. You write feature files under spec/. The spec is living: rewrite scenarios when they are wrong, vague, or untestable. If src/ contains code, describe behaviour that is true of it. If a concrete example is missing, ask_human once. One behaviour per scenario, concrete values. Tag each Feature with @epic:<area>. Do not list_files — the current spec is in the user prompt. Write each feature file at most once, then call done(). Do not write blog files, tests, or src/. Only create files under spec/."),
    ("designer", "You are the DESIGNER in a shalt pipeline. Layout only: fill beat regions under mockups/, not full HTML documents, not product nav, not a stylesheet. The shalt sketch sheet is the look (ink on paper). If kit.json has no platform, ask_human once (Phone, Tablet, or Desktop) with a guess; yolo takes the guess. Color, style, fonts, and layout wait for a separate design interview after every screen is drawn. shalt injects primary nav and beat list from THIS spec. One HTML file per journey at mockups/journeys/<journey>/<journey>.html; beats are data-rid sections. Per journey write mockups/journeys/<journey>/storyboard.json {journey,kind,spec_hash,frames:[{rid,file,caption}]}. Never write mockups/storyboard.json or HTML at the mockups root. Copy names, prices, and URLs from the spec. Clickable; unproven paths are a red stub. spec_hash must match the prompt. kind=none when there is no UI. Do not restyle controls or set light text on buttons. tokens.sketch.css is :root vars only if missing. tokens.final.css is the interview polish, one file for every mockup. Do not rewrite drawn journeys. Do not write spec/, tests, contract/, src/, or .shalt/. Call done() as soon as every empty beat has HTML and a storyboard frame."),
    ("stepwright", "You are the STEPWRIGHT in a shalt BDD pipeline. You see the spec and NOTHING of the implementation -- that is deliberate. The spec can change; re-bind scenarios when it does. This workspace is a Rust crate. Write shalt's step harness in tests/shalt.rs (Cargo [[test]] name = shalt). cucumber-rs 0.23: #[given(expr = \"...\")] / #[when(expr = \"...\")] / #[then(expr = \"...\")] whenever the step captures {string} or {int}; a literal with no capture may use #[when(\"...\")]. Data tables take an extra `step: &cucumber::gherkin::Step` (never cucumber::Step — that alias is generic over World). World is #[derive(Debug, Default, World)] and holds only Default+Debug fields (primitives, or contract types that derive both). fn main must write cucumber JSON to env SHALT_REPORT (default .shalt/cucumber.json) with cucumber::writer::Json and .run(\"spec\") — not run_and_exit. Match examples/rust-billing/tests/cucumber.rs. Declare the public API in contract/interface.md and import only that surface. Do not write Python, pytest-bdd, or files under steps/. Never weaken an assertion to make it easier to satisfy; you are the oracle, not the builder. If a scenario cannot be tested as written, do not invent a vacuous test — skip it so the author can fix the spec. Only create files under tests/ and contract/."),
    ("implementer", "You are the IMPLEMENTER in a shalt BDD pipeline. You see the spec, the interface contract, mockups/ (sketched screens), and the failing test output -- you do NOT see the step definitions, and you cannot edit them. This workspace is a Rust crate. Write Rust under src/ that satisfies the specified behaviour against the contract. Fill the module the failing test needs. Do not rewrite every file. Write each file at most once this turn, then done(). Match mockups when the work is UI; the spec still wins if they disagree. Do not write Python. Do not special-case test inputs or hard-code expected outputs; implement the behaviour. If the spec is wrong, incomplete, or contradicts itself, call ask_human — do not invent the missing behaviour and do not weaken the contract. Only create files under src/."),
    ("auditor", "You are the TEST AUDITOR in a shalt BDD pipeline. A smaller model wrote the tests. You see spec/, the step files, and contract/. You do NOT see src/. You write nothing. Read, then call done(). First line of done() must be PASS or FAIL. PASS only if the tests are a real oracle: they would fail if the behaviour were missing. FAIL for empty bodies, pending, assert-true, stubs, or hard-coded answers. Then at most 8 short findings. Do not write files."),
    ("code_auditor", "You are the CODE AUDITOR in a shalt BDD pipeline. A smaller model wrote src/ to make tests pass. You see spec, contract, tests, src, and mockups. You write nothing. Read, then call done(). First line of done() must be PASS or FAIL. PASS only if the code implements the named behaviour. FAIL if it hard-codes expected outputs, special-cases this ticket, or leaves not-implemented stubs. Then at most 8 short findings. Do not write files."),
];

const STEPWRIGHT_JS: &str = "You are the STEPWRIGHT in a shalt BDD pipeline. You see the spec and NOTHING of the implementation -- that is deliberate. The spec can change; re-bind scenarios when it does. This workspace is JavaScript (Node, cucumber-js ESM). steps/world.js is the World template — keep it. One steps/<journey>.steps.js per journey. cucumber-js: import { Given, When, Then } from '@cucumber/cucumber'; function (not arrow) so World is `this`; {string}/{int} captures; DataTable last arg. Assert with node:assert/strict. Import only contract/interface.md from '../src/....js'. Never define the same Given/When/Then phrase twice. Keep steps that already bind. Write each file at most once, then done(). Do not write Python, pytest-bdd, tests/shalt.rs, or src/. If a scenario cannot be tested as written, return 'pending'. Only create files under steps/ and contract/.";
const IMPLEMENTER_JS: &str = "You are the IMPLEMENTER in a shalt BDD pipeline. You see the spec, the interface contract, mockups/ (the product UI), and the failing test output -- you do NOT see the step definitions, and you cannot edit them. This workspace is JavaScript (Node, ESM). The polished prototype is the final UI (src/ui/ after promote, mockups/ before). Do not invent a second interface. src/*.js modules match contract/interface.md; stubs throw `not implemented: <name>`. Fill the stub the failing test imports. Wire src/ui/boot.js to those modules if it is still the shalt prototype boot. Do not rewrite every file. Write each file at most once this turn, then done(). The spec still wins if mockups disagree. Do not write Python or Rust. Do not special-case test inputs. If the spec is wrong, call ask_human. Only create files under src/.";
const STEPWRIGHT_PYTHON: &str = "You are the STEPWRIGHT in a BDD pipeline. You see the approved spec and NOTHING of the implementation -- that is deliberate. Write pytest-bdd step definitions under steps/ that bind each scenario to the behaviour it describes, and declare the public API surface you call in contract/interface.md. Import only from that declared surface. Never weaken an assertion to make it easier to satisfy; you are the oracle, not the builder. Only create files under steps/ and contract/.";
const IMPLEMENTER_PYTHON: &str = "You are the IMPLEMENTER in a BDD pipeline. You see the spec, the interface contract, and the failing test output -- you do NOT see the step definitions, and you cannot edit them. Write Python under src/ that satisfies the specified behaviour against the contract. Do not special-case test inputs or hard-code expected outputs; implement the behaviour. Only create files under src/.";
const STEPWRIGHT_GENERIC: &str = "You are the STEPWRIGHT in a BDD pipeline. You see the approved spec and NOTHING of the implementation -- that is deliberate. Write step definitions in this stack's steps zone (see shalt.toml [zones].steps) for its Cucumber-family runner. Declare the public API in contract/interface.md and import only that surface. Never weaken an assertion. Only create files in the steps zone and contract/.";
const IMPLEMENTER_GENERIC: &str = "You are the IMPLEMENTER in a BDD pipeline. You see the spec, the interface contract, and the failing tests -- not the step definitions. Write implementation in the src zone from shalt.toml. Do not special-case test inputs. Only create files in the src zone.";

pub fn stepwright_user_prompt(stack: &str) -> &'static str {
    match stack {
        "javascript" => {
            "Write cucumber-js ESM step definitions under steps/ and contract/interface.md. import { Given, When, Then, setWorldConstructor } from '@cucumber/cucumber'; function (not arrow) World; {string}/{int} captures; import from '../src/....js'. Bind every approved scenario. Do not weaken assertions."
        }
        "python" => {
            "Write pytest-bdd step definitions under steps/ and contract/interface.md. Bind every approved scenario. Do not weaken assertions."
        }
        "rust" | "" => {
            "Write shalt's Rust step harness in tests/shalt.rs and contract/interface.md. cucumber-rs 0.23 with expr = captures, cucumber::gherkin::Step for tables, SHALT_REPORT Json writer, .run(\"spec\"). Bind every approved scenario. Do not write Python. Do not weaken assertions."
        }
        _ => {
            "Write step definitions in this stack's steps zone (shalt.toml) and contract/interface.md. Bind every approved scenario. Do not weaken assertions."
        }
    }
}

pub fn system_for(role: &str) -> &'static str {
    system_for_stack(role, crate::config::DEFAULT_STACK)
}

fn packed_system(role: &str, model: &str, stage: &Path) -> String {
    let stack = crate::config::Config::load(stage)
        .map(|c| c.stack)
        .unwrap_or_else(|_| "rust".into());
    let tools = "\n\nYou work only through the provided tools. Paths are relative to your working root. Write complete files, not diffs. Call done() as soon as the work is covered.";
    if role == "stepwright" {
        let sign = if crate::alloc::is_write_model(model) && !model.contains("8b") {
            ""
        } else {
            crate::journal::SIGN_OFF
        };
        return format!(
            "{}{tools}{sign}",
            crate::brief::stepwright_system(model, &stack)
        );
    }
    format!(
        "{}{tools}{}",
        system_for_stack(role, &stack),
        crate::journal::SIGN_OFF
    )
}

fn packed_user(role: &str, prompt: &str, stage: &Path) -> String {
    if crate::brief::uses_packed_brief(role) {
        prompt.to_string()
    } else {
        format!("{prompt}\n\nFiles you can see:\n{}", tree(stage))
    }
}

pub fn system_for_stack(role: &str, stack: &str) -> &'static str {
    if role == "author"
        || role == "designer"
        || role == "auditor"
        || role == "code_auditor"
        || stack == "rust"
        || stack.is_empty()
    {
        return ROLE_SYSTEM
            .iter()
            .find(|(r, _)| *r == role)
            .map(|(_, s)| *s)
            .unwrap_or("You are an agent in a shalt workspace. Work only through the provided tools.");
    }
    match (role, stack) {
        ("stepwright", "javascript") => STEPWRIGHT_JS,
        ("implementer", "javascript") => IMPLEMENTER_JS,
        ("stepwright", "python") => STEPWRIGHT_PYTHON,
        ("implementer", "python") => IMPLEMENTER_PYTHON,
        ("stepwright", _) => STEPWRIGHT_GENERIC,
        ("implementer", _) => IMPLEMENTER_GENERIC,
        _ => ROLE_SYSTEM
            .iter()
            .find(|(r, _)| *r == role)
            .map(|(_, s)| *s)
            .unwrap_or("You are an agent in a shalt workspace. Work only through the provided tools."),
    }
}

#[derive(Debug)]
pub struct ToolPathError(pub String);

pub fn safe_path(stage: &Path, raw: &str) -> Result<PathBuf, ToolPathError> {
    let cleaned = raw.trim();
    if cleaned.is_empty() || cleaned == "." || cleaned == "/" {
        return Err(ToolPathError("a file path is required".into()));
    }
    let candidate = Path::new(cleaned);
    if cleaned.starts_with('/') || cleaned.starts_with('\\') || candidate.is_absolute() {
        return Err(ToolPathError(format!(
            "absolute paths are not allowed: {raw:?} -- use a path relative to your root"
        )));
    }
    if candidate.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err(ToolPathError(format!("path escapes the working root: {raw:?}")));
    }
    let target = stage.join(candidate);
    let stage_res = stage.canonicalize().unwrap_or_else(|_| stage.to_path_buf());
    let mut probe = target.clone();
    loop {
        if probe.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false) {
            return Err(ToolPathError(format!("symlinked path is not allowed: {raw:?}")));
        }
        if probe == *stage || probe.parent() == Some(&probe) {
            break;
        }
        match probe.parent() {
            Some(p) if p != probe => probe = p.to_path_buf(),
            _ => break,
        }
    }
    let resolved_parent = target.parent().and_then(|p| p.canonicalize().ok());
    if let Some(rp) = resolved_parent {
        if rp != stage_res && !rp.starts_with(&stage_res) {
            return Err(ToolPathError(format!("path escapes the working root: {raw:?}")));
        }
    }
    Ok(target)
}

fn is_stage_noise(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    name == ".DS_Store" || name.contains(".thumb.") || name.ends_with(".thumb.svg")
}

fn tree(stage: &Path) -> String {
    let mut out = Vec::new();
    let mut skipped = 0usize;
    for (p, is_link) in crate::integrity::iter_files(stage) {
        if is_link || is_stage_noise(&p) {
            skipped += 1;
            continue;
        }
        let rel = p.strip_prefix(stage).unwrap_or(&p).display();
        let sz = fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        out.push(format!("{rel}  ({sz} bytes)"));
    }
    out.sort();
    const CAP: usize = 80;
    let extra = out.len().saturating_sub(CAP);
    if extra > 0 {
        out.truncate(CAP);
        out.push(format!("… {extra} more files"));
    }
    if skipped > 0 {
        out.push(format!("({skipped} thumbs/noise omitted)"));
    }
    if out.is_empty() {
        "(no files yet)".into()
    } else {
        out.join("\n")
    }
}

fn compact_write_file_args(messages: &mut [Value]) {
    let mut write_idx: Vec<usize> = Vec::new();
    for (i, m) in messages.iter().enumerate() {
        if m.get("role").and_then(|r| r.as_str()) != Some("assistant") {
            continue;
        }
        let Some(calls) = m.get("tool_calls").and_then(|c| c.as_array()) else {
            continue;
        };
        if calls.iter().any(|c| {
            c.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                == Some("write_file")
        }) {
            write_idx.push(i);
        }
    }
    if write_idx.len() <= 1 {
        return;
    }
    let drop_end = write_idx.len() - 1;
    for &i in &write_idx[..drop_end] {
        let Some(calls) = messages[i].get_mut("tool_calls").and_then(|c| c.as_array_mut()) else {
            continue;
        };
        for call in calls {
            let Some(func) = call.get_mut("function") else {
                continue;
            };
            if func.get("name").and_then(|n| n.as_str()) != Some("write_file") {
                continue;
            }
            let Some(raw) = func.get("arguments").and_then(|a| a.as_str()) else {
                continue;
            };
            let Ok(mut args) = serde_json::from_str::<Value>(raw) else {
                continue;
            };
            let n = args
                .get("content")
                .and_then(|c| c.as_str())
                .map(|s| s.chars().count())
                .unwrap_or(0);
            if n <= COMPACT_TOOL_AFTER {
                continue;
            }
            let path = args
                .get("path")
                .and_then(|p| p.as_str())
                .unwrap_or("")
                .to_string();
            args["content"] = json!(format!("[already written, {n} chars]"));
            args["path"] = json!(path);
            func["arguments"] = json!(args.to_string());
        }
    }
}

fn compact_messages(messages: &mut [Value]) {
    let tool_idx: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.get("role").and_then(|r| r.as_str()) == Some("tool"))
        .map(|(i, _)| i)
        .collect();
    if tool_idx.len() > KEEP_TOOL_RESULTS {
        let drop_end = tool_idx.len() - KEEP_TOOL_RESULTS;
        for &i in &tool_idx[..drop_end] {
            let Some(c) = messages[i].get("content").and_then(|v| v.as_str()) else {
                continue;
            };
            if c.len() <= COMPACT_TOOL_AFTER || c.starts_with("[earlier tool result") {
                continue;
            }
            let n = c.chars().count();
            messages[i]["content"] = json!(format!(
                "[earlier tool result, {n} chars — already applied]"
            ));
        }
    }
    compact_write_file_args(messages);
}

/// How many model rounds a role may take. Author used to spin to 40 rewriting
/// the same five feature files.
pub fn max_steps_for(role: &str) -> usize {
    match role {
        "author" => 10,
        "designer" => 18,
        "stepwright" => 16,
        "implementer" => 16,
        "auditor" | "code_auditor" => 8,
        _ => 24,
    }
}

pub fn dispatch(stage: &Path, name: &str, args: &Value) -> String {
    match name {
        "list_files" => tree(stage),
        "read_file" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
            match safe_path(stage, path) {
                Err(e) => format!("REFUSED: {}", e.0),
                Ok(p) if !p.is_file() => format!("ERROR: no such file: {path:?}"),
                Ok(p) => fs::read_to_string(&p)
                    .unwrap_or_default()
                    .chars()
                    .take(MAX_READ)
                    .collect(),
            }
        }
        "write_file" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
            if content.len() > MAX_WRITE {
                return format!("ERROR: content too large ({} bytes)", content.len());
            }
            match safe_path(stage, path) {
                Err(e) => format!("REFUSED: {}", e.0),
                Ok(p) => {
                    if let Some(parent) = p.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    match fs::write(&p, content) {
                        Ok(()) => format!(
                            "wrote {} ({} bytes)",
                            p.strip_prefix(stage).unwrap_or(&p).display(),
                            content.len()
                        ),
                        Err(e) => format!("ERROR: {e}"),
                    }
                }
            }
        }
        "done" => "acknowledged".into(),
        other => format!("ERROR: unknown tool {other:?}"),
    }
}

fn tools_json(role: &str) -> Value {
    let mut tools = vec![
        json!({"type":"function","function":{"name":"list_files","description":"List every file you can see, with its size in bytes.","parameters":{"type":"object","properties":{},"required":[]}}}),
        json!({"type":"function","function":{"name":"read_file","description":"Read one file.","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}}),
        json!({"type":"function","function":{"name":"write_file","description":"Create or overwrite one file with the complete content.","parameters":{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}}}),
        json!({"type":"function","function":{"name":"done","description":"Call this when the work is complete.","parameters":{"type":"object","properties":{"summary":{"type":"string"}},"required":["summary"]}}}),
    ];
    // Fillers and auditors do not interview. Asking burns the step budget on yolo guesses.
    if !matches!(role, "stepwright" | "auditor" | "code_auditor") {
        tools.push(json!({"type":"function","function":{"name":"ask_human","description":"Ask the human one clarifying question. Always include a concrete guess they can accept or edit.","parameters":{"type":"object","properties":{"question":{"type":"string","description":"One question, in plain language."},"guess":{"type":"string","description":"Your best concrete answer: a typical example, default, or recommended choice. Always provide one."}},"required":["question","guess"]}}}));
    }
    json!(tools)
}

pub struct OpenAICompatBackend {
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub max_steps: usize,
    pub timeout_secs: u64,
    pub on_progress: Option<Box<dyn FnMut(&str) + Send>>,
    /// Called before each model round. `None` = continue, `Some(text)` = inject a user
    /// message then continue, `Err` = stop the turn.
    pub on_gate: Option<Box<dyn FnMut() -> Result<Option<String>, String> + Send>>,
    /// Clarifying questions. If unset, the model is told to assume.
    pub on_ask: Option<Box<dyn FnMut(&str, &str) -> Result<String, String> + Send>>,
    /// After a successful write_file: (relative path, content).
    pub on_write: Option<Box<dyn FnMut(&str, &str) + Send>>,
    /// Prompt and completion tokens from one API response.
    pub on_usage: Option<Box<dyn FnMut(i64, i64) + Send>>,
    pub last_prompt_tokens: i64,
    pub last_completion_tokens: i64,
}



impl OpenAICompatBackend {
    pub fn from_preset(
        preset: &str,
        model: Option<&str>,
        base_url: Option<&str>,
    ) -> Result<Self, String> {
        let (default_url, default_model, key_env, key_required) = match preset {
            "grok" => ("https://api.x.ai/v1", "grok-4.5", "XAI_API_KEY", true),
            "openai" => ("https://api.openai.com/v1", "gpt-4.1", "OPENAI_API_KEY", true),
            "codex" => ("https://api.openai.com/v1", "gpt-5-codex", "OPENAI_API_KEY", true),
            "claude" => ("https://api.anthropic.com/v1", "claude-sonnet-4-5", "ANTHROPIC_API_KEY", true),
            "qwen" | "ollama" => ("http://127.0.0.1:11434/v1", DEFAULT_QWEN_MODEL, "", false),
            other => return Err(format!("unknown api preset {other:?}")),
        };
        let api_key = if key_env.is_empty() {
            String::new()
        } else if key_env == "XAI_API_KEY" {
            xai_api_key().unwrap_or_default()
        } else if key_env == "ANTHROPIC_API_KEY" {
            config_secret(&["ANTHROPIC_API_KEY"], &["anthropic", "claude"]).unwrap_or_default()
        } else if key_env == "OPENAI_API_KEY" {
            config_secret(&["OPENAI_API_KEY"], &["openai", "codex"]).unwrap_or_default()
        } else {
            std::env::var(key_env).unwrap_or_default()
        };
        if key_required && api_key.is_empty() {
            return Err(format!(
                "no API key for '{preset}': set {key_env}, or put it in ~/.shalt/config.toml under [keys]"
            ));
        }
        let chosen = model.unwrap_or(default_model).to_string();
        let local = preset == "qwen" || preset == "ollama";
        Ok(Self {
            name: preset.into(),
            base_url: base_url.unwrap_or(default_url).trim_end_matches('/').into(),
            model: chosen.clone(),
            api_key,
            max_steps: 40,
            // Tiny local models answer in seconds. 27B MLX still needs minutes;
            // a 45s abort kills a live generation and Ollama keeps talking to a dead socket.
            timeout_secs: if local {
                crate::alloc::local_timeout_secs(&chosen)
            } else {
                180
            },
            on_progress: None,
            on_gate: None,
            on_ask: None,
            on_write: None,
            on_usage: None,
            last_prompt_tokens: 0,
            last_completion_tokens: 0,
        })
    }

    fn with_local_runtime(&self, mut payload: Value) -> Value {
        if !self.local() {
            return payload;
        }
        let max = crate::alloc::local_max_tokens(&self.model);
        let ctx = crate::alloc::local_num_ctx(&self.model);
        payload["max_tokens"] = json!(max);
        payload["keep_alive"] = json!(crate::alloc::LOCAL_KEEP_ALIVE);
        payload["options"] = json!({
            "num_ctx": ctx,
            "num_predict": max,
        });
        // Thinking eats decode speed. Writers need >>200 tok/s; 27B review
        // should sit at 20–30 tok/s unencumbered, not a hidden-think stall.
        payload["think"] = json!(false);
        payload["stream"] = json!(false);
        payload
    }

    fn emit(&mut self, line: &str) {
        if let Some(cb) = &mut self.on_progress {
            cb(line);
        }
    }

    fn local(&self) -> bool {
        self.name == "qwen" || self.name == "ollama"
    }

    fn note_usage(&mut self, data: &Value) {
        let u = match data.get("usage") {
            Some(v) => v,
            None => return,
        };
        let p = u.get("prompt_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
        let c = u
            .get("completion_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        self.last_prompt_tokens += p;
        self.last_completion_tokens += c;
        if let Some(cb) = &mut self.on_usage {
            cb(p, c);
        }
        if p + c > 0 {
            self.emit(&format!("tokens +{} (prompt {p} · completion {c})", p + c));
        }
    }

    fn fmt_elapsed(d: Duration) -> String {
        let s = d.as_secs();
        if s < 60 {
            format!("{s}s")
        } else {
            format!("{}m {:02}s", s / 60, s % 60)
        }
    }

    fn post(&mut self, payload: &Value) -> Result<Value, String> {
        // Native /api/chat honors options.num_ctx. OpenAI /v1/chat/completions
        // was loading 8B at 256k context and 0.6B at 40k.
        let url = if self.local() {
            "http://127.0.0.1:11434/api/chat".to_string()
        } else {
            format!("{}/chat/completions", self.base_url)
        };
        let slice = Duration::from_secs(self.timeout_secs);
        let deadline = Instant::now() + slice;
        let t0 = Instant::now();
        let mut n = 0u32;
        loop {
            n += 1;
            if let Some(gate) = &mut self.on_gate {
                match gate() {
                    Err(stop) => return Err(stop),
                    Ok(Some(_)) | Ok(None) => {}
                }
            }
            if Instant::now() > deadline {
                return Err(
                    "the model didn't respond in time. Retry when Ollama is free.".into(),
                );
            }
            let stop = Arc::new(AtomicBool::new(false));
            let stop2 = stop.clone();
            let model = self.model.clone();
            let cb = Arc::new(Mutex::new(self.on_progress.take()));
            let cb2 = cb.clone();
            let hb = thread::spawn(move || {
                let mut ticks = 0u32;
                while !stop2.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(500));
                    if stop2.load(Ordering::Relaxed) {
                        break;
                    }
                    ticks += 1;
                    // Every 2s so the desk moves while the HTTP call is outstanding.
                    if ticks % 4 != 0 {
                        continue;
                    }
                    let msg = format!(
                        "thinking · {model} · {}",
                        Self::fmt_elapsed(t0.elapsed())
                    );
                    if let Ok(mut g) = cb2.lock() {
                        if let Some(f) = g.as_mut() {
                            f(&msg);
                        }
                    }
                }
            });
            let remain = deadline.saturating_duration_since(Instant::now());
            let mut req = ureq::post(&url)
                .set("Content-Type", "application/json")
                .timeout(remain);
            if self.name == "claude" {
                req = req
                    .set("x-api-key", &self.api_key)
                    .set("anthropic-version", "2023-06-01");
            } else if !self.api_key.is_empty() {
                req = req.set("Authorization", &format!("Bearer {}", self.api_key));
            }
            let body = if self.local() {
                ollama_native_payload(payload)
            } else {
                payload.clone()
            };
            let result = req.send_json(body);
            stop.store(true, Ordering::Relaxed);
            let _ = hb.join();
            self.on_progress = cb.lock().ok().and_then(|mut g| g.take());
            match result {
                Ok(r) => {
                    let data: Value = r.into_json().map_err(|e| e.to_string())?;
                    let data = if self.local() {
                        openai_from_ollama(&data)
                    } else {
                        data
                    };
                    self.note_usage(&data);
                    if self.local() {
                        let secs = t0.elapsed().as_secs_f64();
                        if let Some(rate) = ollama_eval_rate(&data) {
                            self.emit(&format!("{rate:.0} tok/s · {}", self.model));
                            crate::speed::record(&self.model, rate, secs);
                        } else if secs > 0.0 {
                            crate::speed::record(&self.model, 0.0, secs);
                        }
                    }
                    return Ok(data);
                }
                Err(ureq::Error::Status(code, r)) => {
                    let detail: String = r.into_string().unwrap_or_default().chars().take(600).collect();
                    if matches!(code, 429 | 500 | 502 | 503 | 529) {
                        self.emit(&format!(
                            "HTTP {code} after {} — retrying",
                            Self::fmt_elapsed(t0.elapsed())
                        ));
                        thread::sleep(Duration::from_secs(2));
                        continue;
                    }
                    return Err(format!("{} API error HTTP {code}: {detail}", self.name));
                }
                Err(e) => {
                    let msg = e.to_string();
                    let timeout = msg.contains("timed out") || msg.contains("timeout");
                    if timeout {
                        return Err(if self.local() {
                            "the model didn't respond in time. Retry when Ollama is free.".into()
                        } else {
                            "the model didn't respond in time.".into()
                        });
                    }
                    if n < 4 && Instant::now() < deadline {
                        thread::sleep(Duration::from_secs(1 << n.min(3)));
                        continue;
                    }
                    return Err(format!("could not reach {}: {msg}", self.base_url));
                }
            }
        }
    }

    pub fn complete(&mut self, system: &str, history: &[(String, String)]) -> Result<String, String> {
        let mut messages = vec![json!({"role": "system", "content": system})];
        for (role, content) in history {
            messages.push(json!({"role": role, "content": content}));
        }
        let payload = self.with_local_runtime(json!({
            "model": self.model,
            "messages": messages,
            "temperature": 0.3,
        }));
        let data = self.post(&payload)?;
        let text = data
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if text.is_empty() {
            Err("empty reply".into())
        } else {
            Ok(text)
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelChoice {
    pub id: String,
    pub backend: String,
    pub label: String,
    pub kind: String,
}

/// An assignable worker: agent (who) plus the models it can run.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentInfo {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub models: Vec<String>,
    pub default_model: String,
}

/// Local default: Qwen 3.8 MLX on Ollama. Cloud default is Grok.
pub const DEFAULT_QWEN_MODEL: &str = "qwen3.8:27b-mlx";
pub const DEFAULT_GROK_MODEL: &str = "grok-4";

/// Always-on roster for assignment. Not a live dump of whoever answered /v1/models.
pub fn agent_roster() -> Vec<AgentInfo> {
    vec![
        AgentInfo {
            id: "qwen".into(),
            label: "Qwen".into(),
            kind: "local".into(),
            models: vec![
                "qwen3:0.6b".into(),
                "qwen3:1.7b".into(),
                "qwen3.5:2b-mlx".into(),
                "qwen3:8b".into(),
                DEFAULT_QWEN_MODEL.into(),
                "qwen3.8:27b-mtp-q4_K_M".into(),
            ],
            default_model: DEFAULT_QWEN_MODEL.into(),
        },
        AgentInfo {
            id: "grok".into(),
            label: "Grok".into(),
            kind: "cloud".into(),
            models: vec!["grok-4".into(), "grok-4.5".into()],
            default_model: "grok-4".into(),
        },
        AgentInfo {
            id: "openai".into(),
            label: "OpenAI".into(),
            kind: "cloud".into(),
            models: vec!["gpt-4.1".into(), "gpt-4o".into()],
            default_model: "gpt-4.1".into(),
        },
        AgentInfo {
            id: "claude".into(),
            label: "Claude".into(),
            kind: "cloud".into(),
            models: vec!["claude-sonnet-4-5".into(), "claude-opus-4-1".into()],
            default_model: "claude-sonnet-4-5".into(),
        },
    ]
}

pub fn roster_models() -> Vec<ModelChoice> {
    let mut out = Vec::new();
    for a in agent_roster() {
        for id in &a.models {
            out.push(ModelChoice {
                id: id.clone(),
                backend: a.id.clone(),
                label: format!("{} · {} ({})", a.label, id, a.kind),
                kind: a.kind.clone(),
            });
        }
    }
    out
}

#[derive(serde::Deserialize, Default)]
struct KeysFile {
    #[serde(default)]
    keys: KeyVals,
}

#[derive(serde::Deserialize, Default)]
struct KeyVals {
    #[serde(default)]
    xai: String,
    #[serde(default)]
    grok: String,
    #[serde(default)]
    openai: String,
    #[serde(default)]
    anthropic: String,
    #[serde(default)]
    claude: String,
    #[serde(default)]
    codex: String,
    #[serde(default)]
    markup: String,
    #[serde(default)]
    markup_secret: String,
    #[serde(default)]
    markup_agent: String,
}

pub fn config_secret_for_markup(envs: &[&str], toml_keys: &[&str]) -> Option<String> {
    config_secret(envs, toml_keys)
}

fn config_secret(envs: &[&str], toml_keys: &[&str]) -> Option<String> {
    for var in envs {
        if let Ok(v) = std::env::var(var) {
            let v = v.trim().to_string();
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    let path = crate::org::Org::home_dir().join("config.toml");
    let raw = std::fs::read_to_string(path).ok()?;
    let file: KeysFile = toml::from_str(&raw).ok()?;
    let vals = [
        ("xai", file.keys.xai.as_str()),
        ("grok", file.keys.grok.as_str()),
        ("openai", file.keys.openai.as_str()),
        ("anthropic", file.keys.anthropic.as_str()),
        ("claude", file.keys.claude.as_str()),
        ("codex", file.keys.codex.as_str()),
        ("markup", file.keys.markup.as_str()),
        ("markup_secret", file.keys.markup_secret.as_str()),
        ("markup_agent", file.keys.markup_agent.as_str()),
    ];
    for want in toml_keys {
        for (k, v) in vals {
            if k == *want {
                let v = v.trim();
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

/// xAI key: `XAI_API_KEY`, then `GROK_API_KEY`, then `~/.shalt/config.toml`.
pub fn xai_api_key() -> Option<String> {
    for var in ["XAI_API_KEY", "GROK_API_KEY"] {
        if let Ok(v) = std::env::var(var) {
            let v = v.trim().to_string();
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    let path = crate::org::Org::home_dir().join("config.toml");
    let raw = std::fs::read_to_string(path).ok()?;
    let file: KeysFile = toml::from_str(&raw).ok()?;
    for v in [file.keys.xai, file.keys.grok] {
        let v = v.trim();
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    None
}

pub fn ollama_reachable() -> bool {
    ureq::get("http://127.0.0.1:11434/v1/models")
        .timeout(Duration::from_secs(1))
        .call()
        .is_ok()
}

/// Other-session 35B-128k. Writers may unload 27B; they must not drop this one.
pub fn ollama_leave_loaded(name: &str) -> bool {
    name.contains("35b-128k")
}

/// Drop loaded Ollama models except `keep` and protected 35B-128k.
pub fn unload_local_except(keep: &str) {
    let Ok(r) = ureq::get("http://127.0.0.1:11434/api/ps")
        .timeout(Duration::from_secs(2))
        .call()
    else {
        return;
    };
    let Ok(v) = r.into_json::<Value>() else {
        return;
    };
    let Some(models) = v.get("models").and_then(|m| m.as_array()) else {
        return;
    };
    for m in models {
        let name = m.get("name").and_then(|n| n.as_str()).unwrap_or("");
        if name.is_empty() || name == keep || ollama_leave_loaded(name) {
            continue;
        }
        let body = json!({ "model": name, "keep_alive": 0 });
        let _ = ureq::post("http://127.0.0.1:11434/api/generate")
            .timeout(Duration::from_secs(4))
            .set("Content-Type", "application/json")
            .send_json(body);
    }
}

/// Load `model` at the capped context and pin it. Unload 27B so flash writers are not starved.
pub fn keep_local_model(model: &str) {
    if model.trim().is_empty() {
        return;
    }
    unload_local_except(model);
    let body = json!({
        "model": model,
        "keep_alive": crate::alloc::LOCAL_KEEP_ALIVE,
        "stream": false,
        "options": { "num_ctx": crate::alloc::local_num_ctx(model) },
    });
    let wait = if crate::alloc::is_fast_model(model) { 20 } else { 45 };
    let _ = ureq::post("http://127.0.0.1:11434/api/generate")
        .timeout(Duration::from_secs(wait))
        .set("Content-Type", "application/json")
        .send_json(body);
}

fn ollama_eval_rate(data: &Value) -> Option<f64> {
    let n = data
        .get("eval_count")
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as f64;
    let ns = data
        .get("eval_duration")
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as f64;
    if n > 0.0 && ns > 0.0 {
        Some(n / (ns / 1e9))
    } else {
        None
    }
}

/// Map Ollama /api/chat JSON onto the OpenAI shape the tool loop already reads.
pub(crate) fn openai_from_ollama(data: &Value) -> Value {
    if data.get("choices").is_some() {
        return data.clone();
    }
    let msg = data.get("message").cloned().unwrap_or(json!({}));
    let mut message = json!({
        "role": msg.get("role").and_then(|r| r.as_str()).unwrap_or("assistant"),
        "content": msg.get("content").cloned().unwrap_or(json!("")),
    });
    if let Some(calls) = msg.get("tool_calls").and_then(|c| c.as_array()) {
        let mapped: Vec<Value> = calls
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let func = c.get("function").cloned().unwrap_or(json!({}));
                let args = func.get("arguments").cloned().unwrap_or(json!({}));
                let args_s = if args.is_string() {
                    args.as_str().unwrap_or("{}").to_string()
                } else {
                    args.to_string()
                };
                json!({
                    "id": c.get("id").cloned().unwrap_or(json!(format!("c{i}"))),
                    "type": "function",
                    "function": {
                        "name": func.get("name").cloned().unwrap_or(json!("")),
                        "arguments": args_s,
                    }
                })
            })
            .collect();
        if !mapped.is_empty() {
            message["tool_calls"] = json!(mapped);
        }
    }
    let p = data
        .get("prompt_eval_count")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let c = data.get("eval_count").and_then(|v| v.as_i64()).unwrap_or(0);
    let mut out = json!({
        "choices": [{ "message": message }],
        "usage": { "prompt_tokens": p, "completion_tokens": c },
    });
    if let Some(n) = data.get("eval_count") {
        out["eval_count"] = n.clone();
    }
    if let Some(n) = data.get("eval_duration") {
        out["eval_duration"] = n.clone();
    }
    out
}

/// Ollama /api/chat wants tool `arguments` as a JSON object. The OpenAI loop
/// stores them as strings; sending the string makes Ollama 400
/// "Value looks like object, but can't find closing '}' symbol".
pub(crate) fn ollama_native_payload(payload: &Value) -> Value {
    let mut out = payload.clone();
    let Some(msgs) = out.get_mut("messages").and_then(|m| m.as_array_mut()) else {
        return out;
    };
    for m in msgs {
        let Some(calls) = m.get_mut("tool_calls").and_then(|c| c.as_array_mut()) else {
            continue;
        };
        for call in calls {
            let Some(func) = call.get_mut("function") else {
                continue;
            };
            let Some(raw) = func.get("arguments") else {
                continue;
            };
            if raw.is_object() || raw.is_array() {
                continue;
            }
            let parsed = raw
                .as_str()
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .unwrap_or_else(|| json!({}));
            func["arguments"] = parsed;
        }
    }
    out
}

fn id_has_date(id: &str) -> bool {
    let b = id.as_bytes();
    b.windows(10).any(|w| {
        w[4] == b'-'
            && w[7] == b'-'
            && w[0..4].iter().all(u8::is_ascii_digit)
            && w[5..7].iter().all(u8::is_ascii_digit)
            && w[8..10].iter().all(u8::is_ascii_digit)
    })
}

fn usable_model(id: &str, kind: &str) -> bool {
    let l = id.to_lowercase();
    for skip in [
        "embed",
        "tts",
        "whisper",
        "dall-e",
        "image",
        "audio",
        "realtime",
        "moderation",
        "davinci",
        "babbage",
        "sora",
        "transcribe",
        "imagine",
        "computer-use",
    ] {
        if l.contains(skip) {
            return false;
        }
    }
    if kind == "cloud" && id_has_date(id) {
        return false;
    }
    true
}

fn push_openai_compat(out: &mut Vec<ModelChoice>, url: &str, key: Option<&str>, backend: &str, kind: &str) {
    let mut req = ureq::get(url).timeout(Duration::from_secs(2));
    if let Some(k) = key {
        if !k.is_empty() {
            req = req.set("Authorization", &format!("Bearer {k}"));
        }
    }
    let Ok(r) = req.call() else { return };
    let Ok(v) = r.into_json::<Value>() else { return };
    let Some(data) = v.get("data").and_then(|d| d.as_array()) else { return };
    for m in data {
        let id = m.get("id").and_then(|i| i.as_str()).unwrap_or("");
        if id.is_empty() {
            continue;
        }
        if !usable_model(id, kind) {
            continue;
        }
        out.push(ModelChoice {
            id: id.into(),
            backend: backend.into(),
            label: format!("{id} ({kind})"),
            kind: kind.into(),
        });
    }
}

pub fn list_models() -> Vec<ModelChoice> {
    let mut out = Vec::new();
    if let Some(key) = xai_api_key() {
        let before = out.len();
        push_openai_compat(&mut out, "https://api.x.ai/v1/models", Some(&key), "grok", "cloud");
        if out.len() == before {
            out.push(ModelChoice {
                id: "grok-4.5".into(),
                backend: "grok".into(),
                label: "grok-4.5 (cloud)".into(),
                kind: "cloud".into(),
            });
        }
    }
    if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        if !key.trim().is_empty() {
            push_openai_compat(
                &mut out,
                "https://api.openai.com/v1/models",
                Some(key.trim()),
                "openai",
                "cloud",
            );
        }
    }
    push_openai_compat(
        &mut out,
        "http://127.0.0.1:11434/v1/models",
        None,
        "ollama",
        "local",
    );
    out.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.id.cmp(&b.id)));
    out
}

pub fn preferred_model() -> Option<(String, String)> {
    let path = crate::org::Org::home_dir().join("config.toml");
    let raw = std::fs::read_to_string(path).ok()?;
    let table: toml::Table = raw.parse().ok()?;
    let m = table.get("model")?.as_table()?;
    let backend = m.get("backend")?.as_str()?.to_string();
    let id = m.get("id")?.as_str()?.to_string();
    if backend.is_empty() || id.is_empty() {
        None
    } else {
        Some((backend, id))
    }
}

pub fn save_preferred_model(backend: &str, id: &str) -> std::io::Result<()> {
    let path = crate::org::Org::home_dir().join("config.toml");
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut table: toml::Table = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_default();
    let mut model = toml::Table::new();
    model.insert("backend".into(), toml::Value::String(backend.into()));
    model.insert("id".into(), toml::Value::String(id.into()));
    table.insert("model".into(), toml::Value::Table(model));
    std::fs::write(path, format!("{table}"))
}

/// Which cloud keys are present — never the secret itself.
#[derive(Debug, Clone, serde::Serialize, Default)]
pub struct KeySlot {
    pub set: bool,
    /// `env`, `config`, or empty.
    pub from: String,
}

#[derive(Debug, Clone, serde::Serialize, Default)]
pub struct KeysStatus {
    pub xai: KeySlot,
    pub openai: KeySlot,
    pub anthropic: KeySlot,
    pub path: String,
}

/// Fields that are `None` are left alone. Empty string clears the saved key.
#[derive(Debug, Clone, serde::Deserialize, Default)]
pub struct KeysPatch {
    pub xai: Option<String>,
    pub openai: Option<String>,
    pub anthropic: Option<String>,
}

fn slot_from(envs: &[&str], config_vals: &[&str]) -> KeySlot {
    for var in envs {
        if let Ok(v) = std::env::var(var) {
            if !v.trim().is_empty() {
                return KeySlot {
                    set: true,
                    from: "env".into(),
                };
            }
        }
    }
    for v in config_vals {
        if !v.trim().is_empty() {
            return KeySlot {
                set: true,
                from: "config".into(),
            };
        }
    }
    KeySlot::default()
}

pub fn keys_config_path() -> PathBuf {
    crate::org::Org::home_dir().join("config.toml")
}

pub fn keys_status() -> KeysStatus {
    keys_status_at(&keys_config_path())
}

pub fn keys_status_at(path: &Path) -> KeysStatus {
    let file: KeysFile = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default();
    KeysStatus {
        xai: slot_from(
            &["XAI_API_KEY", "GROK_API_KEY"],
            &[file.keys.xai.as_str(), file.keys.grok.as_str()],
        ),
        openai: slot_from(
            &["OPENAI_API_KEY"],
            &[file.keys.openai.as_str(), file.keys.codex.as_str()],
        ),
        anthropic: slot_from(
            &["ANTHROPIC_API_KEY"],
            &[file.keys.anthropic.as_str(), file.keys.claude.as_str()],
        ),
        path: path.display().to_string(),
    }
}

pub fn save_keys(patch: KeysPatch) -> Result<KeysStatus, String> {
    save_keys_at(&keys_config_path(), patch)
}

pub fn save_keys_at(path: &Path, patch: KeysPatch) -> Result<KeysStatus, String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut table: toml::Table = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_default();
    let mut keys = table
        .get("keys")
        .and_then(|v| v.as_table())
        .cloned()
        .unwrap_or_default();
    let apply = |keys: &mut toml::Table, field: &str, val: &Option<String>| {
        let Some(s) = val else { return };
        let s = s.trim();
        if s.is_empty() {
            keys.remove(field);
        } else {
            keys.insert(field.into(), toml::Value::String(s.into()));
        }
    };
    apply(&mut keys, "xai", &patch.xai);
    apply(&mut keys, "openai", &patch.openai);
    apply(&mut keys, "anthropic", &patch.anthropic);
    table.insert("keys".into(), toml::Value::Table(keys));
    std::fs::write(path, format!("{table}\n")).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(keys_status_at(path))
}

#[cfg(test)]
mod packed_prompt_tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn stepwright_user_message_is_the_brief_not_the_tree() {
        let t = TempDir::new().unwrap();
        let u = packed_user("stepwright", "Fill pending bodies.", t.path());
        assert_eq!(u, "Fill pending bodies.");
        assert!(!u.contains("Files you can see:"));
        let a = packed_user("author", "Write spec.", t.path());
        assert!(a.contains("Files you can see:"));
        let s = packed_system("stepwright", "qwen3.5:2b-mlx", t.path());
        assert!(!s.contains("function (not arrow)"));
        assert!(s.len() < 900, "{}", s.len());
    }
}

#[cfg(test)]
mod key_tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn save_keys_merges_without_clobbering_model() {
        let t = TempDir::new().unwrap();
        let path = t.path().join("config.toml");
        std::fs::write(&path, "[model]\nbackend = \"qwen\"\nid = \"qwen3\"\n").unwrap();
        save_keys_at(
            &path,
            KeysPatch {
                xai: Some("xai-test-key".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("xai-test-key"));
        assert!(raw.contains("qwen"));
        assert!(keys_status_at(&path).xai.set);
    }

    #[test]
    fn keys_status_never_returns_the_secret() {
        let t = TempDir::new().unwrap();
        let path = t.path().join("config.toml");
        save_keys_at(
            &path,
            KeysPatch {
                openai: Some("sk-secret-do-not-echo".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let json = serde_json::to_string(&keys_status_at(&path)).unwrap();
        assert!(!json.contains("sk-secret-do-not-echo"));
        assert!(json.contains("\"set\":true"));
    }

    #[test]
    fn empty_patch_clears_a_saved_key() {
        let t = TempDir::new().unwrap();
        let path = t.path().join("config.toml");
        save_keys_at(
            &path,
            KeysPatch {
                anthropic: Some("claude-key".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(keys_status_at(&path).anthropic.set);
        save_keys_at(
            &path,
            KeysPatch {
                anthropic: Some("".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!keys_status_at(&path).anthropic.set);
    }

    #[test]
    fn compact_messages_keeps_recent_tool_results() {
        let mut msgs = vec![
            json!({"role":"system","content":"sys"}),
            json!({"role":"user","content":"go"}),
        ];
        for i in 0..5 {
            msgs.push(json!({"role":"assistant","content":"","tool_calls":[{"id":format!("c{i}")}]}));
            msgs.push(json!({
                "role":"tool",
                "tool_call_id": format!("c{i}"),
                "content": "x".repeat(3000)
            }));
        }
        compact_messages(&mut msgs);
        let tools: Vec<&str> = msgs
            .iter()
            .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("tool"))
            .map(|m| m.get("content").and_then(|c| c.as_str()).unwrap_or(""))
            .collect();
        assert_eq!(tools.len(), 5);
        assert!(tools[0].starts_with("[earlier tool result"), "{}", tools[0]);
        assert!(tools[2].starts_with("[earlier tool result"), "{}", tools[2]);
        assert_eq!(tools[3].len(), 3000);
        assert_eq!(tools[4].len(), 3000);
    }

    #[test]
    fn compact_messages_stubs_old_write_file_payloads() {
        let mut msgs = vec![json!({"role":"user","content":"go"})];
        for i in 0..3 {
            msgs.push(json!({
                "role":"assistant",
                "content":"",
                "tool_calls":[{
                    "id": format!("w{i}"),
                    "function":{
                        "name":"write_file",
                        "arguments": serde_json::to_string(&json!({
                            "path": format!("spec/f{i}.feature"),
                            "content": "x".repeat(4000)
                        })).unwrap()
                    }
                }]
            }));
            msgs.push(json!({"role":"tool","tool_call_id":format!("w{i}"),"content":"wrote ok"}));
        }
        compact_messages(&mut msgs);
        let args: Vec<String> = msgs
            .iter()
            .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("assistant"))
            .filter_map(|m| {
                m.get("tool_calls")
                    .and_then(|c| c.as_array())
                    .and_then(|a| a[0].get("function"))
                    .and_then(|f| f.get("arguments"))
                    .and_then(|a| a.as_str())
                    .map(|s| s.to_string())
            })
            .collect();
        assert_eq!(args.len(), 3);
        assert!(args[0].contains("already written"), "{}", args[0]);
        assert!(args[1].contains("already written"), "{}", args[1]);
        assert!(args[2].contains(&"x".repeat(4000)), "latest write stays full");
    }

    #[test]
    fn leave_the_35b_128k_session_loaded() {
        assert!(ollama_leave_loaded("qwen3.5:35b-128k"));
        assert!(!ollama_leave_loaded("qwen3.8:27b-mlx"));
        assert!(!ollama_leave_loaded("qwen3:0.6b"));
    }

    #[test]
    fn author_is_capped_to_ten_model_rounds() {
        assert_eq!(max_steps_for("author"), 10);
        assert_eq!(max_steps_for("designer"), 18);
        assert_eq!(max_steps_for("auditor"), 8);
        assert_eq!(max_steps_for("code_auditor"), 8);
        assert!(max_steps_for("author") < max_steps_for("implementer"));
    }

    #[test]
    fn local_runtime_caps_fast_model() {
        let b = OpenAICompatBackend::from_preset("qwen", Some("qwen3.5:2b-mlx"), None).unwrap();
        assert_eq!(b.timeout_secs, 240);
        let p = b.with_local_runtime(json!({"model": b.model, "messages": []}));
        assert_eq!(p["max_tokens"], 3072);
        assert_eq!(p["keep_alive"], "45m");
        assert_eq!(p["options"]["num_ctx"], 8192);
        assert_eq!(p["options"]["num_predict"], 3072);
        assert_eq!(p["think"], false);
    }

    #[test]
    fn local_runtime_caps_flash_model() {
        let b = OpenAICompatBackend::from_preset("qwen", Some("qwen3:0.6b"), None).unwrap();
        assert_eq!(b.timeout_secs, 90);
        let p = b.with_local_runtime(json!({"model": b.model, "messages": []}));
        assert_eq!(p["max_tokens"], 1536);
        assert_eq!(p["options"]["num_ctx"], 4096);
        assert_eq!(p["think"], false);
    }

    #[test]
    fn local_runtime_caps_review_model() {
        let b = OpenAICompatBackend::from_preset("qwen", Some("qwen3.8:27b-mlx"), None).unwrap();
        assert!(b.timeout_secs >= 600);
        let p = b.with_local_runtime(json!({"model": b.model, "messages": []}));
        assert_eq!(p["max_tokens"], 4096);
        assert_eq!(p["options"]["num_ctx"], 16384);
        assert_eq!(p["think"], false);
    }

    #[test]
    fn ollama_chat_maps_to_openai_tools() {
        let raw = json!({
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "function": { "name": "done", "arguments": { "summary": "PASS" } }
                }]
            },
            "prompt_eval_count": 10,
            "eval_count": 4,
            "eval_duration": 200_000_000
        });
        let v = openai_from_ollama(&raw);
        assert_eq!(v["usage"]["completion_tokens"], 4);
        let args = v["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"]
            .as_str()
            .unwrap();
        assert!(args.contains("PASS"), "{args}");
        assert!((ollama_eval_rate(&v).unwrap() - 20.0).abs() < 0.1);
    }

    #[test]
    fn ollama_native_payload_parses_argument_strings() {
        let p = json!({
            "model": "qwen3:8b",
            "messages": [
                {"role":"system","content":"sys"},
                {
                    "role":"assistant",
                    "content":"",
                    "tool_calls":[{
                        "id":"c0",
                        "type":"function",
                        "function":{
                            "name":"write_file",
                            "arguments": "{\"path\":\"steps/a.js\",\"content\":\"x\"}"
                        }
                    }]
                }
            ]
        });
        let n = ollama_native_payload(&p);
        let args = &n["messages"][1]["tool_calls"][0]["function"]["arguments"];
        assert!(args.is_object(), "{args}");
        assert_eq!(args["path"], "steps/a.js");
        assert_eq!(args["content"], "x");
    }

    #[test]
    fn cloud_runtime_does_not_add_ollama_options() {
        let b = OpenAICompatBackend::from_preset("openai", Some("gpt-4.1"), None);
        if let Ok(b) = b {
            let p = b.with_local_runtime(json!({"model": b.model, "messages": []}));
            assert!(p.get("keep_alive").is_none());
            assert!(p.get("options").is_none());
        }
    }
}

impl Backend for OpenAICompatBackend {
    fn name(&self) -> &str {
        &self.name
    }

    fn usage(&self) -> (i64, i64) {
        (self.last_prompt_tokens, self.last_completion_tokens)
    }

    fn run(&mut self, role: &str, prompt: &str, stage: &Path) -> Result<String, String> {
        self.last_prompt_tokens = 0;
        self.last_completion_tokens = 0;
        let mut messages = vec![
            json!({"role":"system","content": packed_system(role, &self.model, stage)}),
            json!({"role":"user","content": packed_user(role, prompt, stage)}),
        ];
        let mut transcript = Vec::new();
        self.emit(&format!("contacting {}…", self.model));
        let steps = self.max_steps.min(max_steps_for(role));
        for step in 0..steps {
            if let Some(gate) = &mut self.on_gate {
                match gate() {
                    Ok(None) => {}
                    Ok(Some(inject)) => {
                        self.emit("prompt revised by human — injecting into the next round");
                        messages.push(json!({"role": "user", "content": inject}));
                    }
                    Err(stop) => {
                        self.emit(&stop);
                        return Err(stop);
                    }
                }
            }
            let verb = match role {
                "author" => "specifying",
                "designer" => "drawing",
                "stepwright" => "writing tests",
                "implementer" => "writing code",
                "auditor" | "code_auditor" => "auditing",
                _ => "working",
            };
            self.emit(&format!(
                "step {} of {} · {} is {verb}…",
                step + 1,
                steps,
                self.model
            ));
            let payload = self.with_local_runtime(json!({
                "model": self.model,
                "messages": messages,
                "tools": tools_json(role),
                "tool_choice": "auto",
                "temperature": 0.0,
            }));
            let data = self.post(&payload)?;
            let choices = data.get("choices").and_then(|c| c.as_array()).cloned().unwrap_or_default();
            if choices.is_empty() {
                return Err(format!("{}: empty response: {}", self.name, data.to_string().chars().take(400).collect::<String>()));
            }
            let msg = choices[0].get("message").cloned().unwrap_or(json!({}));
            let calls = msg.get("tool_calls").cloned().unwrap_or(json!([]));
            let mut assistant = json!({"role":"assistant","content": msg.get("content").cloned().unwrap_or(json!(""))});
            if let Some(arr) = calls.as_array() {
                if !arr.is_empty() {
                    assistant["tool_calls"] = calls.clone();
                }
            }
            messages.push(assistant);
            if let Some(text) = msg.get("content").and_then(|c| c.as_str()) {
                if !text.trim().is_empty() {
                    transcript.push(text.to_string());
                }
            }
            let arr = calls.as_array().cloned().unwrap_or_default();
            if arr.is_empty() {
                let text = msg.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string();
                if text.contains('?') {
                    if let Some(cb) = &mut self.on_ask {
                        let a = cb(&text, "")?;
                        messages.push(json!({
                            "role": "user",
                            "content": format!("The human answered:\n{a}\n\nContinue. Ask at most one question at a time with ask_human, or write the spec files.")
                        }));
                        continue;
                    }
                }
                if !text.trim().is_empty() {
                    self.emit(&text);
                }
                transcript.push(text);
                break;
            }
            let mut finished = false;
            for call in arr {
                let fn_obj = call.get("function").cloned().unwrap_or(json!({}));
                let fname = fn_obj.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let args = match serde_json::from_str::<Value>(fn_obj.get("arguments").and_then(|a| a.as_str()).unwrap_or("{}")) {
                    Ok(v) => v,
                    Err(_) => json!({}),
                };
                let result = if fn_obj.get("arguments").and_then(|a| a.as_str()).map(|s| serde_json::from_str::<Value>(s).is_err()).unwrap_or(false) {
                    "ERROR: arguments were not valid JSON".into()
                } else if fname == "ask_human" {
                    if matches!(role, "stepwright" | "auditor" | "code_auditor") {
                        "Do not ask. Fill pending bodies in the one file in the brief, then done().".into()
                    } else {
                    let q = args.get("question").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
                    if q.is_empty() {
                        "ERROR: question is required".into()
                    } else {
                        let guess = args.get("guess").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
                        let replied = if let Some(cb) = &mut self.on_ask {
                            cb(&q, &guess)
                        } else {
                            Ok(if guess.is_empty() { ASSUME_REPLY.to_string() } else { guess.clone() })
                        };
                        match replied {
                            Ok(a) => a,
                            Err(e) => return Err(e),
                        }
                    }
                    }
                } else {
                    let result = dispatch(stage, fname, &args);
                    if fname == "write_file" && result.starts_with("wrote ") {
                        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
                        let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
                        if let Some(cb) = &mut self.on_write {
                            cb(path, content);
                        }
                    }
                    result
                };
                let line = match fname {
                    "ask_human" => "[ask_human] answered".into(),
                    "read_file" => format!(
                        "[read_file] {}",
                        args.get("path").and_then(|v| v.as_str()).unwrap_or("")
                    ),
                    "list_files" => {
                        let n = result.lines().filter(|l| !l.is_empty() && *l != "(no files yet)").count();
                        format!("[list_files] {n} file(s)")
                    }
                    "write_file" => result.chars().take(120).collect::<String>(),
                    "done" => {
                        let summary = args.get("summary").and_then(|v| v.as_str()).unwrap_or("");
                        if !summary.is_empty() {
                            transcript.push(summary.to_string());
                        }
                        format!("[done] {}", summary.chars().take(80).collect::<String>())
                    }
                    other => format!("[{other}] {}", result.chars().take(80).collect::<String>()),
                };
                self.emit(&line);
                transcript.push(line);
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call.get("id").cloned().unwrap_or(json!("")),
                    "content": result.chars().take(MAX_READ).collect::<String>(),
                }));
                if fname == "done" {
                    finished = true;
                }
            }
            compact_messages(&mut messages);
            if finished {
                break;
            }
            if step + 1 == steps {
                transcript.push(format!("(stopped after {steps} steps)"));
            }
        }
        Ok(transcript.join("\n"))
    }
}
