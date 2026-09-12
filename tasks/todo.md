# Shalt — thin end-to-end slice

**Goal:** prove or kill the core claim — an English prompt becomes Gherkin a human approves,
which becomes step definitions written by an *isolated* agent, against which an implementer
agent builds until green, with a ledger whose "green" is trustworthy.

**Reading taken:** dogfood-first pipeline for Rivlet's own agentic workflows; OSS core
(pipeline + ledger format) with a commercial ledger/PM layer later.

## Plan

- [x] 1. Scaffold repo, pyproject, package layout
- [x] 2. Spec model: parse Gherkin -> stable scenario IDs (`F-001/S-002`)
- [x] 3. Integrity engine: content hashing, spec lock, write-guard, turn verification
- [x] 4. Ledger: portable versioned JSON, scenario status, shalt rule (green never silently regresses)
- [x] 5. Runner: drive pytest-bdd, parse per-scenario outcomes back into the ledger
- [x] 6. Role adapters: pluggable backends (fixture / claude-cli / anthropic) for author, stepwright, implementer
- [x] 7. CLI: init, author, approve, steps, build, status, verify
- [x] 8. Worked example that runs end-to-end offline
- [x] 9. Cheat demo: implementer edits tests to go green -> guard rejects the turn
- [x] 10. Unit tests + full verification run

## Review

**Outcome: the thin slice works, and the core claim survives adversarial testing.**

Built `shalt`: four zones (`spec/`, `steps/`, `contract/`, `src/`), three agent roles staged
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

**Grok backend (follow-up).** Added an OpenAI-compatible adapter, so one class covers Grok,
OpenAI, and any other endpoint on that protocol. A live run is impossible from this environment
(no key; the egress gateway returns 403 to CONNECT for `api.x.ai`), so the adapter is proven
against a mock that speaks the xAI wire protocol: the full author → approve → steps → build
pipeline runs over real HTTP, and the workspace guard still rejects an API-driven role that
writes outside its zone. 60 tests. What remains unproven is the model's judgement, not the
plumbing.

**Language agnostic + breakdown + visuals (follow-up).** The zone model, ledger, identity and
guards were already language-neutral; only the runner was not, so it moved into `shalt.toml`
with presets for Python, JavaScript, Go, Java, Ruby and .NET. The unlock: `@rid:` is a Gherkin
tag, so it survives into every Cucumber-family report — binding needs no filename matching and
no per-language shim, which also retired the fragile basename matching the review had flagged.
Added Cucumber JSON and Cucumber Messages parsers, where skipped/pending/undefined all count as
not-passed.

The breakdown is epic -> story -> task, expressed entirely in Gherkin: `@epic:` tags (or the
directory under `spec/`), the feature plus its `As a / I want / So that` narrative, and one
scenario per task. The key realisation: that narrative sentence *is* a use case diagram, so the
diagrams are derived rather than drawn and cannot drift. `shalt tree`, `stories`, `diagrams`
(Mermaid) and `dashboard` (one self-contained HTML file) all read from the ledger. 92 tests.

**Naming (settled).** Renamed to **shalt** — `shalt` on PyPI, `@rivlet/shalt` on npm, CLI
`shalt`. The reasoning: "the system shall..." is the canonical form of a requirement (RFC 2119
normative keyword, the backbone of EARS syntax), and `shalt` is that verb turned to face the
system. A spec is not a claim about what is true, it is an obligation about what must hold — so
the ledger says `upheld`, and an obligation is owed against its exact wording.

Rejected along the way, with reasons worth keeping: **ratchet** (taken on all three registries,
plus `sethvargo/ratchet` uses the same metaphor for CI, plus slang); **entails** (sounds like
"entrails"); **deontic** (deontic.ai is a live imec.istart company doing structurally the same
pipeline — natural-language regulation → extracted requirements → executable scenarios → V&V —
with an EU international registration in classes 9/35/42: a product-shape collision, not a name
collision); **realizer** (buildable but Realizer GmbH holds a live mark whose German/international
specification may be broader than the US 6/7 extension — unverified, EUIPO unreachable);
**succedent** (came back completely clean, but reads one consonant from "decedent" and every
short form of the CLI is bad). Gherkin puns were dropped on principle: GHERKIN is not registered
for software, but every surviving gherkin-derived tool is a *complement*, and a substitute for
the same class-9 buyers is where confusion analysis gets uncomfortable.

Standing caveat: USPTO primary, EUIPO, UK IPO, WIPO and DPMA were all refused at the network
layer in this environment. Everything above is third-party mirrors of the US register. A paid
clearance search is still outstanding.

**Old note — "shalt" is not viable publicly: taken on PyPI, npm and crates, with two prominent
GitHub projects including one using the same ratcheting metaphor for CI, plus unhelpful slang.
Shortlist researched and pending a decision; `jumar` (a rope ascender that grips one way only)
is the leading candidate and is free everywhere checked.

**Mutation testing the oracle (follow-up).** Closed the last unguarded link: nothing checked
that the stepwright's assertions test anything. `shalt mutate` mutates the *implementation*
and asks which scenarios notice, attributing each kill to specific scenarios so every scenario
gets its own oracle-strength count. A scenario's kills also reveal which files it provably
executes, which gives coverage-like attribution with no language-specific coverage tool.

The design took one real correction: "green scenario that killed zero mutants" cleared a step
definition asserting only `is not None`, because a vacuous assertion still catches crashes. The
fix is a second signal — a survivor in a file the scenario provably runs — with the two exposed
as a `weak_oracles` union, since which one fires depends on whether the sampled mutations crash
or merely change a value. 108 tests.

On the worked example it found three genuine gaps in a spec I wrote by hand and believed
complete: a config entry masked by its own default, no scenario for an unsupported currency
code, and no odd-cent rounding case.

**Known gaps, in priority order:** never run against a live model (fixtures only); no escape
hatch for non-functional requirements; the stepwright is unverified (mutation testing over step
definitions is the obvious next move); no cost or turn accounting; Python-only runner.
