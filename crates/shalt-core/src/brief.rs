//! Packed first messages for write-pool models. No workspace dump, no copy-paste cheat-sheet.

use crate::bindings::{step_has_table, step_kw_and_phrase};
use crate::spec::Feature;

pub const CANNED_JS: &str = "\
Example (import src, assert, keep the signature):\n\
import { createRecipe } from '../src/recipes.js';\n\
When('I create a recipe titled {string}', function (title) {\n\
  this.lastRecipe = createRecipe(this.currentUser, title);\n\
});\n";

pub const CHEAT_SHEET: &str = "function (not arrow)";
pub const TREE_MARK: &str = "Files you can see:";

#[derive(Debug, Clone)]
pub struct StepBrief {
    pub stack: String,
    pub model: String,
    pub journey: String,
    pub file_rel: String,
    pub file_text: String,
    pub unbound: Vec<String>,
    pub contract_names: Vec<String>,
    pub bound_phrases: Vec<String>,
}

pub fn stepwright_system(model: &str, stack: &str) -> String {
    let stack = if stack.is_empty() { "rust" } else { stack };
    let _ = model;
    format!(
        "You are the STEPWRIGHT. Fill pending bodies for ONE scenario in the brief. \
Import from src/. Assert. Do not return pending. Do not change signatures. Do not add files. \
Stack: {stack}. done() is refused while those bodies are still pending."
    )
}

pub fn stepwright_user(b: &StepBrief) -> String {
    let mut lines = vec![
        format!(
            "Fill this ONE scenario in `{}` (journey `{}`). Other pending bodies can wait.",
            b.file_rel, b.journey
        ),
        "Do not change signatures. Do not add files. Import from src/. Assert.".into(),
        String::new(),
        CANNED_JS.trim_end().into(),
        String::new(),
        format!("Pending bodies to fill in `{}`:", b.file_rel),
        b.file_text.clone(),
    ];
    if !b.unbound.is_empty() {
        lines.push(String::new());
        lines.push("This scenario:".into());
        for u in b.unbound.iter().take(20) {
            lines.push(u.clone());
        }
    }
    if !b.contract_names.is_empty() {
        lines.push(String::new());
        lines.push("Call these from src/:".into());
        for n in b.contract_names.iter().take(24) {
            lines.push(format!("- {n}"));
        }
    }
    let out = lines.join("\n");
    debug_assert!(!out.contains(CHEAT_SHEET));
    debug_assert!(!out.contains(TREE_MARK));
    out
}

pub fn scenario_lines(f: &Feature, s: &crate::spec::Scenario) -> Vec<String> {
    let mut out = vec![format!(
        "- {} {}",
        s.rid.clone().unwrap_or_default(),
        s.name
    )];
    let mut last = String::new();
    for st in f.background.iter().chain(s.steps.iter()) {
        if st.lines().next().unwrap_or("").trim().starts_with('|') {
            continue;
        }
        let (kw, phrase) = step_kw_and_phrase(st, &last);
        last = kw.clone();
        let table = if step_has_table(st) { " [table]" } else { "" };
        out.push(format!("    {kw} {phrase}{table}"));
    }
    out
}

pub fn unbound_lines(features: &[Feature], journey: &str) -> Vec<String> {
    for f in features {
        if crate::bindings::journey_of(f) != journey {
            continue;
        }
        if let Some(s) = f.scenarios.first() {
            return scenario_lines(f, s);
        }
    }
    Vec::new()
}

/// Bodies in `file_text` that this scenario still needs. Empty → show the whole file.
pub fn pending_blocks_for(file_text: &str, f: &Feature, s: &crate::spec::Scenario) -> String {
    let mut last = String::new();
    let mut seen = std::collections::HashSet::new();
    let mut blocks = Vec::new();
    for st in f.background.iter().chain(s.steps.iter()) {
        if st.lines().next().unwrap_or("").trim().starts_with('|') {
            continue;
        }
        let (kw, phrase) = step_kw_and_phrase(st, &last);
        last = kw.clone();
        if !seen.insert(format!("{kw}|{phrase}")) {
            continue;
        }
        if let Some(block) = extract_js_fn(file_text, &kw, &phrase) {
            blocks.push(block);
        }
    }
    blocks.join("\n\n")
}

fn extract_js_fn(src: &str, kw: &str, pattern: &str) -> Option<String> {
    let title = match kw {
        "when" => "When",
        "then" => "Then",
        _ => "Given",
    };
    let a = format!("{title}('{pattern}'");
    let b = format!("{title}(\"{pattern}\"");
    let i = src.find(&a).or_else(|| src.find(&b))?;
    let rest = &src[i..];
    let end = rest.find("});")?;
    Some(rest[..=end + 2].trim().to_string())
}

/// `export function name` in top-level `src/*.js`. Used when the contract markdown is not a surface.
pub fn src_export_names(root: &std::path::Path) -> Vec<String> {
    let dir = root.join("src");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let re = regex::Regex::new(r"export\s+(?:async\s+)?function\s+([A-Za-z_][A-Za-z0-9_]*)")
        .expect("export fn");
    let mut names = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|s| s.to_str()) != Some("js") {
            continue;
        }
        let Ok(s) = std::fs::read_to_string(&p) else {
            continue;
        };
        for cap in re.captures_iter(&s) {
            names.push(cap[1].to_string());
        }
    }
    names.sort();
    names.dedup();
    names
}

pub fn uses_packed_brief(role: &str) -> bool {
    matches!(role, "stepwright" | "implementer")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_system_has_no_cheat_sheet_or_tree() {
        let s = stepwright_system("qwen3.5:2b-mlx", "javascript");
        assert!(s.len() < 600, "{}", s.len());
        assert!(!s.contains(CHEAT_SHEET));
        assert!(!s.contains(TREE_MARK));
        let u = stepwright_user(&StepBrief {
            stack: "javascript".into(),
            model: "qwen3.5:2b-mlx".into(),
            journey: "ingredients".into(),
            file_rel: "steps/ingredients.steps.js".into(),
            file_text: "Given('x', function () { return 'pending'; });\n".into(),
            unbound: vec!["- S-1 Group".into()],
            contract_names: vec!["createPacket".into()],
            bound_phrases: vec!["given|I am signed in as {string}".into()],
        });
        assert!(!u.contains(CHEAT_SHEET));
        assert!(!u.contains(TREE_MARK));
        assert!(u.contains("steps/ingredients.steps.js"));
        assert!(u.contains("createPacket"));
        assert!(u.contains(CANNED_JS.lines().next().unwrap()));
        assert!(u.contains("ONE scenario"));
        assert!(s.contains("done() is refused"));
        let last = stepwright_system("qwen3:8b", "javascript");
        assert!(last.contains("ONE scenario"));
        assert!(!last.contains("Write complete files"));
        assert!(last.contains("done() is refused"));
    }

    #[test]
    fn pending_blocks_are_one_function() {
        let src = "Given('x', function () { return 'pending'; });\nWhen('y', function () { return 'pending'; });\n";
        let block = extract_js_fn(src, "given", "x").expect("fn");
        assert!(block.contains("Given('x'"));
        assert!(!block.contains("When('y'"));
    }
}
