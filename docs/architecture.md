# Architecture

## The problem

An agent that writes the specification, the tests, and the implementation can always make the
suite pass. A green suite then proves internal consistency, not correctness — self-graded
homework. Every mechanism in `shalt` exists to take some part of that capability away
structurally, rather than asking for it politely in a prompt.

Three distinct failure modes, which need three distinct defences:

| failure | what it looks like | defence |
|---|---|---|
| **tampering** | the implementer edits or weakens the tests | zones + write guard ([isolation.md](isolation.md)) |
| **overfitting** | the implementer special-cases the examples it was shown | holdout scenarios ([hierarchy.md](hierarchy.md)) |
| **weak oracle** | the stepwright writes assertions that do not check anything | mutation testing ([mutation.md](mutation.md)) |

A fourth failure is not an agent's fault at all but is just as damaging: **drift**, where the
spec changes and the suite keeps passing because nobody re-checked. That one is handled by
binding status to a content hash ([identity.md](identity.md)).

## Core concepts

| term | meaning |
|---|---|
| **zone** | a directory writable by exactly one role: `spec/`, `steps/`, `contract/`, `src/` |
| **role** | one agent job: `author`, `stepwright`, `implementer` |
| **rid** | a durable scenario id (`@rid:S-b291b8fd`) stamped into the feature file at approval |
| **canonical hash** | a hash of everything that changes a scenario's meaning, and nothing that doesn't |
| **spec lock** | the record of who approved which scenario contents, and when |
| **ledger** | `.shalt/ledger.json` — the durable record of every scenario's status |
| **upheld** | a bound test passes, against the current meaning of the scenario |
| **holdout** | an approved scenario never shown to the implementer, used to detect overfitting |
| **oracle** | the step definitions: the thing that decides whether behaviour is correct |
| **blind spot** | a surviving mutation in a file a passing scenario provably executes |

## The pipeline

```mermaid
graph LR
  req["Plain-English request"] --> author["author agent"]
  author --> story["User story<br/>As a / I want / So that"]
  story --> gherkin["Gherkin scenarios<br/>Given / When / Then"]
  gherkin --> gate{"Human<br/>approval"}
  gate -->|"stamps @rid,<br/>records content hashes"| locked["Approved spec"]
  locked --> sw["stepwright agent<br/>sees spec only"]
  sw --> steps["Step definitions"]
  sw --> contract["Interface contract"]
  contract --> impl["implementer agent<br/>never sees the steps"]
  locked --> impl
  impl --> src["Implementation"]
  steps --> run["Test run"]
  src --> run
  run --> ledger["Scenario ledger"]
  ledger -->|"still failing"| impl
  ledger --> hold["Held-out scenarios<br/>checked, never shown"]
  ledger --> mut["shalt mutate<br/>does the oracle mean anything?"]

  style gate fill:#a9761b,stroke:#7d570f,color:#ffffff
  style locked fill:#3d4f7c,stroke:#2a3757,color:#ffffff
  style ledger fill:#1f7a4c,stroke:#155a38,color:#ffffff
  style hold fill:#6e5a86,stroke:#514166,color:#ffffff
  style mut fill:#b3382e,stroke:#8a2b23,color:#ffffff
```

The human gate sits at the Gherkin, deliberately. Gherkin's real virtue is not that it is
executable — it is that it is **cheap for a non-engineer to read**. That makes it the only place
in the pipeline where human review is both meaningful and affordable.

## Zones and roles

Five zones, each with exactly one writer (human may write all):

| zone | written by | read by | contains |
|---|---|---|---|
| `spec/` | author | everyone | Feature files and user stories |
| `mockups/` | **designer** | designer, implementer, human | HTML storyboards (sketched until green) |
| `steps/` | **stepwright** | the test runner | executable step definitions — the oracle |
| `contract/` | **stepwright** | implementer | the API surface the steps will call |
| `src/` | implementer | implementer, test runner | the implementation |

Read access is narrower than write access, and that asymmetry is the point:

```mermaid
graph TD
  subgraph author["author"]
    a1["writes spec/"]
  end
  subgraph stepwright["stepwright — cannot see src/"]
    s1["reads spec/"]
    s2["writes steps/ + contract/"]
  end
  subgraph implementer["implementer — cannot see steps/"]
    i1["reads spec/ + contract/"]
    i2["writes src/"]
  end
  a1 --> s1
  s2 --> i1
  i2 --> tr["test runner<br/>the only thing that sees steps/ and src/ together"]
  s2 --> tr
  tr --> led["ledger"]

  style stepwright fill:#eef1f7,stroke:#3d4f7c,color:#1a1f26
  style implementer fill:#eef1f7,stroke:#3d4f7c,color:#1a1f26
  style tr fill:#3d4f7c,stroke:#2a3757,color:#ffffff
  style led fill:#1f7a4c,stroke:#155a38,color:#ffffff
```

The implementer receives the spec, the interface contract the stepwright declared, and the
failing test output — but never the step definitions themselves. It therefore has to implement
the *behaviour* rather than the *assertions*. The `contract/` zone exists precisely so that
withholding `steps/` does not also withhold the API surface the implementer legitimately needs.

## Module map

3,335 lines across 15 modules. Dependencies flow strictly downward — no cycles except the
deliberate lazy import between `backends` and `api_backend`.

```mermaid
graph TD
  cli["cli.py — 557"]
  viz["viz.py — 552"]
  mutate["mutate.py — 372"]
  spec["spec.py — 330"]
  api["api_backend.py — 265"]
  ledger["ledger.py — 238"]
  integrity["integrity.py — 202"]
  config["config.py — 162"]
  reports["reports.py — 146"]
  backends["backends.py — 118"]
  roles["roles.py — 117"]
  plugin["pytest_plugin.py — 103"]
  runner["runner.py — 90"]
  narrative["narrative.py — 77"]

  cli --> viz
  cli --> mutate
  cli --> roles
  cli --> ledger
  cli --> integrity
  cli --> runner
  cli --> config
  cli --> spec
  cli --> backends
  cli --> narrative
  mutate --> runner
  mutate --> config
  roles --> integrity
  roles --> spec
  runner --> reports
  runner --> config
  reports --> spec
  integrity --> spec
  viz --> narrative
  spec --> narrative
  backends <--> api
  plugin --> spec

  style cli fill:#3d4f7c,stroke:#2a3757,color:#ffffff
  style spec fill:#1f7a4c,stroke:#155a38,color:#ffffff
  style ledger fill:#1f7a4c,stroke:#155a38,color:#ffffff
  style integrity fill:#b3382e,stroke:#8a2b23,color:#ffffff
```

| module | responsibility |
|---|---|
| `spec.py` | Gherkin parsing, scenario identity, canonical hashing, holdout stripping |
| `narrative.py` | user-story grammar (`As a / I want / So that`) |
| `ledger.py` | the scenario ledger, its status rules and its invariants |
| `integrity.py` | zones, read/write maps, write guards, rollback, the standing audit |
| `roles.py` | staged, guarded turns — the isolation mechanism |
| `config.py` | `shalt.toml`, runner presets: what makes the tool language-agnostic |
| `reports.py` | Cucumber JSON and Cucumber Messages parsers, bound by `@rid` tag |
| `runner.py` | invokes the configured runner, folds results back |
| `pytest_plugin.py` | native Python binding: pytest-bdd outcomes → rids |
| `backends.py` | the `Backend` protocol; fixture and `claude -p` adapters |
| `api_backend.py` | OpenAI-compatible adapter (Grok, OpenAI) with a scoped tool loop |
| `mutate.py` | mutation testing the oracle |
| `viz.py` | the tree, Mermaid diagrams, the HTML dashboard |
| `cli.py` | command surface |

## Workspace layout

```
myproject/
  shalt.toml              runner command and report format
  spec/                   Gherkin + user stories        (author writes)
    billing/
      invoice.feature
  steps/                  step definitions              (stepwright writes)
  contract/
    interface.md          the API surface the steps call (stepwright writes)
  src/                    implementation                (implementer writes)
  .shalt/
    ledger.json           the durable record
    last_run.json         most recent run report (transient)
  docs/
    dashboard.html        generated by `shalt dashboard`
    diagrams/*.mmd        generated by `shalt diagrams`
```

`.shalt/stage/` and `.shalt/backup/` are transient and git-ignored. `.shalt/ledger.json` is
the artifact worth committing: it is the record of what was approved and what has been upheld.

## Data flow for one build turn

```mermaid
sequenceDiagram
  participant CLI as cli.cmd_build
  participant R as runner
  participant L as ledger
  participant Ro as roles.run_role
  participant G as GuardedTurn
  participant B as backend

  CLI->>R: run_suite(root, cfg)
  R->>R: invoke configured runner command
  R->>R: parse report by format, key by @rid
  R-->>CLI: {results, harness_error, collection_error}
  CLI->>L: apply_run(results, blocked=collection_error)
  L->>L: green / red / stale / pending, record regressions
  CLI->>CLI: still-failing visible scenarios?
  CLI->>Ro: run_role("implementer", prompt, hide_holdouts=True)
  Ro->>Ro: stage readable zones OUTSIDE the workspace
  Ro->>Ro: strip @holdout scenarios from the staged spec
  Ro->>G: enter — hash workspace + ledger, back up protected zones
  Ro->>B: backend.run(role, prompt, stage)
  B-->>Ro: transcript, files written into the stage
  Ro->>Ro: scan stage for out-of-zone writes and symlinks
  Ro->>Ro: diff read-only staged zones
  Ro->>Ro: mirror own zones back (deletions included)
  Ro->>G: exit — re-hash, any protected change is a violation
  G-->>CLI: IntegrityViolation → turn rejected and rolled back
```

The loop terminates on all-visible-upheld, or on `--max-turns`. Then a **final verification run
includes the holdouts**: if every visible scenario is upheld and a held-out one is not, that is
reported as overfitting.
