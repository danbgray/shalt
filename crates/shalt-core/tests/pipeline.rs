//! Play continues spec → tests → code. Stage selection must not skip the test run.

use shalt_core::config::init_workspace;
use shalt_core::jobs::{JobKind, JobQueue, JobStatus};
use shalt_core::org::Org;
use shalt_core::pipeline::{continue_project, next_stage, play_chains_after, Stage};
use std::fs;
use std::sync::Mutex;
use tempfile::TempDir;

static HOME: Mutex<()> = Mutex::new(());

const FEATURE: &str = r#"Feature: Money

  Scenario: Add two amounts
    Given amounts "1.00" and "2.00"
    Then the total is "3.00"
"#;

const REAL_STEPS: &str = r#"
from pytest_bdd import given, then

@given('amounts {string} and {string}')
def amounts(a, b):
    return (a, b)

@then('the total is {string}')
def total(total):
    assert total == "3.00"
"#;

const RUST_STEPS: &str = r#"
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
"#;

fn write_feature_only(root: &std::path::Path) {
    fs::create_dir_all(root.join("spec")).unwrap();
    fs::write(root.join("spec/money.feature"), FEATURE).unwrap();
}

fn write_feature(root: &std::path::Path) {
    write_feature_only(root);
    skip_design(root);
}

fn skip_design(root: &std::path::Path) {
    let features = shalt_core::load_specs(&root.join("spec"), false).unwrap();
    let wall = shalt_core::films(root, &features, &shalt_core::Ledger::default());
    for film in wall {
        let dir = root.join("mockups/journeys").join(&film.journey);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("storyboard.json"),
            format!(
                r#"{{"journey":"{}","kind":"none","spec_hash":"{}","frames":[]}}"#,
                film.journey, film.spec_hash
            ),
        )
        .unwrap();
    }
}

#[test]
fn empty_workspace_is_still_author() {
    let t = TempDir::new().unwrap();
    init_workspace(t.path(), "python", "empty").unwrap();
    assert_eq!(next_stage(t.path()), Stage::Author);
}

#[test]
fn spec_without_tests_is_steps() {
    let t = TempDir::new().unwrap();
    init_workspace(t.path(), "python", "money").unwrap();
    write_feature(t.path());
    assert_eq!(next_stage(t.path()), Stage::Steps);
}

#[test]
fn pending_js_stubs_keep_the_tests_gate() {
    let t = TempDir::new().unwrap();
    init_workspace(t.path(), "javascript", "money").unwrap();
    write_feature(t.path());
    shalt_core::apply_step_stubs(t.path(), "money").unwrap();
    assert!(t.path().join("steps/money.steps.js").is_file());
    assert_eq!(next_stage(t.path()), Stage::Steps);
    assert_eq!(shalt_core::work_gate(t.path()), "tests");
}

#[test]
fn focused_stepwright_stage_is_one_journey() {
    let t = TempDir::new().unwrap();
    init_workspace(t.path(), "javascript", "cook").unwrap();
    write_feature(t.path());
    std::fs::write(
        t.path().join("spec/other.feature"),
        "@epic:other\nFeature: Other\n  Scenario: X\n    When y\n    Then z\n",
    )
    .unwrap();
    shalt_core::apply_step_stubs(t.path(), "money").unwrap();
    std::fs::write(
        t.path().join("steps/other.steps.js"),
        "Given('y', function () { return 1; });\n",
    )
    .unwrap();
    let rels = shalt_core::focused_stepwright_rels(t.path(), "money");
    assert!(
        rels.iter().any(|p| p.ends_with("money.steps.js")),
        "{rels:?}"
    );
    assert!(
        !rels.iter().any(|p| p.contains("other.steps.js")),
        "{rels:?}"
    );
    assert!(
        !rels.iter().any(|p| p.contains("other.feature")),
        "{rels:?}"
    );
}

#[test]
fn spec_without_storyboards_is_design() {
    let t = TempDir::new().unwrap();
    init_workspace(t.path(), "python", "money").unwrap();
    write_feature_only(t.path());
    assert_eq!(next_stage(t.path()), Stage::Design);
    assert_eq!(shalt_core::work_gate(t.path()), "design");
}

#[test]
fn storyboard_json_without_html_is_still_design() {
    let t = TempDir::new().unwrap();
    init_workspace(t.path(), "python", "money").unwrap();
    write_feature_only(t.path());
    let features = shalt_core::load_specs(&t.path().join("spec"), false).unwrap();
    let wall = shalt_core::films(t.path(), &features, &shalt_core::Ledger::default());
    let film = &wall[0];
    let dir = t.path().join("mockups/journeys").join(&film.journey);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("storyboard.json"),
        format!(
            r#"{{"journey":"{}","kind":"ui","spec_hash":"{}","frames":[]}}"#,
            film.journey, film.spec_hash
        ),
    )
    .unwrap();
    assert_eq!(next_stage(t.path()), Stage::Design);
}

#[test]
fn spec_without_a_language_waits_for_the_dropdown() {
    let t = TempDir::new().unwrap();
    shalt_core::config::init_plan_workspace(t.path(), "money").unwrap();
    write_feature(t.path());
    assert_eq!(next_stage(t.path()), Stage::Language);
    assert_eq!(shalt_core::work_gate(t.path()), "language");
}

#[test]
fn spec_with_tests_and_pending_is_a_survey_run() {
    let t = TempDir::new().unwrap();
    init_workspace(t.path(), "python", "money").unwrap();
    write_feature(t.path());
    fs::write(t.path().join("steps/test_money.py"), REAL_STEPS).unwrap();
    assert_eq!(next_stage(t.path()), Stage::Run);
}

#[test]
fn a_placeholder_file_is_still_steps() {
    let t = TempDir::new().unwrap();
    init_workspace(t.path(), "python", "money").unwrap();
    write_feature(t.path());
    fs::write(
        t.path().join("steps/test_money.py"),
        "def test_placeholder():\n    assert True\n",
    )
    .unwrap();
    assert_eq!(next_stage(t.path()), Stage::Steps);
}

#[test]
fn empty_survey_does_not_queue_another_run() {
    let _g = HOME.lock().unwrap();
    let home = TempDir::new().unwrap();
    std::env::set_var("SHALT_HOME", home.path());
    let proj = home.path().join("envelope");
    init_workspace(&proj, "rust", "envelope").unwrap();
    write_feature(&proj);
    fs::create_dir_all(proj.join("tests")).unwrap();
    fs::write(
        proj.join("tests/shalt.rs"),
        "// Placeholder until shalt Play writes the cucumber harness (stepwright zone).\nfn main() {}\n",
    )
    .unwrap();
    let mut org = Org {
        name: "local".into(),
        projects: vec![],
    };
    let p = org.add(&proj).unwrap();
    org.save().unwrap();

    let first = continue_project(&p.id).unwrap().expect("tests job");
    assert_eq!(
        first.kind,
        JobKind::Steps,
        "placeholder harness is not tests — write real steps first, got {:?}",
        first.kind
    );
    let mut q = JobQueue::load();
    q.set_status(&first.id, JobStatus::Pending);
    q.save().unwrap();
    let again = continue_project(&p.id).unwrap();
    assert!(
        again.is_none()
            || again
                .as_ref()
                .map(|j| j.kind != JobKind::Run)
                .unwrap_or(true),
        "must not skip ahead to a survey while tests are still stubs"
    );
    std::env::remove_var("SHALT_HOME");
}

#[test]
fn after_a_survey_play_builds() {
    let t = TempDir::new().unwrap();
    init_workspace(t.path(), "python", "money").unwrap();
    write_feature(t.path());
    fs::write(t.path().join("steps/test_money.py"), REAL_STEPS).unwrap();
    shalt_core::spec::stamp_rids(&t.path().join("spec")).unwrap();
    let features = shalt_core::load_specs(&t.path().join("spec"), false).unwrap();
    let mut led = shalt_core::ledger::Ledger::default();
    led.sync_spec(&features);
    for e in led.entries.values_mut() {
        e.last_run_at = Some("2026-09-15T00:00:00Z".into());
        e.status = "red".into();
        e.failure = Some("asserted 0 == 3".into());
    }
    led.save(&t.path().join(".shalt/ledger.json")).unwrap();
    assert_eq!(next_stage(t.path()), Stage::Build);
}

#[test]
fn spec_shaped_failures_send_play_back_to_the_gherkin() {
    let _g = HOME.lock().unwrap();
    let home = TempDir::new().unwrap();
    std::env::set_var("SHALT_HOME", home.path());
    let proj = home.path().join("money");
    init_workspace(&proj, "python", "money").unwrap();
    write_feature(&proj);
    fs::write(proj.join("steps/test_money.py"), REAL_STEPS).unwrap();
    let mut org = Org {
        name: "local".into(),
        projects: vec![],
    };
    let p = org.add(&proj).unwrap();
    org.save().unwrap();
    shalt_core::spec::stamp_rids(&proj.join("spec")).unwrap();
    let features = shalt_core::load_specs(&proj.join("spec"), false).unwrap();
    let mut led = shalt_core::ledger::Ledger::default();
    led.sync_spec(&features);
    for e in led.entries.values_mut() {
        e.last_run_at = Some("2026-09-15T00:00:00Z".into());
        e.status = "red".into();
        e.failure = Some("this is underspecified / ambiguous".into());
    }
    led.save(&proj.join(".shalt/ledger.json")).unwrap();
    let job = continue_project(&p.id).unwrap().expect("refine spec");
    assert_eq!(job.kind, JobKind::Author);
    std::env::remove_var("SHALT_HOME");
}

#[test]
fn empty_spec_play_queues_author() {
    let _g = HOME.lock().unwrap();
    let home = TempDir::new().unwrap();
    std::env::set_var("SHALT_HOME", home.path());
    let proj = home.path().join("existing");
    init_workspace(&proj, "rust", "existing").unwrap();
    let mut org = Org {
        name: "local".into(),
        projects: vec![],
    };
    let p = org.add(&proj).unwrap();
    org.save().unwrap();
    assert_eq!(next_stage(&proj), Stage::Author);
    let job = continue_project(&p.id).unwrap().expect("author job");
    assert_eq!(job.kind, JobKind::Author);
    std::env::remove_var("SHALT_HOME");
}

#[test]
fn parked_author_blocks_writing_tests() {
    let _g = HOME.lock().unwrap();
    let home = TempDir::new().unwrap();
    std::env::set_var("SHALT_HOME", home.path());
    let proj = home.path().join("onboard");
    init_workspace(&proj, "rust", "onboard").unwrap();
    write_feature(&proj);
    let mut org = Org {
        name: "local".into(),
        projects: vec![],
    };
    let p = org.add(&proj).unwrap();
    org.save().unwrap();
    let mut q = JobQueue::load();
    let author = q.enqueue(JobKind::Author, &p.id);
    q.set_status(&author.id, JobStatus::Interrupted);
    q.save().unwrap();
    assert_eq!(next_stage(&proj), Stage::Steps);
    assert!(
        continue_project(&p.id).unwrap().is_none(),
        "must resume the spec, not skip to tests"
    );
    std::env::remove_var("SHALT_HOME");
}

#[test]
fn play_queues_steps_then_does_not_double_queue() {
    let _g = HOME.lock().unwrap();
    let home = TempDir::new().unwrap();
    std::env::set_var("SHALT_HOME", home.path());
    let proj = home.path().join("money");
    init_workspace(&proj, "python", "money").unwrap();
    write_feature(&proj);
    let mut org = Org {
        name: "local".into(),
        projects: vec![],
    };
    let p = org.add(&proj).unwrap();
    org.save().unwrap();

    let job = continue_project(&p.id).unwrap().expect("steps job");
    assert_eq!(job.kind, JobKind::Steps);
    assert_eq!(job.status, JobStatus::Pending);

    assert!(
        continue_project(&p.id).unwrap().is_none(),
        "a live steps job must block a second play"
    );

    org.set_paused(&p.id, true);
    org.save().unwrap();
    let mut q = JobQueue::load();
    q.set_status(&job.id, JobStatus::Done);
    q.save().unwrap();
    assert!(
        continue_project(&p.id).unwrap().is_none(),
        "paused project must not start the code loop"
    );

    std::env::remove_var("SHALT_HOME");
}

#[test]
fn after_tests_exist_play_queues_a_build_job_per_ticket() {
    let _g = HOME.lock().unwrap();
    let home = TempDir::new().unwrap();
    std::env::set_var("SHALT_HOME", home.path());
    let proj = home.path().join("money");
    init_workspace(&proj, "rust", "money").unwrap();
    fs::create_dir_all(proj.join("spec")).unwrap();
    fs::write(
        proj.join("spec/add.feature"),
        r#"Feature: Add

  Scenario: Add two amounts
    Given amounts "1.00" and "2.00"
    Then the total is "3.00"
"#,
    )
    .unwrap();
    fs::write(
        proj.join("spec/sub.feature"),
        r#"Feature: Subtract

  Scenario: Subtract two amounts
    Given amounts "5.00" and "2.00"
    Then the difference is "3.00"
"#,
    )
    .unwrap();
    fs::create_dir_all(proj.join("tests")).unwrap();
    fs::write(proj.join("tests/shalt.rs"), RUST_STEPS).unwrap();
    skip_design(&proj);

    let mut org = Org {
        name: "local".into(),
        projects: vec![],
    };
    let p = org.add(&proj).unwrap();
    org.save().unwrap();

    let survey = continue_project(&p.id).unwrap().expect("survey run");
    assert_eq!(survey.kind, JobKind::Run, "after tests, run the whole suite first");
    let mut q = JobQueue::load();
    q.set_status(&survey.id, JobStatus::Done);
    q.save().unwrap();
    shalt_core::spec::stamp_rids(&proj.join("spec")).unwrap();
    let features = shalt_core::load_specs(&proj.join("spec"), false).unwrap();
    let mut led = shalt_core::ledger::Ledger::load(&proj.join(".shalt/ledger.json")).unwrap_or_default();
    led.sync_spec(&features);
    let mut n = 0;
    for e in led.entries.values_mut() {
        n += 1;
        e.last_run_at = Some("2026-09-15T00:00:00Z".into());
        e.status = "red".into();
        e.failure = Some(format!("asserted {n} == 99 on {}", e.name));
    }
    led.save(&proj.join(".shalt/ledger.json")).unwrap();

    let first = continue_project(&p.id).unwrap().expect("build job");
    assert_eq!(first.kind, JobKind::Build, "after the survey, build each ticket");
    assert!(!first.rid.is_empty(), "build aims at a ticket, got {:?}", first.rid);

    let mut q = JobQueue::load();
    q.set_agent(&first.id, "grok", "grok-4");
    q.set_status(&first.id, JobStatus::Running);
    q.save().unwrap();
    let board_path = proj.join(".shalt/board.json");
    let mut board = shalt_core::Board::load(&board_path);
    for it in board.items.clone() {
        board.set_item_agent(&it.rid, "grok", "grok-4");
    }
    board.save(&board_path).unwrap();

    let second = continue_project(&p.id).unwrap().expect("next ticket");
    assert_eq!(second.kind, JobKind::Build);
    assert_ne!(second.rid, first.rid, "next ticket, not a second tests job");

    std::env::remove_var("SHALT_HOME");
}

#[test]
fn failed_build_still_chains_play() {
    let _g = HOME.lock().unwrap();
    let home = TempDir::new().unwrap();
    std::env::set_var("SHALT_HOME", home.path());
    let proj = home.path().join("money");
    init_workspace(&proj, "rust", "money").unwrap();
    write_feature(&proj);
    fs::create_dir_all(proj.join("tests")).unwrap();
    fs::write(proj.join("tests/shalt.rs"), RUST_STEPS).unwrap();

    let mut org = Org {
        name: "local".into(),
        projects: vec![],
    };
    let p = org.add(&proj).unwrap();
    org.save().unwrap();

    let survey = continue_project(&p.id).unwrap().expect("survey run");
    let mut q = JobQueue::load();
    q.set_status(&survey.id, JobStatus::Done);
    q.save().unwrap();
    shalt_core::spec::stamp_rids(&proj.join("spec")).unwrap();
    let features = shalt_core::load_specs(&proj.join("spec"), false).unwrap();
    let mut led = shalt_core::ledger::Ledger::load(&proj.join(".shalt/ledger.json")).unwrap_or_default();
    led.sync_spec(&features);
    for e in led.entries.values_mut() {
        e.last_run_at = Some("2026-09-15T00:00:00Z".into());
        e.status = "red".into();
        e.failure = Some("tests did not compile".into());
    }
    led.save(&proj.join(".shalt/ledger.json")).unwrap();

    let first = continue_project(&p.id).unwrap().expect("build job");
    let mut q = JobQueue::load();
    q.set_status(&first.id, JobStatus::Failed);
    q.save().unwrap();
    let failed = q.get(&first.id).cloned().unwrap();
    assert!(
        play_chains_after(&failed),
        "a timed-out build must not idle Play while tickets remain red"
    );
    let mut credit = failed.clone();
    credit.log.push_str("\nfailed: xAI refused the request: this team is out of credits or hit its spending limit.\n");
    if shalt_core::api::ollama_reachable() {
        assert!(
            play_chains_after(&credit),
            "Grok out of credits + local Ollama → keep Play, next job skips Grok"
        );
    } else {
        assert!(
            !play_chains_after(&credit),
            "out of credits with no local model must not keep enqueueing"
        );
    }
    let again = continue_project(&p.id)
        .unwrap()
        .expect("Play continues after a failed build");
    assert_eq!(again.kind, JobKind::Build);

    std::env::remove_var("SHALT_HOME");
}

#[test]
fn compile_dump_in_step_harness_rewrites_steps_not_src() {
    let _g = HOME.lock().unwrap();
    let home = TempDir::new().unwrap();
    std::env::set_var("SHALT_HOME", home.path());
    let proj = home.path().join("money");
    init_workspace(&proj, "rust", "money").unwrap();
    write_feature(&proj);
    fs::create_dir_all(proj.join("tests")).unwrap();
    fs::write(proj.join("tests/shalt.rs"), RUST_STEPS).unwrap();

    let mut org = Org {
        name: "local".into(),
        projects: vec![],
    };
    let p = org.add(&proj).unwrap();
    org.save().unwrap();

    let survey = continue_project(&p.id).unwrap().expect("survey run");
    let mut q = JobQueue::load();
    q.set_status(&survey.id, JobStatus::Done);
    q.save().unwrap();
    shalt_core::spec::stamp_rids(&proj.join("spec")).unwrap();
    let features = shalt_core::load_specs(&proj.join("spec"), false).unwrap();
    let mut led = shalt_core::ledger::Ledger::load(&proj.join(".shalt/ledger.json")).unwrap_or_default();
    led.sync_spec(&features);
    led.suite_error = Some(
        "error[E0107]: missing generics for type alias `cucumber::Step`\n  --> tests/shalt.rs:25:74\nerror: could not compile `money` (test \"shalt\")".into(),
    );
    for e in led.entries.values_mut() {
        e.last_run_at = Some("2026-09-15T00:00:00Z".into());
        e.status = "red".into();
        e.failure = Some("tests did not compile".into());
    }
    led.save(&proj.join(".shalt/ledger.json")).unwrap();

    let next = continue_project(&p.id).unwrap().expect("steps rewrite");
    assert_eq!(
        next.kind,
        JobKind::Steps,
        "a compile dump in tests/shalt.rs is the stepwright's, got {:?}",
        next.kind
    );

    std::env::remove_var("SHALT_HOME");
}

#[test]
fn javascript_syntax_error_in_steps_rewrites_steps_not_src() {
    let _g = HOME.lock().unwrap();
    let home = TempDir::new().unwrap();
    std::env::set_var("SHALT_HOME", home.path());
    let proj = home.path().join("money");
    init_workspace(&proj, "javascript", "money").unwrap();
    write_feature(&proj);
    fs::create_dir_all(proj.join("steps")).unwrap();
    fs::write(
        proj.join("steps/money.js"),
        "import { Given } from '@cucumber/cucumber';\nGiven('amounts {string} and {string}', function (a, b) { this.a = a; });\n",
    )
    .unwrap();

    let mut org = Org {
        name: "local".into(),
        projects: vec![],
    };
    let p = org.add(&proj).unwrap();
    org.save().unwrap();

    let survey = continue_project(&p.id).unwrap().expect("survey run");
    let mut q = JobQueue::load();
    q.set_status(&survey.id, JobStatus::Done);
    q.save().unwrap();
    shalt_core::spec::stamp_rids(&proj.join("spec")).unwrap();
    let features = shalt_core::load_specs(&proj.join("spec"), false).unwrap();
    let mut led = shalt_core::ledger::Ledger::load(&proj.join(".shalt/ledger.json")).unwrap_or_default();
    led.sync_spec(&features);
    led.suite_error = Some(
        "file:///.../steps/money.js:2\nGiven('amounts {string} and {string}', function (a, b) {\n^\nSyntaxError: Unexpected token".into(),
    );
    for e in led.entries.values_mut() {
        e.last_run_at = Some("2026-09-19T00:00:00Z".into());
        e.status = "red".into();
        e.failure = Some("steps did not load".into());
    }
    led.save(&proj.join(".shalt/ledger.json")).unwrap();

    let next = continue_project(&p.id).unwrap().expect("steps rewrite");
    assert_eq!(
        next.kind,
        JobKind::Steps,
        "a SyntaxError in steps/ is the stepwright's, got {:?}",
        next.kind
    );

    std::env::remove_var("SHALT_HOME");
}

#[test]
fn switch_play_model_keeps_play_on() {
    let _g = HOME.lock().unwrap();
    let home = TempDir::new().unwrap();
    std::env::set_var("SHALT_HOME", home.path());
    let proj = home.path().join("money");
    init_workspace(&proj, "rust", "money").unwrap();
    write_feature(&proj);
    fs::create_dir_all(proj.join(".shalt")).unwrap();
    let mut board = shalt_core::Board::default();
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-1".into(),
        backend: "qwen".into(),
        model: "qwen3.5:35b-128k".into(),
        ..Default::default()
    });
    board.save(&proj.join(".shalt/board.json")).unwrap();

    let mut org = Org {
        name: "local".into(),
        projects: vec![],
    };
    let p = org.add(&proj).unwrap();
    org.save().unwrap();

    let mut q = JobQueue::load();
    let job = q.enqueue_full(
        JobKind::Steps,
        &p.id,
        "",
        "qwen",
        "qwen3.5:35b-128k",
    );
    q.set_status(&job.id, JobStatus::Running);
    q.set_work(&job.id, "Money", "S-1");
    q.save().unwrap();

    let switched = shalt_core::switch_play_model(
        &p.id,
        "qwen",
        "qwen3.8:27b-mlx",
        Some(&job.id),
        Some("S-1"),
    )
    .unwrap();
    assert_eq!(switched.model, "qwen3.8:27b-mlx");
    assert_eq!(switched.status, JobStatus::Running);

    let org = Org::load();
    assert!(
        !org.get(&p.id).unwrap().paused,
        "switching model must not Pause the project"
    );
    let q = JobQueue::load();
    let j = q.get(&job.id).unwrap();
    assert_eq!(j.status, JobStatus::Running);
    assert_eq!(j.backend, "qwen");
    assert_eq!(j.model, "qwen3.8:27b-mlx");
    assert!(
        j.log.contains("Play continues"),
        "{}",
        j.log
    );
    let board = shalt_core::Board::load(&proj.join(".shalt/board.json"));
    let item = board.items.iter().find(|i| i.rid == "S-1").unwrap();
    assert_eq!(item.model, "qwen3.8:27b-mlx");

    std::env::remove_var("SHALT_HOME");
}

#[test]
fn play_on_empty_spec_starts_author_not_the_test_loop() {
    let _g = HOME.lock().unwrap();
    let home = TempDir::new().unwrap();
    std::env::set_var("SHALT_HOME", home.path());
    let proj = home.path().join("empty");
    init_workspace(&proj, "python", "empty").unwrap();
    let p = shalt_core::org::Org::ensure_registered(&proj).unwrap();
    let job = continue_project(&p.id).unwrap().expect("author");
    assert_eq!(job.kind, JobKind::Author);
    std::env::remove_var("SHALT_HOME");
}

#[test]
fn focus_journey_scopes_the_tests_job() {
    let _g = HOME.lock().unwrap();
    let home = TempDir::new().unwrap();
    std::env::set_var("SHALT_HOME", home.path());
    let proj = home.path().join("sje");
    init_workspace(&proj, "rust", "sje").unwrap();
    fs::create_dir_all(proj.join("spec")).unwrap();
    fs::write(proj.join("spec/desk.feature"), "@epic:desk\nFeature: Desk\n  Scenario: Open\n    When the buyer opens the desk\n    Then the desk lists 4 travelers\n").unwrap();
    fs::write(proj.join("spec/traveler.feature"), "@epic:traveler\nFeature: Traveler\n  Scenario: Missing\n    When the user opens \"/p/x\"\n    Then the page shows missing\n").unwrap();
    skip_design(&proj);
    let mut board = shalt_core::Board::load(&proj.join(".shalt/board.json"));
    board.focus_journey = "traveler".into();
    board.save(&proj.join(".shalt/board.json")).unwrap();
    let mut org = Org {
        name: "local".into(),
        projects: vec![],
    };
    let p = org.add(&proj).unwrap();
    org.save().unwrap();
    let job = continue_project(&p.id).unwrap().expect("steps");
    assert_eq!(job.kind, JobKind::Steps);
    assert_eq!(job.epic, "traveler");
    std::env::remove_var("SHALT_HOME");
}
