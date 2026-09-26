//! Storyboards are a reading of the spec: journey → film, scenario → frame.

use shalt_core::ledger::{Ledger, GREEN, PENDING, RED};
use shalt_core::mockups::{
    assemble_mockup, design_needed, films, pick_design_journey, promote_sketch_to_final,
    verify_mockups, Frame,
};
use shalt_core::spec::load_specs;
use std::fs;
use tempfile::TempDir;

const SPEC: &str = r#"@epic:envelope
Feature: Seal a job

  As a sender
  I want to seal a job
  So that it cannot be opened in transit

  @rid:S-aaa11111
  Scenario: Open the inbox
    When the sender opens the inbox
    Then they see waiting jobs

  @rid:S-bbb22222
  Scenario: Write the envelope
    When the sender writes the envelope
    Then it is ready to seal
"#;

fn write_spec(root: &std::path::Path) {
    fs::create_dir_all(root.join("spec")).unwrap();
    fs::write(root.join("spec/seal.feature"), SPEC).unwrap();
}

fn load(root: &std::path::Path) -> (Vec<shalt_core::Feature>, Ledger) {
    let features = load_specs(&root.join("spec"), false).unwrap();
    let mut led = Ledger::default();
    led.sync_spec(&features);
    (features, led)
}

#[test]
fn empty_mockups_still_derive_frames_in_spec_order() {
    let t = TempDir::new().unwrap();
    write_spec(t.path());
    let (features, led) = load(t.path());
    let wall = films(t.path(), &features, &led);
    assert_eq!(wall.len(), 1);
    assert_eq!(wall[0].journey, "envelope");
    assert!(wall[0].story.contains("sender"));
    assert_eq!(wall[0].kind, "ui");
    assert!(!wall[0].stale);
    let rids: Vec<_> = wall[0].frames.iter().map(|f| f.rid.as_str()).collect();
    assert_eq!(rids, vec!["S-aaa11111", "S-bbb22222"]);
    assert!(wall[0].frames.iter().all(|f| f.file.is_empty()));
    assert!(wall[0].frames.iter().all(|f| !f.built));
}

#[test]
fn pick_design_journey_is_the_next_undrawn_wall() {
    let t = TempDir::new().unwrap();
    write_spec(t.path());
    fs::write(
        t.path().join("spec/share.feature"),
        r#"@epic:share
Feature: Share
  @rid:S-ccc33333
  Scenario: Send a link
    When they share
    Then a link exists
"#,
    )
    .unwrap();
    let features = load_specs(&t.path().join("spec"), false).unwrap();
    assert_eq!(
        pick_design_journey(t.path(), &features, "").as_deref(),
        Some("envelope")
    );
    let (features, led) = load(t.path());
    let hash = films(t.path(), &features, &led)[0].spec_hash.clone();
    let dir = t.path().join("mockups/journeys/envelope");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("storyboard.json"),
        format!(
            r#"{{"journey":"envelope","kind":"ui","spec_hash":"{hash}","frames":[
            {{"rid":"S-aaa11111","file":"inbox.html"}},
            {{"rid":"S-bbb22222","file":"compose.html"}}
        ]}}"#
        ),
    )
    .unwrap();
    fs::write(dir.join("inbox.html"), "<p>inbox</p>").unwrap();
    fs::write(dir.join("compose.html"), "<p>compose</p>").unwrap();
    let features = load_specs(&t.path().join("spec"), false).unwrap();
    assert_eq!(
        pick_design_journey(t.path(), &features, "").as_deref(),
        Some("share")
    );
    assert_eq!(
        pick_design_journey(t.path(), &features, "envelope").as_deref(),
        Some("share"),
        "drawn focus falls through to the next empty journey"
    );
}

#[test]
fn screens_alias_joins_like_frames() {
    let t = TempDir::new().unwrap();
    write_spec(t.path());
    let (features, led) = load(t.path());
    let hash = films(t.path(), &features, &led)[0].spec_hash.clone();
    let dir = t.path().join("mockups/journeys/envelope");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("storyboard.json"),
        format!(
            r#"{{
          "journey": "envelope",
          "kind": "ui",
          "spec_hash": "{hash}",
          "screens": [
            {{"rid": "S-aaa11111", "file": "inbox.html", "caption": "Inbox"}},
            {{"rid": "S-bbb22222", "file": "compose.html", "caption": "Write"}}
          ]
        }}"#
        ),
    )
    .unwrap();
    fs::write(dir.join("inbox.html"), "<p>inbox</p>").unwrap();
    fs::write(dir.join("compose.html"), "<p>compose</p>").unwrap();
    let wall = films(t.path(), &features, &led);
    let files: Vec<_> = wall[0].frames.iter().map(|f| f.file.as_str()).collect();
    assert_eq!(
        files,
        vec![
            "journeys/envelope/inbox.html",
            "journeys/envelope/compose.html"
        ]
    );
    assert!(!design_needed(t.path(), &features));
}

#[test]
fn json_array_order_is_ignored() {
    let t = TempDir::new().unwrap();
    write_spec(t.path());
    let dir = t.path().join("mockups/journeys/envelope");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("storyboard.json"),
        r#"{
          "journey": "envelope",
          "kind": "ui",
          "spec_hash": "ignore-me",
          "frames": [
            {"rid": "S-bbb22222", "file": "compose.html", "caption": "Write"},
            {"rid": "S-aaa11111", "file": "inbox.html", "caption": "Inbox"}
          ]
        }"#,
    )
    .unwrap();
    fs::write(dir.join("inbox.html"), "<p>inbox</p>").unwrap();
    fs::write(dir.join("compose.html"), "<p>compose</p>").unwrap();
    let (features, led) = load(t.path());
    let wall = films(t.path(), &features, &led);
    let files: Vec<_> = wall[0].frames.iter().map(|f| f.file.as_str()).collect();
    assert_eq!(
        files,
        vec![
            "journeys/envelope/inbox.html",
            "journeys/envelope/compose.html"
        ]
    );
}

#[test]
fn kind_none_needs_no_html() {
    let t = TempDir::new().unwrap();
    write_spec(t.path());
    let (features, led) = load(t.path());
    let hash = films(t.path(), &features, &led)[0].spec_hash.clone();
    let dir = t.path().join("mockups/journeys/envelope");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("storyboard.json"),
        format!(
            r#"{{"journey":"envelope","kind":"none","spec_hash":"{hash}","frames":[]}}"#
        ),
    )
    .unwrap();
    let wall = films(t.path(), &features, &led);
    assert_eq!(wall[0].kind, "none");
    assert!(!wall[0].stale);
    assert!(!design_needed(t.path(), &features));
    assert!(verify_mockups(t.path(), &features).is_empty());
}

#[test]
fn green_rid_marks_the_frame_built_pending_does_not() {
    let t = TempDir::new().unwrap();
    write_spec(t.path());
    let (features, mut led) = load(t.path());
    if let Some(e) = led.entries.get_mut("S-aaa11111") {
        e.status = GREEN.into();
    }
    if let Some(e) = led.entries.get_mut("S-bbb22222") {
        e.status = RED.into();
    }
    let wall = films(t.path(), &features, &led);
    let by: Vec<(&str, bool)> = wall[0]
        .frames
        .iter()
        .map(|f| (f.rid.as_str(), f.built))
        .collect();
    assert_eq!(by, vec![("S-aaa11111", true), ("S-bbb22222", false)]);
    if let Some(e) = led.entries.get_mut("S-aaa11111") {
        e.status = PENDING.into();
    }
    let wall = films(t.path(), &features, &led);
    assert!(!wall[0].frames[0].built);
}

#[test]
fn orphan_rid_and_missing_file_fail_verify() {
    let t = TempDir::new().unwrap();
    write_spec(t.path());
    let dir = t.path().join("mockups/journeys/envelope");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("storyboard.json"),
        r#"{
          "journey": "envelope",
          "kind": "ui",
          "frames": [
            {"rid": "S-aaa11111", "file": "inbox.html"},
            {"rid": "S-deadbeef", "file": "ghost.html"}
          ]
        }"#,
    )
    .unwrap();
    let (features, _) = load(t.path());
    let problems = verify_mockups(t.path(), &features);
    assert!(
        problems.iter().any(|p| p.contains("S-deadbeef")),
        "{problems:?}"
    );
    assert!(
        problems.iter().any(|p| p.contains("inbox.html")),
        "{problems:?}"
    );
}

#[test]
fn missing_json_is_not_a_verify_failure_but_design_is_needed() {
    let t = TempDir::new().unwrap();
    write_spec(t.path());
    let (features, _) = load(t.path());
    assert!(verify_mockups(t.path(), &features).is_empty());
    assert!(design_needed(t.path(), &features));
}

#[test]
fn json_without_html_still_needs_design() {
    let t = TempDir::new().unwrap();
    write_spec(t.path());
    let (features, led) = load(t.path());
    let hash = films(t.path(), &features, &led)[0].spec_hash.clone();
    let dir = t.path().join("mockups/journeys/envelope");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("storyboard.json"),
        format!(
            r#"{{"journey":"envelope","kind":"ui","spec_hash":"{hash}","frames":[{{"rid":"S-aaa11111","file":"","caption":"Inbox"}}]}}"#
        ),
    )
    .unwrap();
    assert!(design_needed(t.path(), &features));
}

#[test]
fn html_on_disk_clears_design_needed() {
    let t = TempDir::new().unwrap();
    write_spec(t.path());
    let (features, led) = load(t.path());
    let hash = films(t.path(), &features, &led)[0].spec_hash.clone();
    let dir = t.path().join("mockups/journeys/envelope");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("inbox.html"), "<p>inbox</p>").unwrap();
    fs::write(dir.join("compose.html"), "<p>compose</p>").unwrap();
    fs::write(
        dir.join("storyboard.json"),
        format!(
            r#"{{"journey":"envelope","kind":"ui","spec_hash":"{hash}","frames":[{{"rid":"S-aaa11111","file":"inbox.html"}},{{"rid":"S-bbb22222","file":"compose.html"}}]}}"#
        ),
    )
    .unwrap();
    assert!(!design_needed(t.path(), &features));
}

#[test]
fn stale_hash_is_a_badge_not_a_verify_failure() {
    let t = TempDir::new().unwrap();
    write_spec(t.path());
    let dir = t.path().join("mockups/journeys/envelope");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("inbox.html"), "<p>x</p>").unwrap();
    fs::write(dir.join("compose.html"), "<p>y</p>").unwrap();
    fs::write(
        dir.join("storyboard.json"),
        r#"{
          "journey": "envelope",
          "kind": "ui",
          "spec_hash": "old",
          "frames": [
            {"rid": "S-aaa11111", "file": "inbox.html"},
            {"rid": "S-bbb22222", "file": "compose.html"}
          ]
        }"#,
    )
    .unwrap();
    let (features, led) = load(t.path());
    let wall = films(t.path(), &features, &led);
    assert!(wall[0].stale);
    assert!(design_needed(t.path(), &features));
    assert!(verify_mockups(t.path(), &features).is_empty());
}

#[test]
fn designer_prompt_marks_empty_frames_and_focus_rid() {
    let t = TempDir::new().unwrap();
    write_spec(t.path());
    let all = shalt_core::designer_user_prompt(t.path());
    assert!(all.contains("No look kit yet"));
    assert!(all.contains("empty UI frame"));
    assert!(all.contains("S-aaa11111"));
    assert!(all.contains("(empty)"));
    assert!(all.contains("mockups/journeys/envelope/envelope.html"));
    assert!(all.contains("Do not write mockups/storyboard.json"));
    assert!(!all.contains("← this beat"));
    let one = shalt_core::designer_user_prompt_for(t.path(), "S-aaa11111");
    assert!(one.contains("Focus: draw only rid `S-aaa11111`"));
    assert!(one.contains("S-aaa11111"));
    assert!(one.contains("← this beat"));
    assert!(one.contains("S-bbb22222"));
}

#[test]
fn designer_prompt_nav_is_this_spec_not_envelope_roles() {
    let t = TempDir::new().unwrap();
    write_spec(t.path());
    let prompt = shalt_core::designer_user_prompt(t.path());
    assert!(
        prompt.contains("this spec's journeys (envelope)"),
        "{prompt}"
    );
    assert!(
        prompt.contains("Do not write an <html> document"),
        "designer fills the sketch framework, not a full page: {prompt}"
    );
    for leak in ["Traveler", "Buyer", "Desk, Traveler"] {
        assert!(
            !prompt.contains(leak),
            "another product's journeys must not appear in this spec's designer prompt: {leak} in {prompt}"
        );
    }
}

#[test]
fn html_thumb_svg_keeps_the_heading_and_is_saved() {
    let t = TempDir::new().unwrap();
    write_spec(t.path());
    let (features, led) = load(t.path());
    let hash = films(t.path(), &features, &led)[0].spec_hash.clone();
    let dir = t.path().join("mockups/journeys/envelope");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("inbox.html"),
        r#"<html><body><nav><a href="compose.html">Write</a></nav><h1>Open the inbox</h1><button>Seal</button></body></html>"#,
    )
    .unwrap();
    fs::write(dir.join("compose.html"), "<p>c</p>").unwrap();
    fs::write(
        dir.join("storyboard.json"),
        format!(
            r#"{{"journey":"envelope","kind":"ui","spec_hash":"{hash}","frames":[{{"rid":"S-aaa11111","file":"inbox.html"}},{{"rid":"S-bbb22222","file":"compose.html"}}]}}"#
        ),
    )
    .unwrap();
    let svg = shalt_core::html_to_thumb_svg(
        &fs::read_to_string(dir.join("inbox.html")).unwrap(),
        "Inbox",
    );
    assert!(svg.contains("<svg"), "{svg}");
    assert!(svg.contains("Open the inbox"), "{svg}");
    assert!(svg.contains("Seal"), "{svg}");
    assert_eq!(
        shalt_core::ensure_thumb(t.path(), "journeys/envelope/inbox.html", "Inbox"),
        "journeys/envelope/inbox.thumb.svg"
    );
    let wall = films(t.path(), &features, &led);
    assert_eq!(wall[0].frames[0].thumb, "journeys/envelope/inbox.thumb.svg");
    assert!(t.path().join("mockups/journeys/envelope/inbox.thumb.svg").is_file());
}

#[test]
fn mockup_rel_normalizes_parent_links() {
    assert_eq!(
        shalt_core::normalize_rel("journeys/award/../desk/x.html").as_deref(),
        Some("journeys/desk/x.html")
    );
    assert!(shalt_core::normalize_rel("../etc/passwd").is_none());
}

#[test]
fn mockup_inject_adds_journey_nav() {
    let t = TempDir::new().unwrap();
    write_spec(t.path());
    let (features, led) = load(t.path());
    let hash = films(t.path(), &features, &led)[0].spec_hash.clone();
    let dir = t.path().join("mockups/journeys/envelope");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("inbox.html"), "<p>inbox</p>").unwrap();
    fs::write(dir.join("compose.html"), "<p>compose</p>").unwrap();
    fs::write(
        dir.join("storyboard.json"),
        format!(
            r#"{{"journey":"envelope","kind":"ui","spec_hash":"{hash}","frames":[{{"rid":"S-aaa11111","file":"inbox.html","caption":"Inbox"}},{{"rid":"S-bbb22222","file":"compose.html","caption":"Write"}}]}}"#
        ),
    )
    .unwrap();
    let (head, body) =
        shalt_core::mockup_inject(t.path(), "proj", "journeys/envelope/inbox.html", "", "S-aaa11111");
    assert!(head.contains("NAV"), "{head}");
    assert!(head.contains("scrollIntoView"), "{head}");
    assert!(head.contains("focusBeat"), "{head}");
    assert!(body.contains("shalt-film-nav"), "{body}");
    assert!(body.contains("shalt-app-bar"), "{body}");
    assert!(body.contains("compose.html"), "{body}");
    assert!(body.contains("S-bbb22222"), "{body}");
    assert!(body.contains("data-shalt-step"), "{body}");
    assert!(head.contains("shalt: 'step'") || head.contains("shalt:\"step\""), "{head}");
}

#[test]
fn sketch_css_lets_desktop_chrome_fit_the_stage() {
    assert!(
        shalt_core::SKETCH_CSS.contains(".shalt-stage .chrome"),
        "desktop sketches fix a 1024px chrome; the inject stylesheet must shrink it to the stage"
    );
    assert!(shalt_core::SKETCH_CSS.contains("max-width: 100%"), "{}", shalt_core::SKETCH_CSS);
}

#[test]
fn sketch_css_keeps_controls_readable() {
    let css = shalt_core::SKETCH_CSS;
    assert!(
        css.contains("html.shalt-sketch button"),
        "pencil fill is sketch-only so product tokens can color primary actions: {css}"
    );
    assert!(
        css.contains("color: var(--ink) !important"),
        "sketch controls stay ink-on-paper even if a project token sets light text: {css}"
    );
    assert!(
        !css.contains(
            "button, .btn, input, select, textarea, .card, nav, header, aside {\n  border: 1.5px solid var(--ink) !important;"
        ),
        "unscoped cream !important on every button makes .primary unreadable: {css}"
    );
}

#[test]
fn mockup_inject_maps_desk_and_traveler_across_journeys() {
    let t = TempDir::new().unwrap();
    write_spec(t.path());
    let (features, led) = load(t.path());
    let hash = films(t.path(), &features, &led)[0].spec_hash.clone();
    let award = t.path().join("mockups/journeys/award");
    let desk = t.path().join("mockups/journeys/desk");
    fs::create_dir_all(&award).unwrap();
    fs::create_dir_all(&desk).unwrap();
    fs::write(
        award.join("refuse.html"),
        r##"<a href="#">Desk</a> <a href="../desk/desk-seed.html">Desk path</a>"##,
    )
    .unwrap();
    fs::write(desk.join("desk-seed.html"), "<p>desk</p>").unwrap();
    fs::write(
        award.join("storyboard.json"),
        format!(
            r#"{{"journey":"award","frames":[{{"rid":"S-aaa11111","file":"refuse.html","caption":"Refuse"}}]}}"#
        ),
    )
    .unwrap();
    let _ = hash;
    fs::write(
        desk.join("storyboard.json"),
        r#"{"journey":"desk","frames":[{"rid":"S-desk0001","file":"desk-seed.html","caption":"Desk"}]}"#,
    )
    .unwrap();
    let (head, body) = shalt_core::mockup_inject(
        t.path(),
        "proj",
        "journeys/award/refuse.html",
        "",
        "S-aaa11111",
    );
    assert!(body.contains("shalt-app-bar"), "{body}");
    assert!(body.contains("Desk"), "{body}");
    assert!(body.contains("journeys/desk/desk-seed.html"), "{body}");
    assert!(head.contains("resolveLabel"), "{head}");
    assert!(head.contains("byRel"), "{head}");
}

#[test]
fn mockup_inject_brand_follows_this_project_not_desk() {
    let t = TempDir::new().unwrap();
    fs::create_dir_all(t.path().join("spec")).unwrap();
    fs::write(
        t.path().join("shalt.toml"),
        "[project]\nname = \"Recipe list\"\nstack = \"rust\"\n",
    )
    .unwrap();
    fs::write(
        t.path().join("spec/recipes.feature"),
        "@epic:recipes\nFeature: Catalog\n  Scenario: List\n    Then ok\n",
    )
    .unwrap();
    let dir = t.path().join("mockups/journeys/recipes");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("list.html"), "<h1>Catalog</h1>").unwrap();
    fs::write(
        dir.join("storyboard.json"),
        r#"{"journey":"recipes","kind":"ui","frames":[{"rid":"S-aaa11111","file":"list.html","caption":"List"}]}"#,
    )
    .unwrap();
    let (_head, body) = shalt_core::mockup_inject(
        t.path(),
        "proj",
        "journeys/recipes/list.html",
        "",
        "S-aaa11111",
    );
    assert!(body.contains("shalt-brand"), "{body}");
    assert!(
        body.contains(">Recipe list</a>"),
        "primary nav brand is this project: {body}"
    );
    assert!(body.contains(">Recipes</a>"), "{body}");
    assert!(
        !body.contains(">Desk</a>"),
        "recipes must not wear Envelope's Desk brand: {body}"
    );
    assert!(!body.contains("Traveler"), "{body}");
    assert!(!body.contains("Buyer"), "{body}");
    let head_l = _head.to_ascii_lowercase();
    for leak in [
        "journeyhome('award')",
        "journeyhome('desk')",
        "journeyhome('traveler')",
        "buyer|award",
        "traveler",
    ] {
        assert!(
            !head_l.contains(leak),
            "inject JS must not carry another product's journeys into this workspace: {leak}\n{_head}"
        );
    }
}

#[test]
fn kit_json_is_honored_in_the_designer_prompt() {
    let t = TempDir::new().unwrap();
    write_spec(t.path());
    assert!(shalt_core::load_kit(t.path()).is_none());
    fs::create_dir_all(t.path().join("mockups")).unwrap();
    fs::write(
        t.path().join("mockups/kit.json"),
        r#"{"platform":"desktop","style":"industrial sketch","color":"cream and charcoal","layout":"left nav","guessed":true}"#,
    )
    .unwrap();
    let kit = shalt_core::load_kit(t.path()).expect("kit");
    assert_eq!(kit.style, "industrial sketch");
    assert!(kit.guessed);
    let prompt = shalt_core::designer_user_prompt(t.path());
    assert!(prompt.contains("Platform is set"), "{prompt}");
    assert!(prompt.contains("Layout HTML only") || prompt.contains("Do not restyle"), "{prompt}");
    assert!(prompt.contains("industrial sketch"), "{prompt}");
    assert!(prompt.contains("design interview"), "{prompt}");
    assert!(!prompt.contains("Honor it in tokens.sketch.css and every frame"));
    assert!(!prompt.contains("No look kit yet"));
}

#[test]
fn designer_prompt_is_layout_not_a_stylesheet() {
    let t = TempDir::new().unwrap();
    write_spec(t.path());
    let prompt = shalt_core::designer_user_prompt(t.path());
    assert!(prompt.contains("Layout only"), "{prompt}");
    assert!(prompt.contains("ink on paper"), "{prompt}");
    assert!(
        prompt.contains(":root vars only") || prompt.contains("--paper,--ink,--accent"),
        "{prompt}"
    );
    assert!(prompt.contains("tokens.final.css waits"), "{prompt}");
}

#[test]
fn kit_without_platform_asks_phone_tablet_or_desktop_first() {
    let t = TempDir::new().unwrap();
    write_spec(t.path());
    fs::create_dir_all(t.path().join("mockups")).unwrap();
    fs::write(
        t.path().join("mockups/kit.json"),
        r#"{"style":"industrial sketch","color":"cream","layout":"left nav"}"#,
    )
    .unwrap();
    let prompt = shalt_core::designer_user_prompt(t.path());
    assert!(prompt.contains("platform is not"));
    assert!(prompt.contains("Phone, Tablet, or Desktop"));
    assert!(!prompt.contains("Look is already set"));
}

#[test]
fn empty_kit_asks_platform_first() {
    let t = TempDir::new().unwrap();
    write_spec(t.path());
    let prompt = shalt_core::designer_user_prompt(t.path());
    assert!(prompt.contains("No look kit yet"));
    assert!(prompt.contains("Phone, Tablet, or Desktop"));
}

#[test]
fn normalize_platform_aliases() {
    assert_eq!(shalt_core::normalize_platform("Phone"), "phone");
    assert_eq!(shalt_core::normalize_platform("iPad"), "tablet");
    assert_eq!(shalt_core::normalize_platform("web"), "desktop");
    assert_eq!(shalt_core::normalize_platform(""), "");
    assert_eq!(shalt_core::kit_platform(None), "desktop");
}

#[test]
fn mockup_inject_stubs_unmapped_paths_and_marks_the_beat() {
    let t = TempDir::new().unwrap();
    write_spec(t.path());
    fs::create_dir_all(t.path().join("mockups")).unwrap();
    fs::write(
        t.path().join("mockups/kit.json"),
        r#"{"platform":"phone","style":"sketch","color":"cream","layout":"top nav"}"#,
    )
    .unwrap();
    let (features, led) = load(t.path());
    let hash = films(t.path(), &features, &led)[0].spec_hash.clone();
    let dir = t.path().join("mockups/journeys/envelope");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("inbox.html"),
        r#"<body><header>Product</header><main data-rid="S-aaa11111"><h1>Inbox</h1><input name="q"><button>Go</button></main></body>"#,
    )
    .unwrap();
    fs::write(dir.join("compose.html"), "<p>compose</p>").unwrap();
    fs::write(
        dir.join("storyboard.json"),
        format!(
            r#"{{"journey":"envelope","kind":"ui","spec_hash":"{hash}","frames":[{{"rid":"S-aaa11111","file":"inbox.html","caption":"Inbox"}},{{"rid":"S-bbb22222","file":"compose.html","caption":"Write"}}]}}"#
        ),
    )
    .unwrap();
    let (head, _body) =
        shalt_core::mockup_inject(t.path(), "proj", "journeys/envelope/inbox.html", "", "S-aaa11111");
    assert!(head.contains("shalt-stub"), "{head}");
    assert!(head.contains("The test does not pass"), "{head}");
    assert!(head.contains("shalt-platform-phone"), "{head}");
    assert!(head.contains("on-beat"), "{head}");
    assert!(head.contains("scrollIntoView"), "{head}");
    assert!(head.contains("removeAttribute('disabled')"), "{head}");
}

#[test]
fn fragment_is_wrapped_in_the_sketch_shell() {
    let t = TempDir::new().unwrap();
    write_spec(t.path());
    let dir = t.path().join("mockups/journeys/envelope");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("storyboard.json"),
        r#"{"journey":"envelope","kind":"ui","frames":[{"rid":"S-aaa11111","file":"inbox.html","caption":"Inbox"}]}"#,
    )
    .unwrap();
    let page = assemble_mockup(
        t.path(),
        "proj",
        "journeys/envelope/inbox.html",
        r#"<section data-rid="S-aaa11111"><h1>Inbox</h1></section>"#,
        "",
        "S-aaa11111",
    );
    assert!(page.contains("<!doctype html>"), "{page}");
    assert!(page.contains("shalt-app-bar"), "{page}");
    assert!(page.contains("data-rid=\"S-aaa11111\""), "{page}");
    assert!(page.contains("<h1>Inbox</h1>"), "{page}");
}

#[test]
fn full_document_keeps_its_markup() {
    let t = TempDir::new().unwrap();
    write_spec(t.path());
    let page = assemble_mockup(
        t.path(),
        "proj",
        "journeys/envelope/inbox.html",
        "<!doctype html><html><head><title>Mine</title></head><body><p>keep</p></body></html>",
        "",
        "S-aaa11111",
    );
    assert!(page.contains("<title>Mine</title>"), "{page}");
    assert!(page.contains("<p>keep</p>"), "{page}");
    assert_eq!(page.matches("<!doctype html>").count(), 1);
}

#[test]
fn final_css_is_the_product_look_even_before_green() {
    let t = TempDir::new().unwrap();
    write_spec(t.path());
    fs::create_dir_all(t.path().join("mockups")).unwrap();
    fs::write(t.path().join("mockups/tokens.final.css"), "/* product */\n").unwrap();
    let page = assemble_mockup(
        t.path(),
        "proj",
        "journeys/envelope/inbox.html",
        r#"<section data-rid="S-aaa11111"><h1>Inbox</h1></section>"#,
        "",
        "S-aaa11111",
    );
    assert!(
        page.contains("tokens.final.css"),
        "polished prototype is the product, not only after green: {page}"
    );
}

#[test]
fn promote_sketch_to_final_copies_once() {
    let t = TempDir::new().unwrap();
    fs::create_dir_all(t.path().join("mockups")).unwrap();
    fs::write(t.path().join("mockups/tokens.sketch.css"), "/* sketch */\n").unwrap();
    fs::write(
        t.path().join("mockups/kit.json"),
        r#"{"platform":"desktop","color":"cream","style":"industrial"}"#,
    )
    .unwrap();
    promote_sketch_to_final(t.path());
    let final_css = fs::read_to_string(t.path().join("mockups/tokens.final.css")).unwrap();
    assert!(final_css.contains("/* sketch */"), "{final_css}");
    assert!(final_css.contains("kit color: cream"), "{final_css}");
    fs::write(t.path().join("mockups/tokens.final.css"), "LOCKED").unwrap();
    promote_sketch_to_final(t.path());
    let again = fs::read_to_string(t.path().join("mockups/tokens.final.css")).unwrap();
    assert_eq!(again, "LOCKED");
}

#[allow(dead_code)]
fn frame_named<'a>(frames: &'a [Frame], rid: &str) -> &'a Frame {
    frames.iter().find(|f| f.rid == rid).unwrap()
}
