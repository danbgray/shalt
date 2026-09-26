//! Live draft: Gherkin on disk/in-job becomes browsable entities, not a markdown dump.

use shalt_core::draft::Draft;
use shalt_core::jobs::{JobKind, JobQueue, JobStatus};
use tempfile::TempDir;

const FEATURE: &str = r#"
@epic:billing
Feature: Invoice totals

  As a billing clerk
  I want invoice totals computed exactly
  So that customers are never billed the wrong amount

  Scenario: An invoice with a single line item
    Given an invoice with a single line item
    When I compute the invoice total
    Then the total is "10.00"

  @holdout
  Scenario: An invoice the implementer was never shown
    Given an invoice with several line items
    When I compute the invoice total
    Then the total is "21.50"
"#;

#[test]
fn draft_turns_gherkin_into_stories_tasks_and_actors() {
    let mut d = Draft::default();
    d.put("spec/invoice.feature", FEATURE);
    let v = d.view();
    assert_eq!(v.stories.len(), 1, "{:?}", v.stories);
    assert_eq!(v.stories[0].name, "Invoice totals");
    assert_eq!(v.stories[0].actor, "billing clerk");
    assert_eq!(v.stories[0].capability, "invoice totals computed exactly");
    assert_eq!(v.epics.len(), 1);
    assert_eq!(v.epics[0].name, "billing");
    assert_eq!(v.tasks.len(), 2);
    assert_eq!(v.tasks[0].name, "An invoice with a single line item");
    assert_eq!(v.tasks[0].steps[0].keyword, "Given");
    assert!(v.tasks[0].steps[0].text.contains("single line item"));
    assert!(v.tasks[1].holdout);
    assert_eq!(v.actors.len(), 1);
    assert_eq!(v.actors[0].name, "billing clerk");
    assert_eq!(v.files.len(), 1);
    assert_eq!(v.files[0].path, "spec/invoice.feature");
    assert!(v.files[0].body.contains("Feature: Invoice totals"));
}

#[test]
fn put_spec_file_stays_under_spec() {
    let t = TempDir::new().unwrap();
    let spec = t.path().join("spec");
    std::fs::create_dir_all(&spec).unwrap();
    assert!(shalt_core::put_spec_file(&spec, "../escape.feature", "Feature: x\n").is_err());
    assert!(shalt_core::put_spec_file(&spec, "notes.md", "hi").is_err());
    let rel = shalt_core::put_spec_file(&spec, "spec/invoice.feature", FEATURE).unwrap();
    assert_eq!(rel, "invoice.feature");
    assert!(spec.join("invoice.feature").exists());
}

#[test]
fn spec_talk_roundtrip() {
    let t = TempDir::new().unwrap();
    let mut talk = shalt_core::SpecTalk::default();
    talk.append("user", "add a holdout");
    talk.save(t.path()).unwrap();
    let loaded = shalt_core::SpecTalk::load(t.path());
    assert_eq!(loaded.messages.len(), 1);
    assert_eq!(loaded.messages[0].content, "add a holdout");
}

#[test]
fn untagged_features_are_not_dumped_under_unassigned() {
    let mut d = Draft::default();
    d.put(
        "spec/boms.feature",
        "Feature: Bills of materials\n\n  Scenario: Create a BOM\n    Given a product\n",
    );
    let v = d.view();
    assert!(v.epics.is_empty(), "{:?}", v.epics);
    assert_eq!(v.stories.len(), 1);
    assert_eq!(v.stories[0].epic, "");
    assert_eq!(v.stories[0].name, "Bills of materials");
}

#[test]
fn actor_is_inferred_from_gherkin_when_story_triad_is_missing() {
    let mut d = Draft::default();
    d.put(
        "spec/compose.feature",
        r#"Feature: Compose a traveler
  A buyer seals a quoteable traveler.

  Scenario: Seal it
    Given the role is "buyer"
    When the buyer opens "/new"
    Then it is L0
"#,
    );
    let v = d.view();
    assert_eq!(v.stories[0].actor.to_lowercase(), "buyer");
    assert_eq!(v.actors.len(), 1);
    assert_eq!(v.actors[0].name.to_lowercase(), "buyer");
}

#[test]
fn draft_on_a_job_survives_save() {
    let t = TempDir::new().unwrap();
    let path = t.path().join("jobs.json");
    let mut q = JobQueue::default();
    let job = q.enqueue(JobKind::Author, "invoice");
    let id = job.id.clone();
    q.put_file(&id, "spec/invoice.feature", FEATURE);
    q.save_to(&path).unwrap();
    let q2 = JobQueue::load_from(&path);
    let v = q2.jobs[0].draft.view();
    assert_eq!(v.tasks.len(), 2);
}

#[test]
fn project_view_prefers_stamped_disk_over_stale_job_draft() {
    let t = TempDir::new().unwrap();
    let spec = t.path().join("spec");
    std::fs::create_dir_all(&spec).unwrap();
    std::fs::write(spec.join("invoice.feature"), FEATURE).unwrap();
    shalt_core::stamp_rids(&spec).unwrap();
    let stamped = std::fs::read_to_string(spec.join("invoice.feature")).unwrap();
    assert!(stamped.contains("@rid:"), "{stamped}");

    let mut q = JobQueue::default();
    let job = q.enqueue(JobKind::Author, "p");
    q.put_file(&job.id, "spec/invoice.feature", FEATURE);
    q.set_status(&job.id, JobStatus::Done);

    let v = Draft::for_project(&spec, &q.jobs, "p").view();
    assert!(
        v.tasks.iter().all(|t| t.rid.is_some()),
        "stamped disk must win over a finished job's unstamped snapshot: {:?}",
        v.tasks.iter().map(|t| (&t.name, &t.rid)).collect::<Vec<_>>()
    );
}

#[test]
fn abandon_for_restart_drops_paused_work() {
    let mut q = JobQueue::default();
    let id = q.enqueue(JobKind::Build, "invoice").id.clone();
    q.set_status(&id, JobStatus::Paused);
    let gone = q.abandon_for_restart("invoice");
    assert_eq!(gone, vec![id.clone()]);
    assert_eq!(q.jobs[0].status, JobStatus::Done);
    assert!(q.resumable_for_project("invoice").is_empty());
}

#[test]
fn abandon_for_restart_keeps_an_unfinished_author() {
    let mut q = JobQueue::default();
    let id = q.enqueue(JobKind::Author, "invoice").id.clone();
    q.set_status(&id, JobStatus::Interrupted);
    assert!(q.abandon_for_restart("invoice").is_empty());
    assert_eq!(q.jobs[0].status, JobStatus::Interrupted);
    assert_eq!(q.resumable_for_project("invoice"), vec![id]);
}

#[test]
fn finished_author_is_not_reopened_just_because_it_was_paused_once() {
    let mut q = JobQueue::default();
    let id = q.enqueue(JobKind::Author, "pack").id.clone();
    q.set_status(&id, JobStatus::Done);
    q.append(&id, "parked while waiting on the model (stopped (Paused))");
    q.append(&id, "wrote 5: spec/recipes.feature, spec/sharing.feature");
    assert!(!q.authoring_open("pack"), "a finished spec write must not reopen");
    assert!(q.reopen_cut_short_author("pack").is_none());
}

#[test]
fn cut_short_author_reopens_for_play() {
    let mut q = JobQueue::default();
    let id = q.enqueue(JobKind::Author, "pack").id.clone();
    q.set_status(&id, JobStatus::Done);
    q.append(&id, "abandoned — Play restarts from the current spec and stack");
    assert!(q.authoring_open("pack"));
    assert_eq!(q.reopen_cut_short_author("pack").as_deref(), Some(id.as_str()));
    assert_eq!(q.get(&id).unwrap().status, JobStatus::Interrupted);
}

#[test]
fn failed_author_is_not_auto_resumed() {
    let mut q = JobQueue::default();
    let id = q.enqueue(JobKind::Author, "pack").id.clone();
    q.set_status(&id, JobStatus::Failed);
    assert!(
        q.resumable_for_project("pack").is_empty(),
        "Failed is terminal; Play hops via continue_project, got {:?}",
        q.resumable_for_project("pack")
    );
    assert!(!q.authoring_open("pack"));
    assert!(
        q.resume_target("pack").is_none(),
        "a failed author must not own the Play resume slot"
    );
    let _ = id;
}

#[test]
fn failed_job_can_be_set_running_again() {
    let mut q = JobQueue::default();
    let id = q.enqueue(JobKind::Author, "invoice").id.clone();
    q.set_status(&id, JobStatus::Failed);
    q.set_status(&id, JobStatus::Running);
    assert_eq!(q.jobs[0].status, JobStatus::Running);
}

#[test]
fn draft_restore_writes_feature_files_back() {
    let t = TempDir::new().unwrap();
    let mut d = Draft::default();
    d.put("spec/invoice.feature", "Feature: X\n  Scenario: Y\n    Given a\n");
    d.restore_to(t.path()).unwrap();
    let body = std::fs::read_to_string(t.path().join("spec/invoice.feature")).unwrap();
    assert!(body.contains("Scenario: Y"));
}

#[test]
fn turns_from_log_pairs_questions_and_answers() {
    let log = r#"
step 1: waiting on the model…
? Which modules?
? Which modules?
  you: Production Planning
  you: Production Planning
[ask_human] Production Planning
? What workflows?
  you: BOMs and work orders
"#;
    let t = shalt_core::jobs::turns_from_log(log);
    assert_eq!(t.len(), 2, "{t:?}");
    assert_eq!(t[0].answer, "Production Planning");
    assert_eq!(t[1].answer, "BOMs and work orders");
}

#[test]
fn author_prompt_includes_decided_answers() {
    let turns = vec![shalt_core::jobs::TalkTurn {
        question: "Which modules?".into(),
        answer: "Production Planning".into(),
        ..Default::default()
    }];
    let p = shalt_core::author_prompt("build an ERP", &turns);
    assert!(p.contains("Production Planning"));
    assert!(p.contains("Do not ask these again"));
}

#[test]
fn author_prompt_embeds_spec_and_forbids_listing() {
    let p = shalt_core::author_prompt_with_spec(
        "share recipes",
        &[],
        false,
        "--- spec/recipes.feature ---\nFeature: Recipes\n  Scenario: Publish a link\n    Then it works\n",
    );
    assert!(p.contains("CURRENT SPEC"));
    assert!(p.contains("spec/recipes.feature"));
    assert!(p.contains("Do not call list_files"));
    assert!(
        p.contains("Review") && p.contains("Override"),
        "seeded spec must be reviewed, not rubber-stamped: {p}"
    );
    assert!(p.contains("done()"), "{p}");
}

#[test]
fn author_prompt_markdown_spec_must_be_rewritten() {
    let p = shalt_core::author_prompt_with_spec(
        "share recipes",
        &[],
        false,
        "--- spec/recipe.feature ---\n# Feature: Recipe Sharing\n- Authors can write a recipe\n",
    );
    assert!(
        p.contains("not valid") || p.contains("no Scenario"),
        "{p}"
    );
    assert!(p.contains("Markdown bullets are not a spec"));
    assert!(
        !p.contains("done() immediately"),
        "markdown must not count as a finished spec: {p}"
    );
}

#[test]
fn xai_key_loads_from_shalt_config_when_env_missing() {
    let t = tempfile::TempDir::new().unwrap();
    let prev_home = std::env::var("SHALT_HOME").ok();
    let prev_xai = std::env::var("XAI_API_KEY").ok();
    let prev_grok = std::env::var("GROK_API_KEY").ok();
    std::env::set_var("SHALT_HOME", t.path());
    std::env::remove_var("XAI_API_KEY");
    std::env::remove_var("GROK_API_KEY");
    std::fs::write(
        t.path().join("config.toml"),
        "[keys]\nxai = \"xai-test-from-file\"\n",
    )
    .unwrap();
    let got = shalt_core::xai_api_key();
    match prev_home {
        Some(v) => std::env::set_var("SHALT_HOME", v),
        None => std::env::remove_var("SHALT_HOME"),
    }
    match prev_xai {
        Some(v) => std::env::set_var("XAI_API_KEY", v),
        None => std::env::remove_var("XAI_API_KEY"),
    }
    match prev_grok {
        Some(v) => std::env::set_var("GROK_API_KEY", v),
        None => std::env::remove_var("GROK_API_KEY"),
    }
    assert_eq!(got.as_deref(), Some("xai-test-from-file"));
}

#[test]
fn human_error_explains_xai_spend_limit() {
    let raw = r#"grok API error HTTP 403: {"code":"permission-denied","error":"Your team has used all available credits"}"#;
    assert!(shalt_core::jobs::human_error(raw).contains("credits"));
}

#[test]
fn chat_on_job_rejects_empty_and_unknown() {
    assert!(shalt_core::chat_on_job("J-missing", "hello").is_err());
    assert!(shalt_core::chat_on_job("J-missing", "   ").is_err());
}

#[test]
fn ask_stores_a_guess_on_the_open_turn() {
    let mut q = JobQueue::default();
    let id = q.enqueue(JobKind::Author, "invoice").id.clone();
    q.ask(&id, "What does a typical BOM look like?", "widget x2, gasket x1");
    assert_eq!(q.jobs[0].turns[0].guess, "widget x2, gasket x1");
}

#[test]
fn append_does_not_repeat_the_same_line_twice_in_a_row() {
    let mut q = JobQueue::default();
    let id = q.enqueue(JobKind::Author, "invoice").id.clone();
    q.append(&id, "you: BOM levels");
    q.append(&id, "you: BOM levels");
    let log = &q.jobs[0].log;
    assert_eq!(log.matches("you: BOM levels").count(), 1, "{log}");
}
