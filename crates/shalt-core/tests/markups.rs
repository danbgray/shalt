use shalt_core::markups::{
    designer_markup_prompt, embed_tag, load_interview, load_markup, markup_enabled,
    page_polish_prompt, polish_prompt, save_interview, save_markup, Interview, InterviewTurn,
    Markup, Note, Stroke,
};
use shalt_core::{assemble_mockup, designer_user_prompt};
use tempfile::TempDir;

#[test]
fn save_and_load_red_pen_and_notes() {
    let t = TempDir::new().unwrap();
    let saved = save_markup(
        t.path(),
        Markup {
            rid: "S-aaa11111".into(),
            journey: "recipes".into(),
            file: "journeys/recipes/recipes.html".into(),
            strokes: vec![Stroke {
                color: "#c0392b".into(),
                width: 2.4,
                points: vec![[0.1, 0.2], [0.3, 0.4]],
            }],
            notes: vec![Note {
                id: "n1".into(),
                x: 0.5,
                y: 0.2,
                text: "Primary action should be bigger".into(),
            }],
        },
    )
    .unwrap();
    assert_eq!(saved.rid, "S-aaa11111");
    let loaded = load_markup(t.path(), "recipes", "S-aaa11111");
    assert_eq!(loaded.notes.len(), 1);
    assert_eq!(loaded.notes[0].text, "Primary action should be bigger");
    assert_eq!(loaded.strokes[0].points.len(), 2);
}

#[test]
fn designer_prompt_includes_interview_and_markup_notes() {
    let t = TempDir::new().unwrap();
    std::fs::create_dir_all(t.path().join("spec")).unwrap();
    std::fs::write(
        t.path().join("spec/recipes.feature"),
        "@epic:recipes\nFeature: Cook\n  @rid:S-aaa11111\n  Scenario: Make soup\n    When they cook\n    Then soup exists\n",
    )
    .unwrap();
    save_markup(
        t.path(),
        Markup {
            rid: "S-aaa11111".into(),
            journey: "recipes".into(),
            notes: vec![Note {
                id: "n1".into(),
                x: 0.4,
                y: 0.3,
                text: "Move the save button up".into(),
            }],
            ..Default::default()
        },
    )
    .unwrap();
    save_interview(
        t.path(),
        Interview {
            polish: "Cookbook magazine, cream paper, indigo accent".into(),
            typeface: "Serif headlines, sans UI".into(),
            density: "comfortable".into(),
            answers: vec![InterviewTurn {
                q: "What should polished feel like?".into(),
                a: "A Sunday supplement, not a pencil sketch".into(),
            }],
            ..Default::default()
        },
    )
    .unwrap();
    let appendix = designer_markup_prompt(t.path(), "recipes", "S-aaa11111");
    assert!(appendix.contains("DESIGN INTERVIEW"), "{appendix}");
    assert!(appendix.contains("Cookbook magazine"), "{appendix}");
    assert!(appendix.contains("tokens.final.css"), "{appendix}");
    assert!(appendix.contains("HUMAN MARKUPS"), "{appendix}");
    assert!(appendix.contains("Move the save button up"), "{appendix}");
    let prompt = designer_user_prompt(t.path());
    assert!(prompt.contains("Move the save button up"), "{prompt}");
    assert!(prompt.contains("tokens.final.css"), "{prompt}");
    let iv = load_interview(t.path());
    assert_eq!(iv.density, "comfortable");
}

#[test]
fn polish_prompt_is_one_look_for_every_mockup() {
    let p = polish_prompt();
    assert!(p.contains("every mockup"), "{p}");
    assert!(p.contains("tokens.final.css"), "{p}");
    assert!(p.contains("template comes first"), "{p}");
    assert!(!p.contains("Focus rid"), "{p}");
}

#[test]
fn page_polish_waits_until_the_template_exists() {
    let t = TempDir::new().unwrap();
    save_interview(
        t.path(),
        Interview {
            color: "cream and indigo".into(),
            typeface: "serif headlines".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let before = designer_markup_prompt(t.path(), "recipes", "S-aaa11111");
    assert!(before.contains("Write mockups/tokens.final.css"), "{before}");
    assert!(before.contains("Individual screens are polished only after"), "{before}");
    std::fs::create_dir_all(t.path().join("mockups")).unwrap();
    std::fs::write(t.path().join("mockups/tokens.final.css"), "/* product */\n").unwrap();
    let after = designer_markup_prompt(t.path(), "recipes", "S-aaa11111");
    assert!(after.contains("already applied"), "{after}");
    assert!(after.contains("Do not rewrite the template"), "{after}");
    assert!(after.contains("Page polish only"), "{after}");
    assert!(!after.contains("Write mockups/tokens.final.css from this"), "{after}");
    let page = page_polish_prompt("S-aaa11111", "recipes");
    assert!(page.contains("Do not rewrite that file"), "{page}");
    assert!(page.contains("S-aaa11111"), "{page}");
    assert!(page.contains("recipes"), "{page}");
}

#[test]
fn bad_rid_is_refused() {
    let t = TempDir::new().unwrap();
    let err = save_markup(
        t.path(),
        Markup {
            rid: "../etc/passwd".into(),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(err.contains("rid"), "{err}");
}

fn write_markup_toml(root: &std::path::Path, key: &str, secret: &str) {
    std::fs::write(
        root.join("shalt.toml"),
        format!("[markup]\nkey = {key:?}\nsecret = {secret:?}\n"),
    )
    .unwrap();
}

#[test]
fn shalt_toml_markup_key_injects_embed_js() {
    let t = TempDir::new().unwrap();
    write_markup_toml(t.path(), "rmk_pub_testonly", "rmk_sec_neverprint");
    assert!(markup_enabled(t.path()));
    let tag = embed_tag(t.path(), "proj", "S-aaa11111");
    assert!(tag.contains("markup.rivlet.io/embed.js"), "{tag}");
    assert!(tag.contains("rmk_pub_testonly"), "{tag}");
    assert!(tag.contains("data-label=\"Pin\""), "{tag}");
    assert!(!tag.contains("rmk_sec"), "{tag}");
    assert!(!tag.contains("neverprint"), "{tag}");
    let page = assemble_mockup(
        t.path(),
        "proj",
        "journeys/recipes/recipes.html",
        "<section data-rid=\"S-aaa11111\"><h1>Soup</h1></section>",
        "",
        "S-aaa11111",
    );
    assert!(page.contains("markup.rivlet.io/embed.js"), "{page}");
    assert!(page.contains("rmk_pub_testonly"), "{page}");
    assert!(!page.contains("rmk_sec_neverprint"), "{page}");
    let appendix = designer_markup_prompt(t.path(), "recipes", "S-aaa11111");
    assert!(appendix.contains("markup.rivlet.io"), "{appendix}");
    assert!(!appendix.contains("rmk_sec"), "{appendix}");
    assert!(!appendix.contains("neverprint"), "{appendix}");
}

#[test]
fn embed_is_empty_without_a_project_key() {
    let t = TempDir::new().unwrap();
    std::fs::write(t.path().join("shalt.toml"), "[project]\nname = \"x\"\n").unwrap();
    let tag = embed_tag(t.path(), "proj", "S-aaa11111");
    if markup_enabled(t.path()) {
        // A machine-wide MARKUP_PUB_KEY / ~/.shalt keys.markup is set; still no secret.
        assert!(!tag.contains("rmk_sec"), "{tag}");
        return;
    }
    assert!(tag.is_empty(), "{tag}");
    let page = assemble_mockup(t.path(), "proj", "x.html", "<p>hi</p>", "", "S-aaa11111");
    assert!(!page.contains("embed.js"), "{page}");
}
