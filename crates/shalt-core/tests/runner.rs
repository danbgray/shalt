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
fn rust_and_javascript_are_supported_stacks() {
    use shalt_core::config::{stack_support, stack_support_note, StackSupport, STACK_SUPPORT_ORDER};
    assert_eq!(stack_support("rust"), Some(StackSupport::Supported));
    assert_eq!(stack_support("javascript"), Some(StackSupport::Supported));
    assert_eq!(stack_support("python"), Some(StackSupport::Next));
    assert_eq!(STACK_SUPPORT_ORDER[0], "rust");
    assert_eq!(STACK_SUPPORT_ORDER[1], "javascript");
    assert_eq!(STACK_SUPPORT_ORDER[2], "python");
    assert!(stack_support_note("rust").is_none());
    assert!(stack_support_note("javascript").is_none());
    let py = stack_support_note("python").unwrap();
    assert!(py.contains("python is next"), "{py}");
}

#[test]
fn rust_stepwright_prompt_is_cucumber_rs() {
    let s = shalt_core::api::system_for("stepwright");
    assert!(s.contains("cucumber"), "{s}");
    assert!(s.contains("tests/shalt.rs"), "{s}");
    assert!(s.contains("Do not write Python"), "{s}");
    let js = shalt_core::api::system_for_stack("stepwright", "javascript");
    assert!(js.contains("cucumber-js"), "{js}");
    assert!(js.contains("@cucumber/cucumber"), "{js}");
    assert!(js.contains("steps/world.js"), "{js}");
    assert!(js.contains("steps/<journey>.steps.js"), "{js}");
    let impl_js = shalt_core::api::system_for_stack("implementer", "javascript");
    assert!(impl_js.contains("JavaScript"), "{impl_js}");
    assert!(impl_js.contains("src/"), "{impl_js}");
    let py = shalt_core::api::system_for_stack("stepwright", "python");
    assert!(py.contains("pytest-bdd"), "{py}");
    let aud = shalt_core::api::system_for("auditor");
    assert!(aud.contains("write nothing") || aud.contains("Do not write"), "{aud}");
    assert!(aud.contains("PASS"), "{aud}");
    let code = shalt_core::api::system_for("code_auditor");
    assert!(code.contains("hard-code") || code.contains("hard-codes"), "{code}");
}

#[test]
fn rust_init_writes_a_cucumber_harness() {
    let t = TempDir::new().unwrap();
    init_workspace(t.path(), "rust", "demo").unwrap();
    let cfg = Config::load(t.path()).unwrap();
    assert_eq!(cfg.stack, "rust");
    assert!(t.path().join("Cargo.toml").exists());
    assert!(t.path().join("tests").is_dir());
    assert!(
        !t.path().join("tests/shalt.rs").exists(),
        "scaffold must not fake a shalt harness; Play still has to run the stepwright"
    );
    assert!(shalt_core::list_step_files(t.path(), "tests").is_empty());
    std::fs::write(t.path().join("tests/shalt.rs"), "fn main() {}\n").unwrap();
    assert_eq!(
        shalt_core::list_step_files(t.path(), "tests"),
        vec!["tests/shalt.rs".to_string()]
    );
    assert!(t.path().join("src/lib.rs").exists());
    let toml = std::fs::read_to_string(t.path().join("shalt.toml")).unwrap();
    assert!(toml.contains("stack = \"rust\""));
    assert!(!toml.contains("python3"));
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
fn javascript_init_writes_a_cucumber_harness() {
    let t = TempDir::new().unwrap();
    init_workspace(t.path(), "javascript", "demo").unwrap();
    let cfg = Config::load(t.path()).unwrap();
    assert_eq!(cfg.stack, "javascript");
    assert!(t.path().join("package.json").exists());
    assert!(t.path().join("steps").is_dir());
    assert!(
        shalt_core::list_step_files(t.path(), "steps").is_empty(),
        "scaffold must not fake cucumber-js steps; Play still has to run the stepwright"
    );
    std::fs::write(t.path().join("steps/world.js"), "export {};\n").unwrap();
    assert_eq!(
        shalt_core::list_step_files(t.path(), "steps"),
        vec!["steps/world.js".to_string()]
    );
    assert!(t.path().join("src/index.js").exists());
    let toml = std::fs::read_to_string(t.path().join("shalt.toml")).unwrap();
    assert!(toml.contains("stack = \"javascript\""));
    assert!(toml.contains("cucumber-js"));
    assert!(toml.contains("--import"));
    let pkg = std::fs::read_to_string(t.path().join("package.json")).unwrap();
    assert!(pkg.contains("@cucumber/cucumber"), "{pkg}");
    assert!(pkg.contains("\"type\": \"module\""), "{pkg}");
    assert!(
        !t.path().join("tests/shalt.rs").exists(),
        "javascript workspaces must not grow a rust harness"
    );
}

#[test]
fn run_suite_on_an_empty_javascript_workspace_is_not_a_harness_failure() {
    let t = TempDir::new().unwrap();
    init_workspace(t.path(), "javascript", "empty").unwrap();
    let cfg = Config::load(t.path()).unwrap();
    let run = run_suite(t.path(), &cfg);
    assert!(
        !run.harness_error,
        "fresh `shalt init --stack javascript` has no tests yet; that is not a harness crash.\nstderr: {}\nstdout: {}",
        run.stderr, run.stdout
    );
    assert!(run.results.is_empty());
}

#[test]
fn javascript_with_step_files_does_not_demand_pytest_bdd() {
    let t = TempDir::new().unwrap();
    init_workspace(t.path(), "javascript", "cook").unwrap();
    std::fs::create_dir_all(t.path().join("steps")).unwrap();
    std::fs::write(
        t.path().join("steps/world.js"),
        "import { setWorldConstructor } from '@cucumber/cucumber';\nfunction World() {}\nsetWorldConstructor(World);\n",
    )
    .unwrap();
    let cfg = Config::load(t.path()).unwrap();
    assert!(cfg.command.contains("cucumber-js"), "{}", cfg.command);
    let run = run_suite(t.path(), &cfg);
    assert!(
        !run.stderr.contains("pytest-bdd"),
        "javascript must not require pytest-bdd.\nstderr: {}",
        run.stderr
    );
}

#[test]
fn python_reporter_does_not_register_bdd_hooks_unless_pytest_bdd_imports() {
    let src = python_reporter_template();
    assert!(
        src.contains("import pytest_bdd") && src.contains("ImportError"),
        "the reporter must not define pytest_bdd_* hooks when pytest-bdd is not installed"
    );
}
