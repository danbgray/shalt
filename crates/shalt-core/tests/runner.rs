//! Runner: an empty python workspace must not look like a broken harness.

use shalt_core::config::{init_workspace, python_reporter_template, Config};
use shalt_core::runner::{harness_report, run_suite, SuiteRun};
use std::collections::HashMap;
use tempfile::TempDir;

#[test]
fn run_suite_on_an_empty_python_workspace_is_not_a_harness_failure() {
    let t = TempDir::new().unwrap();
    init_workspace(t.path(), "python", "empty").unwrap();
    let cfg = Config::load(t.path()).unwrap();
    let run = run_suite(t.path(), &cfg);
    assert!(
        !run.harness_error,
        "fresh `shalt init` has no tests yet; that is not a harness crash.\nstderr: {}\nstdout: {}",
        run.stderr, run.stdout
    );
    assert!(run.results.is_empty());
}

#[test]
fn run_suite_without_a_workspace_names_the_missing_toml() {
    let t = TempDir::new().unwrap();
    let cfg = Config::default();
    let run = run_suite(t.path(), &cfg);
    assert!(run.harness_error);
    assert!(
        run.stderr.contains("shalt.toml"),
        "expected to mention shalt.toml, got: {}",
        run.stderr
    );
}

#[test]
fn harness_report_includes_pytest_stdout() {
    let run = SuiteRun {
        results: HashMap::new(),
        stdout: "INTERNALERROR> unknown hook 'pytest_bdd_before_scenario'\n".into(),
        stderr: String::new(),
        run_id: "run-failed".into(),
        duration: 0.0,
        returncode: 3,
        harness_error: true,
        collection_error: String::new(),
    };
    let text = harness_report(&run);
    assert!(
        text.contains("pytest_bdd_before_scenario"),
        "the CLI was printing only stderr, so pytest INTERNALERROR on stdout vanished: {text:?}"
    );
}

#[test]
fn rust_init_does_not_drop_a_python_conftest() {
    let t = TempDir::new().unwrap();
    init_workspace(t.path(), "rust", "app").unwrap();
    assert!(
        !t.path().join("steps/conftest.py").exists(),
        "rust workspaces must not grow a pytest conftest"
    );
    assert!(
        !t.path().join(".shalt/shalt_report.py").exists(),
        "python reporter belongs only on the python stack"
    );
    let cfg = Config::load(t.path()).unwrap();
    assert_eq!(cfg.stack, "rust");
}

#[test]
fn python_reporter_does_not_register_bdd_hooks_unless_pytest_bdd_imports() {
    let src = python_reporter_template();
    assert!(
        src.contains("import pytest_bdd") && src.contains("ImportError"),
        "the reporter must not define pytest_bdd_* hooks when pytest-bdd is not installed"
    );
}
