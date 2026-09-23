//! Packed first messages for write-pool models. No workspace dump, no copy-paste cheat-sheet.

use crate::alloc::is_write_model;
use crate::bindings::{step_has_table, step_kw_and_phrase};
use crate::spec::Feature;

pub const CANNED_JS: &str = "\
Example (fill pending bodies; keep the signatures):\n\
Given('I am signed in as {string}', function (email) {\n\
  this.currentUser = email;\n\
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
    let small = is_write_model(model) && !model.contains("8b");
    if small {
        format!(
            "You are the STEPWRIGHT. Fill pending bodies in the one steps file in the brief. \
Do not change signatures. Do not add files. Do not rewrite a working step. \
Stack: {stack}. Call done() when pending bodies for this journey are filled. \
Do not write a blog file."
        )
    } else {
        format!(
            "You are the STEPWRIGHT in a shalt BDD pipeline. You see the spec and NOTHING of src/. \
Fill pending step bodies for this journey only. Keep existing signatures. \
Never define the same Given/When/Then phrase twice. Do not add extra step files. \
Stack: {stack}. Write complete files, not diffs. Call done() when this journey's pending bodies are filled. \
Do not write a blog file. A short JOURNAL: line in done() is enough."
        )
    }
}

pub fn stepwright_user(b: &StepBrief) -> String {
    let mut lines = vec![
        format!("Fill pending bodies in `{}` for journey `{}`.", b.file_rel, b.journey),
        "Do not change signatures. Do not add files.".into(),
        String::new(),
        CANNED_JS.trim_end().into(),
        String::new(),
        format!("File `{}`:", b.file_rel),
        b.file_text.clone(),
    ];
    if !b.unbound.is_empty() {
        lines.push(String::new());
        lines.push("Unbound scenarios:".into());
        for u in b.unbound.iter().take(14) {
            lines.push(u.clone());
        }
    }
    if !b.bound_phrases.is_empty() {
        lines.push(String::new());
        lines.push("Already bound (do not redefine):".into());
        for p in b.bound_phrases.iter().take(40) {
            lines.push(format!("- {p}"));
        }
    }
    if !b.contract_names.is_empty() {
        lines.push(String::new());
        lines.push("Contract exports:".into());
        for n in &b.contract_names {
            lines.push(format!("- {n}"));
        }
    }
    let out = lines.join("\n");
    debug_assert!(!out.contains(CHEAT_SHEET));
    debug_assert!(!out.contains(TREE_MARK));
    out
}

pub fn unbound_lines(features: &[Feature], journey: &str) -> Vec<String> {
    let mut out = Vec::new();
    for f in features {
        if crate::bindings::journey_of(f) != journey {
            continue;
        }
        for s in &f.scenarios {
            let rid = s.rid.clone().unwrap_or_default();
            out.push(format!("- {} {}", rid, s.name));
            let mut last = String::new();
            for st in &s.steps {
                if st.lines().next().unwrap_or("").trim().starts_with('|') {
                    continue;
                }
                let (kw, phrase) = step_kw_and_phrase(st, &last);
                last = kw.clone();
                let table = if step_has_table(st) { " [table]" } else { "" };
                out.push(format!("    {kw} {phrase}{table}"));
            }
        }
    }
    out
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
    }
}
