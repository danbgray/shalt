# Ratchet — thin end-to-end slice

**Goal:** prove or kill the core claim — an English prompt becomes Gherkin a human approves,
which becomes step definitions written by an *isolated* agent, against which an implementer
agent builds until green, with a ledger whose "green" is trustworthy.

**Reading taken:** dogfood-first pipeline for Rivlet's own agentic workflows; OSS core
(pipeline + ledger format) with a commercial ledger/PM layer later.

## Plan

- [x] 1. Scaffold repo, pyproject, package layout
- [x] 2. Spec model: parse Gherkin -> stable scenario IDs (`F-001/S-002`)
- [x] 3. Integrity engine: content hashing, spec lock, write-guard, turn verification
- [x] 4. Ledger: portable versioned JSON, scenario status, ratchet rule (green never silently regresses)
- [x] 5. Runner: drive pytest-bdd, parse per-scenario outcomes back into the ledger
- [x] 6. Role adapters: pluggable backends (fixture / claude-cli / anthropic) for author, stepwright, implementer
- [x] 7. CLI: init, author, approve, steps, build, status, verify
- [x] 8. Worked example that runs end-to-end offline
- [x] 9. Cheat demo: implementer edits tests to go green -> guard rejects the turn
- [x] 10. Unit tests + full verification run

## Review

**Outcome: the thin slice works, and the core claim survives adversarial testing.**

Built `ratchet`: four zones (`spec/`, `steps/`, `contract/`, `src/`), three agent roles staged
into directories containing only what each may read, a portable scenario ledger, and a CLI.
37 tests; `examples/invoice/demo.sh` runs all three demos offline with no API key.

Design decisions worth keeping:

- **Durable scenario ids** (`@rid:S-xxxxxxxx`) stamped at approval. Identity survives renaming,
  rewording and reordering, which is what lets the ledger be durable across spec revisions.
- **Canonical hashing.** Green is bound to the meaning it was green against; change the meaning
  and it stales rather than silently persisting.
- **A fourth zone, `contract/`.** The stepwright declares the API surface it will call; the
  implementer implements that surface without ever seeing the step definitions. This came out of
  asking what the implementer actually needs to know, versus what lets it cheat.
- **Holdout scenarios.** The write guard stops test tampering; it does nothing about
  overfitting. Holdouts are the answer, and the demo shows them catching a hardcoding
  implementer.

**What the adversarial review changed.** A reviewer agent that had not seen the build found 15
defects. The first invalidated the premise: `GuardedTurn` was never called by the pipeline —
only by my own tests — and the stage directory sat inside the workspace it was meant to isolate,
so `../../../steps` reached the real tests. Also fixed: symlink escapes, held-out scenarios'
expected values leaking into the implementer's prompt, `orphan` being an absorbing state that
let deleting failing scenarios show 100%, an approval lock that stored counts but no content
hashes and so verified nothing, line-based Gherkin manipulation that mis-stamped and leaked
around docstrings, and basename collisions in scenario-to-test binding. See `tasks/lessons.md`.

**Known gaps, in priority order:** never run against a live model (fixtures only); no escape
hatch for non-functional requirements; the stepwright is unverified (mutation testing over step
definitions is the obvious next move); no cost or turn accounting; Python-only runner.
