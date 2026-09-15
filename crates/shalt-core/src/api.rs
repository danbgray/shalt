//! OpenAI-compatible chat-completions backend (Grok / OpenAI).
//!
//! Four scoped tools. Path refusals return `REFUSED:` to the model. The workspace
//! guard around `run_role` is still the guarantee.

use crate::backends::Backend;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

const MAX_READ: usize = 60_000;
const MAX_WRITE: usize = 400_000;
pub const ASSUME_REPLY: &str = "The human is not taking questions this turn. Make a reasonable assumption, write it into the scenario as a concrete example, and continue.";

pub const ROLE_SYSTEM: &[(&str, &str)] = &[
    ("author", "You are the SPEC AUTHOR in a BDD pipeline. You write Gherkin feature files under spec/. If src/ contains code, this is an existing system: describe behaviour that is already implemented, do not invent features, do not modify src/. If src/ is empty, translate the human's request into new scenarios. If anything needed to write *concrete* scenarios is missing or ambiguous, call ask_human *before* write_file. Ask one question at a time. One behaviour per scenario, concrete example values. Tag each Feature with @epic:<area> (one token, e.g. @epic:planning) so related features group; omit the tag rather than inventing a junk area. Do not write code, tests, or step definitions. Only create files under spec/."),
    ("stepwright", "You are the STEPWRIGHT in a BDD pipeline. You see the approved Gherkin spec and NOTHING of the implementation -- that is deliberate. Write pytest-bdd step definitions under steps/ that bind each scenario to the behaviour it describes, and declare the public API surface you call in contract/interface.md. Import only from that declared surface. Never weaken an assertion to make it easier to satisfy; you are the oracle, not the builder. Only create files under steps/ and contract/."),
    ("implementer", "You are the IMPLEMENTER in a BDD pipeline. You see the spec, the interface contract, and the failing test output -- you do NOT see the step definitions, and you cannot edit them. Write code under src/ that satisfies the specified behaviour against the contract. Do not special-case test inputs or hard-code expected outputs; implement the behaviour. Only create files under src/."),
];

pub fn system_for(role: &str) -> &'static str {
    ROLE_SYSTEM
        .iter()
        .find(|(r, _)| *r == role)
        .map(|(_, s)| *s)
        .unwrap_or("You are an agent in a shalt workspace. Work only through the provided tools.")
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

fn tree(stage: &Path) -> String {
    let mut out = Vec::new();
    for (p, is_link) in crate::integrity::iter_files(stage) {
        if is_link {
            continue;
        }
        let rel = p.strip_prefix(stage).unwrap_or(&p).display();
        let sz = fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        out.push(format!("{rel}  ({sz} bytes)"));
    }
    out.sort();
    if out.is_empty() {
        "(no files yet)".into()
    } else {
        out.join("\n")
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

fn tools_json() -> Value {
    json!([
        {"type":"function","function":{"name":"list_files","description":"List every file you can see, with its size in bytes.","parameters":{"type":"object","properties":{},"required":[]}}},
        {"type":"function","function":{"name":"read_file","description":"Read one file.","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}},
        {"type":"function","function":{"name":"write_file","description":"Create or overwrite one file with the complete content.","parameters":{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}}},
        {"type":"function","function":{"name":"done","description":"Call this when the work is complete.","parameters":{"type":"object","properties":{"summary":{"type":"string"}},"required":["summary"]}}},
        {"type":"function","function":{"name":"ask_human","description":"Ask the human one clarifying question. Always include a concrete guess they can accept or edit.","parameters":{"type":"object","properties":{"question":{"type":"string","description":"One question, in plain language."},"guess":{"type":"string","description":"Your best concrete answer: a typical example, default, or recommended choice. Always provide one."}},"required":["question","guess"]}}}
    ])
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
            "qwen" | "ollama" => ("http://127.0.0.1:11434/v1", "qwen3.5:35b-128k", "", false),
            other => return Err(format!("unknown api preset {other:?}")),
        };
        let api_key = if key_env.is_empty() {
            String::new()
        } else if key_env == "XAI_API_KEY" {
            xai_api_key().unwrap_or_default()
        } else {
            std::env::var(key_env).unwrap_or_default()
        };
        if key_required && api_key.is_empty() {
            return Err(format!(
                "no API key for '{preset}': set {key_env} (or GROK_API_KEY), or put it in ~/.shalt/config.toml"
            ));
        }
        Ok(Self {
            name: preset.into(),
            base_url: base_url.unwrap_or(default_url).trim_end_matches('/').into(),
            model: model.unwrap_or(default_model).into(),
            api_key,
            max_steps: 40,
            timeout_secs: if preset == "qwen" || preset == "ollama" { 45 } else { 180 },
            on_progress: None,
            on_gate: None,
            on_ask: None,
            on_write: None,
            on_usage: None,
            last_prompt_tokens: 0,
            last_completion_tokens: 0,
        })
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

    fn post(&mut self, payload: &Value) -> Result<Value, String> {
        let url = format!("{}/chat/completions", self.base_url);
        let slice = Duration::from_secs(if self.local() { 45 } else { self.timeout_secs });
        let deadline = std::time::Instant::now()
            + Duration::from_secs(if self.local() { 900 } else { self.timeout_secs * 4 });
        let mut n = 0u32;
        loop {
            n += 1;
            if let Some(gate) = &mut self.on_gate {
                match gate() {
                    Err(stop) => return Err(stop),
                    Ok(Some(_)) | Ok(None) => {}
                }
            }
            if std::time::Instant::now() > deadline {
                return Err(
                    "the model didn't respond in time. Retry when Ollama is free.".into(),
                );
            }
            if n > 1 {
                self.emit("still waiting on the model…");
            }
            let mut req = ureq::post(&url)
                .set("Content-Type", "application/json")
                .timeout(slice);
            if !self.api_key.is_empty() {
                req = req.set("Authorization", &format!("Bearer {}", self.api_key));
            }
            match req.send_json(payload.clone()) {
                Ok(r) => {
                    let data: Value = r.into_json().map_err(|e| e.to_string())?;
                    self.note_usage(&data);
                    return Ok(data);
                }
                Err(ureq::Error::Status(code, r)) => {
                    let detail: String = r.into_string().unwrap_or_default().chars().take(600).collect();
                    if matches!(code, 429 | 500 | 502 | 503 | 529) {
                        thread::sleep(Duration::from_secs(2));
                        continue;
                    }
                    return Err(format!("{} API error HTTP {code}: {detail}", self.name));
                }
                Err(e) => {
                    let msg = e.to_string();
                    let timeout = msg.contains("timed out") || msg.contains("timeout");
                    if timeout && self.local() {
                        continue;
                    }
                    if n < 4 {
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
        let payload = json!({
            "model": self.model,
            "messages": messages,
            "temperature": 0.3,
        });
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

impl Backend for OpenAICompatBackend {
    fn name(&self) -> &str {
        &self.name
    }

    fn run(&mut self, role: &str, prompt: &str, stage: &Path) -> Result<String, String> {
        let mut messages = vec![
            json!({"role":"system","content": format!(
                "{}\n\nYou work only through the provided tools. Every path is relative to your working root. Read what you need first, then write complete files -- never fragments or diffs. Call done() when finished.",
                system_for(role)
            )}),
            json!({"role":"user","content": format!("{prompt}\n\nFiles you can see:\n{}", tree(stage))}),
        ];
        let mut transcript = Vec::new();
        self.emit(&format!("contacting {}…", self.model));
        for step in 0..self.max_steps {
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
            self.emit(&format!("step {}: waiting on the model…", step + 1));
            let payload = json!({
                "model": self.model,
                "messages": messages,
                "tools": tools_json(),
                "tool_choice": "auto",
                "temperature": 0.0,
            });
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
                let line = if fname == "ask_human" {
                    "[ask_human] answered".into()
                } else {
                    format!("[{fname}] {}", result.chars().take(200).collect::<String>())
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
            if finished {
                break;
            }
            if step + 1 == self.max_steps {
                transcript.push(format!("(stopped after {} steps)", self.max_steps));
            }
        }
        Ok(transcript.join("\n"))
    }
}
