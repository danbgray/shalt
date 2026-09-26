use shalt_core::bindings::{
    bound_count, def_matches, parse_step_defs, pick_steps_journey, scenario_is_bound, step_text,
};
use shalt_core::spec::{load_specs, Feature};

fn feature(src: &str) -> Feature {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("spec")).unwrap();
    std::fs::write(dir.path().join("spec/desk.feature"), src).unwrap();
    load_specs(&dir.path().join("spec"), false).unwrap().remove(0)
}

const DESK: &str = r#"
@epic:desk
Feature: Desk list
  Scenario: Seed desk
    When the buyer opens the desk
    Then the desk lists 4 travelers
"#;

const TRAVELER: &str = r#"
@epic:traveler
Feature: View a traveler
  Scenario: Missing id
    When the user opens "/p/missing"
    Then the page shows not on desk
"#;

#[test]
fn cucumber_js_binds_money_steps() {
    let defs = parse_step_defs(
        r#"
import { Given, When, Then, setWorldConstructor } from '@cucumber/cucumber';
import assert from 'node:assert/strict';
import { total } from '../src/index.js';

class World { constructor() { this.a = ''; this.b = ''; this.sum = ''; } }
setWorldConstructor(World);

Given('amounts {string} and {string}', function (a, b) {
  this.a = a;
  this.b = b;
});

Then('the total is {string}', function (expected) {
  assert.equal(total(this.a, this.b), expected);
});
"#,
    );
    assert_eq!(
        defs.len(),
        2,
        "{:?}",
        defs.iter().map(|d| d.pattern.clone()).collect::<Vec<_>>()
    );
    assert!(defs.iter().all(|d| !d.stub), "stubs: {:?}", defs);
    assert!(def_matches(&defs[0], r#"Given amounts "1.00" and "2.00""#), "{:?}", defs[0]);
    assert!(def_matches(&defs[1], r#"Then the total is "3.00""#), "{:?}", defs[1]);
}

#[test]
fn cucumber_js_one_liner_with_equals_still_parses() {
    let defs = parse_step_defs(
        "Given('I am signed in as {string}', function (a) { this.user = a; });\n",
    );
    assert_eq!(defs.len(), 1, "{:?}", defs);
    assert_eq!(defs[0].pattern, "I am signed in as {string}");
}

#[test]
fn all_pending_steps_are_all_stubs() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("steps")).unwrap();
    std::fs::write(
        dir.path().join("steps/recipes.steps.js"),
        "Given('x', function () {\n  return 'pending';\n});\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("shalt.toml"),
        "[project]\nstack = \"javascript\"\n[zones]\nsteps = \"steps\"\nsrc = \"src\"\n",
    )
    .unwrap();
    assert!(shalt_core::bindings::steps_all_stubs(dir.path()));
    std::fs::write(
        dir.path().join("steps/recipes.steps.js"),
        "Given('x', function (a) { this.x = a; });\n",
    )
    .unwrap();
    assert!(!shalt_core::bindings::steps_all_stubs(dir.path()));
    assert!(
        !shalt_core::bindings::fill_target_is_bound(dir.path()),
        "stage dir without fill-target.json is unbound — else dumps look finished"
    );
    assert!(
        shalt_core::bindings::fill_target_stub_count(dir.path()) >= 1,
        "missing target must not report 0 stubs (that reverted every write)"
    );
}

#[test]
fn fill_target_stays_unbound_until_the_scenario_is_real() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("spec")).unwrap();
    std::fs::create_dir_all(dir.path().join("steps")).unwrap();
    std::fs::write(
        dir.path().join("shalt.toml"),
        "[project]\nstack = \"javascript\"\n[zones]\nsteps = \"steps\"\nsrc = \"src\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("spec/recipes.feature"),
        "@epic:recipes\nFeature: Recipes\n  Scenario: Create a recipe\n    When I create a recipe titled \"Pasta\"\n    Then the recipe \"Pasta\" has 3 ingredients\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("steps/recipes.steps.js"),
        "When('I create a recipe titled {string}', function (a) {\n  return 'pending';\n});\nThen('the recipe {string} has 3 ingredients', function (a) {\n  return 'pending';\n});\n",
    )
    .unwrap();
    shalt_core::bindings::save_fill_target(
        dir.path(),
        &shalt_core::bindings::FillTarget {
            journey: "recipes".into(),
            rid: String::new(),
            name: "Create a recipe".into(),
        },
    );
    assert!(!shalt_core::bindings::fill_target_is_bound(dir.path()));
    std::fs::write(
        dir.path().join("steps/recipes.steps.js"),
        "When('I create a recipe titled {string}', function (title) { this.t = title; });\nThen('the recipe {string} has 3 ingredients', function (t) { assert.equal(t, this.t); });\n",
    )
    .unwrap();
    assert!(shalt_core::bindings::fill_target_is_bound(dir.path()));
}

#[test]
fn fill_target_stub_count_drops_when_a_body_is_real() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("spec")).unwrap();
    std::fs::create_dir_all(dir.path().join("steps")).unwrap();
    std::fs::write(
        dir.path().join("shalt.toml"),
        "[project]\nstack = \"javascript\"\n[zones]\nsteps = \"steps\"\nsrc = \"src\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("spec/recipes.feature"),
        "@epic:recipes\nFeature: Recipes\n  Scenario: Create a recipe\n    When I create a recipe titled \"Pasta\"\n    Then the recipe \"Pasta\" has 3 ingredients\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("steps/recipes.steps.js"),
        "When('I create a recipe titled {string}', function (a) {\n  return 'pending';\n});\nThen('the recipe {string} has 3 ingredients', function (a) {\n  return 'pending';\n});\n",
    )
    .unwrap();
    shalt_core::bindings::save_fill_target(
        dir.path(),
        &shalt_core::bindings::FillTarget {
            journey: "recipes".into(),
            rid: String::new(),
            name: "Create a recipe".into(),
        },
    );
    assert_eq!(shalt_core::bindings::fill_target_stub_count(dir.path()), 2);
    std::fs::write(
        dir.path().join("steps/recipes.steps.js"),
        "When('I create a recipe titled {string}', function (title) { this.t = title; });\nThen('the recipe {string} has 3 ingredients', function (a) {\n  return 'pending';\n});\n",
    )
    .unwrap();
    assert_eq!(shalt_core::bindings::fill_target_stub_count(dir.path()), 1);
    assert!(shalt_core::bindings::is_steps_fill_path("steps/recipes.steps.js"));
    assert!(!shalt_core::bindings::is_steps_fill_path("steps/world.js"));
    assert!(!shalt_core::bindings::is_steps_fill_path("contract/interface.md"));
    assert!(!shalt_core::bindings::is_steps_fill_path("steps/../.shalt/ledger.json"));
}

#[test]
fn cucumber_js_pending_body_is_a_stub() {
    let defs = parse_step_defs(
        r#"
Given('amounts {string} and {string}', function (a, b) {
  return 'pending';
});
"#,
    );
    assert_eq!(defs.len(), 1);
    assert!(defs[0].stub);
}

#[test]
fn rust_harness_binds_money_steps() {
    let defs = parse_step_defs(
        r#"
use cucumber::{given, then, World};
#[derive(Debug, Default, World)]
pub struct W {}
#[given("amounts {string} and {string}")]
fn amounts(_w: &mut W, _a: String, _b: String) { let _ = 1; }
#[then("the total is {string}")]
fn total(_w: &mut W, _t: String) { assert!(!_t.is_empty()); }
#[then("the difference is {string}")]
fn diff(_w: &mut W, _t: String) { assert!(!_t.is_empty()); }
fn main() {}
"#,
    );
    assert_eq!(defs.len(), 3, "{:?}", defs.iter().map(|d| d.pattern.clone()).collect::<Vec<_>>());
    assert!(defs.iter().all(|d| !d.stub), "stubs: {:?}", defs);
    assert!(def_matches(&defs[0], r#"Given amounts "1.00" and "2.00""#), "{:?}", defs[0]);
}

#[test]
fn cucumber_string_binds_a_quoted_step() {
    let defs = parse_step_defs(
        r#"
#[when("the buyer opens the desk")]
fn open(_w: &mut W) { world.ok(); }

#[then("the desk lists {int} travelers")]
fn lists(_w: &mut W, n: i32) { assert!(n > 0); }
"#,
    );
    assert_eq!(defs.len(), 2);
    assert!(!defs[0].stub && !defs[1].stub);
    assert!(def_matches(&defs[0], "When the buyer opens the desk"));
    assert!(def_matches(&defs[1], "Then the desk lists 4 travelers"));
}

#[test]
fn todo_body_is_a_stub_and_does_not_bind() {
    let defs = parse_step_defs(
        r#"
#[when("the buyer opens the desk")]
fn open(_w: &mut W) { todo!() }
"#,
    );
    assert_eq!(defs.len(), 1);
    assert!(defs[0].stub);
    assert!(!def_matches(&defs[0], "When the buyer opens the desk"));
}

#[test]
fn placeholder_main_is_not_a_test() {
    let defs = parse_step_defs("// Placeholder\nfn main() {}\n");
    assert!(defs.is_empty());
}

#[test]
fn step_text_strips_keyword() {
    assert_eq!(step_text("When the buyer opens the desk"), "the buyer opens the desk");
}

#[test]
fn scenario_bound_requires_every_step() {
    let f = feature(DESK);
    let sc = &f.scenarios[0];
    let defs = parse_step_defs(
        r#"
#[when("the buyer opens the desk")]
fn open(_w: &mut W) { let _ = 1; }

#[then("the desk lists {int} travelers")]
fn lists(_w: &mut W, n: i32) { assert_eq!(n, 4); }
"#,
    );
    assert!(scenario_is_bound(&f, sc, &defs));
    let one = parse_step_defs(
        r#"
#[when("the buyer opens the desk")]
fn open(_w: &mut W) { let _ = 1; }
"#,
    );
    assert!(!scenario_is_bound(&f, sc, &one));
}

#[test]
fn two_defs_for_the_same_phrase_do_not_bind() {
    let f = feature(DESK);
    let sc = &f.scenarios[0];
    let defs = parse_step_defs(
        r#"
#[when("the buyer opens the desk")]
fn open(_w: &mut W) { let _ = 1; }
#[then("the desk lists {int} travelers")]
fn lists(_w: &mut W, n: i32) { assert_eq!(n, 4); }
#[when("the buyer opens the desk")]
fn open_again(_w: &mut W) { let _ = 2; }
#[then("the desk lists {int} travelers")]
fn lists_again(_w: &mut W, n: i32) { assert_eq!(n, 4); }
"#,
    );
    assert_eq!(defs.len(), 4);
    assert!(!scenario_is_bound(&f, sc, &defs));
    assert_eq!(bound_count(&[f.clone()], &defs, "desk"), 0);
}

#[test]
fn pick_completes_one_journey_before_the_next() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("spec")).unwrap();
    std::fs::write(dir.path().join("spec/desk.feature"), DESK).unwrap();
    std::fs::write(dir.path().join("spec/traveler.feature"), TRAVELER).unwrap();
    let features = load_specs(&dir.path().join("spec"), false).unwrap();
    let none = vec![];
    assert_eq!(
        pick_steps_journey(&features, &none, "").as_deref(),
        Some("desk")
    );
    assert_eq!(
        pick_steps_journey(&features, &none, "traveler").as_deref(),
        Some("traveler")
    );
    let desk_defs = parse_step_defs(
        r#"
#[when("the buyer opens the desk")]
fn open(_w: &mut W) { let _ = 1; }
#[then("the desk lists {int} travelers")]
fn lists(_w: &mut W, n: i32) { assert_eq!(n, 4); }
"#,
    );
    assert_eq!(bound_count(&features, &desk_defs, "desk"), 1);
    assert_eq!(
        pick_steps_journey(&features, &desk_defs, "").as_deref(),
        Some("traveler")
    );
    assert!(pick_steps_journey(&features, &desk_defs, "desk").is_none());
}
