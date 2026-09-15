# Shalt

**English sentence → formalized logic → tests → code.** A plain request becomes user stories,
then Gherkin scenarios; isolated agents write the tests, then the
implementation, until every scenario passes — recorded in a ledger whose "green" you can
actually trust.

CLI: `shalt` — a Rust binary. Localhost UI: `shalt ui`. Models: `--backend grok` or `--backend qwen` (Ollama).

### Why "Shalt"

Requirements have been written in one grammatical form for fifty years: **"the system shall…"**
[RFC 2119](https://www.rfc-editor.org/rfc/rfc2119) makes SHALL and MUST the normative keywords —
the ones that state an absolute requirement rather than a preference — and
[EARS](https://alistairmavin.com/ears/) builds its whole requirements syntax on top of that verb.

`shalt` is that word turned to face the system. You are not describing what the software does;
you are stating what it *shall* do, and then holding it to that. Which is the distinction the
whole tool rests on: a specification is not a claim about what is true, it is an obligation about
what must hold. The ledger records which obligations are upheld, and the rule below follows
directly — an obligation is owed against its exact wording, so reword it and nothing has been
discharged.

**Technical documentation:** [docs/](docs/) — [architecture](docs/architecture.md) ·
[isolation & threat model](docs/isolation.md) · [ledger schema](docs/ledger.md) ·
[identity & hashing](docs/identity.md) · [runners](docs/runners.md) ·
[hierarchy](docs/hierarchy.md) · [mutation testing](docs/mutation.md) ·
[backends](docs/backends.md) · [CLI](docs/cli.md) · [testing](docs/testing.md) ·
[limitations](docs/limitations.md)

```bash
cargo test --workspace
cargo run -p shalt -- --help
shall total invoices exactly in the customer's currency   # spec, then y/n each scenario, then tests
shall --yes total invoices exactly                        # accept every scenario, then tests
shalt ui                                                  # same compose box in the browser
examples/invoice/demo.sh                                  # offline fixture loop
```

---

## The problem this is built around

An agent that writes the spec, the tests, and the implementation can always make the suite
pass. A green suite then proves internal consistency, not correctness — self-graded homework.
Everything below is an attempt to take that capability away structurally rather than asking for
it politely in a prompt.

## Language agnostic

The zone model, scenario identity, the ledger and the guards are all language-neutral. Only the
runner is not, so it lives in `shalt.toml`:

```toml
[runner]
command = "npx cucumber-js {spec} --require {steps} --format message:{report}"
format  = "cucumber-messages"     # shalt | cucumber-json | cucumber-messages
report  = ".shalt/messages.ndjson"
```

The reason this works cleanly: **`@rid:` is a Gherkin tag, and a tag survives into every
Cucumber-family report.** So binding a result back to a scenario needs no filename matching, no
per-language shim, and no guessing — the identity is carried in the report itself.

`shalt init --stack <python|javascript|go|java|ruby|dotnet|rust>` writes a starting config for
that toolchain. A Python workspace keeps the rid reporter in `.shalt/shalt_report.py`, not in
`steps/` — that zone is for the stepwright. Anything that emits Cucumber JSON or Cucumber
Messages works without new code. `shall` on a new directory inits **Rust** (cucumber-rs).

The claim is exercised, not asserted: [`examples/rust-billing`](examples/rust-billing) is a real
Cargo project driven through cucumber-rs. It found two bugs on first contact — a tag-spelling
incompatibility that silently bound nothing, and a stale-binary hazard that made mutation scores
non-deterministic for every compiled language. Both fixed, both with regression tests.

One rule the parsers enforce: a scenario is green only if **every** step passed. Skipped,
pending, undefined and ambiguous all count as not-passed — a step nobody implemented is not
evidence of success, and a suite full of undefined steps must never read as green.

## Four roles, four zones

| zone        | written by     | read by                       |
|-------------|----------------|-------------------------------|
| `spec/`     | author         | everyone                      |
| `steps/`    | **stepwright** | the test runner only          |
| `contract/` | **stepwright** | implementer                   |
| `src/`      | implementer    | implementer, the test runner  |

Each role runs in a **staged directory containing only the zones it may read**. The stepwright
physically cannot see the implementation. The implementer physically cannot see the step
definitions it must satisfy — it gets the spec, the interface contract the stepwright declared,
and the failing test output, and has to implement the behaviour rather than the assertions.

A turn that writes outside its zone is **rejected wholesale and rolled back**. Nothing it
produced is kept.

Isolation is enforced in three layers, because each one alone is defeatable:

1. **The stage lives outside the workspace.** Relative traversal out of it (`../../../steps`)
   lands in a scratch directory, not in the real spec or tests.
2. **The stage is scanned** for files outside the role's write zones, and for symlinks
   anywhere — a symlink inside an allowed zone is a write to wherever it points.
3. **The real workspace is hashed before and after** the turn, so a backend that writes by
   absolute path is still caught, and protected zones are restored from backup. The ledger
   and the board overlay are protected on every turn; no role may write them.

`crates/shalt-core/tests/invariants.rs` is the record of this: each isolation test is an escape
that worked against an earlier version — relative traversal, absolute writes, a file at the
stage root, and rewriting the ledger to forge an approval. Tests go through `run_role` with a
hostile backend, never the guard helper, because a helper with passing tests that the pipeline
never called is how this product was once a lie.

```
turn 1 REJECTED -- role 'implementer' wrote outside its zone -> steps: steps/test_invoice.py
  nothing from this turn was kept; the spec and tests are untouched.
```

## Scenario identity

At approval time, every scenario is stamped with a durable id:

```gherkin
  @rid:S-b291b8fd
  Scenario: An invoice with a single line item
```

Rename it, reword it, move it to another file — the ledger still tracks the same scenario.
Delete the tag and you have deleted the scenario, explicitly.

## An obligation is bound to its wording

A scenario's **canonical hash** covers everything that changes its meaning (steps, tables,
docstrings, background, tags) and nothing that doesn't (whitespace, tag order, the id itself).
A scenario is recorded as upheld *against a canonical hash*.

Change what the scenario means and the status goes `stale`, not green. The obligation was owed
against particular words; reword it and nothing has been discharged.

This is the one rule that stops the oldest failure in spec-driven work: the spec drifted, the
suite still passes, nobody noticed.

```
  + green    A half-cent total rounds up, not down      S-2e8e2670
  ~ stale    An invoice with several line items         S-81d2b5ca   <- spec changed under it
```

Statuses are deliberately not binary:

- `pending` — approved, but no test is bound to it. **Never counted as green.** The absence of
  a test is not success.
- `red` — bound test fails.
- `green` — bound test passes against the current meaning of the scenario.
- `stale` — was green, meaning has changed since, needs re-verification.
- `orphan` — a ledger entry whose scenario has left the spec.

A `green → red` transition is recorded as a **regression** with its own permanent entry.

## Holdouts

The write guard stops test *tampering*. It does nothing about test *overfitting* — an
implementer that special-cases the examples it was shown. So scenarios tagged `@holdout` are
approved, and they run during verification, but they are never staged for the implementer:

```
turn 2: 4/4 visible green

OVERFIT: every visible scenario is green but held-out scenarios fail.
  S-344237b7  An invoice the implementer was never shown
  the implementation satisfies the examples it saw, not the behaviour.
```

## User stories, and the diagram hiding in them

A feature's description block holds a user story:

```gherkin
@epic:billing
Feature: Invoice totals

  As a billing clerk
  I want invoice totals computed exactly
  So that customers are never billed the wrong amount
```

That sentence *is* a use case diagram. "As a &lt;actor&gt;" is the actor, "I want
&lt;capability&gt;" is the use case, and their appearing in one story is the association. So the
diagrams are **derived, not drawn** — which means they cannot drift away from the spec. There is
no second artefact to keep in sync.

The breakdown is three levels, all expressed in Gherkin:

| level | where it comes from |
|---|---|
| **epic** | an `@epic:` tag on the feature, else the directory under `spec/` |
| **story** | the feature, plus its `As a / I want / So that` narrative |
| **task** | one scenario |

Scheduling (goals, milestones, optional sprints, rank) is a Linear-style **overlay**
(`.shalt/board.json`). It points at rids; it does not copy their text. Rank and sprint
assignment do not rewrite the spec. Changing a story's wording still writes `spec/` and goes
stale. `shalt verify` fails if the board points at a rid that is gone.

```
$ shalt tree

EPIC BILLING  7/9 verified
 ├── STORY Currency presentation
 │        As a billing clerk, I want amounts shown in the customer's own currency
 │   ├── x red      Euros use the euro sign S-30d6398c
 │   ├── + green    US dollars lead with the symbol and group thousands S-d36e2796
 │   └── + green    Yen rounds to whole units rather than truncating [holdout] S-0bae3a95
 └── STORY Invoice totals
     ├── ~ stale    An invoice with no line items S-8031be66
     └── + green    An invoice with several line items S-0c1fba78
```

`shalt stories` lists who wants what, and names any feature missing a narrative.

## UI

`shalt ui` binds `127.0.0.1` (default port 7700). The home screen is a compose box: describe
the project in English, pick **Grok** or a local **Qwen** (Ollama), and shalt authors Gherkin
into a new project. The sidebar lists projects; each has a board of rids. The CLI can do
everything the UI can.

```
shalt ui              # start, or reopen the one that's already up
shalt ui status
shalt ui stop
shalt ui restart
```

There is one UI. If 7700 is taken by something else, `shalt ui` binds the next free port in 7700–7799. `--port` is only used when nothing is running.

`shalt org add PATH` registers a git workspace. Overlay edits (rank, milestone, sprint) write
the board; content edits of a story still write Gherkin.

## Diagrams and dashboard

`shalt diagrams` writes Mermaid to `docs/diagrams/` — three views, all generated from the
ledger and the spec:

- **use-cases** — actors and the capabilities they want
- **breakdown** — epic → story → scenario, coloured by verified state
- **pipeline** — how a request becomes verified behaviour, and who may touch what

`.mmd` plus `.md` wrappers, so they render in GitHub, in pull requests and in most editors with
no toolchain.

`shalt dashboard` writes `docs/dashboard.html`: one self-contained snapshot for git. The live
UI is the working surface; regenerate the snapshot after a run.

Human intent is set in a serif and machine state in a mono. Progress is discrete notches, one
per scenario — obligations are discrete.

## Mutation-testing the oracle

The write guard stops an implementer tampering with the tests. Holdouts catch an implementer
overfitting to the examples it saw. Neither says anything about the **stepwright** — nothing
above checks that the generated step definitions actually assert what their scenario claims. A
step definition ending in `assert result is not None` passes every time, and the ledger shows
green.

`shalt mutate` runs the check in the opposite direction to the obvious one: rather than
mutating the step definitions, it mutates the **implementation** and asks whether the scenarios
notice. Break the rounding rule; if "a half-cent total rounds up" stays green, that scenario is
not testing rounding, whatever its name says.

```
mutation score 91.7%  (22 killed, 2 survived, 0 invalid)

BLIND SPOTS — 2 mutation(s) ran inside scenarios that stayed green:
  src/invoice.py:5  number  0.0 -> 1.0
      missed by  A half-cent total rounds up, not down
      missed by  An invoice with several line items
```

Two signals, and **neither alone is sufficient**:

- **vacuous** — the scenario detected nothing at all.
- **blind spot** — the scenario has a healthy kill count but still ran broken code silently.

The engine is `text` (comparisons, booleans, literals) and is language-agnostic. Mutants that
break the suite itself are `invalid` and excluded. After the campaign the sources are restored
and mtimes bumped so compiled-language toolchains rebuild; a baseline that does not reproduce
is refused rather than scored. Silence is not evidence of survival.

## The ledger

`.shalt/ledger.json` — portable, versioned (`shalt.ledger/1`), deliberately not tied to any
runner or vendor. One artifact that is simultaneously the requirement, the test binding, the
ticket, and the progress bar:

```
[######################------] 80.0%  4 green / 1 red / 0 stale / 0 pending
```

That percentage cannot be gamed the way story points can, because `pending` never counts as
green and green expires when the spec moves.

## Commands

```
shall --model                    list every available model and pick a default
shall --model=qwen3.5:2b …       use this model for this run
shall <sentence>                 clarify if needed, then spec, y/n each scenario, then tests
shall --yes <sentence>           no questions; accept every scenario; then tests
shalt init [--stack NAME]        scaffold a workspace
shalt author "<request>"         English → Gherkin under spec/ (long form)
shalt approve --yes --by <you>   re-lock hashes after you edit the spec
shalt steps                      stepwright writes steps/ + contract/
shalt build [--max-turns N]      implementer loop until green, then verify with holdouts
shalt run                        run the suite, update the ledger
shalt status                     the ledger, as a progress view
shalt verify                     standing integrity audit (spec lock + overlay drift)
shalt tree                       epic → story → scenario, with status
shalt stories                    who wants what, and what is missing a narrative
shalt diagrams                   Mermaid use-case, breakdown and pipeline diagrams
shalt dashboard                  a self-contained HTML snapshot
shalt mutate                     mutation-test the oracle
shalt play / shalt loop          thin loop: tests, then code, until green (Hermes/Claude/Codex entry)
shalt ui [status|stop|restart]   one localhost UI
shalt org add|list|remove|play   local catalog of projects; org play runs the same loop
shalt board                      overlay: list / unschedule
shalt job add|list|show|pause|resume  durable job queue; a running job shows its prompt in the UI
```

## Backends

- `--backend fixture --fixtures <dir>` — replays recorded turns. Offline, deterministic; this
  is what the tests and `examples/invoice/demo.sh` use.
- `--backend grok` — xAI chat-completions. Needs `XAI_API_KEY`. Default model `grok-4.5`.
- `--backend qwen` — local Ollama at `http://127.0.0.1:11434/v1`. Default `qwen3.5:35b-128k`. No API key.
- `--backend openai` — same adapter as Grok. Needs `OPENAI_API_KEY`.

Adding a backend is one type that implements `Backend::run(role, prompt, stage)`.

### Running it against Grok

```bash
export XAI_API_KEY=...
shall --backend grok --root ./work "what you want built"   # spec + tests
shalt --root ./work --backend grok build --max-turns 8
shalt --root ./work verify
shalt --root ./work mutate
```

If you edit Gherkin afterwards, `shalt approve --yes` re-locks hashes so green stays bound to the new wording. That is a re-lock, not a gate on the first sentence.

`--model` overrides the default. `--base-url` points the same adapter at any other
OpenAI-compatible endpoint.

The model works through four scoped tools — `list_files`, `read_file`, `write_file`, `done` —
rather than a shell. Every path is resolved inside the stage first: absolute paths are refused
outright rather than reinterpreted, traversal is refused, and any symlinked component is
refused. A refusal goes back to the model as a tool result. That sandbox is the first of the
three layers, not a replacement for the workspace guard, which still hashes and rolls back
around every turn.

## What this does and does not prove

Demonstrated, end to end, in `examples/invoice/demo.sh` and `cargo test --workspace` (32 tests):

- the pipeline runs: prompt → Gherkin → human approval → step definitions → implementation → green
- an implementer that edits the tests is caught and rolled back
- an implementer that overfits to visible examples is caught by holdouts
- a stepwright that writes assertions testing nothing is caught by mutation testing
- editing an approved scenario stales exactly that scenario's green
- losing the test that proved a scenario is a regression; orphan is not absorbing
- the API path sandbox refuses absolute paths and traversal without a network

Not yet addressed, in rough order of how much they matter:

1. **Gherkin's expressiveness ceiling.** Latency, cost, security posture, UI behaviour — Gherkin
   is bad at all of them. There is currently no escape hatch.
2. **Holdouts are a sample, not a proof.** They catch crude overfitting.
3. **Live judgement.** The Grok adapter is in the binary. Whether a real model writes Gherkin a
   stakeholder would approve is an empirical question for a key with credits.
4. **No cost or turn accounting.** A build loop that runs 40 turns should say what it spent.

## Layout

```
crates/shalt-core/     spec, ledger, isolation, overlay, jobs, mutate, viz, grok/openai adapter
crates/shalt/          CLI + embedded localhost UI
examples/invoice/      offline fixture loop (Python only as the project-under-test)
examples/rust-billing/ cucumber-rs workspace
docs/                  architecture, isolation, ledger, CLI
```

## Licence

**Not yet licensed.** This repository is private and all rights are reserved.

The intention is to release it under the **MIT License**. Until then there is deliberately no
`LICENSE` file, because adding one would be the grant itself. What is in place instead is the
machinery that keeps the option open: contributions require DCO sign-off, dependency licences are
tracked, and `CONTRIBUTING.md` carries the checklist for whoever flips the switch.

If you have been given access to this repository, that is access — not a licence.
