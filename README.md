# Shalt

**English sentence → formalized logic → tests → code.** A plain request becomes user stories,
then Gherkin scenarios you approve as the contract; isolated agents write the tests, then the
implementation, until every scenario passes — recorded in a ledger whose "green" you can
actually trust.

`shalt` on PyPI · `@rivlet/shalt` on npm · CLI: `shalt`

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

Status: working prototype. The full loop runs offline, with no API key, via recorded fixtures.

**Technical documentation:** [docs/](docs/) — [architecture](docs/architecture.md) ·
[isolation & threat model](docs/isolation.md) · [ledger schema](docs/ledger.md) ·
[identity & hashing](docs/identity.md) · [runners](docs/runners.md) ·
[hierarchy](docs/hierarchy.md) · [mutation testing](docs/mutation.md) ·
[backends](docs/backends.md) · [CLI](docs/cli.md) · [testing](docs/testing.md) ·
[limitations](docs/limitations.md)

```bash
# Rust binary (primary)
cargo test --workspace
cargo run -p shalt -- --help
cargo run -p shalt -- ui          # localhost UI, Linear-shaped org of projects

# Python prototype (still in-tree; invoice demo)
pip install -e .
examples/invoice/demo.sh
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
that toolchain. Anything that emits Cucumber JSON or Cucumber Messages works without new code.

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
   itself is protected on every turn; no role may write it.

`tests/test_isolation.py` is the record of this: each test there is an escape that worked
against an earlier version — relative traversal, absolute writes, symlinked directories,
symlinked files masquerading as source modules, and rewriting the ledger to forge an approval.

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

The breakdown is three levels, all expressed in Gherkin with nothing on the side:

| level | where it comes from |
|---|---|
| **epic** | an `@epic:` tag on the feature, else the directory under `spec/` |
| **story** | the feature, plus its `As a / I want / So that` narrative |
| **task** | one scenario |

```
$ shalt tree

EPIC BILLING  7/9 verified
 ├── STORY Currency presentation
 │        As a billing clerk, I want amounts shown in the customer's own currency, so that
 │        an invoice is never misread as the wrong figure
 │   ├── x red      Euros use the euro sign S-30d6398c
 │   ├── + green    US dollars lead with the symbol and group thousands S-d36e2796
 │   └── + green    Yen rounds to whole units rather than truncating [holdout] S-0bae3a95
 └── STORY Invoice totals
     ├── ~ stale    An invoice with no line items S-8031be66
     └── + green    An invoice with several line items S-0c1fba78
```

`shalt stories` lists who wants what, and names any feature missing a narrative.

## Diagrams and dashboard

`shalt diagrams` writes Mermaid to `docs/diagrams/` — three views, all generated from the
ledger and the spec:

- **use-cases** — actors and the capabilities they want
- **breakdown** — epic → story → scenario, coloured by verified state
- **pipeline** — how a request becomes verified behaviour, and who may touch what

`.mmd` plus `.md` wrappers, so they render in GitHub, in pull requests and in most editors with
no toolchain.

`shalt dashboard` writes `docs/dashboard.html`: one self-contained file, no network and no
build step, showing the verified-progress meter, the derived use case diagram, and the full
breakdown with each scenario's status, id and failing assertion. Light and dark, and it works at
phone width.

Two deliberate choices in it. Human intent is set in a serif and machine state in a mono, so a
story's "As a billing clerk…" reads as prose while every id, hash and status reads as fact. And
the progress meter is drawn as discrete notches rather than a smooth bar — one per scenario,
because obligations are discrete. A scenario is either upheld or it is not; there is no such
thing as 63% of a scenario, so the ornament encodes a real count instead of a percentage.

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

Attribution is what makes this more useful than a suite-wide mutation score. Every mutant
records which scenarios went red, so each scenario gets its own count — and a scenario's kills
reveal which files it provably executes, giving coverage-like attribution with no
language-specific coverage tool.

Two signals, and **neither alone is sufficient**:

- **vacuous** — the scenario detected nothing at all.
- **blind spot** — the scenario has a healthy kill count but still ran broken code silently.

The second exists because the first is too weak on its own: a vacuous assertion still notices
mutations that *crash* the code, so a meaningless assertion can post a good kill count. What it
cannot notice is a well-formed wrong value. Which signal fires depends on whether the sampled
mutations crash or merely change a value, so `weak_oracles` is the union and the thing to check.

A blind spot is one of two defects, and they need different fixes:

- the assertion does not check the value → **the step definitions are weak**;
- no scenario exercises the case the mutation changes → **the spec is missing a scenario**,
  usually a boundary the narrative implies but no example pins down.

This cannot tell them apart, because it does not know which inputs each scenario uses. Both are
real defects in the spec/test pair, so both get surfaced. On the worked example — a spec written
by hand and believed complete — it found a redundant config entry masked by its own default, a
missing scenario for an unsupported currency code, and a missing odd-cent rounding case.

Two limitations that bound what the score means:

1. **Equivalent mutants.** Some mutations do not change behaviour at all. Those survive for
   reasons unrelated to the oracle. A survivor is a question, not a proven defect.
2. **Coverage confounds strength.** A mutant on a line no scenario reaches survives because
   nothing runs it. Survivors are reported with file and line so a human keeps that distinction.

Docstrings are never mutated — a mutated docstring always survives, which would fill the
survivor list with findings nobody can act on. Mutants that break the suite itself are counted
`invalid` and excluded rather than scored, because they prove nothing about the assertions.

Engines: `python` mutates the AST (comparisons, arithmetic, booleans, numeric, string and
boolean literals); `text` is deliberately crude and works on Go, JavaScript, Java, Ruby and C#.
`auto` picks by stack.

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
shalt init                       scaffold a workspace
shalt author "<request>"         English -> Gherkin under spec/
shalt approve --by <you>         human sign-off; stamps ids, locks the spec
shalt steps                      stepwright writes steps/ + contract/
shalt build [--max-turns N]      implementer loop until green, then verify with holdouts
shalt run                        run the suite, update the ledger
shalt status                     the ledger, as a progress view
shalt verify                     standing integrity audit
shalt tree                       epic -> story -> scenario, with status
shalt stories                    who wants what, and what is missing a narrative
shalt diagrams                   Mermaid use-case, breakdown and pipeline diagrams
shalt dashboard                  a self-contained HTML dashboard
shalt mutate                     mutation-test the oracle: do the scenarios mean anything?
```

## Backends

- `--backend fixture --fixtures <dir>` — replays recorded turns. Offline, deterministic; this
  is what the tests and the demo use.
- `--backend grok` — xAI, via its OpenAI-compatible API. Needs `XAI_API_KEY`.
- `--backend openai` — same adapter, different preset. Needs `OPENAI_API_KEY`.
- `--backend claude-cli` — runs each role as a headless `claude -p` turn inside its staged
  directory.

Adding a backend is one class with a `run(role, prompt, stage)` method.

### Running it against Grok

```bash
export XAI_API_KEY=...
shalt --root ./work init
shalt --root ./work --backend grok author "<what you want built>"
$EDITOR work/spec/*.feature          # this is the review gate; it is the cheap one
shalt --root ./work approve --by you@example.com
shalt --root ./work --backend grok steps
shalt --root ./work --backend grok build --max-turns 8
shalt --root ./work verify
```

`--model` overrides the default (`grok-4`); if that name is wrong the error lists what your key
can actually see. `--base-url` points the same adapter at any other OpenAI-compatible endpoint.

The model works through four scoped tools — `list_files`, `read_file`, `write_file`, `done` —
rather than a shell. Every path is resolved inside the stage first: absolute paths are refused
outright rather than reinterpreted, traversal is refused, and any symlinked component is
refused. A refusal goes back to the model as a tool result, so it can correct course instead of
crashing the turn. That sandbox is the first of the three layers, not a replacement for the
workspace guard, which still hashes and rolls back around every turn.

## What this prototype does and does not prove

Demonstrated, end to end, in `examples/invoice/demo.sh` and the 37 tests:

- the pipeline runs: prompt → Gherkin → human approval → step definitions → implementation → green
- an implementer that edits the tests is caught and rolled back — by five different routes
- an implementer that overfits to visible examples is caught by holdouts
- a stepwright that writes assertions testing nothing is caught by mutation testing
- held-out scenarios' failure text is never shown to the implementer
- editing an approved scenario stales exactly that scenario's green and no other
- `shalt verify` detects any edit, addition or deletion made since sign-off
- breaking a passing scenario, or deleting the test that proved it, is recorded as a regression
- deleting failing scenarios does not silently inflate the completion figure

Not yet addressed, in rough order of how much they matter:

0. **This has never run against a live model.** The Grok/OpenAI adapter is fully exercised —
   the whole pipeline runs over real HTTP against a mock that speaks the xAI wire protocol
   (`tests/mock_llm.py`), covering request construction, the tool loop, refusals, retries and
   token accounting. But a mock cannot have judgement. Nothing here shows whether a real model
   writes *good* Gherkin, whether the stepwright writes honest assertions, or how many turns a
   real build takes. The `claude-cli` backend is written and entirely unexercised.
1. **Gherkin's expressiveness ceiling.** Latency, cost, security posture, UI behaviour — Gherkin
   is bad at all of them. There is currently no escape hatch, which means a spec written only in
   Gherkin is a lie by omission.
2. **Holdouts are a sample, not a proof.** They catch crude overfitting. A sufficiently capable
   implementer generalises just far enough to pass them.
4. **No cost or turn accounting.** A build loop that runs 40 turns should say what it spent.
5. **Non-Python runners.** The zone model is language-agnostic; the runner is not.

## Layout

```
shalt/
  spec.py           Gherkin parsing, scenario identity, canonical hashing, holdouts
  ledger.py         the scenario ledger and its status rules
  integrity.py      zones, write guards, rollback, standing audit
  roles.py          staged, guarded turns
  backends.py       fixture and claude-cli backends
  runner.py         drives pytest-bdd
  pytest_plugin.py  maps pytest-bdd outcomes back to scenario ids
  cli.py
  narrative.py      user stories: As a / I want / So that
  config.py         shalt.toml -- what makes the runner language-agnostic
  reports.py        Cucumber JSON and Cucumber Messages parsers, bound by @rid tag
  viz.py            the tree, the Mermaid diagrams, the HTML dashboard
  mutate.py         mutation testing: is the oracle actually asserting anything?
```

## Licence

**Not yet licensed.** This repository is private and all rights are reserved.

The intention is to release it under the **MIT License**. Until then there is deliberately no
`LICENSE` file, because adding one would be the grant itself. What is in place instead is the
machinery that keeps the option open: contributions require DCO sign-off, dependency licences are
tracked, and `CONTRIBUTING.md` carries the checklist for whoever flips the switch.

If you have been given access to this repository, that is access — not a licence.
