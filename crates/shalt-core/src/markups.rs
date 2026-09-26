//! Red-pen strokes and comments on mockup frames, plus the polish interview.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const MAX_STROKES: usize = 80;
const MAX_POINTS: usize = 400;
const MAX_NOTES: usize = 40;
const MAX_NOTE: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Markup {
    #[serde(default)]
    pub rid: String,
    #[serde(default)]
    pub journey: String,
    #[serde(default)]
    pub file: String,
    #[serde(default)]
    pub strokes: Vec<Stroke>,
    #[serde(default)]
    pub notes: Vec<Note>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Stroke {
    #[serde(default = "red_ink")]
    pub color: String,
    #[serde(default = "pen_width")]
    pub width: f32,
    #[serde(default)]
    pub points: Vec<[f32; 2]>,
}

fn red_ink() -> String {
    "#c0392b".into()
}
fn pen_width() -> f32 {
    2.4
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Note {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Interview {
    #[serde(default)]
    pub style: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub typeface: String,
    #[serde(default)]
    pub layout: String,
    #[serde(default)]
    pub density: String,
    /// Older files used this as the look line. Still accepted.
    #[serde(default)]
    pub polish: String,
    #[serde(default)]
    pub answers: Vec<InterviewTurn>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InterviewTurn {
    #[serde(default)]
    pub q: String,
    #[serde(default)]
    pub a: String,
}

pub fn safe_rid(rid: &str) -> Option<String> {
    let t = rid.trim();
    if t.len() < 6 || t.len() > 40 {
        return None;
    }
    let mut chars = t.chars();
    if chars.next()? != 'S' || chars.next()? != '-' {
        return None;
    }
    if !chars.all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(t.to_string())
}

fn safe_journey(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .take(48)
        .collect()
}

fn markup_path(root: &Path, journey: &str, rid: &str) -> PathBuf {
    let j = if journey.trim().is_empty() {
        "_".into()
    } else {
        safe_journey(journey)
    };
    root.join("mockups/markups").join(j).join(format!("{rid}.json"))
}

pub fn load_markup(root: &Path, journey: &str, rid: &str) -> Markup {
    let Some(rid) = safe_rid(rid) else {
        return Markup::default();
    };
    let raw = fs::read_to_string(markup_path(root, journey, &rid)).unwrap_or_default();
    if !raw.is_empty() {
        return serde_json::from_str(&raw).unwrap_or_default();
    }
    load_markup_any(root, &rid)
}

pub fn load_markup_any(root: &Path, rid: &str) -> Markup {
    let Some(rid) = safe_rid(rid) else {
        return Markup::default();
    };
    let dir = root.join("mockups/markups");
    let Ok(journeys) = fs::read_dir(&dir) else {
        return Markup {
            rid,
            ..Default::default()
        };
    };
    for jdir in journeys.flatten() {
        let p = jdir.path().join(format!("{rid}.json"));
        if let Ok(raw) = fs::read_to_string(p) {
            if let Ok(m) = serde_json::from_str::<Markup>(&raw) {
                return m;
            }
        }
    }
    Markup {
        rid,
        ..Default::default()
    }
}

pub fn save_markup(root: &Path, mut m: Markup) -> Result<Markup, String> {
    let rid = safe_rid(&m.rid).ok_or_else(|| "rid is required".to_string())?;
    m.rid = rid.clone();
    m.journey = safe_journey(&m.journey);
    m.strokes.truncate(MAX_STROKES);
    for s in &mut m.strokes {
        if s.color.trim().is_empty() {
            s.color = red_ink();
        }
        if s.width <= 0.0 {
            s.width = pen_width();
        }
        s.points.truncate(MAX_POINTS);
        for p in &mut s.points {
            p[0] = p[0].clamp(0.0, 1.0);
            p[1] = p[1].clamp(0.0, 1.0);
        }
    }
    m.notes.truncate(MAX_NOTES);
    for n in &mut m.notes {
        n.text = n.text.chars().take(MAX_NOTE).collect();
        n.x = n.x.clamp(0.0, 1.0);
        n.y = n.y.clamp(0.0, 1.0);
        if n.id.trim().is_empty() {
            n.id = format!("n-{:x}", (n.x * 1000.0) as u32);
        }
    }
    let dest = markup_path(root, &m.journey, &rid);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let body = serde_json::to_string_pretty(&m).map_err(|e| e.to_string())?;
    fs::write(dest, body).map_err(|e| e.to_string())?;
    Ok(m)
}

pub fn interview_path(root: &Path) -> PathBuf {
    root.join("mockups/interview.json")
}

pub fn load_interview(root: &Path) -> Interview {
    let raw = fs::read_to_string(interview_path(root)).unwrap_or_default();
    serde_json::from_str(&raw).unwrap_or_default()
}

pub fn save_interview(root: &Path, mut iv: Interview) -> Result<Interview, String> {
    iv.style = iv.style.chars().take(200).collect();
    iv.color = iv.color.chars().take(200).collect();
    iv.typeface = iv.typeface.chars().take(200).collect();
    iv.layout = iv.layout.chars().take(200).collect();
    iv.polish = iv.polish.chars().take(400).collect();
    iv.density = iv.density.chars().take(80).collect();
    iv.answers.truncate(12);
    for t in &mut iv.answers {
        t.q = t.q.chars().take(200).collect();
        t.a = t.a.chars().take(800).collect();
    }
    fs::create_dir_all(root.join("mockups")).map_err(|e| e.to_string())?;
    let body = serde_json::to_string_pretty(&iv).map_err(|e| e.to_string())?;
    fs::write(interview_path(root), body).map_err(|e| e.to_string())?;
    Ok(iv)
}

/// Prompt appendix so the designer honors red pen and the polish interview.
pub fn designer_markup_prompt(root: &Path, focus_journey: &str, focus_rid: &str) -> String {
    let mut s = String::new();
    let iv = load_interview(root);
    if interview_set(&iv) {
        let applied = root.join("mockups/tokens.final.css").is_file();
        if applied {
            s.push_str("DESIGN INTERVIEW is already applied (tokens.final.css). Honor it. Do not rewrite the template.\n");
        } else {
            s.push_str("DESIGN INTERVIEW — one look for every mockup. Write mockups/tokens.final.css from this, not per-screen CSS.\n");
        }
        for (k, v) in [
            ("style", iv.style.as_str()),
            ("color", iv.color.as_str()),
            ("type", iv.typeface.as_str()),
            ("layout", iv.layout.as_str()),
            ("density", iv.density.as_str()),
            ("look", iv.polish.as_str()),
        ] {
            if !v.trim().is_empty() {
                s.push_str(&format!("  {k}: {}\n", v.trim()));
            }
        }
        for t in &iv.answers {
            if t.a.trim().is_empty() {
                continue;
            }
            s.push_str(&format!("  Q: {}\n  A: {}\n", t.q.trim(), t.a.trim()));
        }
        if applied {
            s.push_str(
                "Page polish only: edit the focused journey HTML. Leave other journeys. Honor pins on this frame.\n\n",
            );
        } else {
            s.push_str(
                "Same template on every journey. No pencil borders. Do not rewrite journey HTML unless the interview asks for a layout change — then apply it on every journey. Unbuilt beats stay sketched. Individual screens are polished only after this template exists.\n\n",
            );
        }
    }
    let dir = root.join("mockups/markups");
    if dir.is_dir() {
        let focus = focus_journey.trim();
        let mut n = 0usize;
        let mut block = String::new();
        if let Ok(journeys) = fs::read_dir(&dir) {
            let mut journeys: Vec<_> = journeys.flatten().collect();
            journeys.sort_by_key(|e| e.file_name());
            for jdir in journeys {
                if !jdir.path().is_dir() {
                    continue;
                }
                let jname = jdir.file_name().to_string_lossy().into_owned();
                if !focus.is_empty() && jname != focus {
                    continue;
                }
                let Ok(files) = fs::read_dir(jdir.path()) else {
                    continue;
                };
                let mut files: Vec<_> = files.flatten().collect();
                files.sort_by_key(|e| e.file_name());
                for f in files {
                    if f.path().extension().and_then(|x| x.to_str()) != Some("json") {
                        continue;
                    }
                    let Ok(raw) = fs::read_to_string(f.path()) else {
                        continue;
                    };
                    let Ok(m) = serde_json::from_str::<Markup>(&raw) else {
                        continue;
                    };
                    if m.notes.is_empty() && m.strokes.is_empty() {
                        continue;
                    }
                    n += 1;
                    if n > 24 {
                        break;
                    }
                    block.push_str(&format!("  {} ({})\n", m.rid, m.journey));
                    for note in &m.notes {
                        if note.text.trim().is_empty() {
                            continue;
                        }
                        block.push_str(&format!("    note: {}\n", note.text.trim()));
                    }
                    if !m.strokes.is_empty() {
                        block.push_str(&format!("    red pen: {} stroke(s)\n", m.strokes.len()));
                    }
                }
            }
        }
        if !block.is_empty() {
            s.push_str("HUMAN MARKUPS — red pen and comments on the frames. Honor them.\n");
            s.push_str(&block);
            s.push('\n');
        }
    }
    let pid = project_id_for(root);
    let file = mockup_file_for(root, focus_journey, focus_rid);
    if markup_enabled(root) {
        s.push_str(
            "Pins live on markup.rivlet.io (the widget on the mockup). Honor MARKUP.RIVLET.IO PINS below. Local mockups/markups JSON is a fallback.\n\n",
        );
    }
    if !pid.is_empty() && !file.is_empty() && !focus_rid.trim().is_empty() {
        s.push_str(&remote_pin_prompt(root, &pid, &file, focus_rid));
    }
    s
}

pub const MARKUP_API: &str = "https://markup.rivlet.io";

#[derive(serde::Deserialize, Default)]
struct MarkupTomlFile {
    #[serde(default)]
    markup: MarkupToml,
}

#[derive(serde::Deserialize, Default)]
struct MarkupToml {
    #[serde(default)]
    key: String,
    #[serde(default)]
    api: String,
    #[serde(default)]
    secret: String,
}

fn toml_markup(root: &Path) -> MarkupToml {
    let raw = fs::read_to_string(root.join("shalt.toml")).unwrap_or_default();
    toml::from_str::<MarkupTomlFile>(&raw)
        .map(|f| f.markup)
        .unwrap_or_default()
}

/// Publishable widget key (`rmk_pub_…`). Safe to put in the mockup iframe.
pub fn markup_pub_key(root: &Path) -> String {
    let t = toml_markup(root);
    if !t.key.trim().is_empty() {
        return t.key.trim().to_string();
    }
    crate::api::config_secret_for_markup(&["MARKUP_PUB_KEY", "MARKUP_KEY"], &["markup"])
        .unwrap_or_default()
}

pub fn markup_api(root: &Path) -> String {
    let t = toml_markup(root);
    if !t.api.trim().is_empty() {
        return t.api.trim().trim_end_matches('/').to_string();
    }
    MARKUP_API.to_string()
}

fn markup_secret(root: &Path) -> String {
    let t = toml_markup(root);
    if !t.secret.trim().is_empty() {
        return t.secret.trim().to_string();
    }
    crate::api::config_secret_for_markup(
        &["MARKUP_SECRET_KEY", "MARKUP_AGENT_KEY"],
        &["markup_secret", "markup_agent"],
    )
    .unwrap_or_default()
}

pub fn markup_enabled(root: &Path) -> bool {
    !markup_pub_key(root).is_empty()
}

fn esc_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('"', "&quot;")
}

/// Script tag injected into assembled mockups. Empty if no publishable key.
/// Never includes the secret key.
pub fn embed_tag(root: &Path, project_id: &str, rid: &str) -> String {
    let key = markup_pub_key(root);
    if key.is_empty() {
        return String::new();
    }
    let api = markup_api(root);
    format!(
        r#"<script src="{api}/embed.js" data-key="{key}" data-api="{api}" data-label="Pin" data-rid="{rid}" data-project="{pid}" data-source="shalt" defer></script>"#,
        api = esc_attr(&api),
        key = esc_attr(&key),
        rid = esc_attr(rid),
        pid = esc_attr(project_id),
    )
}

fn project_id_for(root: &Path) -> String {
    crate::org::Org::load()
        .find_by_path(root)
        .map(|p| p.id.clone())
        .unwrap_or_else(|| {
            root.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
}

fn mockup_file_for(root: &Path, journey: &str, rid: &str) -> String {
    if !rid.trim().is_empty() {
        let m = load_markup_any(root, rid);
        if !m.file.trim().is_empty() {
            return m.file.trim().trim_start_matches('/').to_string();
        }
    }
    let j = safe_journey(journey);
    if j.is_empty() || j == "_" {
        return String::new();
    }
    format!("journeys/{j}/{j}.html")
}

fn pin_page_url(project_id: &str, file: &str, rid: &str) -> String {
    let base = crate::uis::desk_url().unwrap_or_else(|| "http://127.0.0.1:7702".into());
    let base = base.trim_end_matches('/');
    format!(
        "{base}/api/project/{pid}/mockups/{file}?rid={rid}",
        pid = project_id,
        file = file.trim_start_matches('/'),
        rid = rid,
    )
}

/// Open Markup pins for this frame, for the designer prompt. Network failure is silent.
/// Uses the secret/agent key only — never the publishable widget key, never printed.
pub fn remote_pin_prompt(root: &Path, project_id: &str, file: &str, rid: &str) -> String {
    let key = markup_secret(root);
    if key.is_empty() {
        return String::new();
    }
    let api = markup_api(root);
    let url = pin_page_url(project_id, file, rid);
    let req = format!(
        "{api}/api/reviews?url={u}",
        u = urlencoding_lite(&url)
    );
    let body = match ureq::get(&req)
        .set("Authorization", &format!("Bearer {key}"))
        .timeout(std::time::Duration::from_secs(4))
        .call()
    {
        Ok(r) => r.into_string().unwrap_or_default(),
        Err(_) => return String::new(),
    };
    let v: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::json!({}));
    let mut out = String::new();
    let mut n = 0usize;
    if let Some(reviews) = v.get("reviews").and_then(|x| x.as_array()) {
        for rev in reviews {
            let pins = rev.get("pins").and_then(|x| x.as_array()).cloned().unwrap_or_default();
            for pin in pins {
                let comments = pin
                    .get("comments")
                    .and_then(|x| x.as_array())
                    .cloned()
                    .unwrap_or_default();
                for c in comments {
                    let text = c
                        .get("body")
                        .or_else(|| c.get("text"))
                        .and_then(|b| b.as_str())
                        .unwrap_or("")
                        .trim();
                    if text.is_empty() {
                        continue;
                    }
                    n += 1;
                    if n > 24 {
                        break;
                    }
                    out.push_str(&format!("    pin: {text}\n"));
                }
            }
        }
    }
    if out.is_empty() {
        return String::new();
    }
    format!("MARKUP.RIVLET.IO PINS on {url}\n{out}\n")
}

fn urlencoding_lite(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                o.push(b as char)
            }
            _ => o.push_str(&format!("%{b:02X}")),
        }
    }
    o
}

fn interview_set(iv: &Interview) -> bool {
    !iv.style.trim().is_empty()
        || !iv.color.trim().is_empty()
        || !iv.typeface.trim().is_empty()
        || !iv.layout.trim().is_empty()
        || !iv.density.trim().is_empty()
        || !iv.polish.trim().is_empty()
        || iv.answers.iter().any(|t| !t.a.trim().is_empty())
}

pub fn polish_prompt() -> String {
    "Finalize the design template from the interview. This is one look for every mockup, not a single beat.\n\
     Write mockups/tokens.final.css: color, type, density, shadows; same structure as the sketches; no pencil borders.\n\
     If the interview asks for a layout change, apply it on every journey's HTML. Do not invent screens.\n\
     Do not polish individual screens this turn — the template comes first.\n\
     Do not write spec/, tests, contract/, src/, or .shalt/. Call done() when the template is written."
        .into()
}

/// After the overall template exists: polish one journey/beat, not tokens.final.css.
pub fn page_polish_prompt(rid: &str, journey: &str) -> String {
    format!(
        "The overall look is already set in mockups/tokens.final.css. Honor it. Do not rewrite that file.\n\
         Polish only this screen: journey `{journey}`, rid `{rid}`.\n\
         Edit mockups/journeys/{journey}/{journey}.html for that beat. Honor Markup pins on this frame.\n\
         Leave other journeys. Do not write spec/, tests, contract/, src/, or .shalt/. Call done() when this screen matches the template."
    )
}
