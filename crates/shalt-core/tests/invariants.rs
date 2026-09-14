//! Ports of the Python invariant tests: identity, ledger, isolation, overlay.

use shalt_core::backends::FnBackend;
use shalt_core::board::Board;
use shalt_core::integrity::{diff_snap, snapshot, GuardedTurn};
use shalt_core::jobs::{JobKind, JobQueue, JobStatus};
use shalt_core::ledger::{Ledger, GREEN, ORPHAN, PENDING, RED, STALE};
use shalt_core::roles::{run_role, RoleError};
use shalt_core::spec::{holdout_rids, load_specs, stamp_rids, strip_holdouts};
use shalt_core::RunResult;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

const FEATURE: &str = r#"Feature: Money

  @billing
  Scenario: Add two amounts
    Given amounts "1.00" and "2.00"
    Then the total is "3.00"

  @holdout
  Scenario: Unseen amounts
    Given amounts "4.00" and "5.00"
    Then the total is "9.00"
"#;

fn write_spec(dir: &Path, text: &str) -> std::path::PathBuf {
    let d = dir.join("spec");
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join("money.feature"), text).unwrap();
    d
}

fn hash_of(d: &Path, name: &str) -> String {
    for f in load_specs(d, true).unwrap() {
        for s in f.scenarios {
            if s.name == name {
                return s.spec_hash(&f.background);
            }
        }
    }
    panic!("{name}");
}

#[test]
fn rid_is_stamped_once_and_is_stable() {
    let t = TempDir::new().unwrap();
    let d = write_spec(t.path(), FEATURE);
    let minted = stamp_rids(&d).unwrap();
    assert_eq!(minted.len(), 2);
    let first: HashMap<_, _> = load_specs(&d, true)
        .unwrap()
        .into_iter()
        .flat_map(|f| f.scenarios)
        .map(|s| (s.name, s.rid))
        .collect();
    stamp_rids(&d).unwrap();
    let second: HashMap<_, _> = load_specs(&d, true)
        .unwrap()
        .into_iter()
        .flat_map(|f| f.scenarios)
        .map(|s| (s.name, s.rid))
        .collect();
    assert_eq!(first, second);
}

#[test]
fn rid_survives_renaming_and_reordering() {
    let t = TempDir::new().unwrap();
    let d = write_spec(t.path(), FEATURE);
    stamp_rids(&d).unwrap();
    let path = d.join("money.feature");
    let before: std::collections::HashSet<_> = load_specs(&d, true)
        .unwrap()
        .into_iter()
        .flat_map(|f| f.scenarios)
        .filter_map(|s| s.rid)
        .collect();
    let text = fs::read_to_string(&path).unwrap().replace("Add two amounts", "Sum two amounts");
    fs::write(&path, text).unwrap();
    let after: std::collections::HashSet<_> = load_specs(&d, true)
        .unwrap()
        .into_iter()
        .flat_map(|f| f.scenarios)
        .filter_map(|s| s.rid)
        .collect();
    assert_eq!(before, after);
}

#[test]
fn cosmetic_edits_do_not_change_the_spec_hash() {
    let t = TempDir::new().unwrap();
    let d = write_spec(t.path(), FEATURE);
    stamp_rids(&d).unwrap();
    let h = hash_of(&d, "Add two amounts");
    let p = d.join("money.feature");
    let text = fs::read_to_string(&p)
        .unwrap()
        .replace(r#"Given amounts "1.00" and "2.00""#, r#"  Given amounts "1.00" and "2.00"   "#);
    fs::write(&p, text).unwrap();
    assert_eq!(hash_of(&d, "Add two amounts"), h);
}

#[test]
fn meaning_changes_do_change_the_spec_hash() {
    let t = TempDir::new().unwrap();
    let d = write_spec(t.path(), FEATURE);
    stamp_rids(&d).unwrap();
    let h = hash_of(&d, "Add two amounts");
    let p = d.join("money.feature");
    let text = fs::read_to_string(&p)
        .unwrap()
        .replace(r#"the total is "3.00""#, r#"the total is "4.00""#);
    fs::write(&p, text).unwrap();
    assert_ne!(hash_of(&d, "Add two amounts"), h);
}

fn ledger_for(d: &Path) -> Ledger {
    let features = load_specs(d, true).unwrap();
    let mut led = Ledger::default();
    led.sync_spec(&features);
    led
}

fn passed_all(led: &Ledger) -> HashMap<String, RunResult> {
    led.entries
        .keys()
        .map(|r| {
            (
                r.clone(),
                RunResult {
                    outcome: "passed".into(),
                    detail: String::new(),
                    nodeid: "n".into(),
                },
            )
        })
        .collect()
}

#[test]
fn a_scenario_with_no_test_is_pending_never_green() {
    let t = TempDir::new().unwrap();
    let d = write_spec(t.path(), FEATURE);
    stamp_rids(&d).unwrap();
    let mut led = ledger_for(&d);
    led.apply_run(&HashMap::new(), "r1", "");
    assert!(led.entries.values().all(|e| e.status == PENDING));
    assert_eq!(led.summary()["completion_pct"], serde_json::json!(0.0));
}

#[test]
fn green_goes_stale_when_its_scenario_changes_meaning() {
    let t = TempDir::new().unwrap();
    let d = write_spec(t.path(), FEATURE);
    stamp_rids(&d).unwrap();
    let mut led = ledger_for(&d);
    let rid = led
        .entries
        .iter()
        .find(|(_, e)| e.name == "Add two amounts")
        .unwrap()
        .0
        .clone();
    led.apply_run(&passed_all(&led), "r1", "");
    assert_eq!(led.entries[&rid].status, GREEN);
    let p = d.join("money.feature");
    let text = fs::read_to_string(&p)
        .unwrap()
        .replace(r#"the total is "3.00""#, r#"the total is "4.00""#);
    fs::write(&p, text).unwrap();
    led.sync_spec(&load_specs(&d, true).unwrap());
    assert_eq!(led.entries[&rid].status, STALE, "green must not survive a change of meaning");
    for (r, e) in &led.entries {
        if r != &rid {
            assert_eq!(e.status, GREEN, "unrelated scenarios keep their green");
        }
    }
}

#[test]
fn green_to_red_is_recorded_as_a_regression() {
    let t = TempDir::new().unwrap();
    let d = write_spec(t.path(), FEATURE);
    stamp_rids(&d).unwrap();
    let mut led = ledger_for(&d);
    let rid = led.entries.keys().next().unwrap().clone();
    led.apply_run(
        &HashMap::from([(
            rid.clone(),
            RunResult {
                outcome: "passed".into(),
                detail: String::new(),
                nodeid: "n".into(),
            },
        )]),
        "r1",
        "",
    );
    let out = led.apply_run(
        &HashMap::from([(
            rid.clone(),
            RunResult {
                outcome: "failed".into(),
                detail: "boom".into(),
                nodeid: "n".into(),
            },
        )]),
        "r2",
        "",
    );
    assert_eq!(out.regressions.len(), 1);
    assert_eq!(led.entries[&rid].status, RED);
}

#[test]
fn blocked_suite_reports_red_not_pending() {
    let t = TempDir::new().unwrap();
    let d = write_spec(t.path(), FEATURE);
    stamp_rids(&d).unwrap();
    let mut led = ledger_for(&d);
    led.apply_run(&HashMap::new(), "r1", "ImportError: no module named 'money'");
    assert!(led.entries.values().all(|e| e.status == RED));
}

#[test]
fn ledger_round_trips() {
    let t = TempDir::new().unwrap();
    let d = write_spec(t.path(), FEATURE);
    stamp_rids(&d).unwrap();
    let led = ledger_for(&d);
    let path = t.path().join(".shalt/ledger.json");
    led.save(&path).unwrap();
    let again = Ledger::load(&path).unwrap();
    assert_eq!(
        again.entries.keys().collect::<std::collections::HashSet<_>>(),
        led.entries.keys().collect::<std::collections::HashSet<_>>()
    );
}

#[test]
fn holdouts_are_stripped_for_the_implementer_but_stay_in_the_ledger() {
    let t = TempDir::new().unwrap();
    let d = write_spec(t.path(), FEATURE);
    stamp_rids(&d).unwrap();
    let features = load_specs(&d, true).unwrap();
    assert_eq!(holdout_rids(&features).len(), 1);
    let visible = strip_holdouts(&fs::read_to_string(d.join("money.feature")).unwrap());
    assert!(!visible.contains("Unseen amounts"));
    assert!(visible.contains("Add two amounts"));
    assert!(!visible.contains("@holdout"));
}

fn workspace(tmp: &Path) -> std::path::PathBuf {
    for z in ["spec", "steps", "contract", "src"] {
        fs::create_dir_all(tmp.join(z)).unwrap();
    }
    fs::write(tmp.join("steps/test_x.py"), "assert True\n").unwrap();
    fs::write(tmp.join("spec/x.feature"), FEATURE).unwrap();
    tmp.to_path_buf()
}

#[test]
fn implementer_editing_the_tests_is_rejected_and_rolled_back() {
    let t = TempDir::new().unwrap();
    let root = workspace(t.path());
    let original = fs::read_to_string(root.join("steps/test_x.py")).unwrap();
    let err = GuardedTurn::enter(&root, "implementer", &root.join(".shalt/backup"))
        .ok()
        .and_then(|g| {
            fs::write(root.join("src/x.py"), "ok\n").unwrap();
            fs::write(root.join("steps/test_x.py"), "assert False  # weakened\n").unwrap();
            g.commit().err()
        });
    let ei = err.expect("expected integrity violation");
    assert!(ei.offences.contains_key("steps"));
    assert_eq!(fs::read_to_string(root.join("steps/test_x.py")).unwrap(), original);
}

#[test]
fn implementer_writing_only_to_src_is_allowed() {
    let t = TempDir::new().unwrap();
    let root = workspace(t.path());
    let g = GuardedTurn::enter(&root, "implementer", &root.join(".shalt/backup")).unwrap();
    fs::write(root.join("src/x.py"), "ok\n").unwrap();
    g.commit().unwrap();
    assert!(root.join("src/x.py").exists());
}

#[test]
fn snapshot_diff_detects_content_change() {
    let t = TempDir::new().unwrap();
    let root = workspace(t.path());
    let a = snapshot(&root, shalt_core::ALL_ZONES);
    fs::write(root.join("src/y.py"), "1\n").unwrap();
    let b = snapshot(&root, shalt_core::ALL_ZONES);
    assert_eq!(diff_snap(&a, &b).get("src").unwrap(), &vec!["src/y.py".to_string()]);
}

const ISO_FEATURE: &str = r#"Feature: Money

  @rid:S-11111111
  Scenario: Add
    Given amounts "1.00" and "2.00"
    Then the total is "3.00"
"#;
const TEST_SRC: &str = "def test_real():\n    assert True\n";

fn iso_workspace(tmp: &Path) -> std::path::PathBuf {
    for z in ["spec", "steps", "contract", "src", ".shalt"] {
        fs::create_dir_all(tmp.join(z)).unwrap();
    }
    fs::write(tmp.join("spec/money.feature"), ISO_FEATURE).unwrap();
    fs::write(tmp.join("steps/test_money.py"), TEST_SRC).unwrap();
    Ledger::default().save(&tmp.join(".shalt/ledger.json")).unwrap();
    tmp.to_path_buf()
}

fn assert_intact(ws: &Path) {
    assert_eq!(fs::read_to_string(ws.join("steps/test_money.py")).unwrap(), TEST_SRC);
    assert_eq!(fs::read_to_string(ws.join("spec/money.feature")).unwrap(), ISO_FEATURE);
}

#[test]
fn relative_traversal_out_of_the_stage_cannot_reach_the_tests() {
    let t = TempDir::new().unwrap();
    let ws = iso_workspace(t.path());
    let mut backend = FnBackend {
        f: |stage: &Path| {
            fs::create_dir_all(stage.join("src")).unwrap();
            fs::write(stage.join("src/ok.py"), "x = 1\n").unwrap();
            let escape = stage.join("../../../steps/test_money.py");
            let _ = fs::create_dir_all(escape.parent().unwrap());
            let _ = fs::write(&escape, "assert False  # weakened\n");
        },
    };
    run_role(&ws, "implementer", "p", &mut backend, false).unwrap();
    assert_intact(&ws);
}

#[test]
fn absolute_write_to_the_tests_is_caught_and_rolled_back() {
    let t = TempDir::new().unwrap();
    let ws = iso_workspace(t.path());
    let ws2 = ws.clone();
    let mut backend = FnBackend {
        f: move |stage: &Path| {
            fs::create_dir_all(stage.join("src")).unwrap();
            fs::write(stage.join("src/ok.py"), "x = 1\n").unwrap();
            fs::write(ws2.join("steps/test_money.py"), "assert False  # weakened\n").unwrap();
        },
    };
    let err = run_role(&ws, "implementer", "p", &mut backend, false).unwrap_err();
    match err {
        RoleError::Integrity(ei) => assert!(ei.offences.contains_key("steps")),
        other => panic!("{other:?}"),
    }
    assert_intact(&ws);
}

#[test]
fn a_role_cannot_rewrite_the_ledger() {
    let t = TempDir::new().unwrap();
    let ws = iso_workspace(t.path());
    let ws2 = ws.clone();
    let mut backend = FnBackend {
        f: move |stage: &Path| {
            fs::create_dir_all(stage.join("src")).unwrap();
            fs::write(stage.join("src/ok.py"), "x = 1\n").unwrap();
            fs::write(
                ws2.join(".shalt/ledger.json"),
                r#"{"schema": "shalt.ledger/1", "scenarios": {}, "spec_lock": {"approved_by": "nobody"}}"#,
            )
            .unwrap();
        },
    };
    let err = run_role(&ws, "implementer", "p", &mut backend, false).unwrap_err();
    match err {
        RoleError::Integrity(ei) => assert!(ei.offences.contains_key("ledger")),
        other => panic!("{other:?}"),
    }
    let lock = Ledger::load(&ws.join(".shalt/ledger.json")).unwrap().spec_lock;
    assert!(
        lock.get("approved_by").is_none(),
        "forged approval must not survive rollback: {lock}"
    );
}

#[test]
fn writing_only_to_its_own_zone_is_allowed() {
    let t = TempDir::new().unwrap();
    let ws = iso_workspace(t.path());
    let mut backend = FnBackend {
        f: |stage: &Path| {
            fs::create_dir_all(stage.join("src")).unwrap();
            fs::write(stage.join("src/money.py"), "def total(): return '3.00'\n").unwrap();
        },
    };
    let res = run_role(&ws, "implementer", "p", &mut backend, false).unwrap();
    assert_eq!(res.wrote, vec!["src/money.py"]);
    assert!(ws.join("src/money.py").exists());
}

#[test]
fn a_role_can_delete_a_file_in_its_own_zone() {
    let t = TempDir::new().unwrap();
    let ws = iso_workspace(t.path());
    fs::write(ws.join("src/stale.py"), "old\n").unwrap();
    let mut backend = FnBackend {
        f: |stage: &Path| {
            fs::create_dir_all(stage.join("src")).unwrap();
            let _ = fs::remove_file(stage.join("src/stale.py"));
            fs::write(stage.join("src/fresh.py"), "new\n").unwrap();
        },
    };
    let res = run_role(&ws, "implementer", "p", &mut backend, false).unwrap();
    assert_eq!(res.removed, vec!["src/stale.py"]);
    assert!(!ws.join("src/stale.py").exists());
    assert!(ws.join("src/fresh.py").exists());
}

#[test]
fn stepwright_sees_its_own_previous_steps_but_never_the_implementation() {
    let t = TempDir::new().unwrap();
    let ws = iso_workspace(t.path());
    fs::write(ws.join("src/secret.py"), "SECRET = 1\n").unwrap();
    let seen = std::sync::Arc::new(std::sync::Mutex::new((Vec::new(), false)));
    let seen2 = seen.clone();
    let mut backend = FnBackend {
        f: move |stage: &Path| {
            let mut names: Vec<String> = fs::read_dir(stage)
                .unwrap()
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            let has = stage.join("steps/test_money.py").exists();
            *seen2.lock().unwrap() = (names, has);
        },
    };
    run_role(&ws, "stepwright", "p", &mut backend, false).unwrap();
    let (zones, has_steps) = seen.lock().unwrap().clone();
    assert!(!zones.iter().any(|z| z == "src"), "the stepwright must not see the implementation");
    assert!(has_steps, "it must see its own previous work to revise it");
}

#[test]
fn implementer_never_sees_the_step_definitions() {
    let t = TempDir::new().unwrap();
    let ws = iso_workspace(t.path());
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    let mut backend = FnBackend {
        f: move |stage: &Path| {
            let mut names: Vec<String> = fs::read_dir(stage)
                .unwrap()
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            *seen2.lock().unwrap() = names;
        },
    };
    run_role(&ws, "implementer", "p", &mut backend, false).unwrap();
    let zones = seen.lock().unwrap().clone();
    assert!(!zones.iter().any(|z| z == "steps"));
    assert!(zones.iter().any(|z| z == "spec") && zones.iter().any(|z| z == "contract"));
}

#[test]
fn a_file_at_the_stage_root_is_rejected() {
    let t = TempDir::new().unwrap();
    let ws = iso_workspace(t.path());
    let mut backend = FnBackend {
        f: |stage: &Path| {
            fs::write(stage.join("loose.py"), "x = 1\n").unwrap();
        },
    };
    let err = run_role(&ws, "implementer", "p", &mut backend, false).unwrap_err();
    assert!(matches!(err, RoleError::Integrity(_)));
}

const SIMPLE: &str = r#"Feature: X

  @rid:S-00000001
  Scenario: A
    Given a thing
    Then it works

  @rid:S-00000002
  Scenario: B
    Given a thing
    Then it works
"#;

#[test]
fn orphan_is_not_an_absorbing_state() {
    let t = TempDir::new().unwrap();
    let d = t.path().join("spec");
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join("f.feature"), SIMPLE).unwrap();
    let mut led = Ledger::default();
    led.sync_spec(&load_specs(&d, true).unwrap());
    led.apply_run(
        &HashMap::from([
            (
                "S-00000001".into(),
                RunResult {
                    outcome: "passed".into(),
                    detail: String::new(),
                    nodeid: "n".into(),
                },
            ),
            (
                "S-00000002".into(),
                RunResult {
                    outcome: "failed".into(),
                    detail: "x".into(),
                    nodeid: "n".into(),
                },
            ),
        ]),
        "r1",
        "",
    );
    let only_a = SIMPLE.split("  @rid:S-00000002").next().unwrap();
    fs::write(d.join("f.feature"), only_a).unwrap();
    led.sync_spec(&load_specs(&d, true).unwrap());
    assert_eq!(led.entries["S-00000002"].status, ORPHAN);
    fs::write(d.join("f.feature"), SIMPLE).unwrap();
    led.sync_spec(&load_specs(&d, true).unwrap());
    assert_eq!(
        led.entries["S-00000002"].status, PENDING,
        "a restored scenario must prove itself again, not stay invisible"
    );
    assert_ne!(led.summary()["completion_pct"], serde_json::json!(100.0));
}

#[test]
fn losing_the_test_that_proved_a_scenario_is_a_regression() {
    let t = TempDir::new().unwrap();
    let d = t.path().join("spec");
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join("f.feature"), SIMPLE).unwrap();
    let mut led = Ledger::default();
    led.sync_spec(&load_specs(&d, true).unwrap());
    led.apply_run(&passed_all(&led), "r1", "");
    assert!(led.entries.values().all(|e| e.status == GREEN));
    let out = led.apply_run(&HashMap::new(), "r2", "");
    assert_eq!(out.regressions.len(), 2);
    assert!(led.entries.values().all(|e| e.status == PENDING));
    assert!(led.entries.values().all(|e| e.verified_spec_hash.is_empty()));
}

#[test]
fn ledger_tolerates_unknown_fields() {
    let t = TempDir::new().unwrap();
    let d = t.path().join("spec");
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join("f.feature"), SIMPLE).unwrap();
    let mut led = Ledger::default();
    led.sync_spec(&load_specs(&d, true).unwrap());
    let path = t.path().join(".shalt/ledger.json");
    led.save(&path).unwrap();
    let mut raw: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    raw["scenarios"]["S-00000001"]["owner"] = serde_json::json!("dan");
    fs::write(&path, serde_json::to_string(&raw).unwrap()).unwrap();
    let loaded = Ledger::load(&path).unwrap();
    assert_eq!(
        loaded.entries.keys().cloned().collect::<std::collections::HashSet<_>>(),
        ["S-00000001".into(), "S-00000002".into()].into()
    );
}

#[test]
fn overlay_auto_adds_and_unschedule_does_not_touch_spec() {
    let t = TempDir::new().unwrap();
    let d = t.path().join("spec");
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join("f.feature"), SIMPLE).unwrap();
    let features = load_specs(&d, true).unwrap();
    let mut board = Board::default();
    board.sync_new_rids(&features);
    assert_eq!(board.items.len(), 2);
    let spec_before = fs::read_to_string(d.join("f.feature")).unwrap();
    assert!(board.unschedule("S-00000001"));
    assert_eq!(board.items.len(), 1);
    assert_eq!(fs::read_to_string(d.join("f.feature")).unwrap(), spec_before);
}

#[test]
fn verify_fails_on_dangling_overlay_rid() {
    let t = TempDir::new().unwrap();
    let d = t.path().join("spec");
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join("f.feature"), SIMPLE).unwrap();
    let features = load_specs(&d, true).unwrap();
    let mut board = Board::default();
    board.sync_new_rids(&features);
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-deadbeef".into(),
        rank: 99,
        goal_id: None,
        milestone_id: None,
        sprint_id: None,
    });
    let drift = shalt_core::board::verify_drift(&board, &features);
    assert!(drift.iter().any(|p| p.contains("S-deadbeef")));
}

#[test]
fn ui_registry_prunes_dead_pids() {
    let t = TempDir::new().unwrap();
    std::env::set_var("SHALT_HOME", t.path());
    shalt_core::uis::record(shalt_core::uis::UiInstance {
        pid: 999_999_999,
        port: 7700,
        url: "http://127.0.0.1:7700/".into(),
        root: "/tmp".into(),
        started_at: "now".into(),
    });
    let live = shalt_core::uis::prune();
    assert!(live.is_empty(), "dead pid must not count as a live shalt ui: {live:?}");
    std::env::remove_var("SHALT_HOME");
}

#[test]
fn job_pause_and_prompt_edit() {
    let t = TempDir::new().unwrap();
    let path = t.path().join("jobs.json");
    let mut q = JobQueue::default();
    let job = q.enqueue_full(JobKind::Author, "invoice", "old request", "qwen", "qwen3.5:2b");
    q.set_status(&job.id, JobStatus::Running);
    q.set_status(&job.id, JobStatus::Paused);
    q.set_prompt(&job.id, "new request");
    q.save_to(&path).unwrap();
    let q2 = JobQueue::load_from(&path);
    assert_eq!(q2.jobs[0].status, JobStatus::Paused);
    assert_eq!(q2.jobs[0].prompt, "new request");
}

#[test]
fn job_log_appends_without_clobbering() {
    let t = TempDir::new().unwrap();
    let path = t.path().join("jobs.json");
    let mut q = JobQueue::default();
    q.enqueue(JobKind::Author, "invoice");
    q.append(&q.jobs[0].id.clone(), "step 1: waiting on the model…");
    q.append(&q.jobs[0].id.clone(), "[write_file] wrote spec/invoice.feature (120 bytes)");
    q.save_to(&path).unwrap();
    let q2 = JobQueue::load_from(&path);
    assert!(q2.jobs[0].log.contains("step 1"));
    assert!(q2.jobs[0].log.contains("invoice.feature"));
}

#[test]
fn jobs_are_durable_and_interrupted_is_retryable() {
    let t = TempDir::new().unwrap();
    let path = t.path().join("jobs.json");
    let mut q = JobQueue::default();
    q.enqueue(JobKind::Author, "invoice");
    q.jobs[0].status = JobStatus::Running;
    q.save_to(&path).unwrap();
    let mut q2 = JobQueue::load_from(&path);
    q2.interrupt_running();
    assert_eq!(q2.jobs[0].status, JobStatus::Interrupted);
    q2.jobs[0].status = JobStatus::Pending;
    q2.save_to(&path).unwrap();
    let q3 = JobQueue::load_from(&path);
    assert_eq!(q3.jobs[0].status, JobStatus::Pending);
}

#[test]
fn holdout_tag_matching_is_exact() {
    let t = TempDir::new().unwrap();
    let d = t.path().join("spec");
    fs::create_dir_all(&d).unwrap();
    fs::write(
        d.join("f.feature"),
        r#"Feature: X

  @holdout_wip @rid:S-cccccccc
  Scenario: Not actually held out
    Given a thing
    Then it works
"#,
    )
    .unwrap();
    let features = load_specs(&d, true).unwrap();
    assert!(holdout_rids(&features).is_empty());
    let visible = strip_holdouts(&fs::read_to_string(d.join("f.feature")).unwrap());
    assert!(visible.contains("Not actually held out"));
}

#[test]
fn same_basename_in_different_directories_does_not_collide() {
    let t = TempDir::new().unwrap();
    let d = t.path().join("spec");
    for (sub, rid) in [("billing", "S-0000000a"), ("refunds", "S-0000000b")] {
        fs::create_dir_all(d.join(sub)).unwrap();
        fs::write(
            d.join(sub).join("m.feature"),
            format!(
                "Feature: {sub}\n\n  @rid:{rid}\n  Scenario: Totals are correct\n    Given a thing\n    Then it works\n"
            ),
        )
        .unwrap();
    }
    let mut mapping = HashMap::new();
    for f in load_specs(&d, true).unwrap() {
        for s in f.scenarios {
            mapping.insert((f.file.replace('\\', "/"), s.name), s.rid);
        }
    }
    assert_eq!(mapping.len(), 2);
}

#[test]
fn qwen_backend_does_not_need_an_api_key() {
    let b = shalt_core::OpenAICompatBackend::from_preset("qwen", Some("qwen3.5:2b"), None).unwrap();
    assert_eq!(b.name, "qwen");
    assert!(b.base_url.contains("11434"));
}

#[test]
fn compose_creates_a_workspace_and_a_job() {
    let t = TempDir::new().unwrap();
    std::env::set_var("SHALT_HOME", t.path());
    let (project, job) = shalt_core::start_project(shalt_core::ComposeRequest {
        prompt: "The system shall total invoices exactly.".into(),
        backend: "qwen".into(),
        model: "qwen3.5:2b".into(),
        name: Some("invoices".into()),
    })
    .unwrap();
    assert_eq!(project.id, "invoices");
    assert!(Path::new(&project.path).join("spec").is_dir());
    assert_eq!(job.kind, shalt_core::jobs::JobKind::Author);
    assert_eq!(job.backend, "qwen");
    std::env::remove_var("SHALT_HOME");
}

#[test]
fn api_sandbox_refuses_absolute_and_traversal_paths() {
    let t = TempDir::new().unwrap();
    let stage = t.path();
    fs::create_dir_all(stage.join("src")).unwrap();
    let abs = shalt_core::api::dispatch(stage, "write_file", &serde_json::json!({"path":"/etc/passwd","content":"x"}));
    assert!(abs.starts_with("REFUSED:"), "{abs}");
    let trav = shalt_core::api::dispatch(stage, "write_file", &serde_json::json!({"path":"../escape.py","content":"x"}));
    assert!(trav.starts_with("REFUSED:"), "{trav}");
    let ok = shalt_core::api::dispatch(stage, "write_file", &serde_json::json!({"path":"src/ok.py","content":"x=1\n"}));
    assert!(ok.starts_with("wrote "), "{ok}");
    assert_eq!(fs::read_to_string(stage.join("src/ok.py")).unwrap(), "x=1\n");
}

#[test]
fn text_engine_finds_comparison_mutants() {
    let t = TempDir::new().unwrap();
    let src = t.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("n.py"), "def f(x):\n    return x == 1\n").unwrap();
    let ms = shalt_core::mutate::text_mutants(t.path(), &src);
    assert!(ms.iter().any(|(m, _)| m.operator == "comparison"), "{ms:?}");
}

#[test]
fn crlf_line_endings_survive_stamping() {
    let t = TempDir::new().unwrap();
    let d = t.path().join("spec");
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join("f.feature"), b"Feature: X\r\n\r\n  Scenario: A\r\n    Given a thing\r\n").unwrap();
    stamp_rids(&d).unwrap();
    let raw = fs::read(d.join("f.feature")).unwrap();
    assert!(raw.windows(2).any(|w| w == b"\r\n"));
    let n = raw.iter().filter(|b| **b == b'\n').count();
    let rn = raw.windows(2).filter(|w| *w == b"\r\n").count();
    assert_eq!(n, rn, "no mixed line endings");
    assert!(raw.windows(5).any(|w| w == b"@rid:"));
}
