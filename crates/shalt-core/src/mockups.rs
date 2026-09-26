//! Storyboards are a reading of the spec: one film per journey, one frame per scenario.

use crate::ledger::{Ledger, GREEN};
use crate::narrative::slug;
use crate::spec::Feature;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Film {
    pub journey: String,
    pub kind: String,
    pub story: String,
    pub spec_hash: String,
    pub stale: bool,
    pub frames: Vec<Frame>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DesignKit {
    /// `phone`, `tablet`, or `desktop`. Empty means not chosen yet.
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub style: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub layout: String,
    #[serde(default)]
    pub guessed: bool,
}

pub fn load_kit(root: &Path) -> Option<DesignKit> {
    let raw = fs::read_to_string(root.join("mockups/kit.json")).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Canonical platform id, or empty if none was chosen.
pub fn normalize_platform(raw: &str) -> &'static str {
    let t = raw.trim().to_ascii_lowercase();
    if t.is_empty() {
        ""
    } else if t.contains("phone") || t.contains("mobile") || t == "ios" || t == "android" {
        "phone"
    } else if t.contains("tablet") || t.contains("ipad") {
        "tablet"
    } else {
        "desktop"
    }
}

/// Platform used to render chrome. Missing kit → desktop.
pub fn kit_platform(kit: Option<&DesignKit>) -> &'static str {
    let p = kit
        .map(|k| normalize_platform(&k.platform))
        .unwrap_or("");
    if p.is_empty() {
        "desktop"
    } else {
        p
    }
}

pub fn kit_has_platform(kit: &DesignKit) -> bool {
    !normalize_platform(&kit.platform).is_empty()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Frame {
    pub rid: String,
    pub name: String,
    pub file: String,
    pub caption: String,
    pub built: bool,
    pub stale: bool,
    #[serde(default)]
    pub thumb: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct StoryboardFile {
    #[serde(default)]
    #[allow(dead_code)]
    journey: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    spec_hash: String,
    #[serde(default, alias = "screens")]
    frames: Vec<StoryboardFrameFile>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct StoryboardFrameFile {
    #[serde(default)]
    rid: String,
    #[serde(default)]
    file: String,
    #[serde(default)]
    caption: String,
}

fn sha_text(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

pub fn journey_slug(feature: &Feature) -> String {
    let e = feature.epic();
    if !e.is_empty() {
        return e;
    }
    slug(&feature.name, "story")
}

pub fn journey_spec_hash(features: &[Feature], journey: &str) -> String {
    let mut parts = Vec::new();
    for f in features {
        if journey_slug(f) != journey {
            continue;
        }
        for s in &f.scenarios {
            parts.push(s.spec_hash(&f.background));
        }
    }
    parts.sort();
    sha_text(&parts.join("\n"))
}

fn storyboard_dir(root: &Path, journey: &str) -> PathBuf {
    root.join("mockups/journeys").join(journey)
}

fn load_json(root: &Path, journey: &str) -> Option<StoryboardFile> {
    let p = storyboard_dir(root, journey).join("storyboard.json");
    let raw = fs::read_to_string(p).ok()?;
    serde_json::from_str(&raw).ok()
}

fn story_caption(features: &[Feature], journey: &str) -> String {
    for f in features {
        if journey_slug(f) != journey {
            continue;
        }
        let st = f.story();
        if st.complete() {
            return st.one_line();
        }
        if !f.name.is_empty() {
            return f.name.clone();
        }
    }
    journey.to_string()
}

fn rel_file(journey: &str, file: &str) -> String {
    let file = file.trim().replace('\\', "/");
    if file.is_empty() {
        return String::new();
    }
    if file.starts_with("journeys/") || file.starts_with("mockups/") {
        return file.trim_start_matches("mockups/").to_string();
    }
    format!("journeys/{journey}/{file}")
}

pub fn films(root: &Path, features: &[Feature], ledger: &Ledger) -> Vec<Film> {
    let mut order: Vec<String> = Vec::new();
    let mut grouped: BTreeMap<String, Vec<(&Feature, &crate::spec::Scenario)>> = BTreeMap::new();
    for f in features {
        let j = journey_slug(f);
        if !order.contains(&j) {
            order.push(j.clone());
        }
        for s in &f.scenarios {
            grouped.entry(j.clone()).or_default().push((f, s));
        }
    }
    order
        .into_iter()
        .map(|journey| {
            let hash = journey_spec_hash(features, &journey);
            let json = load_json(root, &journey);
            let kind = json
                .as_ref()
                .map(|j| {
                    if j.kind.trim().is_empty() {
                        "ui"
                    } else {
                        j.kind.trim()
                    }
                })
                .unwrap_or("ui")
                .to_string();
            let stale = json
                .as_ref()
                .map(|j| !j.spec_hash.is_empty() && j.spec_hash != hash)
                .unwrap_or(false);
            let by_rid: BTreeMap<&str, &StoryboardFrameFile> = json
                .as_ref()
                .map(|j| {
                    j.frames
                        .iter()
                        .map(|fr| (fr.rid.as_str(), fr))
                        .collect()
                })
                .unwrap_or_default();
            let frames = grouped
                .get(&journey)
                .map(|pairs| {
                    pairs
                        .iter()
                        .map(|(_f, s)| {
                            let rid = s.rid.clone().unwrap_or_default();
                            let join = by_rid.get(rid.as_str());
                            let file = join
                                .map(|fr| rel_file(&journey, &fr.file))
                                .unwrap_or_default();
                            let caption = join
                                .map(|fr| fr.caption.trim().to_string())
                                .filter(|c| !c.is_empty())
                                .unwrap_or_else(|| s.name.clone());
                            let built = ledger
                                .entries
                                .get(&rid)
                                .map(|e| e.status == GREEN)
                                .unwrap_or(false);
                            let thumb = if file.is_empty() {
                                String::new()
                            } else {
                                let t = thumb_rel(&file);
                                if root.join("mockups").join(&t).is_file() {
                                    t
                                } else {
                                    String::new()
                                }
                            };
                            Frame {
                                rid,
                                name: s.name.clone(),
                                file,
                                caption,
                                built,
                                stale,
                                thumb,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            Film {
                story: story_caption(features, &journey),
                journey,
                kind,
                spec_hash: hash,
                stale,
                frames,
            }
        })
        .collect()
}

pub fn frame_is_drawn(root: &Path, fr: &Frame) -> bool {
    !fr.file.is_empty() && root.join("mockups").join(&fr.file).is_file()
}

pub fn design_needed(root: &Path, features: &[Feature]) -> bool {
    pick_design_journey(root, features, "").is_some()
}

fn film_needs_drawing(root: &Path, film: &Film) -> bool {
    if load_json(root, &film.journey).is_none() || film.stale {
        return true;
    }
    if film.kind == "none" {
        return false;
    }
    film.frames.iter().any(|fr| !frame_is_drawn(root, fr))
}

/// Next journey the designer should fill. One journey per design job.
pub fn pick_design_journey(root: &Path, features: &[Feature], focus_journey: &str) -> Option<String> {
    if features.iter().all(|f| f.scenarios.is_empty()) {
        return None;
    }
    let films = films(root, features, &Ledger::default());
    let focus = focus_journey.trim();
    if !focus.is_empty() {
        if let Some(f) = films.iter().find(|f| f.journey == focus && film_needs_drawing(root, f)) {
            return Some(f.journey.clone());
        }
    }
    films
        .into_iter()
        .find(|f| film_needs_drawing(root, f))
        .map(|f| f.journey)
}

pub fn verify_mockups(root: &Path, features: &[Feature]) -> Vec<String> {
    let mut problems = Vec::new();
    let live: std::collections::HashSet<String> = features
        .iter()
        .flat_map(|f| f.scenarios.iter())
        .filter_map(|s| s.rid.clone())
        .collect();
    let led = Ledger::default();
    for film in films(root, features, &led) {
        let Some(json) = load_json(root, &film.journey) else {
            continue;
        };
        if json.kind.trim() == "none" {
            continue;
        }
        for fr in &json.frames {
            if fr.rid.is_empty() {
                continue;
            }
            if !live.contains(&fr.rid) {
                problems.push(format!(
                    "orphan mockup frame {} on journey {}",
                    fr.rid, film.journey
                ));
            }
            let rel = rel_file(&film.journey, &fr.file);
            if rel.is_empty() {
                continue;
            }
            if !root.join("mockups").join(&rel).is_file() {
                problems.push(format!("missing mockup file mockups/{rel}"));
            }
        }
    }
    problems
}

fn html_esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Collapse `..` so in-mockup links like `../desk/seed.html` stay under mockups/.
pub fn normalize_rel(rest: &str) -> Option<String> {
    let mut out: Vec<&str> = Vec::new();
    for part in rest.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            out.pop()?;
            continue;
        }
        if part.contains('\\') || part.contains('\0') {
            return None;
        }
        out.push(part);
    }
    Some(out.join("/"))
}

fn file_basename(file: &str) -> &str {
    file.rsplit('/').next().unwrap_or(file)
}

/// Journey slug for a mockup path `journeys/<slug>/file.html`.
pub fn journey_of_rel(rel: &str) -> Option<&str> {
    let mut parts = rel.split('/');
    if parts.next()? != "journeys" {
        return None;
    }
    parts.next()
}

fn title_slug(slug: &str) -> String {
    let mut parts = Vec::new();
    for p in slug.split(|c: char| c == '-' || c == '_') {
        if p.is_empty() {
            continue;
        }
        let mut c = p.chars();
        let Some(f) = c.next() else { continue };
        parts.push(format!("{}{}", f.to_uppercase(), c.as_str()));
    }
    if parts.is_empty() {
        slug.to_string()
    } else {
        parts.join(" ")
    }
}

fn journey_nav_label(slug: &str) -> String {
    match slug {
        "award" => "Buyer".into(),
        "desk" => "Desk".into(),
        "traveler" => "Traveler".into(),
        "compose" => "Compose".into(),
        "quote" => "Quote".into(),
        other => title_slug(other),
    }
}

const PRODUCT_NAV_ORDER: &[&str] = &[
    "desk", "traveler", "compose", "quote", "award", "lifecycle", "role", "shell",
];

#[derive(Clone)]
struct JourneyHome {
    slug: String,
    label: String,
    href: String,
}

fn frame_href(project_id: &str, file: &str, rid: &str) -> String {
    format!("/api/project/{project_id}/mockups/{file}?rid={rid}")
}

/// First drawable frame of every journey, plus a rel→href map for click-through.
fn scan_journeys(
    root: &Path,
    project_id: &str,
) -> (Vec<JourneyHome>, BTreeMap<String, String>) {
    let mut homes = Vec::new();
    let mut by_rel = BTreeMap::new();
    let dir = root.join("mockups/journeys");
    let Ok(rd) = fs::read_dir(&dir) else {
        return (homes, by_rel);
    };
    let mut slugs: Vec<String> = rd
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    slugs.sort();
    slugs.sort_by_key(|s| {
        PRODUCT_NAV_ORDER
            .iter()
            .position(|p| p == s)
            .unwrap_or(100)
    });
    for slug in slugs {
        let Some(json) = load_json(root, &slug) else {
            continue;
        };
        if json.kind.trim().eq_ignore_ascii_case("none") {
            continue;
        }
        let mut home: Option<JourneyHome> = None;
        for fr in &json.frames {
            if fr.file.trim().is_empty() {
                continue;
            }
            let file = rel_file(&slug, &fr.file);
            let href = frame_href(project_id, &file, &fr.rid);
            by_rel.insert(file.clone(), href.clone());
            by_rel.insert(file_basename(&file).to_string(), href.clone());
            if home.is_none() {
                home = Some(JourneyHome {
                    slug: slug.clone(),
                    label: journey_nav_label(&slug),
                    href,
                });
            }
        }
        if let Some(h) = home {
            homes.push(h);
        }
    }
    (homes, by_rel)
}

/// Click-through nav + link rewrite for a served HTML frame.
pub fn mockup_inject(root: &Path, project_id: &str, rel: &str, green: &str, rid: &str) -> (String, String) {
    let journey = journey_of_rel(rel).unwrap_or("");
    let current = file_basename(rel);
    let json = load_json(root, journey);
    let (homes, mut by_rel) = scan_journeys(root, project_id);
    let mut frames_json = Vec::new();
    let mut nav = String::from(r#"<nav class="shalt-film-nav" data-shalt-nav aria-label="This area">"#);
    let mut map = BTreeMap::new();
    if let Some(board) = json.as_ref() {
        for fr in &board.frames {
            if fr.file.trim().is_empty() {
                continue;
            }
            let file = rel_file(journey, &fr.file);
            let base = file_basename(&file).to_string();
            let href = frame_href(project_id, &file, &fr.rid);
            let on = if base == current || fr.rid == rid { " on" } else { "" };
            let caption = if fr.caption.trim().is_empty() {
                base.as_str()
            } else {
                fr.caption.as_str()
            };
            let n = frames_json.len() + 1;
            let label = format!("{n}. {caption}");
            nav.push_str(&format!(
                r#"<a class="{on}" href="{href}" data-rid="{rid}" title="{caption}">{label}</a>"#,
                on = on.trim(),
                href = html_esc(&href),
                rid = html_esc(&fr.rid),
                caption = html_esc(caption),
                label = html_esc(&label)
            ));
            frames_json.push(serde_json::json!({
                "rid": fr.rid,
                "file": file,
                "base": base,
                "href": href,
                "caption": caption,
            }));
            map.insert(base, href.clone());
            by_rel.insert(file, href);
        }
    }
    nav.push_str("</nav>");
    let mut journeys_json = serde_json::Map::new();
    let mut app_bar = String::from(r#"<header class="shalt-app-bar" data-shalt-nav aria-label="Product">"#);
    let brand = homes
        .iter()
        .find(|h| h.slug == "desk")
        .or_else(|| homes.first());
    let brand_href = brand.map(|h| h.href.as_str()).unwrap_or("#");
    let named = crate::config::Config::load(root)
        .unwrap_or_default()
        .name;
    let brand_label = {
        let n = named.trim();
        if !n.is_empty() {
            n
        } else {
            brand.map(|h| h.label.as_str()).unwrap_or("Home")
        }
    };
    app_bar.push_str(&format!(
        r##"<a class="shalt-brand" href="{href}">{label}</a>
<a class="shalt-step" href="#" data-shalt-step="-1" title="Previous beat">←</a>
<a class="shalt-step" href="#" data-shalt-step="1" title="Next beat">→</a>
<nav class="shalt-app-nav">"##,
        href = html_esc(brand_href),
        label = html_esc(brand_label)
    ));
    for h in &homes {
        let on = if h.slug == journey { " on" } else { "" };
        app_bar.push_str(&format!(
            r#"<a class="{on}" href="{href}" data-journey="{slug}">{label}</a>"#,
            on = on.trim(),
            href = html_esc(&h.href),
            slug = html_esc(&h.slug),
            label = html_esc(&h.label)
        ));
        journeys_json.insert(
            h.slug.clone(),
            serde_json::json!({
                "href": h.href,
                "label": h.label,
                "slug": h.slug,
            }),
        );
    }
    app_bar.push_str("</nav></header>");
    let has_nav = !frames_json.is_empty() || !homes.is_empty();
    let payload = serde_json::json!({
        "rid": rid,
        "file": rel,
        "journey": journey,
        "current": current,
        "frames": frames_json,
        "byBase": map,
        "byRel": by_rel,
        "journeys": journeys_json,
    });
    let payload_js = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".into());
    let kit = load_kit(root);
    let layout = kit
        .as_ref()
        .map(|k| k.layout.to_lowercase())
        .unwrap_or_default();
    let layout_class = if layout.contains("left") || layout.contains("side") {
        "shalt-layout-side"
    } else {
        "shalt-layout-top"
    };
    let platform = kit_platform(kit.as_ref());
    let platform_class = format!("shalt-platform-{platform}");
    let mut css_links = String::from(r#"<link rel="stylesheet" href="/api/sketch.css">"#);
    if root.join("mockups/tokens.sketch.css").is_file() {
        css_links.push_str(&format!(
            r#"<link rel="stylesheet" href="/api/project/{}/mockups/tokens.sketch.css">"#,
            html_esc(project_id)
        ));
    }
    let product = root.join("mockups/tokens.final.css").is_file();
    if product {
        css_links.push_str(&format!(
            r#"<link rel="stylesheet" href="/api/project/{}/mockups/tokens.final.css">"#,
            html_esc(project_id)
        ));
    }
    let look_class = if product { "shalt-product" } else { "shalt-sketch" };
    let head = format!(
        r#"{css_links}
<script>
(function(){{
  var NAV = {payload_js};
  var green = {green:?}.split(',').filter(Boolean);
  var rid = NAV.rid || {rid:?};
  document.documentElement.classList.add({layout_class:?}, {platform_class:?}, {look_class:?}, 'shalt-has-chrome');
  function mark(){{
    var n = 0;
    document.querySelectorAll('[data-rid]').forEach(function(el){{
      n++;
      var id = el.getAttribute('data-rid');
      if (green.indexOf(id) >= 0) el.classList.add('built');
    }});
    if (!n && rid && green.indexOf(rid) >= 0) document.documentElement.classList.add('built');
    document.querySelectorAll('input,textarea,select,button').forEach(function(el){{
      el.removeAttribute('disabled');
      el.removeAttribute('readonly');
    }});
    wrapStage();
    focusBeat(rid, true);
  }}
  function wrapStage(){{
    if (document.querySelector('.shalt-stage')) return;
    var bar = document.querySelector('.shalt-app-bar');
    var beats = document.querySelector('.shalt-film-nav');
    var stage = document.createElement('div');
    stage.className = 'shalt-stage';
    var nodes = Array.prototype.slice.call(document.body.childNodes);
    nodes.forEach(function(n){{
      if (n === bar || n === beats) return;
      stage.appendChild(n);
    }});
    document.body.appendChild(stage);
  }}
  function focusBeat(id, instant){{
    if (!id) return false;
    var el = document.querySelector('.shalt-stage [data-rid="'+id+'"]');
    if (!el) {{
      document.querySelectorAll('[data-rid="'+id+'"]').forEach(function(n){{
        if (!el && !n.closest('.shalt-film-nav') && !n.closest('.shalt-app-bar')) el = n;
      }});
    }}
    if (!el) return false;
    rid = id;
    NAV.rid = id;
    document.querySelectorAll('[data-rid].on-beat').forEach(function(n){{ n.classList.remove('on-beat'); }});
    el.classList.add('on-beat');
    document.querySelectorAll('.shalt-film-nav a').forEach(function(a){{
      a.classList.toggle('on', a.getAttribute('data-rid') === id);
    }});
    try {{
      el.scrollIntoView({{ block: 'start', inline: 'nearest', behavior: instant ? 'auto' : 'smooth' }});
    }} catch (e) {{
      el.scrollIntoView(true);
    }}
    try {{ parent.postMessage({{ shalt: 'mockup', rid: id, file: NAV.file, journey: NAV.journey }}, '*'); }} catch (e) {{}}
    return true;
  }}
  function go(href){{
    if (!href) return;
    var frames = NAV.frames || [];
    var f = frames.find(function(x){{
      return x.href === href || (x.rid && href.indexOf('rid='+encodeURIComponent(x.rid)) >= 0) || (x.rid && href.indexOf('rid='+x.rid) >= 0);
    }});
    if (f && f.rid && focusBeat(f.rid, false)) {{
      try {{ history.replaceState(null, '', href); }} catch (e) {{}}
      return;
    }}
    location.href = href;
  }}
  function frameIndex(){{
    var frames = NAV.frames || [];
    return frames.findIndex(function(f){{ return f.base === NAV.current || f.rid === rid; }});
  }}
  function nextHref(){{
    var frames = NAV.frames || [];
    var i = frameIndex();
    if (i >= 0 && i + 1 < frames.length) return frames[i+1].href;
    return '';
  }}
  function prevHref(){{
    var frames = NAV.frames || [];
    var i = frameIndex();
    if (i > 0) return frames[i-1].href;
    return '';
  }}
  function journeyHome(slug){{
    var j = (NAV.journeys || {{}})[slug];
    return j && j.href || '';
  }}
  function resolveLabel(text){{
    var t = String(text || '').toLowerCase().replace(/\s+/g, ' ').trim();
    if (!t) return '';
    var js = NAV.journeys || {{}};
    if (js[t]) return js[t].href;
    for (var k in js) {{
      if (!Object.prototype.hasOwnProperty.call(js, k)) continue;
      var lab = String(js[k].label || k).toLowerCase();
      if (t === lab || t.indexOf(lab) === 0 || t.indexOf(k) >= 0) return js[k].href;
    }}
    return '';
  }}
  function resolveHref(raw){{
    if (!raw) return '';
    var path = String(raw).split('?')[0].split('#')[0];
    if (!path || path === '#') return '';
    var rel = NAV.byRel || {{}};
    if (rel[path]) return rel[path];
    var base = path.split('/').pop();
    var m = path.match(/([A-Za-z0-9_-]+)\/([^/]+\.html)$/);
    if (m) {{
      var key = 'journeys/' + m[1] + '/' + m[2];
      if (rel[key]) return rel[key];
      var home = journeyHome(m[1].toLowerCase());
      if (home) return home;
    }}
    if (NAV.byBase && NAV.byBase[base]) return NAV.byBase[base];
    if (rel[base]) return rel[base];
    return '';
  }}
  function showStub(){{
    var el = document.getElementById('shalt-stub');
    if (!el) {{
      el = document.createElement('div');
      el.id = 'shalt-stub';
      el.className = 'shalt-stub';
      el.innerHTML = '<p class="shalt-stub-msg">This isn\'t proven yet. The test does not pass.</p><button type="button" data-stub-back>Back</button>';
      document.body.appendChild(el);
      el.addEventListener('click', function(ev){{
        if (ev.target && ev.target.getAttribute && ev.target.getAttribute('data-stub-back') != null) {{
          el.hidden = true;
        }}
      }});
    }}
    el.hidden = false;
  }}
  function take(href, ev){{
    ev.preventDefault();
    if (href) go(href);
    else showStub();
  }}
  document.addEventListener('keydown', function(ev){{
    if (ev.key !== 'ArrowLeft' && ev.key !== 'ArrowRight') return;
    var tag = ev.target && ev.target.tagName;
    if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return;
    ev.preventDefault();
    try {{ parent.postMessage({{ shalt: 'step', dir: ev.key === 'ArrowRight' ? 1 : -1, rid: rid, journey: NAV.journey }}, '*'); }} catch (e) {{}}
  }});
  document.addEventListener('submit', function(ev){{
    var form = ev.target;
    if (!form || !form.action) {{ ev.preventDefault(); showStub(); return; }}
    var mapped = resolveHref(form.getAttribute('action') || '');
    take(mapped, ev);
  }}, true);
  document.addEventListener('click', function(ev){{
    var a = ev.target.closest && ev.target.closest('a');
    if (a) {{
      var href = a.getAttribute('href') || '';
      if (a.closest('#shalt-stub')) return;
      var step = a.getAttribute('data-shalt-step');
      if (step != null) {{
        ev.preventDefault();
        var to = Number(step) > 0 ? nextHref() : prevHref();
        if (to) go(to);
        else try {{ parent.postMessage({{ shalt: 'step', dir: Number(step) || 1, rid: rid, journey: NAV.journey }}, '*'); }} catch (e) {{}}
        return;
      }}
      if (a.closest('.shalt-film-nav')) {{
        ev.preventDefault();
        var beat = a.getAttribute('data-rid');
        if (beat && focusBeat(beat, false)) {{
          var dest = a.getAttribute('href');
          if (dest) try {{ history.replaceState(null, '', dest); }} catch (e) {{}}
          return;
        }}
        go(a.getAttribute('href') || '');
        return;
      }}
      if (a.closest('[data-shalt-nav]')) return;
      var mapped = resolveHref(href) || resolveLabel(a.textContent);
      if (mapped) {{ take(mapped, ev); return; }}
      if (!href || href === '#' || href.charAt(0) === '#') {{
        take(resolveLabel(a.textContent) || nextHref(), ev);
      }} else {{
        take('', ev);
      }}
      return;
    }}
    var b = ev.target.closest && ev.target.closest('button, .btn, [role=button], input[type=submit]');
    if (!b || b.closest('[data-shalt-nav]') || b.closest('#shalt-stub')) return;
    if (b.getAttribute && b.getAttribute('data-stub-back') != null) return;
    var t = (b.textContent || b.value || '').toLowerCase();
    if (/cancel|close|back|no\b/.test(t)) {{ take(prevHref(), ev); return; }}
    take(nextHref(), ev);
  }}, true);
  window.addEventListener('message', function(ev){{
    if (!ev.data || ev.data.shalt !== 'focus' || !ev.data.rid) return;
    focusBeat(ev.data.rid, false);
  }});
  try {{ parent.postMessage({{ shalt: 'mockup', rid: rid, file: NAV.file, journey: NAV.journey }}, '*'); }} catch (e) {{}}
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', mark);
  else mark();
}})();
</script>"#,
        css_links = css_links,
        payload_js = payload_js,
        green = green,
        rid = rid,
        layout_class = layout_class,
        platform_class = platform_class
    );
    let embed = crate::markups::embed_tag(root, project_id, rid);
    let head = if embed.is_empty() {
        head
    } else {
        format!("{head}\n{embed}")
    };
    let body = if has_nav {
        format!("{app_bar}{nav}")
    } else {
        String::new()
    };
    (head, body)
}

pub fn is_html_document(html: &str) -> bool {
    let t = html.to_ascii_lowercase();
    t.contains("<html") || t.contains("<!doctype")
}

fn splice_inject(html: &str, inject: &str, nav: &str) -> String {
    let mut out = html.to_string();
    let lower = out.to_ascii_lowercase();
    if let Some(i) = lower.find("</head>") {
        out.insert_str(i, inject);
    } else if let Some(i) = lower.find("<body") {
        out.insert_str(i, &format!("<head>{inject}</head>"));
    } else {
        out = format!("<!doctype html><head>{inject}</head><body>{out}</body>");
    }
    if !nav.is_empty() {
        let lower = out.to_ascii_lowercase();
        if let Some(body_at) = lower.find("<body") {
            if let Some(gt) = lower[body_at..].find('>') {
                out.insert_str(body_at + gt + 1, nav);
            }
        } else {
            out.push_str(nav);
        }
    }
    out
}

/// Serve a mockup through the locked sketch shell. Fragments (no `<html>`) are
/// wrapped; full documents keep their markup and only receive inject chrome.
pub fn assemble_mockup(
    root: &Path,
    project_id: &str,
    rel: &str,
    html: &str,
    green: &str,
    rid: &str,
) -> String {
    let (inject, nav) = mockup_inject(root, project_id, rel, green, rid);
    if is_html_document(html) {
        splice_inject(html, &inject, &nav)
    } else {
        format!(
            "<!doctype html><html><head><meta charset=\"utf-8\">{inject}</head><body>{nav}{html}</body></html>"
        )
    }
}

const FINAL_TOKENS_STARTER: &str = r#"/* Copied from the sketch. Edit font, color, and imagery for the finished product.
   Product nav and beat list stay in the shalt shell. */
:root {
  --paper: #f7f8fa;
  --ink: #1a1c20;
  --font: Inter, "SF Pro Display", ui-sans-serif, system-ui, sans-serif;
}
html, body {
  background: var(--paper);
  color: var(--ink);
  font-family: var(--font);
}
"#;

/// First green ticket copies the sketch look into `tokens.final.css` so a human
/// (or a later design turn) can restyle font, color, and imagery without
/// rewriting chrome.
pub fn promote_sketch_to_final(root: &Path) {
    let dest = root.join("mockups/tokens.final.css");
    if dest.is_file() {
        return;
    }
    let sketch = root.join("mockups/tokens.sketch.css");
    let mut css = if sketch.is_file() {
        fs::read_to_string(&sketch).unwrap_or_default()
    } else {
        FINAL_TOKENS_STARTER.to_string()
    };
    css.push_str("\n\n/* final — change font, color, imagery here. Sketch chrome stays. */\n");
    if let Some(k) = load_kit(root) {
        if !k.color.trim().is_empty() {
            css.push_str(&format!("/* kit color: {} */\n", k.color.trim()));
        }
        if !k.style.trim().is_empty() {
            css.push_str(&format!("/* kit style: {} */\n", k.style.trim()));
        }
        if !k.layout.trim().is_empty() {
            css.push_str(&format!("/* kit layout: {} */\n", k.layout.trim()));
        }
    }
    let _ = fs::create_dir_all(root.join("mockups"));
    let _ = fs::write(dest, css);
}

pub fn promote_final_if_green(root: &Path, led: &Ledger) {
    if led.entries.values().any(|e| e.status == GREEN) {
        promote_sketch_to_final(root);
        let _ = crate::scaffold::promote_prototype(root);
    }
}

/// Default hand-sketched look. Desk injects this; `mockups/tokens.sketch.css` overrides.
pub const SKETCH_CSS: &str = r#"
:root { --paper:#f4efe4; --ink:#2a2620; --rule:#c9bba6; --built:#1a1c20; --built-bg:#f7f8fa; }
html, body { margin:0; background: var(--paper); color: var(--ink);
  font: 15px/1.45 "Segoe Print", "Comic Sans MS", "Chalkboard SE", ui-rounded, system-ui, sans-serif; }
html, body { height: 100%; }
body { padding: 0; }
html.shalt-has-chrome body {
  display: grid;
  grid-template-columns: minmax(12rem, 16rem) 1fr;
  grid-template-rows: auto 1fr;
  grid-template-areas: "bar bar" "beats stage";
  min-height: 100%;
}
* { box-sizing: border-box; }
h1,h2,h3 { font-weight: 600; letter-spacing: -0.02em; transform: rotate(-0.4deg); }
html.shalt-sketch button, html.shalt-sketch .btn, html.shalt-sketch input,
html.shalt-sketch select, html.shalt-sketch textarea, html.shalt-sketch .card,
html.shalt-sketch nav, html.shalt-sketch header, html.shalt-sketch aside {
  border: 1.5px solid var(--ink) !important;
  border-radius: 12px 16px 11px 15px / 14px 11px 16px 12px !important;
  background: #fffdf8 !important;
  color: var(--ink) !important;
  box-shadow: 2px 3px 0 rgba(42,38,32,.12);
}
html.shalt-sketch button, html.shalt-sketch .btn {
  padding: .35rem .8rem; cursor: pointer; transform: rotate(0.3deg);
}
html.shalt-sketch button.primary, html.shalt-sketch .btn.primary {
  font-weight: 700;
}
.built, .built * { font-family: Inter, "SF Pro Display", ui-sans-serif, system-ui, sans-serif !important; }
html.built, html.built body, .built {
  background: var(--built-bg) !important; color: var(--built) !important;
}
html.built button, .built button, .built .btn, .built input {
  border-radius: 8px !important; box-shadow: none !important; transform: none !important;
}
.shalt-app-bar {
  grid-area: bar;
  display: flex; align-items: center; gap: 16px; flex-wrap: wrap;
  padding: 10px 14px;
  border-bottom: 1.5px solid var(--ink);
  background: #fffdf8;
}
.shalt-brand { font-weight: 700; text-decoration: none; color: inherit; }
.shalt-step { color: inherit; text-decoration: none; padding: 2px 8px; font-weight: 700; }
.shalt-app-nav { display: flex; flex-wrap: wrap; gap: 8px 14px; align-items: baseline; }
.shalt-app-nav a { color: inherit; text-decoration: none; padding: 2px 2px; }
.shalt-app-nav a.on { font-weight: 700; text-decoration: underline; }
.shalt-film-nav {
  grid-area: beats;
  display: flex; flex-direction: column; gap: 4px;
  margin: 0; padding: 10px 8px 16px;
  border-right: 1.5px solid var(--ink);
  background: #fffdf8;
  overflow: auto;
}
.shalt-film-nav a {
  color: inherit; text-decoration: none; padding: 6px 8px;
  display: block; font-size: 12px; line-height: 1.3;
  max-width: 100%; overflow: hidden; text-overflow: ellipsis;
}
.shalt-film-nav a.on, .shalt-film-nav a[aria-current] { font-weight: 700; background: color-mix(in srgb, var(--ink) 8%, #fffdf8); }
.shalt-stage { grid-area: stage; padding: 18px 22px 28px; min-width: 0; overflow: auto; }
.shalt-stage .sketch { max-width: 52rem; }
.shalt-stage .chrome,
.shalt-stage .sketch-frame {
  position: relative;
  width: 100% !important;
  max-width: 100%;
  margin-left: 0 !important;
  margin-right: 0 !important;
}
html.shalt-has-chrome .sketch > .nav { display: none; }
html.shalt-has-chrome .journey-nav { display: none; }
html.shalt-layout-side .shalt-film-nav { position: sticky; top: 0; }
[data-rid].on-beat {
  outline: 2px dashed var(--ink); outline-offset: 4px;
  scroll-margin-top: 16px;
}
.shalt-stub {
  position: fixed; inset: 0; z-index: 80;
  display: flex; flex-direction: column; align-items: center; justify-content: center;
  gap: 16px; padding: 28px 20px; text-align: center;
  background: var(--paper); color: var(--ink);
}
.shalt-stub[hidden] { display: none !important; }
.shalt-stub-msg {
  color: #c42b2b; font-weight: 700; font-size: 1.15rem; max-width: 28rem; margin: 0;
}
"#;

pub const BUILT_CSS: &str = r#"
html.built, .built { font-family: Inter, "SF Pro Display", ui-sans-serif, system-ui, sans-serif; }
"#;

fn xml_esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn inner_of(html: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let low = html.to_ascii_lowercase();
    let i = low.find(&open)?;
    let gt = html[i..].find('>')? + i + 1;
    let j = low[gt..].find(&close)?;
    let raw = html[gt..gt + j].to_string();
    let text = regex::Regex::new(r"<[^>]+>")
        .ok()?
        .replace_all(&raw, " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn collect_tag_text(html: &str, tag: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let low = html.to_ascii_lowercase();
    let mut from = 0;
    while out.len() < max {
        let Some(i) = low[from..].find(&open) else {
            break;
        };
        let i = from + i;
        let Some(gt_rel) = html[i..].find('>') else {
            break;
        };
        let gt = i + gt_rel + 1;
        let Some(j) = low[gt..].find(&close) else {
            break;
        };
        let raw = &html[gt..gt + j];
        let text = regex::Regex::new(r"<[^>]+>")
            .ok()
            .map(|re| re.replace_all(raw, " ").to_string())
            .unwrap_or_else(|| raw.to_string());
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if !text.is_empty() {
            out.push(text.chars().take(42).collect());
        }
        from = gt + j + close.len();
    }
    out
}

fn svg_rect(x: f32, y: f32, w: f32, h: f32, fill: &str, extra: &str) -> String {
    format!(
        "<rect x=\"{x:.0}\" y=\"{y:.0}\" width=\"{w:.0}\" height=\"{h:.0}\" fill=\"{fill}\" {extra}/>\n"
    )
}

fn svg_text(x: f32, y: f32, size: u8, fill: &str, label: &str, extra: &str) -> String {
    format!(
        "<text x=\"{x:.0}\" y=\"{y:.0}\" font-size=\"{size}\" font-family=\"system-ui, sans-serif\" fill=\"{fill}\" {extra}>{label}</text>\n",
        label = xml_esc(label)
    )
}

/// Sketch poster for a mockup HTML file. Saved next to the HTML as `*.thumb.svg`.
pub fn html_to_thumb_svg(html: &str, title: &str) -> String {
    let paper = "#f4efe4";
    let card = "#fffdf8";
    let ink = "#2a2620";
    let accent = "#5e6ad2";
    let h1 = inner_of(html, "h1")
        .or_else(|| inner_of(html, "h2"))
        .unwrap_or_else(|| title.trim().to_string());
    let h1 = if h1.chars().count() > 48 {
        format!("{}…", h1.chars().take(47).collect::<String>())
    } else {
        h1
    };
    let nav = collect_tag_text(html, "a", 5);
    let buttons = collect_tag_text(html, "button", 3);
    let mut y = 18.0;
    let mut body = String::from(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 400 250\" width=\"400\" height=\"250\">\n",
    );
    body.push_str(&svg_rect(0.0, 0.0, 400.0, 250.0, paper, ""));
    body.push_str(&svg_rect(
        10.0,
        10.0,
        380.0,
        230.0,
        card,
        "stroke=\"#2a2620\" stroke-width=\"1.6\" rx=\"10\"",
    ));
    if !nav.is_empty() {
        let mut x = 20.0;
        for (i, n) in nav.iter().take(4).enumerate() {
            let w = (n.len() as f32 * 6.2 + 16.0).min(90.0);
            if x + w > 380.0 {
                break;
            }
            let (fill, tfill) = if i == 0 { (ink, paper) } else { (card, ink) };
            body.push_str(&svg_rect(
                x,
                y,
                w,
                18.0,
                fill,
                "stroke=\"#2a2620\" stroke-width=\"1\" rx=\"5\"",
            ));
            body.push_str(&svg_text(
                x + 6.0,
                y + 13.0,
                9,
                tfill,
                &n.chars().take(12).collect::<String>(),
                "",
            ));
            x += w + 6.0;
        }
        y += 28.0;
    }
    body.push_str(&svg_text(
        20.0,
        y + 18.0,
        16,
        ink,
        &h1,
        "font-weight=\"600\"",
    ));
    y += 32.0;
    body.push_str(&svg_rect(
        18.0,
        y,
        364.0,
        88.0,
        paper,
        "stroke=\"#2a2620\" stroke-width=\"1.2\" stroke-dasharray=\"5 3\" rx=\"8\"",
    ));
    y += 108.0;
    let mut x = 20.0;
    let btns = if buttons.is_empty() {
        vec!["Action".to_string()]
    } else {
        buttons
    };
    for (i, b) in btns.iter().enumerate() {
        let w = (b.len() as f32 * 6.4 + 18.0).min(120.0);
        let (fill, tfill) = if i == 0 { (accent, "#ffffff") } else { (card, ink) };
        body.push_str(&svg_rect(
            x,
            y,
            w,
            22.0,
            fill,
            "stroke=\"#2a2620\" stroke-width=\"1.2\" rx=\"6\"",
        ));
        body.push_str(&svg_text(
            x + 8.0,
            y + 15.0,
            10,
            tfill,
            &b.chars().take(16).collect::<String>(),
            "",
        ));
        x += w + 8.0;
    }
    body.push_str("</svg>\n");
    body
}

pub fn thumb_rel(html_rel: &str) -> String {
    match html_rel.rfind('.') {
        Some(i) => format!("{}.thumb.svg", &html_rel[..i]),
        None => format!("{html_rel}.thumb.svg"),
    }
}

pub fn ensure_thumb(root: &Path, html_rel: &str, title: &str) -> String {
    if html_rel.is_empty() {
        return String::new();
    }
    let html_path = root.join("mockups").join(html_rel);
    if !html_path.is_file() {
        return String::new();
    }
    let rel = thumb_rel(html_rel);
    let thumb_path = root.join("mockups").join(&rel);
    if thumb_fresh(&thumb_path, &html_path) {
        return rel;
    }
    let html = fs::read_to_string(&html_path).unwrap_or_default();
    let svg = html_to_thumb_svg(&html, title);
    if let Some(dir) = thumb_path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if fs::write(&thumb_path, svg).is_ok() {
        rel
    } else {
        String::new()
    }
}

fn thumb_fresh(thumb: &Path, html: &Path) -> bool {
    let Ok(t) = fs::metadata(thumb).and_then(|m| m.modified()) else {
        return false;
    };
    let Ok(h) = fs::metadata(html).and_then(|m| m.modified()) else {
        return false;
    };
    t >= h
}

pub fn refresh_thumbs(root: &Path, features: &[Feature]) {
    let led = Ledger::load(&root.join(".shalt/ledger.json")).unwrap_or_default();
    for film in films(root, &features, &led) {
        for fr in film.frames {
            if !fr.file.is_empty() {
                let _ = ensure_thumb(root, &fr.file, &fr.caption);
            }
        }
    }
}
