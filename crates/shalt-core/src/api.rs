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

pub const ROLE_SYSTEM: &[(&str, &str)] = &[
    ("author", "You are the SPEC AUTHOR in a BDD pipeline. You translate a plain-English request into Gherkin feature files under spec/. Write scenarios that a non-engineer stakeholder could read and approve. One behaviour per scenario. Prefer concrete example values over vague wording. Do not write code, tests, or step definitions. Only create files under spec/."),
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
        {"type":"function","function":{"name":"done","description":"Call this when the work is complete.","parameters":{"type":"object","properties":{"summary":{"type":"string"}},"required":["summary"]}}}
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
        } else {
            std::env::var(key_env).unwrap_or_default()
        };
        if key_required && api_key.is_empty() {
            return Err(format!("no API key for '{preset}': set {key_env} in the environment"));
        }
        Ok(Self {
            name: preset.into(),
            base_url: base_url.unwrap_or(default_url).trim_end_matches('/').into(),
            model: model.unwrap_or(default_model).into(),
            api_key,
            max_steps: 40,
            timeout_secs: 180,
            on_progress: None,
            on_gate: None,
        })
    }

    fn emit(&mut self, line: &str) {
        if let Some(cb) = &mut self.on_progress {
            cb(line);
        }
    }

    fn post(&self, payload: &Value) -> Result<Value, String> {
        let url = format!("{}/chat/completions", self.base_url);
        let mut last = String::new();
        for attempt in 0..4 {
            let mut req = ureq::post(&url)
                .set("Content-Type", "application/json")
                .timeout(Duration::from_secs(self.timeout_secs));
            if !self.api_key.is_empty() {
                req = req.set("Authorization", &format!("Bearer {}", self.api_key));
            }
            let resp = req.send_json(payload.clone());
            match resp {
                Ok(r) => {
                    return r.into_json::<Value>().map_err(|e| e.to_string());
                }
                Err(ureq::Error::Status(code, r)) => {
                    let detail: String = r.into_string().unwrap_or_default().chars().take(600).collect();
                    if matches!(code, 429 | 500 | 502 | 503 | 529) && attempt < 3 {
                        thread::sleep(Duration::from_secs(1 << attempt));
                        last = format!("HTTP {code}: {detail}");
                        continue;
                    }
                    return Err(format!("{} API error HTTP {code}: {detail}", self.name));
                }
                Err(e) => {
                    if attempt < 3 {
                        thread::sleep(Duration::from_secs(1 << attempt));
                        last = e.to_string();
                        continue;
                    }
                    return Err(format!("could not reach {}: {e}", self.base_url));
                }
            }
        }
        Err(last)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelChoice {
    pub id: String,
    pub backend: String,
    pub label: String,
    pub kind: String,
}

pub fn ollama_reachable() -> bool {
    ureq::get("http://127.0.0.1:11434/v1/models")
        .timeout(Duration::from_secs(1))
        .call()
        .is_ok()
}

pub fn list_models() -> Vec<ModelChoice> {
    let mut out = Vec::new();
    if std::env::var("XAI_API_KEY").map(|s| !s.is_empty()).unwrap_or(false) {
        out.push(ModelChoice {
            id: "grok-4.5".into(),
            backend: "grok".into(),
            label: "Grok 4.5".into(),
            kind: "cloud".into(),
        });
    }
    if let Ok(r) = ureq::get("http://127.0.0.1:11434/v1/models")
        .timeout(Duration::from_secs(2))
        .call()
    {
        if let Ok(v) = r.into_json::<Value>() {
            if let Some(data) = v.get("data").and_then(|d| d.as_array()) {
                for m in data {
                    let id = m.get("id").and_then(|i| i.as_str()).unwrap_or("");
                    if id.is_empty() {
                        continue;
                    }
                    if !id.to_lowercase().contains("qwen") {
                        continue;
                    }
                    out.push(ModelChoice {
                        id: id.into(),
                        backend: "qwen".into(),
                        label: format!("{id} (local)"),
                        kind: "local".into(),
                    });
                }
            }
        }
    }
    if !out.iter().any(|m| m.backend == "qwen") {
        out.push(ModelChoice {
            id: "qwen3.5:35b-128k".into(),
            backend: "qwen".into(),
            label: "qwen3.5:35b-128k (local)".into(),
            kind: "local".into(),
        });
    }
    out
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
            let Some(arr) = calls.as_array() else {
                let text = msg.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string();
                if !text.trim().is_empty() {
                    self.emit(&text);
                }
                transcript.push(text);
                break;
            };
            if arr.is_empty() {
                let text = msg.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string();
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
                } else {
                    dispatch(stage, fname, &args)
                };
                let line = format!("[{fname}] {}", result.chars().take(200).collect::<String>());
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
