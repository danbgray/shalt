# Testing strategy

`cargo test --workspace` is the suite. Isolation tests go through `run_role` with a hostile
backend, never the guard helper.

```
crates/shalt-core/tests/invariants.rs
  identity, hashing, ledger, isolation, overlay, jobs, mutation text engine
```

The invoice demo (`examples/invoice/demo.sh`) is the end-to-end proof, driven by the Rust
binary. Python remains only as the *project under test* (pytest-bdd step definitions in the
fixtures).

## Three rules this suite is built on

**1. Test the guarantee through the public entry point, never the helper.**

`tests/test_isolation.py` calls `run_role(...)` with a hostile backend, not `GuardedTurn(...)`
directly. This is not a style preference — it is the direct lesson of the worst defect in this
project's history. An earlier version had a guard with passing tests that the pipeline **never
called**. `grep -rn GuardedTurn` would have found only the test file. A test that calls the helper
cannot distinguish "wired in" from "dead code."

**2. One test per confirmed defect, named after the behaviour.**

`test_defects.py` and `test_isolation.py` are regression records. Every test in them corresponds
to something that actually broke, and the test name states the invariant rather than the
mechanism: `test_orphan_is_not_an_absorbing_state`,
`test_losing_the_test_that_proved_a_scenario_is_a_regression`,
`test_a_healthy_kill_count_does_not_clear_a_blind_spot`.

**3. Mock at the protocol boundary, not the adapter boundary.**

`mock_llm.py` is a real HTTP server speaking chat-completions. Mocking the adapter would have
tested nothing; mocking the wire format tests request construction, the multi-step tool loop,
refusals, retries, usage accounting and the subprocess plumbing. The full pipeline runs over it.

## The hostile-backend pattern

```python
class _Backend:
    def __init__(self, action):
        self.action = action
    def run(self, role, prompt, stage):
        self.action(Path(stage))
        return "done"

def test_absolute_write_to_the_tests_is_caught_and_rolled_back(workspace):
    def act(stage):
        (stage / "src" / "ok.py").write_text("x = 1\n")
        (workspace / "steps" / "test_money.py").write_text("assert False  # weakened\n")
    with pytest.raises(IntegrityViolation) as ei:
        run_role(workspace, "implementer", "p", _Backend(act))
    assert "steps" in ei.value.offences
    _assert_workspace_intact(workspace)
```

Every escape route is expressed as an `action` callable, so adding a newly-imagined attack is a
five-line test.

## Adversarial review

The largest quality gain in this project came from running a **separate reviewing agent that had
not seen the code being written**, given the specific escape routes to attempt. It found 15
defects, including the one that invalidated the entire premise. My own testing had found none of
them.

For anything whose value *is* an adversarial guarantee, this should be a standing step, not a
one-off. See `tasks/lessons.md` for the full list of what it caught.

## The demo as a proof

`examples/invoice/demo.sh` runs seven scenes offline with no API key, and is the fastest way to
confirm the system still does what it claims:

| scene | demonstrates |
|---|---|
| 1 | the honest loop reaching 100% |
| 2 | an implementer editing the tests — rejected and rolled back, `steps/` byte-identical |
| 3 | an implementer overfitting — caught by holdouts |
| 4 | the epic → story → scenario tree |
| 5 | who wants what, from the narratives |
| 6 | Mermaid diagrams and the HTML dashboard |
| 7 | a **weak oracle** — 100% green, then mutation testing names the blind spots |

Scene 7 carries a lesson of its own. It originally ran at `--budget 12`, and at that budget the
seeded sample sometimes drew only mutations that crash — which a vacuous assertion *does* catch.
The demo reported a clean 100% and proved nothing. A demo that can silently fail to demonstrate
its own point is worse than no demo; the budget is now 30.

## Diagram validation

The Mermaid diagrams in `docs/` are checked separately, because a broken one is a silent
failure — GitHub renders an error box and the Python suite never notices:

```bash
npm install mermaid@11 jsdom
node scripts/validate_diagrams.mjs docs
```

15 diagrams, 0 invalid at the time of writing. One had a semicolon in a sequence-diagram message,
which Mermaid reads as a statement separator; it would have shipped broken.
