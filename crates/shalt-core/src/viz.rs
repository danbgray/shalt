use crate::deps::WorkMap;
use crate::ledger::{Entry, Ledger, GREEN, ORPHAN, PENDING, RED, STALE};
use crate::spec::Feature;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

fn mid(s: &str) -> String {
    let t = s
        .replace('"', "'")
        .replace('\n', " ")
        .replace('\r', " ")
        .replace('[', "(")
        .replace(']', ")")
        .replace('#', " ")
        .replace(';', ",")
        .replace('{', "(")
        .replace('}', ")")
        .replace('|', "/")
        .replace('<', " ")
        .replace('>', " ");
    format!("\"{}\"", t.trim())
}

fn sty(id: &str, fill: &str) -> String {
    format!("  style {id} fill:{fill},stroke:#1a1f26,color:#ffffff")
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
    let mut lines = vec!["flowchart LR".into()];
    let mut caps: BTreeMap<String, String> = BTreeMap::new();
    for (actor, items) in &acts {
        let aid = format!("A_{}", nid(actor));
        lines.push(format!("  {aid}[{}]", mid(actor)));
        for it in items {
            let cid = format!("U_{}", nid(&it.capability));
            caps.insert(cid.clone(), it.capability.clone());
            lines.push(format!("  {aid} --> {cid}"));
        }
    }
    lines.push(String::new());
    for (cid, cap) in &caps {
        lines.push(format!("  {cid}[{}]", mid(cap)));
    }
    lines.push(String::new());
    for actor in acts.keys() {
        lines.push(sty(&format!("A_{}", nid(actor)), "#3d4f7c"));
    }
    for cid in caps.keys() {
        lines.push(sty(cid, "#3d4f7c"));
    }
    lines.join("\n") + "\n"
}

pub fn mermaid_pipeline() -> String {
    mermaid_pipeline_at("")
}

/// Living loop. `gate` is `work_gate`: plan / language / tests / run / build / idle.
pub fn mermaid_pipeline_at(gate: &str) -> String {
    let current = match gate {
        "plan" => "plan",
        "design" => "design",
        "language" => "lang",
        "tests" => "tests",
        "run" => "survey",
        "build" => "build",
        "idle" => "idle",
        _ => "",
    };
    let mut lines = vec![
        "flowchart LR".into(),
        "  plan[\"Plan\"] --> spec[\"Details\"]".into(),
        "  spec --> design[\"Storyboards\"]".into(),
        "  design --> lang[\"Language\"]".into(),
        "  lang --> tests[\"Tests\"]".into(),
        "  tests --> survey[\"Run all tests\"]".into(),
        "  survey -->|\"spec wrong\"| spec".into(),
        "  survey -->|\"unbound\"| tests".into(),
        "  survey --> build[\"Build tickets\"]".into(),
        "  build -->|\"ticket green\"| build".into(),
        "  build -->|\"spec wrong\"| spec".into(),
        "  build --> idle[\"Idle\"]".into(),
        sty("spec", "#3d4f7c"),
        sty("survey", "#a9761b"),
        sty("build", "#1f7a4c"),
        sty("idle", "#2a2d33"),
    ];
    if !current.is_empty() {
        lines.push(sty(current, "#5e6ad2"));
    }
    lines.join("\n") + "\n"
}

pub fn mermaid_work_map(map: &WorkMap) -> String {
    if map.waves.is_empty() {
        return "flowchart TB\n  none[\"No tickets yet.\"]\n".into();
    }
    let mut lines = vec!["flowchart TB".into()];
    let mut by_rid: BTreeMap<&str, &crate::deps::TicketNode> = BTreeMap::new();
    for n in map.ready.iter().chain(map.blocked.iter()) {
        by_rid.insert(n.rid.as_str(), n);
    }
    for (i, wave) in map.waves.iter().enumerate() {
        let sid = format!("W{}", i + 1);
        let title = if i == 0 { "Start now" } else { "Then" };
        lines.push(format!("  subgraph {sid}[{}]", mid(&format!("{title} wave {}", i + 1))));
        for rid in wave {
            let id = nid(rid);
            let name = by_rid
                .get(rid.as_str())
                .map(|n| n.name.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(rid.as_str());
            lines.push(format!("    {id}[{}]", mid(&format!("{rid}  {name}"))));
        }
        lines.push("  end".into());
    }
    for n in map.blocked.iter() {
        for b in &n.blocked_by {
            lines.push(format!("  {} --> {}", nid(b), nid(&n.rid)));
        }
    }
    for n in map.ready.iter().chain(map.blocked.iter()) {
        let fill = if n.running {
            "#5e6ad2"
        } else if n.ready {
            "#1f7a4c"
        } else if n.status == RED {
            "#b3382e"
        } else if n.status == STALE {
            "#a9761b"
        } else {
            "#3d4f7c"
        };
        lines.push(sty(&nid(&n.rid), fill));
    }
    lines.join("\n") + "\n"
}

fn nid(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let t = t
        .split('_')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    let t = if t.is_empty() { "n".into() } else { t };
    const RESERVED: &[&str] = &[
        "end", "graph", "subgraph", "flowchart", "class", "classDef", "click", "call", "style",
        "linkStyle", "direction",
    ];
    if t.chars().next().is_some_and(|c| c.is_ascii_digit())
        || RESERVED.iter().any(|r| r.eq_ignore_ascii_case(&t))
    {
        format!("n_{t}")
    } else {
        t
    }
}

pub fn mermaid_org(name: &str, projects: &[(String, String, String)]) -> String {
    let mut lines = vec!["flowchart TB".into()];
    let title = if name.trim().is_empty() { "shalt" } else { name };
    lines.push(format!("  org[{}]", mid(title)));
    lines.push(sty("org", "#5e6ad2"));
    if projects.is_empty() {
        lines.push("  none[\"No projects yet.\"]".into());
        lines.push("  org --> none".into());
        return lines.join("\n") + "\n";
    }
    for (id, pname, state) in projects {
        let pid = nid(id);
        lines.push(format!("  {pid}[{}]", mid(&format!("{pname}  {state}"))));
        lines.push(format!("  org --> {pid}"));
        lines.push(format!("  click {pid} href \"#/p/{id}\""));
        let fill = match state.as_str() {
            "playing" | "running" => "#1f7a4c",
            "failed" => "#b3382e",
            "paused" | "interrupted" => "#a9761b",
            "waiting" => "#3d4f7c",
            _ => "#2a2d33",
        };
        lines.push(sty(&pid, fill));
    }
    lines.join("\n") + "\n"
}

#[derive(Debug, Clone, Serialize)]
pub struct Diagrams {
    pub usecase: String,
    pub breakdown: String,
    pub pipeline: String,
    pub work: String,
    pub flow: String,
}

pub fn project_diagrams(
    entries: &[Entry],
    map: &WorkMap,
    gate: &str,
    features: &[Feature],
) -> Diagrams {
    let flow = mermaid_user_flow(features);
    Diagrams {
        usecase: if features.is_empty() {
            mermaid_usecase(entries)
        } else {
            flow.clone()
        },
        breakdown: mermaid_hierarchy(entries),
        pipeline: mermaid_pipeline_at(gate),
        work: mermaid_work_map(map),
        flow,
    }
}

/// One node per feature, swimlanes by inferred actor. `click` hrefs point at `spec/FILE`.
pub fn mermaid_user_flow(features: &[Feature]) -> String {
    if features.is_empty() {
        return "flowchart LR\n  none[\"No spec yet.\"]\n".into();
    }
    let mut by_actor: BTreeMap<String, Vec<&Feature>> = BTreeMap::new();
    for f in features {
        let a = f.inferred_actor();
        let key = if a.is_empty() { "user".into() } else { a };
        by_actor.entry(key).or_default().push(f);
    }
    let mut lines = vec!["flowchart LR".into()];
    for (actor, feats) in &by_actor {
        let sg = format!("SG_{}", nid(actor));
        lines.push(format!("  subgraph {sg}[{}]", mid(actor)));
        let mut prev: Option<String> = None;
        for f in feats {
            let id = format!("F_{}", nid(&f.file));
            lines.push(format!("    {id}[{}]", mid(&f.name)));
            if let Some(p) = &prev {
                lines.push(format!("    {p} --> {id}"));
            }
            prev = Some(id);
        }
        lines.push("  end".into());
    }
    for f in features {
        let id = format!("F_{}", nid(&f.file));
        lines.push(sty(&id, "#3d4f7c"));
        let href = f.file.replace('"', "");
        lines.push(format!("  click {id} href \"spec/{href}\""));
    }
    lines.join("\n") + "\n"
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
    let mut lines = vec!["flowchart TD".into()];
    for (epic, stories) in &by_epic {
        let ek = format!("E_{}", nid(epic));
        lines.push(format!("  {ek}[{}]", mid(&epic.to_uppercase())));
        lines.push(sty(&ek, "#3d4f7c"));
        for (story, scs) in stories {
            let sk = format!("F_{}", nid(story));
            lines.push(format!("  {ek} --> {sk}[{}]", mid(story)));
            lines.push(sty(&sk, "#3d4f7c"));
            for sc in scs {
                let sid = nid(&sc.rid);
                let (mark, fill) = match sc.status.as_str() {
                    GREEN => ("ok", "#1f7a4c"),
                    RED => ("fail", "#b3382e"),
                    STALE => ("stale", "#a9761b"),
                    PENDING => ("pending", "#3d4f7c"),
                    ORPHAN => ("?", "#2a2d33"),
                    _ => ("?", "#2a2d33"),
                };
                lines.push(format!(
                    "  {sk} --> {sid}[{}]",
                    mid(&format!("{mark}  {}", sc.name))
                ));
                lines.push(sty(&sid, fill));
            }
        }
    }
    lines.join("\n") + "\n"
}

pub fn write_mermaid(root: &Path, entries: &[Entry]) -> std::io::Result<Vec<PathBuf>> {
    let dir = root.join("docs/diagrams");
    fs::create_dir_all(&dir)?;
    let board = crate::board::Board::load(&root.join(".shalt/board.json"));
    let led = crate::ledger::Ledger::load(&root.join(".shalt/ledger.json")).unwrap_or_default();
    let map = crate::deps::work_map(&board, &led, &[], "", None);
    let features = crate::spec::load_specs(&root.join("spec"), false).unwrap_or_default();
    let flow = mermaid_user_flow(&features);
    let files = [
        ("user-flow", flow.clone()),
        ("use-cases", flow),
        ("breakdown", mermaid_hierarchy(entries)),
        ("pipeline", mermaid_pipeline()),
        ("work-map", mermaid_work_map(&map)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deps::WorkMap;

    #[test]
    fn pipeline_names_the_loop_and_highlights_the_gate() {
        let m = mermaid_pipeline_at("build");
        assert!(m.contains("Details"));
        assert!(m.contains("Storyboards"));
        assert!(m.contains("Language"));
        assert!(m.contains("Build tickets"));
        assert!(m.contains("style build fill:#5e6ad2"));
    }

    #[test]
    fn org_empty_still_draws_the_desk() {
        let m = mermaid_org("desk", &[]);
        assert!(m.contains("desk"));
        assert!(m.contains("No projects yet"));
    }

    #[test]
    fn org_colors_a_playing_project() {
        let m = mermaid_org(
            "desk",
            &[("erp-1".into(), "ERP".into(), "playing".into())],
        );
        assert!(m.contains("ERP  playing"));
        assert!(m.contains("fill:#1f7a4c"));
        assert!(m.contains("click erp_1 href \"#/p/erp-1\""), "{m}");
    }

    #[test]
    fn work_map_empty() {
        let m = mermaid_work_map(&WorkMap::default());
        assert!(m.contains("No tickets yet"));
    }

    #[test]
    fn hyphenated_feature_names_do_not_emit_mermaid_edges() {
        let e: Entry = serde_json::from_value(serde_json::json!({
            "rid": "S-e6c9cbf6",
            "name": "Create a single-level manufacturing BOM",
            "feature": "Bills of materials (Odoo-style)",
            "feature_file": "spec/bills.feature",
            "actor": "production planner",
            "capability": "Odoo-style multi-level bills of materials",
            "status": "red"
        }))
        .unwrap();
        let h = mermaid_hierarchy(&[e.clone()]);
        assert!(h.contains("F_Bills_of_materials_Odoo_style"), "{h}");
        assert!(!h.contains("bills-of-materials"), "{h}");
        let u = mermaid_usecase(&[e]);
        assert!(!u.contains("U_odoo-style"), "{u}");
        assert!(u.contains("U_Odoo_style_multi_level_bills_of_materials"), "{u}");
    }

    #[test]
    fn user_flow_from_gherkin_has_actor_lane_and_spec_links() {
        let raw = r#"
Feature: Compose a traveler
  A buyer seals a quoteable traveler.

  Scenario: Seal it
    Given the role is "buyer"
    When the buyer opens "/new"
    Then the live level badge is "L0"
"#;
        let f = crate::spec::parse_text(raw, "compose.feature")
            .unwrap()
            .expect("feature");
        assert_eq!(f.inferred_actor().to_lowercase(), "buyer");
        let m = mermaid_user_flow(&[f]);
        assert!(m.contains("subgraph"), "{m}");
        assert!(m.contains("Compose a traveler"), "{m}");
        assert!(m.contains("click F_compose_feature href \"spec/compose.feature\""), "{m}");
        assert!(!m.contains("compose-feature -->"), "{m}");
    }
}
