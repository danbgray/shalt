//! Ports of the Python invariant tests: identity, ledger, isolation, overlay.

use shalt_core::backends::FnBackend;
use shalt_core::board::Board;
use shalt_core::integrity::{diff_snap, snapshot, GuardedTurn};
use shalt_core::jobs::{JobKind, JobQueue, JobStatus};
use shalt_core::org::Org;
use shalt_core::ledger::{Ledger, GREEN, ORPHAN, PENDING, RED, STALE};
use shalt_core::roles::{run_role, RoleError};
use shalt_core::spec::{holdout_rids, load_specs, stamp_rids, strip_holdouts};
use shalt_core::RunResult;
use rand::SeedableRng;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use tempfile::TempDir;

static SHALT_HOME_LOCK: Mutex<()> = Mutex::new(());

fn with_shalt_home<R>(f: impl FnOnce() -> R) -> R {
    let _g = SHALT_HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let out = f();
    std::env::remove_var("SHALT_HOME");
    out
}

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
    assert!(
        led.suite_surveyed(),
        "an empty suite is still a survey — Play must not run forever"
    );
    assert!(led.entries.values().all(|e| e.last_run_at.is_some()));
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
    assert!(led.harness_error().unwrap().contains("ImportError"));
    assert!(
        led.entries
            .values()
            .all(|e| e.failure.as_deref() == Some("tests did not compile")),
        "{:?}",
        led.entries.values().map(|e| e.failure.clone()).collect::<Vec<_>>()
    );
}

#[test]
fn compile_dump_is_not_copied_onto_every_scenario() {
    let t = TempDir::new().unwrap();
    let d = write_spec(t.path(), FEATURE);
    stamp_rids(&d).unwrap();
    let mut led = ledger_for(&d);
    let dump = "Compiling demo v0.1.0\nerror[E0432]: unresolved import `contract::ErpSystem`\nerror: could not compile `demo` (test \"shalt\") due to 3 previous errors";
    led.apply_run(&HashMap::new(), "r1", dump);
    assert!(led.harness_error().unwrap().contains("could not compile"));
    for e in led.entries.values() {
        assert_eq!(e.status, RED);
        assert_eq!(e.failure.as_deref(), Some("tests did not compile"));
    }
}

#[test]
fn shared_compile_dump_is_still_a_suite_error() {
    let dump = "Compiling demo v0.1.0\nerror: could not compile `demo` due to previous errors";
    let mut led = shalt_core::ledger::Ledger::default();
    for i in 0..4 {
        let e: shalt_core::ledger::Entry = serde_json::from_value(serde_json::json!({
            "rid": format!("r{i}"),
            "name": "n",
            "feature": "f",
            "feature_file": "f.feature",
            "status": "red",
            "failure": dump,
        }))
        .unwrap();
        led.entries.insert(format!("r{i}"), e);
    }
    let shown = led.display_harness_error().expect("shared dump");
    assert!(shown.contains("could not compile"), "{shown}");
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
fn dropping_a_scenario_block_leaves_the_other() {
    let t = TempDir::new().unwrap();
    let d = t.path().join("spec");
    fs::create_dir_all(&d).unwrap();
    fs::write(
        d.join("f.feature"),
        "Feature: X\n\n  Scenario: Keep me\n    Given a thing\n    Then it works\n\n  Scenario: Drop me\n    Given a thing\n    Then it works\n",
    )
    .unwrap();
    let features = load_specs(&d, true).unwrap();
    let drop_start = features[0]
        .scenarios
        .iter()
        .find(|s| s.name == "Drop me")
        .unwrap()
        .block_start();
    shalt_core::spec::drop_scenario_blocks(&d, &[("f.feature".into(), drop_start)]).unwrap();
    let names: Vec<_> = load_specs(&d, true)
        .unwrap()
        .into_iter()
        .flat_map(|f| f.scenarios)
        .map(|s| s.name)
        .collect();
    assert_eq!(names, vec!["Keep me".to_string()]);
}

#[test]
fn rewrite_and_stamp_a_pending_scenario() {
    let t = TempDir::new().unwrap();
    let d = t.path().join("spec");
    fs::create_dir_all(&d).unwrap();
    fs::write(
        d.join("f.feature"),
        "Feature: X\n\n  Scenario: Old name\n    Given a thing\n    Then it works\n",
    )
    .unwrap();
    shalt_core::spec::rewrite_scenario(
        &d,
        "f.feature",
        3,
        "New name",
        &["Given a widget".into(), "Then it is listed".into()],
        None,
    )
    .unwrap();
    let body = fs::read_to_string(d.join("f.feature")).unwrap();
    assert!(body.contains("Scenario: New name"), "{body}");
    assert!(body.contains("Given a widget"));
    assert!(!body.contains("Old name"));
    let rid = shalt_core::spec::stamp_scenario(&d, "f.feature", 3)
        .unwrap()
        .expect("rid");
    assert!(rid.starts_with("S-"));
    let body = fs::read_to_string(d.join("f.feature")).unwrap();
    assert!(body.contains(&format!("@rid:{rid}")), "{body}");
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
fn designer_writes_mockups_not_spec() {
    let t = TempDir::new().unwrap();
    let root = workspace(t.path());
    fs::create_dir_all(root.join("mockups")).unwrap();
    let spec = fs::read_to_string(root.join("spec/x.feature")).unwrap();
    let err = GuardedTurn::enter(&root, "designer", &root.join(".shalt/backup"))
        .ok()
        .and_then(|g| {
            fs::write(root.join("spec/x.feature"), "Feature: Tamper\n").unwrap();
            fs::write(root.join("mockups/ok.html"), "<p>x</p>\n").unwrap();
            g.commit().err()
        });
    let ei = err.expect("expected integrity violation");
    assert!(ei.offences.contains_key("spec"), "{:?}", ei.offences);
    assert_eq!(fs::read_to_string(root.join("spec/x.feature")).unwrap(), spec);
}

#[test]
fn designer_writing_only_mockups_is_allowed() {
    let t = TempDir::new().unwrap();
    let root = workspace(t.path());
    let g = GuardedTurn::enter(&root, "designer", &root.join(".shalt/backup")).unwrap();
    fs::create_dir_all(root.join("mockups")).unwrap();
    fs::write(root.join("mockups/ok.html"), "<p>x</p>\n").unwrap();
    g.commit().unwrap();
    assert!(root.join("mockups/ok.html").exists());
}

#[test]
fn implementer_cannot_write_mockups() {
    let t = TempDir::new().unwrap();
    let root = workspace(t.path());
    fs::create_dir_all(root.join("mockups")).unwrap();
    fs::write(root.join("mockups/ok.html"), "<p>old</p>\n").unwrap();
    let err = GuardedTurn::enter(&root, "implementer", &root.join(".shalt/backup"))
        .ok()
        .and_then(|g| {
            fs::write(root.join("src/x.py"), "ok\n").unwrap();
            fs::write(root.join("mockups/ok.html"), "<p>new</p>\n").unwrap();
            g.commit().err()
        });
    let ei = err.expect("expected integrity violation");
    assert!(ei.offences.contains_key("mockups"), "{:?}", ei.offences);
    assert_eq!(
        fs::read_to_string(root.join("mockups/ok.html")).unwrap(),
        "<p>old</p>\n"
    );
}

#[test]
fn stepwright_does_not_read_mockups() {
    use shalt_core::integrity::read_zones;
    let z = read_zones("stepwright", "tests", "src");
    assert!(!z.iter().any(|x| x == "mockups"), "{z:?}");
}

#[test]
fn auditor_reads_tests_not_src_and_writes_nothing() {
    use shalt_core::integrity::{read_zones, write_zones};
    let r = read_zones("auditor", "tests", "src");
    assert!(r.iter().any(|x| x == "spec"), "{r:?}");
    assert!(r.iter().any(|x| x == "tests"), "{r:?}");
    assert!(!r.iter().any(|x| x == "src"), "test auditor must not see src: {r:?}");
    assert!(write_zones("auditor", "tests", "src").is_empty());
    let c = read_zones("code_auditor", "tests", "src");
    assert!(c.iter().any(|x| x == "src"), "{c:?}");
    assert!(c.iter().any(|x| x == "tests"), "{c:?}");
    assert!(write_zones("code_auditor", "tests", "src").is_empty());
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
    shalt_core::config::init_workspace(tmp, "rust", "iso").unwrap();
    fs::write(tmp.join("spec/money.feature"), ISO_FEATURE).unwrap();
    fs::write(tmp.join("tests/shalt.rs"), TEST_SRC).unwrap();
    Ledger::default().save(&tmp.join(".shalt/ledger.json")).unwrap();
    tmp.to_path_buf()
}

fn assert_intact(ws: &Path) {
    assert_eq!(fs::read_to_string(ws.join("tests/shalt.rs")).unwrap(), TEST_SRC);
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
            let escape = stage.join("../../../tests/shalt.rs");
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
            fs::write(ws2.join("tests/shalt.rs"), "assert False  # weakened\n").unwrap();
        },
    };
    let err = run_role(&ws, "implementer", "p", &mut backend, false).unwrap_err();
    match err {
        RoleError::Integrity(ei) => {
            assert!(
                ei.offences.contains_key("tests") || ei.offences.contains_key("steps"),
                "{ei:?}"
            )
        }
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
fn guarded_turn_backup_does_not_live_in_the_workspace() {
    let t = TempDir::new().unwrap();
    let ws = iso_workspace(t.path());
    let mut backend = FnBackend {
        f: |stage: &Path| {
            fs::create_dir_all(stage.join("src")).unwrap();
            fs::write(stage.join("src/ok.py"), "x = 1\n").unwrap();
        },
    };
    run_role(&ws, "implementer", "p", &mut backend, false).unwrap();
    assert!(
        !ws.join(".shalt/backup").exists(),
        "restore copies belong next to the stage, not in the project"
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
    assert!(
        res.wrote.iter().any(|w| w == "src/money.py"),
        "{:?}",
        res.wrote
    );
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
            let has = stage.join("tests/shalt.rs").exists();
            *seen2.lock().unwrap() = (names, has);
        },
    };
    run_role(&ws, "stepwright", "p", &mut backend, false).unwrap();
    let (zones, has_steps) = seen.lock().unwrap().clone();
    assert!(!zones.iter().any(|z| z == "src"), "the stepwright must not see the implementation");
    assert!(has_steps, "it must see its own previous work to revise it");
}

#[test]
fn author_can_read_src_but_cannot_write_it() {
    let t = TempDir::new().unwrap();
    let ws = iso_workspace(t.path());
    fs::write(ws.join("src/existing.py"), "VALUE = 7\n").unwrap();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(false));
    let seen2 = seen.clone();
    let mut backend = FnBackend {
        f: move |stage: &Path| {
            *seen2.lock().unwrap() = stage.join("src/existing.py").exists();
            fs::create_dir_all(stage.join("spec")).unwrap();
            fs::write(stage.join("spec/ok.feature"), "Feature: X\n  Scenario: Y\n    Given a\n").unwrap();
            fs::create_dir_all(stage.join("src")).unwrap();
            fs::write(stage.join("src/hack.py"), "nope\n").unwrap();
        },
    };
    let err = run_role(&ws, "author", "p", &mut backend, false).unwrap_err();
    assert!(seen.lock().unwrap().eq(&true), "author must see existing src/");
    match err {
        RoleError::Integrity(e) => assert!(e.offences.contains_key("src") || e.to_string().contains("src")),
        other => panic!("{other}"),
    }
    assert!(!ws.join("src/hack.py").exists());
    assert_eq!(fs::read_to_string(ws.join("src/existing.py")).unwrap(), "VALUE = 7\n");
}

#[test]
fn ensure_workspace_does_not_clobber_existing_src() {
    let t = TempDir::new().unwrap();
    fs::create_dir_all(t.path().join("src")).unwrap();
    fs::write(t.path().join("src/app.py"), "keep\n").unwrap();
    fs::write(t.path().join("Cargo.toml"), "[package]\nname=\"x\"\nversion=\"0.1.0\"\n").unwrap();
    assert_eq!(shalt_core::config::detect_stack(t.path()), "rust");
    shalt_core::config::ensure_workspace(t.path(), "rust", "x").unwrap();
    assert_eq!(fs::read_to_string(t.path().join("src/app.py")).unwrap(), "keep\n");
    assert!(t.path().join("shalt.toml").exists());
    assert!(t.path().join("spec").is_dir());
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
    assert!(board.promote("S-00000001"));
    assert_eq!(board.items.iter().find(|i| i.rid == "S-00000001").unwrap().rank, 0);
    assert!(board.unschedule("S-00000001"));
    assert_eq!(board.items.len(), 1);
    assert_eq!(fs::read_to_string(d.join("f.feature")).unwrap(), spec_before);
}

#[test]
fn sprint_retro_measures_estimate_vs_spend() {
    let mut board = Board::default();
    let s = board.open_sprint("W38");
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-1".into(),
        rank: 1,
        sprint_id: Some(s.id.clone()),
        token_estimate: 100_000,
        ..Default::default()
    });
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-2".into(),
        rank: 2,
        sprint_id: Some(s.id.clone()),
        token_estimate: 100_000,
        ..Default::default()
    });
    let mut q = JobQueue::default();
    let j = q.enqueue(JobKind::Build, "p");
    q.set_sprint(&j.id, &s.id);
    q.add_tokens(&j.id, 80_000, 40_000);
    let retro = shalt_core::tokens::retro(&board, &q.jobs, "p", &s.id).unwrap();
    assert_eq!(retro.tickets, 2);
    assert_eq!(retro.estimated, 200_000);
    assert_eq!(retro.spent, 120_000);
    assert_eq!(retro.accuracy, Some(0.6));
    assert_eq!(retro.bias, -80_000);
    assert!(board.close_sprint(&s.id, retro.estimated, retro.spent));
    let scaled = shalt_core::tokens::suggest_estimate(&board);
    assert!(
        scaled < shalt_core::tokens::DEFAULT_TICKET_TOKENS,
        "under-spend should lower the next estimate, got {scaled}"
    );
}

fn ledger_ticket(rid: &str, name: &str, epic: &str, status: &str) -> shalt_core::ledger::Entry {
    serde_json::from_value(serde_json::json!({
        "rid": rid,
        "name": name,
        "feature": name,
        "feature_file": "spec.feature",
        "epic": epic,
        "status": status
    }))
    .unwrap()
}

#[test]
fn command_center_rolls_up_epic_allocation_and_model_spend() {
    let mut board = Board::default();
    let s = board.open_sprint("W38");
    board.set_epic_estimate("Billing", 200_000);
    board.set_epic_agent("Billing", "qwen", "qwen3.5:35b");
    board.set_epic_estimate("Inventory", 150_000);
    board.set_epic_agent("Inventory", "grok", "grok-4");
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-1".into(),
        rank: 1,
        sprint_id: Some(s.id.clone()),
        token_estimate: 100_000,
        ..Default::default()
    });
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-2".into(),
        rank: 2,
        sprint_id: Some(s.id.clone()),
        token_estimate: 80_000,
        backend: "grok".into(),
        model: "grok-4".into(),
        ..Default::default()
    });
    let mut led = Ledger::default();
    led.entries.insert("S-1".into(), ledger_ticket("S-1", "Invoice totals", "Billing", "pending"));
    led.entries.insert("S-2".into(), ledger_ticket("S-2", "Receive stock", "Inventory", "pending"));
    let mut q = JobQueue::default();
    let j = q.enqueue_full(JobKind::Build, "p", "", "qwen", "qwen3.5:35b");
    q.set_sprint(&j.id, &s.id);
    q.set_work(&j.id, "Billing", "S-1");
    q.add_tokens(&j.id, 40_000, 20_000);
    let cmd = shalt_core::tokens::command_center(&board, &q.jobs, &led, "p", Some(&s.id));
    assert_eq!(cmd.allocated, 350_000, "{:?}", cmd.epics);
    let billing = cmd.epics.iter().find(|e| e.name == "Billing").unwrap();
    assert_eq!(billing.estimated, 200_000);
    assert_eq!(billing.spent, 60_000);
    assert_eq!(billing.backend, "qwen");
    assert_eq!(billing.items[0].inherited, true);
    let inv = cmd.epics.iter().find(|e| e.name == "Inventory").unwrap();
    assert_eq!(inv.items[0].inherited, false);
    assert_eq!(inv.items[0].backend, "grok");
    let qwen = cmd.models.iter().find(|m| m.backend == "qwen").unwrap();
    assert_eq!(qwen.spent, 60_000);
    let grok = cmd.models.iter().find(|m| m.backend == "grok").unwrap();
    assert_eq!(grok.estimated, 80_000);
}

#[test]
fn spend_snapshot_is_forecast_vs_actual_for_the_inbox() {
    let mut board = Board::default();
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-1".into(),
        rank: 1,
        token_estimate: 100_000,
        time_estimate_secs: 180,
        ..Default::default()
    });
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-2".into(),
        rank: 2,
        token_estimate: 50_000,
        time_estimate_secs: 90,
        ..Default::default()
    });
    let led = Ledger::default();
    let mut q = JobQueue::default();
    let j = q.enqueue_full(JobKind::Build, "p", "", "qwen", "qwen3.8:27b-mlx");
    q.add_tokens(&j.id, 40_000, 20_000);
    let id = j.id.clone();
    if let Some(job) = q.jobs.iter_mut().find(|x| x.id == id) {
        job.created_at = "2026-09-22T00:00:00Z".into();
        job.finished_at = "2026-09-22T00:02:00Z".into();
        job.status = JobStatus::Done;
    }
    let snap = shalt_core::tokens::spend_snapshot(&board, &q.jobs, &led, "p");
    assert_eq!(snap.estimated, 150_000, "{snap:?}");
    assert_eq!(snap.spent, 60_000, "{snap:?}");
    assert_eq!(snap.forecast_secs, 270, "{snap:?}");
    assert_eq!(snap.spent_secs, 120, "{snap:?}");
}

#[test]
fn next_assignment_prefers_ticket_agent_then_epic() {
    let mut board = Board::default();
    let s = board.open_sprint("W38");
    board.set_epic_agent("Billing", "qwen", "qwen3.5:35b");
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-1".into(),
        rank: 1,
        sprint_id: Some(s.id.clone()),
        ..Default::default()
    });
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-2".into(),
        rank: 2,
        sprint_id: Some(s.id.clone()),
        backend: "grok".into(),
        model: "grok-4".into(),
        ..Default::default()
    });
    let mut led = Ledger::default();
    led.entries.insert("S-1".into(), ledger_ticket("S-1", "A", "Billing", "pending"));
    led.entries.insert("S-2".into(), ledger_ticket("S-2", "B", "Billing", "pending"));
    let a = shalt_core::tokens::next_assignment(&board, &led, Some(&s.id)).unwrap();
    assert_eq!(a.rid, "S-1");
    assert_eq!(a.backend, "qwen");
    led.entries.get_mut("S-1").unwrap().status = GREEN.to_string();
    let a = shalt_core::tokens::next_assignment(&board, &led, Some(&s.id)).unwrap();
    assert_eq!(a.rid, "S-2");
    assert_eq!(a.backend, "grok");
}

#[test]
fn work_focus_shows_now_next_and_just_wrote() {
    let mut board = Board::default();
    let s = board.open_sprint("W38");
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-1".into(),
        rank: 1,
        sprint_id: Some(s.id.clone()),
        token_estimate: 100_000,
        ..Default::default()
    });
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-2".into(),
        rank: 2,
        sprint_id: Some(s.id.clone()),
        token_estimate: 80_000,
        ..Default::default()
    });
    let mut led = Ledger::default();
    led.entries.insert(
        "S-1".into(),
        ledger_ticket("S-1", "Create a BOM", "Billing", "pending"),
    );
    led.entries.insert(
        "S-2".into(),
        ledger_ticket("S-2", "Scale quantities", "Billing", "pending"),
    );
    let mut q = JobQueue::default();
    let done = q.enqueue_full(JobKind::Author, "p", "", "qwen", "qwen3.5:35b");
    q.set_status(&done.id, JobStatus::Done);
    q.set_work(&done.id, "Billing", "S-1");
    q.append(&done.id, "[write_file] spec/bom.feature");
    let live = q.enqueue_full(JobKind::Steps, "p", "", "qwen", "qwen3.5:35b");
    q.set_status(&live.id, JobStatus::Running);
    q.set_work(&live.id, "Billing", "S-1");
    q.append(&live.id, "writing step definitions");
    let focus = shalt_core::tokens::work_focus(&board, &led, &q.jobs, "p", Some(&s.id));
    let now = focus.now.expect("now");
    assert_eq!(now.rid, "S-1");
    assert_eq!(now.kind, "steps");
    assert_eq!(focus.next[0].rid, "S-2");
    let wrote = focus.just_wrote.expect("just wrote");
    assert!(
        wrote.files.iter().any(|f| f.contains("bom.feature")),
        "{:?}",
        wrote.files
    );
}

#[test]
fn work_focus_fills_next_from_the_rest_of_the_board() {
    let mut board = Board::default();
    let s = board.open_sprint("C-1");
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-1".into(),
        rank: 1,
        sprint_id: Some(s.id.clone()),
        token_estimate: 120_000,
        time_estimate_secs: 180,
        ..Default::default()
    });
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-2".into(),
        rank: 2,
        token_estimate: 80_000,
        time_estimate_secs: 120,
        ..Default::default()
    });
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-3".into(),
        rank: 3,
        token_estimate: 90_000,
        ..Default::default()
    });
    let mut led = Ledger::default();
    led.entries.insert(
        "S-1".into(),
        ledger_ticket("S-1", "Create a BOM", "Billing", "pending"),
    );
    led.entries.insert(
        "S-2".into(),
        ledger_ticket("S-2", "Scale quantities", "Billing", "pending"),
    );
    led.entries.insert(
        "S-3".into(),
        ledger_ticket("S-3", "Receive stock", "Inventory", "pending"),
    );
    let mut q = JobQueue::default();
    let live = q.enqueue_full(JobKind::Steps, "p", "", "grok", "grok-4.5");
    q.set_status(&live.id, JobStatus::Paused);
    q.set_work(&live.id, "Billing", "S-1");
    let focus = shalt_core::tokens::work_focus(&board, &led, &q.jobs, "p", Some(&s.id));
    assert_eq!(focus.now.as_ref().unwrap().rid, "S-1");
    assert_eq!(focus.now.as_ref().unwrap().estimate, 120_000);
    assert_eq!(focus.now.as_ref().unwrap().forecast_secs, 180);
    let next: Vec<_> = focus.next.iter().map(|n| n.rid.as_str()).collect();
    assert!(next.contains(&"S-2"), "{next:?}");
    assert!(next.contains(&"S-3"), "{next:?}");
    assert!(!next.contains(&"S-1"), "{next:?}");
}

#[test]
fn design_job_without_rid_does_not_wear_the_next_ticket_agent() {
    let mut board = Board::default();
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-1".into(),
        rank: 1,
        backend: "grok".into(),
        model: "grok-4".into(),
        ..Default::default()
    });
    let mut led = Ledger::default();
    led.entries.insert(
        "S-1".into(),
        ledger_ticket("S-1", "Amend bumps revision", "lifecycle", "pending"),
    );
    let mut q = JobQueue::default();
    let live = q.enqueue_full(JobKind::Design, "p", "Draw every empty UI frame", "", "");
    q.set_status(&live.id, JobStatus::Running);
    let focus = shalt_core::tokens::work_focus(&board, &led, &q.jobs, "p", None);
    let now = focus.now.expect("now");
    assert_eq!(now.name, "Drawing storyboards");
    assert!(now.rid.is_empty(), "{}", now.rid);
    assert_ne!(now.model, "grok-4");
    assert_ne!(now.backend, "grok");
}

#[test]
fn live_job_agent_wins_over_the_ticket_assignment() {
    let mut board = Board::default();
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-1".into(),
        rank: 1,
        backend: "grok".into(),
        model: "grok-4".into(),
        ..Default::default()
    });
    let mut led = Ledger::default();
    led.entries.insert(
        "S-1".into(),
        ledger_ticket("S-1", "Create a BOM", "Billing", "pending"),
    );
    let mut q = JobQueue::default();
    let live = q.enqueue_full(JobKind::Steps, "p", "", "qwen", "qwen3.8:27b-mlx");
    q.set_status(&live.id, JobStatus::Running);
    q.set_work(&live.id, "Billing", "S-1");
    let now = shalt_core::tokens::work_focus(&board, &led, &q.jobs, "p", None)
        .now
        .expect("now");
    assert_eq!(now.rid, "S-1");
    assert_eq!(now.backend, "qwen");
    assert_eq!(now.model, "qwen3.8:27b-mlx");
}

#[test]
fn work_focus_idle_still_names_the_next_ticket() {
    let mut board = Board::default();
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-1".into(),
        rank: 1,
        token_estimate: 72_000,
        ..Default::default()
    });
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-2".into(),
        rank: 2,
        token_estimate: 90_000,
        ..Default::default()
    });
    let mut led = Ledger::default();
    led.entries.insert(
        "S-1".into(),
        ledger_ticket("S-1", "Create a BOM", "Billing", "pending"),
    );
    led.entries.insert(
        "S-2".into(),
        ledger_ticket("S-2", "Scale quantities", "Billing", "pending"),
    );
    let focus = shalt_core::tokens::work_focus(&board, &led, &[], "p", None);
    let now = focus.now.expect("idle now is the next ticket");
    assert_eq!(now.rid, "S-1");
    assert_eq!(now.name, "Create a BOM");
    assert_eq!(focus.next[0].rid, "S-2");
}

#[test]
fn stamp_forecasts_fills_missing_token_and_time_not_caps() {
    let mut board = Board::default();
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-1".into(),
        rank: 1,
        token_estimate: 0,
        time_estimate_secs: 0,
        backend: "qwen".into(),
        model: "qwen3.5:35b-128k".into(),
        ..Default::default()
    });
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-2".into(),
        rank: 2,
        token_estimate: 50_000,
        time_estimate_secs: 99,
        backend: "grok".into(),
        model: "grok-4".into(),
        ..Default::default()
    });
    let mut led = Ledger::default();
    led.entries.insert(
        "S-1".into(),
        ledger_ticket("S-1", "A", "Billing", "pending"),
    );
    led.entries.insert(
        "S-2".into(),
        ledger_ticket("S-2", "B", "Billing", "pending"),
    );
    let n = shalt_core::tokens::stamp_forecasts(&mut board, &led, &[], "p", &[]);
    assert!(n >= 2, "expected token+time on S-1, got {n}");
    assert!(board.items[0].token_estimate > 0);
    assert!(board.items[0].time_estimate_secs > 0);
    assert_eq!(board.items[1].token_estimate, 50_000, "do not overwrite");
    assert_eq!(board.items[1].time_estimate_secs, 99, "do not overwrite");
}

fn scenario(rid: &str, name: &str, steps: &[&str], examples: &[&str]) -> shalt_core::Scenario {
    shalt_core::Scenario {
        rid: Some(rid.into()),
        name: name.into(),
        keyword: "Scenario".into(),
        tags: vec![],
        steps: steps.iter().map(|s| s.to_string()).collect(),
        examples: examples.iter().map(|s| s.to_string()).collect(),
        feature_name: "F".into(),
        feature_file: "f.feature".into(),
        line: 1,
        tag_lines: vec![],
        rid_count: 1,
        inherited_tags: vec![],
        oracles: vec![],
    }
}

#[test]
fn stamp_forecasts_scales_with_scenario_size() {
    let features = vec![shalt_core::Feature {
        name: "F".into(),
        file: "f.feature".into(),
        tags: vec![],
        background: vec![],
        description: String::new(),
        scenarios: vec![
            scenario(
                "S-small",
                "Reject empty",
                &[
                    "Given product X",
                    "When I save with no lines",
                    "Then it is rejected",
                ],
                &[],
            ),
            scenario(
                "S-big",
                "Explode a multi-level BOM",
                &[
                    "Given raw material A",
                    "And raw material B",
                    "And product P is manufacturable",
                    "And a manufacturing BOM includes:",
                    "| component | quantity | unit |",
                    "| A | 1.2 | m |",
                    "| B | 0.05 | L |",
                    "When I explode 3 units",
                    "Then the scaled requirement for A is 3.6 m",
                    "And the scaled requirement for B is 0.15 L",
                    "And sub-assemblies are included",
                ],
                &[],
            ),
        ],
    }];
    let mut board = Board::default();
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-small".into(),
        rank: 1,
        token_estimate: 100_000,
        time_estimate_secs: 180,
        ..Default::default()
    });
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-big".into(),
        rank: 2,
        token_estimate: 100_000,
        time_estimate_secs: 180,
        ..Default::default()
    });
    let led = Ledger::default();
    shalt_core::tokens::stamp_forecasts(&mut board, &led, &[], "p", &features);
    let small = board.items.iter().find(|i| i.rid == "S-small").unwrap();
    let big = board.items.iter().find(|i| i.rid == "S-big").unwrap();
    assert!(
        big.token_estimate > small.token_estimate,
        "big {} vs small {}",
        big.token_estimate,
        small.token_estimate
    );
    assert!(
        big.time_estimate_secs > small.time_estimate_secs,
        "big {}s vs small {}s",
        big.time_estimate_secs,
        small.time_estimate_secs
    );
    assert_ne!(small.token_estimate, 100_000);
    assert_ne!(big.token_estimate, 100_000);
}

#[test]
fn prepare_board_stamps_forecasts_from_the_spec() {
    let features = vec![shalt_core::Feature {
        name: "F".into(),
        file: "f.feature".into(),
        tags: vec![],
        background: vec![],
        description: String::new(),
        scenarios: vec![scenario(
            "S-1",
            "Reject empty",
            &[
                "Given product X",
                "When I save with no lines",
                "Then it is rejected",
            ],
            &[],
        )],
    }];
    let mut board = Board::default();
    board.pool = shalt_core::alloc::default_pool();
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-1".into(),
        rank: 1,
        ..Default::default()
    });
    let led = Ledger::default();
    assert!(shalt_core::alloc::prepare_board(
        &mut board, &led, &[], "p", &features
    ));
    assert!(
        board.items[0].token_estimate > 0 && board.items[0].token_estimate != 100_000,
        "spec-sized forecast, got {}",
        board.items[0].token_estimate
    );
    assert!(board.items[0].time_estimate_secs > 0);
}

#[test]
fn org_pause_flashes_a_reason() {
    let t = TempDir::new().unwrap();
    let proj = t.path().join("alpha");
    fs::create_dir_all(&proj).unwrap();
    let mut org = Org {
        name: "local".into(),
        projects: vec![],
    };
    let p = org.add(&proj).unwrap();
    assert!(org.pause(&p.id, true, Some(shalt_core::org::YOU_PAUSED)));
    let got = org.get(&p.id).unwrap();
    assert!(got.paused);
    assert_eq!(got.pause_reason, shalt_core::org::YOU_PAUSED);
    assert_eq!(got.notice, shalt_core::org::YOU_PAUSED);
    org.clear_notice(&p.id);
    assert!(org.get(&p.id).unwrap().notice.is_empty());
    assert_eq!(org.get(&p.id).unwrap().pause_reason, shalt_core::org::YOU_PAUSED);
    org.pause(&p.id, false, None);
    assert!(!org.get(&p.id).unwrap().paused);
    assert!(org.get(&p.id).unwrap().pause_reason.is_empty());
}

#[test]
fn org_yolo_is_off_by_default_and_toggles() {
    let t = TempDir::new().unwrap();
    let proj = t.path().join("yolo-erp");
    fs::create_dir_all(&proj).unwrap();
    let mut org = Org {
        name: "local".into(),
        projects: vec![],
    };
    let p = org.add(&proj).unwrap();
    assert!(!p.yolo);
    assert!(!org.get(&p.id).unwrap().yolo);
    assert!(org.set_yolo(&p.id, true));
    assert!(org.get(&p.id).unwrap().yolo);
    assert!(org.set_yolo(&p.id, false));
    assert!(!org.get(&p.id).unwrap().yolo);
    assert_eq!(org.get(&p.id).unwrap().yolo_mode_enum(), shalt_core::YoloMode::Off);
    assert!(org.set_yolo_mode(&p.id, shalt_core::YoloMode::Plan));
    assert!(!org.get(&p.id).unwrap().yolo, "plan is not all-questions");
    assert_eq!(org.get(&p.id).unwrap().yolo_mode_enum(), shalt_core::YoloMode::Plan);
    assert!(shalt_core::YoloMode::Plan.plans());
    assert!(!shalt_core::YoloMode::Plan.asks_all());
    assert!(shalt_core::YoloMode::All.asks_all());
}

#[test]
fn explain_pauses_backfills_silent_parks() {
    let t = TempDir::new().unwrap();
    let proj = t.path().join("erp");
    fs::create_dir_all(&proj).unwrap();
    let mut org = Org {
        name: "local".into(),
        projects: vec![],
    };
    let p = org.add(&proj).unwrap();
    org.set_paused(&p.id, true);
    assert!(org.get(&p.id).unwrap().pause_reason.is_empty());
    assert!(org.explain_pauses());
    let got = org.get(&p.id).unwrap();
    assert!(!got.pause_reason.is_empty());
    assert!(!got.notice.is_empty());
    org.clear_notice(&p.id);
    assert!(!org.explain_pauses(), "do not re-flash after dismiss");
    assert!(org.get(&p.id).unwrap().notice.is_empty());
}

#[test]
fn allocate_starts_random_then_prefers_cheap_local_when_fits_are_close() {
    let mut board = Board::default();
    for i in 1..=8 {
        board.items.push(shalt_core::board::BoardItem {
            rid: format!("S-{i}"),
            rank: i,
            token_estimate: 100_000,
            ..Default::default()
        });
    }
    let led = Ledger::default();
    let mut rng = rand::rngs::StdRng::seed_from_u64(1);
    let n = shalt_core::alloc::allocate_unassigned(&mut board, &led, &[], "p", &mut rng);
    assert_eq!(n.len(), 8);
    let qwen = n.iter().filter(|a| a.backend == "qwen").count();
    let grok = n.iter().filter(|a| a.backend == "grok").count();
    assert!(qwen >= 1 && grok >= 1, "random mix, qwen={qwen} grok={grok}");
    assert!(n.iter().any(|a| a.why.contains("explore")), "{:?}", n[0].why);

    let kept = board.items[0].backend.clone();
    let kept_rid = board.items[0].rid.clone();
    for it in board.items.iter_mut().skip(1) {
        it.backend.clear();
        it.model.clear();
    }
    let n2 = shalt_core::alloc::allocate_unassigned(&mut board, &led, &[], "p", &mut rng);
    assert!(n2.iter().all(|a| a.rid != kept_rid), "must not overwrite a human/prior assignment");
    assert_eq!(board.items[0].backend, kept);
}

#[test]
fn open_sprint_seats_backlog_and_prepare_assigns_agents() {
    let mut board = Board::default();
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-1".into(),
        rank: 1,
        ..Default::default()
    });
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-2".into(),
        rank: 2,
        sprint_id: Some("C-old".into()),
        ..Default::default()
    });
    let n = board.seat_on_open_sprint();
    assert_eq!(n, 1, "only unseated tickets join the open sprint");
    let sid = board.active_sprint().unwrap().id.clone();
    assert_eq!(board.items[0].sprint_id.as_deref(), Some(sid.as_str()));
    assert_eq!(board.items[1].sprint_id.as_deref(), Some("C-old"));
    let led = Ledger::default();
    assert!(shalt_core::alloc::prepare_board(&mut board, &led, &[], "p", &[]));
    assert!(
        board.items.iter().all(|i| !i.backend.is_empty()),
        "every ticket gets an agent, {:?}",
        board.items
    );
}

#[test]
fn prepare_board_keeps_sprints_to_a_small_slice() {
    let mut board = Board::default();
    board.pool = shalt_core::alloc::default_pool();
    for i in 1..=12 {
        board.items.push(shalt_core::board::BoardItem {
            rid: format!("S-{i}"),
            rank: i,
            token_estimate: 40_000,
            ..Default::default()
        });
    }
    let led = Ledger::default();
    assert!(shalt_core::alloc::prepare_board(&mut board, &led, &[], "p", &[]));
    let sid = board.active_sprint().unwrap().id.clone();
    let on = board
        .items
        .iter()
        .filter(|i| i.sprint_id.as_deref() == Some(sid.as_str()))
        .count();
    assert_eq!(on, shalt_core::SPRINT_TICKET_CAP, "sprint must stay small");
    assert_eq!(
        board.items.iter().filter(|i| i.sprint_id.is_none()).count(),
        12 - shalt_core::SPRINT_TICKET_CAP
    );
}

#[test]
fn reallocate_skips_a_ticket_that_is_already_running() {
    let mut board = Board::default();
    board.pool = shalt_core::alloc::default_pool();
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-live".into(),
        rank: 1,
        backend: "qwen".into(),
        model: "keep-me".into(),
        ..Default::default()
    });
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-idle".into(),
        rank: 2,
        backend: "qwen".into(),
        model: "old".into(),
        ..Default::default()
    });
    let mut q = JobQueue::default();
    let j = q.enqueue_full(
        shalt_core::jobs::JobKind::Build,
        "p",
        "",
        "qwen",
        "keep-me",
    );
    q.set_work(&j.id, "", "S-live");
    q.set_status(&j.id, JobStatus::Running);
    let mut rng = rand::rngs::StdRng::seed_from_u64(3);
    let changed = shalt_core::alloc::reallocate(
        &mut board,
        &Ledger::default(),
        &q.jobs,
        "p",
        &mut rng,
    );
    assert!(changed.iter().all(|c| c.rid != "S-live"), "{changed:?}");
    assert_eq!(board.items[0].model, "keep-me");
    assert!(changed.iter().any(|c| c.rid == "S-idle"));
}

#[test]
fn allocate_cheap_picks_local_when_accuracy_is_similar() {
    let mut board = Board::default();
    board.prefer = "cheap".into();
    board.pool = shalt_core::alloc::default_pool();
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-new".into(),
        rank: 9,
        token_estimate: 100_000,
        ..Default::default()
    });
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-q".into(),
        rank: 1,
        token_estimate: 100_000,
        backend: "qwen".into(),
        model: "qwen3.5:35b-128k".into(),
        ..Default::default()
    });
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-g".into(),
        rank: 2,
        token_estimate: 100_000,
        backend: "grok".into(),
        model: "grok-4".into(),
        ..Default::default()
    });
    let mut led = Ledger::default();
    led.entries.insert("S-q".into(), ledger_ticket("S-q", "A", "Billing", "green"));
    led.entries.insert("S-g".into(), ledger_ticket("S-g", "B", "Billing", "green"));
    led.entries.insert("S-new".into(), ledger_ticket("S-new", "C", "Billing", "pending"));
    let mut q = JobQueue::default();
    for (rid, backend, model, ptok, ctok) in [
        ("S-q", "qwen", "qwen3.5:35b-128k", 50_000, 40_000),
        ("S-q", "qwen", "qwen3.5:35b-128k", 50_000, 40_000),
        ("S-g", "grok", "grok-4", 50_000, 40_000),
        ("S-g", "grok", "grok-4", 50_000, 40_000),
    ] {
        let j = q.enqueue_full(JobKind::Build, "p", "", backend, model);
        q.set_work(&j.id, "Billing", rid);
        q.add_tokens(&j.id, ptok, ctok);
        q.set_status(&j.id, JobStatus::Done);
    }
    let mut rng = rand::rngs::StdRng::seed_from_u64(7);
    let mut last = None;
    for _ in 0..8 {
        let pick = shalt_core::alloc::pick_for(&board, &led, &q.jobs, "p", "Billing", &mut rng);
        last = Some(pick);
        if last.as_ref().unwrap().why.contains("fit") {
            break;
        }
    }
    let pick = last.unwrap();
    if pick.why.contains("fit") {
        assert_eq!(pick.backend, "qwen", "cheap + similar accuracy → local, {:?}", pick.why);
    }
}

#[test]
fn agent_roster_is_not_grok_only() {
    let roster = shalt_core::agent_roster();
    let ids: Vec<_> = roster.iter().map(|a| a.id.as_str()).collect();
    assert!(ids.contains(&"qwen"), "{ids:?}");
    assert!(ids.contains(&"grok"), "{ids:?}");
    assert!(ids.contains(&"openai"), "{ids:?}");
    assert!(ids.contains(&"claude"), "{ids:?}");
    assert!(!ids.contains(&"codex"), "codex is not a supported OpenAI model: {ids:?}");
    let models = shalt_core::roster_models();
    assert!(models.iter().any(|m| m.backend == "qwen"), "{models:?}");
    assert!(
        models.iter().any(|m| m.backend == "qwen")
            && models.iter().any(|m| m.backend == "grok"),
        "assignment list must include local and cloud, got {models:?}"
    );
    let qwen = roster.iter().find(|a| a.id == "qwen").unwrap();
    assert_eq!(qwen.default_model, shalt_core::DEFAULT_QWEN_MODEL);
    assert!(
        qwen.models.iter().any(|m| m == "qwen3.8:27b-mlx"),
        "{:?}",
        qwen.models
    );
    let pool = shalt_core::alloc::default_pool();
    assert!(
        pool.iter().any(|s| s.backend == "qwen" && s.model == "qwen3.8:27b-mlx"),
        "{pool:?}"
    );
    assert!(pool.iter().any(|s| s.backend == "grok"), "{pool:?}");
}

#[test]
fn local_qwen_waits_minutes_not_forty_five_seconds() {
    let b = shalt_core::OpenAICompatBackend::from_preset("qwen", Some("qwen3.8:27b-mlx"), None)
        .unwrap();
    assert!(
        b.timeout_secs >= 600,
        "27B MLX writes take minutes; a 45s HTTP abort kills a live generation, got {}s",
        b.timeout_secs
    );
    let fast = shalt_core::OpenAICompatBackend::from_preset("qwen", Some("qwen3.5:2b-mlx"), None)
        .unwrap();
    assert!(
        fast.timeout_secs <= 300,
        "tiny local models should fail fast, got {}s",
        fast.timeout_secs
    );
}

#[test]
fn grok_failover_is_for_local_stalls_when_grok_is_ready() {
    let qwen = shalt_core::jobs::JobQueue::default().enqueue_full(
        shalt_core::jobs::JobKind::Steps,
        "p",
        "",
        "qwen",
        "qwen3.8:27b-mlx",
    );
    let grok = shalt_core::jobs::JobQueue::default().enqueue_full(
        shalt_core::jobs::JobKind::Steps,
        "p",
        "",
        "grok",
        "grok-4",
    );
    assert!(shalt_core::looks_like_local_stall(
        "the model didn't respond in time. Retry when Ollama is free."
    ));
    assert!(shalt_core::looks_like_local_stall(
        "could not reach http://127.0.0.1:11434/v1: Connection refused"
    ));
    assert!(!shalt_core::looks_like_local_stall("stopped (Paused)"));
    assert!(!shalt_core::looks_like_local_stall("stepwright wrote no test files"));
    assert!(
        shalt_core::grok_failover_target_if(&qwen, true).is_some(),
        "qwen stall + grok key → fail over"
    );
    assert!(
        shalt_core::grok_failover_target_if(&qwen, false).is_none(),
        "no grok key → do not fail over"
    );
    assert!(
        shalt_core::grok_failover_target_if(&grok, true).is_none(),
        "already on grok → do not loop"
    );
}

#[test]
fn grok_quota_is_remembered_so_we_stop_picking_it() {
    let mut q = JobQueue::default();
    let j = q.enqueue_full(
        JobKind::Steps,
        "p",
        "",
        "grok",
        "grok-4",
    );
    q.append(
        &j.id,
        "failed: xAI refused the request: this team is out of credits or hit its spending limit.",
    );
    assert!(shalt_core::backend_quota_exhausted(&q.jobs, "grok"));
    assert!(!shalt_core::backend_quota_exhausted(&q.jobs, "qwen"));
}

#[test]
fn cloud_quota_fails_over_to_local_qwen() {
    let grok = shalt_core::jobs::JobQueue::default().enqueue_full(
        shalt_core::jobs::JobKind::Steps,
        "p",
        "",
        "grok",
        "grok-4",
    );
    let qwen = shalt_core::jobs::JobQueue::default().enqueue_full(
        shalt_core::jobs::JobKind::Steps,
        "p",
        "",
        "qwen",
        "qwen3.8:27b-mlx",
    );
    assert!(shalt_core::looks_like_cloud_quota(
        "xAI refused the request: this team is out of credits or hit its spending limit."
    ));
    assert!(!shalt_core::looks_like_cloud_quota("stepwright wrote no test files"));
    let installed = vec!["qwen3.8:27b-mlx".into(), "llama3".into()];
    let hit = shalt_core::local_failover_target_if(&grok, true, &installed).expect("failover");
    assert_eq!(hit.0, "qwen");
    assert_eq!(hit.1, "qwen3.8:27b-mlx");
    assert!(
        shalt_core::local_failover_target_if(&qwen, true, &installed).is_none(),
        "already local → do not loop"
    );
    assert!(shalt_core::local_failover_target_if(&grok, false, &installed).is_none());
    assert_eq!(
        shalt_core::pick_local_model(&["mistral".into(), "qwen2.5:14b".into()]),
        "qwen2.5:14b"
    );
    let with_fast = vec![
        "qwen3.8:27b-mlx".into(),
        "qwen3.5:2b-mlx".into(),
        "qwen3:8b".into(),
    ];
    assert_eq!(
        shalt_core::pick_local_model(&with_fast),
        "qwen3.5:2b-mlx",
        "inner loop must not failover onto 27B when a tiny model is installed"
    );
    let grok_fast = shalt_core::local_failover_target_if(&grok, true, &with_fast).expect("fast");
    assert_eq!(grok_fast.1, "qwen3.5:2b-mlx");
}

#[test]
fn missing_cloud_key_is_not_always_xai() {
    assert!(shalt_core::looks_like_missing_key(
        "no API key for 'claude': set ANTHROPIC_API_KEY, or put it in ~/.shalt/config.toml under [keys]"
    ));
    assert!(shalt_core::looks_like_missing_key(
        "No Anthropic API key. Put it in ~/.shalt/config.toml or set ANTHROPIC_API_KEY."
    ));
    assert!(!shalt_core::looks_like_missing_key("stepwright wrote no test files"));
    let claude = shalt_core::jobs::human_error(
        "no API key for 'claude': set ANTHROPIC_API_KEY, or put it in ~/.shalt/config.toml under [keys]",
    );
    assert!(
        claude.contains("Anthropic"),
        "Claude missing key must not be labeled xAI, got {claude}"
    );
    assert!(!claude.to_lowercase().contains("xai"));
    let grok = shalt_core::jobs::human_error(
        "no API key for 'grok': set XAI_API_KEY, or put it in ~/.shalt/config.toml under [keys]",
    );
    assert!(grok.contains("xAI"), "{grok}");
    assert!(shalt_core::backend_key_ready("qwen"));
    assert!(shalt_core::backend_key_ready(""));
    with_shalt_home(|| {
        let t = TempDir::new().unwrap();
        std::env::set_var("SHALT_HOME", t.path());
        let prev = std::env::var("ANTHROPIC_API_KEY").ok();
        std::env::remove_var("ANTHROPIC_API_KEY");
        assert!(
            !shalt_core::backend_key_ready("claude"),
            "Claude with no Anthropic key must be unusable"
        );
        match prev {
            Some(v) => std::env::set_var("ANTHROPIC_API_KEY", v),
            None => std::env::remove_var("ANTHROPIC_API_KEY"),
        }
    });
}

#[test]
fn suggest_assignment_prefers_the_agent_closer_to_estimate() {
    let mut board = Board::default();
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-1".into(),
        rank: 1,
        token_estimate: 100_000,
        ..Default::default()
    });
    board.items.push(shalt_core::board::BoardItem {
        rid: "S-2".into(),
        rank: 2,
        token_estimate: 100_000,
        ..Default::default()
    });
    let mut led = Ledger::default();
    led.entries.insert("S-1".into(), ledger_ticket("S-1", "A", "Billing", "green"));
    led.entries.insert("S-2".into(), ledger_ticket("S-2", "B", "Billing", "green"));
    let mut q = JobQueue::default();
    let qwen = q.enqueue_full(JobKind::Build, "p", "", "qwen", "qwen3.5:35b");
    q.set_work(&qwen.id, "Billing", "S-1");
    q.add_tokens(&qwen.id, 50_000, 30_000); // 0.8×
    let grok = q.enqueue_full(JobKind::Build, "p", "", "grok", "grok-4");
    q.set_work(&grok.id, "Billing", "S-2");
    q.add_tokens(&grok.id, 120_000, 70_000); // 1.9×
    let fits = shalt_core::tokens::agent_fits(&board, &q.jobs, &led, "p");
    let pick = shalt_core::tokens::suggest_assignment(&fits, "Billing").unwrap();
    assert_eq!(pick.backend, "qwen", "{fits:?}");
    let next = shalt_core::tokens::suggest_estimate_for(&fits, "qwen", "qwen3.5:35b", "Billing", &board);
    assert!(next < shalt_core::tokens::DEFAULT_TICKET_TOKENS, "under-spend should lower the estimate, got {next}");
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
        ..Default::default()
    });
    let drift = shalt_core::board::verify_drift(&board, &features);
    assert!(drift.iter().any(|p| p.contains("S-deadbeef")));
}

#[test]
fn ui_registry_prunes_dead_pids() {
    with_shalt_home(|| {
        let t = TempDir::new().unwrap();
        std::env::set_var("SHALT_HOME", t.path());
        shalt_core::uis::record(shalt_core::uis::UiInstance {
            pid: 999_999_999,
            port: 7700,
            url: "http://127.0.0.1:7700/".into(),
            root: "/tmp".into(),
            started_at: "now".into(),
        });
        assert!(
            shalt_core::uis::current().is_none(),
            "dead pid must not count as a live shalt ui"
        );
    });
}

#[test]
fn wait_until_up_is_none_when_nothing_listens() {
    with_shalt_home(|| {
        let t = TempDir::new().unwrap();
        std::env::set_var("SHALT_HOME", t.path());
        assert!(
            shalt_core::uis::wait_until_up(std::time::Duration::from_millis(40)).is_none(),
            "an empty SHALT_HOME must not pick up some other desk on the machine"
        );
    });
}

#[test]
fn org_rename_and_remove_are_catalog_only() {
    let t = TempDir::new().unwrap();
    let proj = t.path().join("rivleterp");
    fs::create_dir_all(&proj).unwrap();
    let mut org = Org {
        name: "local".into(),
        projects: vec![],
    };
    let p = org.add(&proj).unwrap();
    assert_eq!(p.name, "rivleterp");
    assert!(org.rename(&p.id, "Rivlet ERP"));
    assert_eq!(org.get(&p.id).unwrap().name, "Rivlet ERP");
    assert!(!org.rename(&p.id, "   "), "empty name is rejected");
    assert!(!org.rename("no-such", "X"));
    assert!(org.remove(&p.id));
    assert!(org.get(&p.id).is_none());
    assert!(proj.exists(), "remove must not delete the git workspace");
}

#[test]
fn org_pause_parks_running_jobs_not_waiting() {
    let mut org = Org {
        name: "local".into(),
        projects: vec![],
    };
    let t = TempDir::new().unwrap();
    let proj = t.path().join("p");
    fs::create_dir_all(&proj).unwrap();
    let p = org.add(&proj).unwrap();
    assert!(!p.paused);
    assert!(org.set_paused(&p.id, true));
    assert!(org.get(&p.id).unwrap().paused);
    let mut q = JobQueue::default();
    let run = q.enqueue(JobKind::Author, &p.id);
    let wait = q.enqueue(JobKind::Author, &p.id);
    q.set_status(&run.id, JobStatus::Running);
    q.set_status(&wait.id, JobStatus::Waiting);
    let paused = q.pause_project(&p.id);
    assert_eq!(paused, vec![run.id.clone()]);
    assert_eq!(q.get(&run.id).unwrap().status, JobStatus::Paused);
    assert_eq!(q.get(&wait.id).unwrap().status, JobStatus::Waiting);
    assert_eq!(q.resumable_for_project(&p.id), vec![run.id]);
}

#[test]
fn inbox_uses_the_latest_job_not_an_old_failure() {
    let mut q = JobQueue::default();
    let old = q.enqueue(JobKind::Author, "p");
    q.set_status(&old.id, JobStatus::Failed);
    q.set_error(&old.id, "old author failed");
    let done = q.enqueue(JobKind::Steps, "p");
    q.set_status(&done.id, JobStatus::Done);
    let jobs: Vec<_> = q.jobs.iter().collect();
    let st = shalt_core::jobs::summarize_jobs(&jobs, false);
    assert_eq!(st.state, "idle", "a later done job must clear an old failure");

    let build = q.enqueue(JobKind::Build, "p");
    q.set_error(&build.id, "the test harness failed to run\nthe python stack needs pytest-bdd\n  python3 -m pip install pytest pytest-bdd");
    q.set_status(&build.id, JobStatus::Failed);
    let jobs: Vec<_> = q.jobs.iter().collect();
    let st = shalt_core::jobs::summarize_jobs(&jobs, false);
    assert_eq!(st.state, "failed");
    assert_eq!(st.phase, "build");
    assert!(
        st.detail.contains("pytest-bdd") || st.detail.contains("pip install"),
        "inbox must say why it failed, got {:?}",
        st.detail
    );
    let issue = st.issue.expect("failed job must carry an issue");
    assert_eq!(issue.job_id, build.id);
    assert!(
        issue.steps.iter().any(|s| s.to_lowercase().contains("rust")),
        "{:?}",
        issue.steps
    );
    assert!(!issue.log.is_empty() || !issue.title.is_empty());
}

#[test]
fn diagnose_spend_limit_points_at_qwen() {
    let mut q = JobQueue::default();
    let id = q.enqueue_full(JobKind::Author, "p", "", "grok", "grok-4").id;
    q.set_error(&id, "spending limit reached");
    q.set_status(&id, JobStatus::Failed);
    let issue = shalt_core::jobs::diagnose(q.get(&id).unwrap());
    assert!(issue.title.to_lowercase().contains("credit") || issue.title.to_lowercase().contains("spending"), "{}", issue.title);
    assert!(issue.steps.iter().any(|s| s.to_lowercase().contains("qwen")), "{:?}", issue.steps);
    assert!(
        issue.steps.iter().any(|s| s.to_lowercase().contains("retry")),
        "{:?}",
        issue.steps
    );
}

#[test]
fn play_one_project_parks_the_other() {
    let mut q = JobQueue::default();
    let a = q.enqueue(JobKind::Author, "alpha");
    let b = q.enqueue(JobKind::Author, "beta");
    q.set_status(&a.id, JobStatus::Running);
    q.set_status(&b.id, JobStatus::Pending);
    assert_eq!(q.pause_project("alpha"), vec![a.id.clone()]);
    assert_eq!(q.get(&a.id).unwrap().status, JobStatus::Paused);
    assert_eq!(q.get(&b.id).unwrap().status, JobStatus::Pending);
    assert_eq!(q.pause_project("beta"), vec![b.id.clone()]);
    assert_eq!(q.get(&b.id).unwrap().status, JobStatus::Paused);
    for id in q.resumable_for_project("alpha") {
        q.set_status(&id, JobStatus::Running);
    }
    assert_eq!(q.get(&a.id).unwrap().status, JobStatus::Running);
    assert_eq!(q.get(&b.id).unwrap().status, JobStatus::Paused);
    assert!(
        q.get(&a.id)
            .unwrap()
            .log
            .contains("another project took the model slot")
            || q.get(&a.id)
                .unwrap()
                .log
                .contains("paused so another project can use the model"),
        "{}",
        q.get(&a.id).unwrap().log
    );
}

#[test]
fn pause_all_parks_running_jobs_and_leaves_waiting() {
    let mut q = JobQueue::default();
    let run = q.enqueue(JobKind::Build, "alpha");
    let pend = q.enqueue(JobKind::Steps, "beta");
    let wait = q.enqueue(JobKind::Author, "gamma");
    q.set_status(&run.id, JobStatus::Running);
    q.set_status(&pend.id, JobStatus::Pending);
    q.set_status(&wait.id, JobStatus::Waiting);
    let parked = q.pause_all("paused — shalt stop\n");
    assert_eq!(parked.len(), 2);
    assert_eq!(q.get(&run.id).unwrap().status, JobStatus::Paused);
    assert_eq!(q.get(&pend.id).unwrap().status, JobStatus::Paused);
    assert_eq!(q.get(&wait.id).unwrap().status, JobStatus::Waiting);
    assert!(q.get(&run.id).unwrap().log.contains("shalt stop"));
}

#[test]
fn org_pause_all_skips_already_paused() {
    let t = TempDir::new().unwrap();
    let a = t.path().join("alpha");
    let b = t.path().join("beta");
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    let mut org = Org {
        name: "local".into(),
        projects: vec![],
    };
    let pa = org.add(&a).unwrap();
    let pb = org.add(&b).unwrap();
    assert!(org.pause(&pa.id, true, Some(shalt_core::org::YOU_PAUSED)));
    let changed = org.pause_all(shalt_core::org::YOU_STOPPED);
    assert_eq!(changed, vec![pb.id.clone()]);
    assert_eq!(
        org.get(&pa.id).unwrap().pause_reason,
        shalt_core::org::YOU_PAUSED
    );
    assert_eq!(
        org.get(&pb.id).unwrap().pause_reason,
        shalt_core::org::YOU_STOPPED
    );
    assert!(org.get(&pb.id).unwrap().paused);
}

#[test]
fn job_ask_waits_for_an_answer() {
    let t = TempDir::new().unwrap();
    let path = t.path().join("jobs.json");
    let mut q = JobQueue::default();
    let job = q.enqueue_full(JobKind::Author, "invoice", "do billing", "qwen", "qwen3.5:2b");
    q.ask(&job.id, "Which currency?", "USD");
    q.save_to(&path).unwrap();
    let mut q2 = JobQueue::load_from(&path);
    assert_eq!(q2.jobs[0].status, JobStatus::Waiting);
    assert!(q2.take_answer(&job.id).is_none());
    q2.set_answer(&job.id, "USD");
    assert_eq!(q2.take_answer(&job.id).as_deref(), Some("USD"));
    assert_eq!(q2.jobs[0].status, JobStatus::Running);
    assert_eq!(q2.jobs[0].turns.len(), 1);
    assert_eq!(q2.jobs[0].turns[0].question, "Which currency?");
    assert_eq!(q2.jobs[0].turns[0].answer, "USD");
    assert_eq!(q2.jobs[0].turns[0].guess, "USD");
}

#[test]
fn answering_a_turn_writes_the_plain_language_spec() {
    let mut q = JobQueue::default();
    let job = q.enqueue_full(
        JobKind::Author,
        "invoice",
        "Build an ERP for manufacturers.",
        "qwen",
        "qwen",
    );
    q.ask(
        &job.id,
        "For bills of materials (BOMs), what should a typical example look like?",
        "a bike",
    );
    q.append_chat(&job.id, "user", "Reference odoo");
    q.append_chat(
        &job.id,
        "assistant",
        "Odoo uses multi-level BOMs.\nANSWER: A BOM is a product plus components with quantity and unit. Components may themselves have BOMs.",
    );
    assert!(q.adopt_chat(&job.id, None));
    let j = q.get(&job.id).unwrap();
    assert!(j.prompt.contains("Build an ERP for manufacturers."));
    assert!(j.prompt.contains("bills of materials"));
    assert!(j.prompt.contains("Components may themselves have BOMs"));
    assert!(j.turns[0].answer.contains("A BOM is a product"));
    assert_eq!(j.status, JobStatus::Running);
}

#[test]
fn fold_into_spec_replaces_the_same_heading() {
    let once = shalt_core::jobs::fold_into_spec("Build an ERP.", "Which currency?", "USD");
    let twice = shalt_core::jobs::fold_into_spec(&once, "Which currency?", "EUR");
    assert!(twice.contains("EUR"), "{twice}");
    assert!(!twice.contains("USD"), "{twice}");
    assert!(twice.contains("Build an ERP."));
}

#[test]
fn job_status_line_explains_waiting() {
    let mut q = JobQueue::default();
    let job = q.enqueue(JobKind::Author, "invoice");
    q.ask(
        &job.id,
        "For bills of materials (BOMs), what should a typical example look like?",
        "a bike frame with two wheels",
    );
    let j = q.get(&job.id).unwrap();
    let line = shalt_core::jobs::status_line(j);
    assert!(line.contains("Waiting for your answer"), "{line}");
    assert!(line.contains("bills of materials"), "{line}");
    assert!(j.log.contains("waiting on you:"), "{}", j.log);
    assert!(
        !j.log.contains("a bike frame"),
        "guess is not the activity line: {}",
        j.log
    );
}

#[test]
fn job_progress_keeps_heartbeats_not_file_dumps() {
    assert!(shalt_core::jobs::progress_line("step 1: waiting on the model…"));
    assert!(shalt_core::jobs::progress_line("still waiting on the model…"));
    assert!(shalt_core::jobs::progress_line(
        "still waiting on qwen3.8:27b-mlx… 1m 12s"
    ));
    assert!(shalt_core::jobs::progress_line(
        "thinking · qwen3.8:27b-mlx · 16s"
    ));
    assert!(shalt_core::jobs::progress_line(
        "step 4 of 16 · qwen3.8:27b-mlx is writing tests…"
    ));
    assert!(shalt_core::jobs::progress_line(
        "qwen didn't respond in time — failing over to grok grok-4"
    ));
    assert!(shalt_core::jobs::progress_line("contacting grok-4.5…"));
    assert!(shalt_core::jobs::progress_line(
        "[write_file] wrote spec/x.feature (12 bytes)"
    ));
    assert!(shalt_core::jobs::progress_line("waiting on you: For BOMs"));
    assert!(!shalt_core::jobs::progress_line(
        "[read_file] Feature: Money\n  Scenario: Add"
    ));
    assert!(!shalt_core::jobs::progress_line(
        "Here is a long assistant essay about manufacturing."
    ));
    assert!(shalt_core::jobs::heartbeat_line(
        "still waiting on qwen3.8:27b-mlx… 1m 12s"
    ));
    assert!(shalt_core::jobs::heartbeat_line(
        "thinking · qwen3.8:27b-mlx · 16s"
    ));
    assert!(!shalt_core::jobs::heartbeat_line(
        "step 4 of 16 · qwen3.8:27b-mlx is writing tests…"
    ));
    assert!(!shalt_core::jobs::heartbeat_line(
        "[write_file] wrote steps/patrons.steps.js (120 bytes)"
    ));
}

#[test]
fn job_status_and_activity_skip_heartbeat_spam() {
    let mut q = JobQueue::default();
    let job = q.enqueue_full(
        JobKind::Steps,
        "recipes",
        "tests for journey patrons",
        "qwen",
        "qwen3.8:27b-mlx",
    );
    q.set_status(&job.id, JobStatus::Running);
    q.append(&job.id, "writing real tests for journey patrons (0 already bound)");
    q.append(&job.id, "step 3 of 16 · qwen3.8:27b-mlx is writing tests…");
    q.append(&job.id, "[write_file] wrote steps/patronage.steps.js (120 bytes)");
    q.append(&job.id, "still waiting on qwen3.8:27b-mlx… 12s");
    q.append(&job.id, "thinking · qwen3.8:27b-mlx · 16s");
    q.append(&job.id, "step 4 of 16 · qwen3.8:27b-mlx is writing tests…");
    q.append(&job.id, "thinking · qwen3.8:27b-mlx · 4s");
    let j = q.get(&job.id).unwrap();
    let line = shalt_core::jobs::status_line(j);
    assert!(
        line.contains("step 4 of 16") && line.contains("writing tests"),
        "{line}"
    );
    assert!(!line.contains("still waiting"), "{line}");
    assert!(!line.contains("thinking ·"), "{line}");
    let act = shalt_core::jobs::activity_lines(&j.log, 8);
    assert!(
        act.iter().any(|l| l.contains("[write_file]")),
        "{act:?}"
    );
    assert!(
        act.iter().any(|l| l.contains("step 4 of 16")),
        "{act:?}"
    );
    assert!(
        act.iter().all(|l| !l.contains("still waiting") && !l.starts_with("thinking ·")),
        "{act:?}"
    );
}

#[test]
fn paused_job_is_not_marked_done() {
    let mut q = JobQueue::default();
    let job = q.enqueue_full(JobKind::Steps, "p", "", "qwen", "qwen3.5:35b-128k");
    q.set_status(&job.id, JobStatus::Running);
    q.set_agent(&job.id, "qwen", "qwen3.8:27b-mlx");
    assert_eq!(q.get(&job.id).unwrap().model, "qwen3.8:27b-mlx");
    q.set_status(&job.id, JobStatus::Paused);
    assert!(
        !q.set_status(&job.id, JobStatus::Done),
        "a late worker must not finish a paused job"
    );
    assert_eq!(q.get(&job.id).unwrap().status, JobStatus::Paused);
    assert!(
        !q.set_status(&job.id, JobStatus::Failed),
        "a late worker must not fail a paused job"
    );
    assert_eq!(q.get(&job.id).unwrap().status, JobStatus::Paused);
    assert!(q.set_status(&job.id, JobStatus::Running));
    assert_eq!(q.get(&job.id).unwrap().status, JobStatus::Running);
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
    with_shalt_home(|| {
        let t = TempDir::new().unwrap();
        std::env::set_var("SHALT_HOME", t.path());
        let (project, job) = shalt_core::start_project(shalt_core::ComposeRequest {
            prompt: "The system shall total invoices exactly.".into(),
            backend: "qwen".into(),
            model: "qwen3.5:2b".into(),
            name: Some("invoices".into()),
            dir: None,
            stack: String::new(),
        })
        .unwrap();
        assert_eq!(project.id, "invoices");
        assert!(Path::new(&project.path).join("spec").is_dir());
        assert!(
            !Path::new(&project.path).join("Cargo.toml").exists(),
            "language is picked after the plan, not at compose"
        );
        let cfg = std::fs::read_to_string(Path::new(&project.path).join("shalt.toml")).unwrap();
        assert!(cfg.contains("stack = \"\""), "{cfg}");
        assert!(!cfg.contains("stack = \"python\""), "{cfg}");
        assert_eq!(job.kind, shalt_core::jobs::JobKind::Author);
        assert_eq!(job.backend, "qwen");
        let plan = std::fs::read_to_string(Path::new(&project.path).join(".shalt/plan.md")).unwrap();
        assert!(plan.contains("invoice"), "{plan}");
    });
}

#[test]
fn compose_puts_a_new_project_in_the_folder_you_pick() {
    with_shalt_home(|| {
        let t = TempDir::new().unwrap();
        std::env::set_var("SHALT_HOME", t.path().join("home"));
        let dest = t.path().join("work").join("invoicing");
        fs::create_dir_all(&dest).unwrap();
        let (project, _) = shalt_core::start_project(shalt_core::ComposeRequest {
            prompt: "The system shall total invoices exactly.".into(),
            backend: "qwen".into(),
            model: "qwen3.5:2b".into(),
            name: Some("invoices".into()),
            dir: Some(dest.clone()),
            stack: String::new(),
        })
        .unwrap();
        assert_eq!(Path::new(&project.path), dest.canonicalize().unwrap_or(dest));
        assert!(Path::new(&project.path).join("shalt.toml").exists());
        assert!(Path::new(&project.path).join("spec").is_dir());
    });
}

#[test]
fn compose_makes_a_child_folder_when_the_pick_is_already_occupied() {
    with_shalt_home(|| {
        let t = TempDir::new().unwrap();
        std::env::set_var("SHALT_HOME", t.path().join("home"));
        let parent = t.path().join("work");
        fs::create_dir_all(parent.join("other")).unwrap();
        fs::write(parent.join("other/readme"), "keep\n").unwrap();
        let (project, _) = shalt_core::start_project(shalt_core::ComposeRequest {
            prompt: "The system shall total invoices exactly.".into(),
            backend: "qwen".into(),
            model: "qwen3.5:2b".into(),
            name: Some("invoices".into()),
            dir: Some(parent.clone()),
            stack: String::new(),
        })
        .unwrap();
        let path = Path::new(&project.path);
        assert_eq!(path.file_name().unwrap(), "invoices");
        assert_eq!(path.parent().unwrap(), parent.canonicalize().unwrap_or(parent));
        assert!(path.join("shalt.toml").exists());
    });
}

#[test]
fn plan_round_trips_on_disk() {
    let t = TempDir::new().unwrap();
    shalt_core::talk::save_plan(t.path(), "We shall invoice in the customer's currency.\n").unwrap();
    let got = shalt_core::talk::load_plan(t.path(), "fallback");
    assert!(got.contains("invoice"), "{got}");
}

#[test]
fn plan_pack_round_trips_into_a_new_project() {
    with_shalt_home(|| {
        let t = TempDir::new().unwrap();
        std::env::set_var("SHALT_HOME", t.path().join("home"));
        let src = t.path().join("src-proj");
        fs::create_dir_all(src.join("spec")).unwrap();
        shalt_core::config::init_workspace(&src, "rust", "src-proj").unwrap();
        shalt_core::talk::save_plan(&src, "We shall invoice in the customer's currency.\n").unwrap();
        fs::write(
            src.join("spec/billing.feature"),
            "Feature: Billing\n  Scenario: Total an invoice\n    Given an amount\n    Then it totals\n",
        )
        .unwrap();
        let pack = shalt_core::export_pack(&src, "Billing").unwrap();
        assert_eq!(pack.schema, shalt_core::pack::SCHEMA);
        assert!(pack.plan.contains("invoice"), "{}", pack.plan);
        assert!(pack.features.keys().any(|k| k.contains("billing.feature")));
        let dest = t.path().join("cloned");
        let project = shalt_core::import_pack(&pack, Some(&dest), Some("cloned")).unwrap();
        assert_eq!(project.id, "cloned");
        let root = Path::new(&project.path);
        assert!(
            root.join("spec").is_dir(),
            "imported project at {} missing spec",
            root.display()
        );
        assert!(root.join("shalt.toml").exists());
        let plan = shalt_core::talk::load_plan(root, "");
        assert!(plan.contains("invoice"), "{plan}");
        let features = load_specs(&root.join("spec"), false).unwrap();
        assert!(!features.is_empty());
        let board = Board::load(&root.join(".shalt/board.json"));
        assert!(!board.items.is_empty(), "imported spec should stamp tickets");
        assert!(
            board.items.iter().all(|i| i.token_estimate > 0 && i.time_estimate_secs > 0),
            "forecasts should be filled, not left for a human"
        );
    });
}

#[test]
fn restack_to_rust_removes_python_steps() {
    let t = TempDir::new().unwrap();
    shalt_core::config::init_workspace(t.path(), "python", "erp").unwrap();
    fs::create_dir_all(t.path().join("steps")).unwrap();
    fs::write(
        t.path().join("steps/bom_steps.py"),
        "from pytest_bdd import given\n",
    )
    .unwrap();
    fs::write(
        t.path().join("contract/interface.md"),
        "# Public API Contract\n\nStep definitions import only from `mrp` (`PYTHONPATH` via `src/`).\n\n```python\nfrom mrp import ManufacturingSystem\n```\n",
    )
    .unwrap();
    fs::write(t.path().join("spec/bom.feature"), "Feature: BOM\n  Scenario: A\n    Given x\n").unwrap();
    let report = shalt_core::restack(t.path(), "rust").unwrap();
    assert_eq!(report.to, "rust");
    assert!(
        report.removed.iter().any(|r| r.contains("steps")),
        "{:?}",
        report.removed
    );
    assert!(
        report.removed.iter().any(|r| r.contains("interface.md")),
        "{:?}",
        report.removed
    );
    assert!(!t.path().join("steps").exists());
    let cfg = shalt_core::Config::load(t.path()).unwrap();
    assert_eq!(cfg.stack, "rust");
    assert_eq!(cfg.steps, "tests");
    assert!(t.path().join("Cargo.toml").exists());
    let contract = fs::read_to_string(t.path().join("contract/interface.md")).unwrap();
    assert!(!contract.contains("python"), "{contract}");
    assert!(t.path().join("spec/bom.feature").exists(), "spec must stay");
}

#[test]
fn restack_to_javascript_writes_package_json() {
    let t = TempDir::new().unwrap();
    shalt_core::config::init_workspace(t.path(), "rust", "recipes").unwrap();
    fs::write(t.path().join("spec/recipes.feature"), "Feature: Recipes\n  Scenario: A\n    Given x\n").unwrap();
    let report = shalt_core::restack(t.path(), "javascript").unwrap();
    assert_eq!(report.to, "javascript");
    let cfg = shalt_core::Config::load(t.path()).unwrap();
    assert_eq!(cfg.stack, "javascript");
    assert_eq!(cfg.steps, "steps");
    assert!(t.path().join("package.json").exists());
    assert!(t.path().join("steps").is_dir());
    let pkg = fs::read_to_string(t.path().join("package.json")).unwrap();
    assert!(pkg.contains("@cucumber/cucumber"), "{pkg}");
    assert!(t.path().join("spec/recipes.feature").exists(), "spec must stay");
    assert!(
        shalt_core::config::stack_support_note("javascript").is_none(),
        "javascript is supported; restack must not print the old next-stack note"
    );
}

#[test]
fn seed_plan_does_not_clobber_an_existing_plan() {
    let t = TempDir::new().unwrap();
    shalt_core::talk::save_plan(t.path(), "Keep this.\n").unwrap();
    shalt_core::talk::seed_plan(t.path(), "The system shall replace this.\n");
    let got = shalt_core::talk::load_plan(t.path(), "");
    assert_eq!(got, "Keep this.\n");
}

#[test]
fn seed_plan_replaces_a_wipe_stub() {
    let t = TempDir::new().unwrap();
    shalt_core::talk::save_plan(t.path(), "# Plan\n\nWiped for a clean token run.\n").unwrap();
    shalt_core::talk::seed_plan(t.path(), "Let's make a tool to share and create recipes.\n");
    let got = shalt_core::talk::load_plan(t.path(), "");
    assert!(got.contains("share and create recipes"), "{got}");
    assert!(!got.to_ascii_lowercase().contains("wiped for a clean"), "{got}");
}

#[test]
fn fold_answers_appends_what_we_know_once() {
    let t = TempDir::new().unwrap();
    shalt_core::talk::seed_plan(t.path(), "The system shall total invoices.\n");
    let turns = vec![shalt_core::jobs::TalkTurn {
        question: "Currency?".into(),
        answer: "Customer's currency.".into(),
        ..Default::default()
    }];
    shalt_core::talk::fold_answers_into_plan(t.path(), "The system shall total invoices.\n", &turns);
    shalt_core::talk::fold_answers_into_plan(t.path(), "The system shall total invoices.\n", &turns);
    let got = shalt_core::talk::load_plan(t.path(), "");
    assert!(got.contains("The system shall total invoices."), "{got}");
    assert_eq!(got.matches("## What we know").count(), 1, "{got}");
    assert!(got.contains("Customer's currency."), "{got}");
}

#[test]
fn work_started_is_false_until_tests_exist() {
    let t = TempDir::new().unwrap();
    shalt_core::config::init_workspace(t.path(), "rust", "money").unwrap();
    assert!(!shalt_core::talk::work_started(t.path()));
    fs::write(t.path().join("tests/shalt.rs"), "#[test] fn t() { assert!(true); }\n").unwrap();
    assert!(shalt_core::talk::work_started(t.path()));
}

#[test]
fn browse_lists_only_directories() {
    let t = TempDir::new().unwrap();
    fs::create_dir_all(t.path().join("alpha")).unwrap();
    fs::write(t.path().join("file.txt"), "x\n").unwrap();
    let listing = shalt_core::list_dirs(t.path()).unwrap();
    let names: Vec<_> = listing.entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"alpha"), "{names:?}");
    assert!(!names.contains(&"file.txt"), "{names:?}");
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
