# Testing

```
cargo test -p shalt-core --offline
cargo test -p shalt --offline
cargo test --manifest-path examples/rust-billing/Cargo.toml --offline --test cucumber
```

Isolation tests go through `run_role` with a hostile backend, never the guard helper.
`crates/shalt-core/tests/invariants.rs` is the record: identity, hashing, ledger, isolation,
overlay, jobs.

`examples/invoice/demo.sh` is the offline loop. Python there is only the *system under test*.
The engine is Rust.

## Three rules

**1. Test the guarantee through the public entry point.**

A helper with passing tests that the pipeline never called is how this product was once a lie.
`run_role` with a hostile backend, not `GuardedTurn` in isolation.

**2. One test per confirmed defect, named after the behaviour.**

`test_orphan_is_not_an_absorbing_state`, `test_losing_the_test_that_proved_a_scenario_is_a_regression`,
`then_that_tags_then_asserts_is_acting`. The name is the invariant.

**3. Mock at the protocol boundary.**

Fixture backends replay recorded turns. Live adapters speak chat-completions. Mocking an internal helper tests nothing.

## The demo

`examples/invoice/demo.sh` — seven scenes, no API key:

| scene | |
|---|---|
| 1 | honest loop to 100% |
| 2 | implementer edits tests — rejected, `steps/` unchanged |
| 3 | overfitting — holdouts fail |
| 4 | epic → story → scenario tree |
| 5 | narratives |
| 6 | diagrams |
| 7 | weak oracle: 100% green, then mutate names the blind spots |

Scene 7 originally used `--budget 12`. At that budget the sample sometimes only drew crashing mutants, which a vacuous assertion *does* catch. The demo reported a clean 100% and proved nothing. Budget is 30.

## Oracle lint

`shalt verify` (and `integrity::audit_in`) reads Then **bodies**. A Then that calls the When, `assert.ok(this.…)`, or uses `||` to always pass is flagged. Titles-only review is taking the spec's word.
