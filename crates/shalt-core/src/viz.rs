use crate::ledger::{Entry, Ledger, GREEN, ORPHAN, PENDING, RED, STALE};
use crate::narrative::slug;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

fn mid(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "'").replace('\n', " ").trim())
}

pub fn mermaid_usecase(entries: &[Entry]) -> String {
    let mut acts: BTreeMap<String, Vec<&Entry>> = BTreeMap::new();
    for e in entries {
        if !e.actor.is_empty() && !e.capability.is_empty() {
            acts.entry(e.actor.clone()).or_default().push(e);
        }
    }
    if acts.is_empty() {
        return "graph LR\n  none[\"No user stories found.\"]\n".into();
    }
    let mut lines = vec!["graph LR".into()];
    let mut caps: BTreeMap<String, String> = BTreeMap::new();
    for (actor, items) in &acts {
        let aid = format!("A_{}", slug(actor, "actor"));
        lines.push(format!("  {aid}([{}])", mid(actor)));
        for it in items {
            let cid = format!("U_{}", &slug(&it.capability, "cap")[..slug(&it.capability, "cap").len().min(40)]);
            caps.insert(cid.clone(), it.capability.clone());
            lines.push(format!("  {aid} --- {cid}"));
        }
    }
    lines.push(String::new());
    for (cid, cap) in &caps {
        lines.push(format!("  {cid}({})", mid(cap)));
    }
    lines.push(String::new());
    for actor in acts.keys() {
        lines.push(format!(
            "  style A_{} fill:#3d4f7c,stroke:#2a3757,color:#ffffff",
            slug(actor, "actor")
        ));
    }
    for cid in caps.keys() {
        lines.push(format!("  style {cid} fill:#eef1f7,stroke:#3d4f7c,color:#1a1f26"));
    }
    lines.join("\n") + "\n"
}

pub fn mermaid_pipeline() -> String {
    r#"graph LR
  req["Plain-English request"] --> author["Author agent"]
  author --> story["User story"]
  story --> gherkin["Gherkin scenarios"]
  gherkin --> gate{"Human approval"}
  gate --> locked["Approved spec"]
  locked --> sw["Stepwright"]
  sw --> steps["Step definitions"]
  sw --> contract["Interface contract"]
  contract --> impl["Implementer"]
  locked --> impl
  impl --> src["Implementation"]
  steps --> run["Test run"]
  src --> run
  run --> ledger["Scenario ledger"]
  ledger -->|"still red"| impl
  style gate fill:#a9761b,stroke:#7d570f,color:#ffffff
  style ledger fill:#1f7a4c,stroke:#155a38,color:#ffffff
"#
    .into()
}

pub fn mermaid_hierarchy(entries: &[Entry]) -> String {
    if entries.is_empty() {
        return "graph TD\n  none[\"No scenarios in the ledger yet.\"]\n".into();
    }
    let mut by_epic: BTreeMap<String, BTreeMap<String, Vec<&Entry>>> = BTreeMap::new();
    for e in entries {
        let epic = if e.epic.is_empty() { "unassigned".into() } else { e.epic.clone() };
        by_epic.entry(epic).or_default().entry(e.feature.clone()).or_default().push(e);
    }
    let mut lines = vec!["graph TD".into()];
    for (epic, stories) in &by_epic {
        let ek = format!("E_{}", slug(epic, "epic"));
        lines.push(format!("  {ek}[{}]", mid(&epic.to_uppercase())));
        lines.push(format!("  style {ek} fill:#3d4f7c,stroke:#2a3757,color:#ffffff"));
        for (story, scs) in stories {
            let sk = format!("S_{}", slug(story, "story"));
            lines.push(format!("  {ek} --> {sk}[{}]", mid(story)));
            lines.push(format!("  style {sk} fill:#eef1f7,stroke:#3d4f7c,color:#1a1f26"));
            for sc in scs {
                let nid = sc.rid.replace('-', "_");
                let mark = match sc.status.as_str() {
                    GREEN => "✓",
                    RED => "✗",
                    STALE => "~",
                    PENDING => "·",
                    ORPHAN => "?",
                    _ => "?",
                };
                lines.push(format!("  {sk} --> {nid}({})", mid(&format!("{mark}  {}", sc.name))));
            }
        }
    }
    lines.join("\n") + "\n"
}

pub fn write_mermaid(root: &Path, entries: &[Entry]) -> std::io::Result<Vec<PathBuf>> {
    let dir = root.join("docs/diagrams");
    fs::create_dir_all(&dir)?;
    let files = [
        ("use-cases", mermaid_usecase(entries)),
        ("breakdown", mermaid_hierarchy(entries)),
        ("pipeline", mermaid_pipeline()),
    ];
    let mut written = Vec::new();
    for (name, body) in files {
        let mmd = dir.join(format!("{name}.mmd"));
        let md = dir.join(format!("{name}.md"));
        fs::write(&mmd, &body)?;
        fs::write(&md, format!("```mermaid\n{body}```\n"))?;
        written.push(mmd);
        written.push(md);
    }
    Ok(written)
}

pub fn write_dashboard(root: &Path, ledger: &Ledger, project: &str) -> std::io::Result<PathBuf> {
    let out = root.join("docs/dashboard.html");
    if let Some(p) = out.parent() {
        fs::create_dir_all(p)?;
    }
    let s = ledger.summary();
    let green = s.get("green").and_then(|v| v.as_i64()).unwrap_or(0);
    let total = s.get("total").and_then(|v| v.as_i64()).unwrap_or(0) - s.get("orphan").and_then(|v| v.as_i64()).unwrap_or(0);
    let mut notches = String::new();
    for i in 0..total.max(0) {
        let cls = if i < green { "ok" } else { "no" };
        notches.push_str(&format!("<i class=\"{cls}\"></i>"));
    }
    let mut rows = String::new();
    let mut keys: Vec<_> = ledger.entries.keys().cloned().collect();
    keys.sort();
    for k in keys {
        let e = &ledger.entries[&k];
        rows.push_str(&format!(
            "<tr><td class=\"mono {}\">{}</td><td>{}</td><td class=\"mono\">{}</td></tr>",
            e.status,
            e.status,
            html_escape(&e.name),
            html_escape(&e.rid)
        ));
    }
    let html = format!(
        r#"<!DOCTYPE html><html><head><meta charset="utf-8"><title>{project}</title>
<style>
body{{font:15px/1.45 Georgia,serif;background:#111;color:#eee;margin:2rem}}
.mono{{font-family:ui-monospace,Menlo,monospace;font-size:12px}}
i{{display:inline-block;width:8px;height:14px;margin-right:2px;background:#333}}
i.ok{{background:#3dba7a}}
.green{{color:#3dba7a}} .red{{color:#e05d4e}} .stale{{color:#d4a017}} .pending{{color:#888}}
table{{border-collapse:collapse;width:100%}} td{{padding:6px 8px;border-bottom:1px solid #333}}
</style></head><body>
<h1>{project}</h1>
<p class="mono">{pct}% upheld · {green}/{total}</p>
<div>{notches}</div>
<table>{rows}</table>
<p class="mono">snapshot from .shalt/ledger.json — regenerate after each run</p>
</body></html>
"#,
        project = html_escape(project),
        pct = s.get("completion_pct").and_then(|v| v.as_f64()).unwrap_or(0.0),
        green = green,
        total = total,
        notches = notches,
        rows = rows,
    );
    fs::write(&out, html)?;
    Ok(out)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
