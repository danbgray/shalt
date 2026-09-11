# Ratchet — thin end-to-end slice

**Goal:** prove or kill the core claim — an English prompt becomes Gherkin a human approves,
which becomes step definitions written by an *isolated* agent, against which an implementer
agent builds until green, with a ledger whose "green" is trustworthy.

**Reading taken:** dogfood-first pipeline for Rivlet's own agentic workflows; OSS core
(pipeline + ledger format) with a commercial ledger/PM layer later.

## Plan

- [ ] 1. Scaffold repo, pyproject, package layout
- [ ] 2. Spec model: parse Gherkin -> stable scenario IDs (`F-001/S-002`)
- [ ] 3. Integrity engine: content hashing, spec lock, write-guard, turn verification
- [ ] 4. Ledger: portable versioned JSON, scenario status, ratchet rule (green never silently regresses)
- [ ] 5. Runner: drive pytest-bdd, parse per-scenario outcomes back into the ledger
- [ ] 6. Role adapters: pluggable backends (fixture / claude-cli / anthropic) for author, stepwright, implementer
- [ ] 7. CLI: init, author, approve, steps, build, status, verify
- [ ] 8. Worked example that runs end-to-end offline
- [ ] 9. Cheat demo: implementer edits tests to go green -> guard rejects the turn
- [ ] 10. Unit tests + full verification run

## Review

(filled in at the end)
