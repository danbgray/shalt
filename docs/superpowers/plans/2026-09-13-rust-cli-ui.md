# Shalt Rust Rewrite Implementation Plan

> **For agentic workers:** Implement task-by-task. The user asked this landed on `main` after a green test set.

**Goal:** Port shalt to a Rust binary (`shalt-core` + `shalt`) with the Python invariants, a Linear-shaped localhost UI, and `cargo test` green, then merge to `main`.

**Architecture:** Library crate holds spec/ledger/integrity/roles/runner/overlay/jobs. Binary is clap CLI plus axum `shalt ui`. Workspace layout on disk is unchanged. Python package stays until a later delete-Python commit; this landing is the Rust product beside it.

**Tech Stack:** Rust 2021, `gherkin`, `serde_json`, `toml`, `clap`, `axum`, `tokio`, `sha2`, `tempfile`.

## Global Constraints

- Integrity tests go through `run_role`, never the guard helper alone.
- Pending is never green. Overlay cannot be written by an agent turn.
- `shalt.ledger/1` and `shalt.board/1` as specified.
- `127.0.0.1` only for the UI.

## Files

- Create: `Cargo.toml`, `crates/shalt-core/**`, `crates/shalt/**`
- Modify: `.gitignore`, `README.md`
- Test: `crates/shalt-core/tests/{spec,ledger,isolation,board,jobs}.rs`

## Tasks

1. Workspace scaffold + `.gitignore` `target/`
2. `spec` + `narrative` + tests ported from `tests/test_core.py` identity/hashing/holdouts and `tests/test_defects.py` docstring/stamp cases
3. `ledger` + tests from test_core ledger cases and defects (orphan, unbound regression)
4. `integrity` + `roles` + tests from `tests/test_isolation.py` (hostile backend through `run_role`)
5. `config` + `reports` + `runner` + fixture backend
6. `org` + `board` + `jobs` + tests (auto-add rid, unschedule vs delete spec, drift, durable queue)
7. CLI parity for init/author/approve/steps/build/run/status/verify/tree/org/board/job/ui
8. Embedded Linear-inspired UI + HTTP tests
9. `cargo test` green, merge to `main`
