# Shalt — Rust engine, CLI, and Linear-shaped UI

**Date:** 2026-09-13
**Status:** draft, awaiting review
**Repo:** this repository, in place (`github.com/rivletio/shalt`)
**Replaces:** the Python package `shalt` 0.1.0

## Goal

Port the existing shalt prototype to a single Rust binary that is the whole product.

A user who has never heard of shalt installs one file and runs `shalt ui`. The browser is the primary surface: a Linear-inspired org of projects, with goals, milestones, epics, stories, tasks, and optional sprints. They queue jobs and watch a spec turn into stories, UX diagrams, and a board. They can prioritize, delete, or modify what appears. Tests and code are the last two stages of that same pipeline, not a separate tool.

A CI job, a script, or a user who prefers the terminal runs the same binary without the UI. Everything the UI can do, the CLI can do.

The product claim does not change. An agent that writes the spec, the tests, and the implementation can always make a suite pass. Shalt takes that capability away structurally: zones, a write guard, holdouts, canonical-hash identity, and mutation-testing the oracle. Green means an obligation is upheld against its exact wording.

The board does not replace that claim. **Epics, stories, and tasks are still a reading of Gherkin.** Org, project, goals, milestones, and sprints are a Linear-style overlay that ranks and schedules those rids. Overlay edits do not rewrite obligations. Content edits of an epic/story/task write through to `spec/` and go through the same approve / hash / stale rules as today. Drift between overlay and spec is a `verify` failure, not a silent disagreement.

## Why rewrite

The Python prototype proved the claim. It did not prove distribution. The runner preset shells out to `python -m pytest`; on a machine where `python` is not the interpreter that has `shalt` installed, the inner suite fails to import `shalt.pytest_plugin` and every scenario goes red with a collection error. That is the same class of bug as “the guard was never wired in”: the mechanism works in tests and fails as a product.

A one-binary install removes the interpreter, the inner-plugin import, and the “is this the right Python?” question. The live UI is the other half: the human gate is reading and shaping work, and that is cheaper on a board than in a terminal dump.

## Non-goals (v1)

- Hosted / multi-user / accounts. The org is local.
- Sync to GitHub Issues, Linear, or Jira.
- A background daemon. `shalt ui` is a blocking localhost server. The job queue lives on disk and runs while a UI or `shalt job run` is alive.
- New obligation types for non-functional requirements. Gherkin’s ceiling stays a stated limitation.
- Mutation-testing the step definitions themselves.
- A Tauri/desktop wrapper. The UI is a browser on localhost.
- Timeline / Gantt. v1 is dashboard, board, list, spec, run.
- Keeping the Python package on PyPI in parallel. This repo becomes the Rust tree. Python is deleted when the port is green, not dual-shipped.

## Key decisions

1. **One binary, two fronts.** `shalt-core` is a library. The `shalt` binary is a clap CLI and, for `shalt ui`, an axum server that calls the same library functions. The UI cannot do anything the CLI cannot; the CLI cannot do anything the UI cannot. There is no private side door.
2. **Replace this repo in place.** `crates/shalt-core`, `crates/shalt`, `ui/`. Python sources go away once the Rust suite, `examples/invoice/demo.sh`, and `examples/rust-billing` are green.
3. **TypeScript SPA, compiled in.** Vite builds `ui/` to static assets. `rust-embed` includes them in the binary. End users need neither Node nor Python. Shalt developers need Node only to build the UI. `ui/dist` is never committed; CI builds it before `cargo build`.
4. **Keep the workspace contract.** Each *project* is still `spec/`, `steps/`, `contract/`, `src/`, `.shalt/ledger.json`, `shalt.toml`. A Python-era workspace is still a shalt project. The engine never runs Python; it runs whatever `[runner].command` says.
5. **Keep `shalt.ledger/1`.** Do not bump the schema unless a field’s meaning must change. Unknown fields remain tolerated.
6. **Drop the in-package pytest plugin.** Binding is by `@rid:` in Cucumber JSON / Cucumber Messages / shalt JSON. `shalt init --stack python` writes a workspace-local reporter template; rust-billing already proved the tag-in-report path. Uninstalling shalt must not break `pytest` in a Python project.
7. **Install is `curl | sh` of a GitHub-release binary** (cargo-dist). `brew` is later, not v1.
8. **`shalt ui` is blocking, opens the browser, streams events over SSE.** No launchd. Close the terminal, the UI is gone; queued jobs remain on disk.
9. **Tests of integrity go through `run_role` with a hostile backend**, never the guard helper.
10. **Absence of evidence never rounds toward success.** Pending is not green. A silent mutation run is invalid, not survived. A bind of zero scenarios when the spec has rids is suspicious and reported.
11. **Org home, many projects.** One `shalt ui` session sees a local org: a catalog of git workspaces. Sidebar is Linear-like. `shalt ui --root DIR` can still open a single project. CLI `--root` always targets one project.
12. **Linear overlay, not Linear invert.** Schema `shalt.board/1`. Goals, milestones, optional sprints, and rank live in the overlay. Epic / story / task identity, wording, and upheld-status live in Gherkin + the ledger. The overlay points at rids; it does not copy their text.
13. **Two kinds of edit, two write paths.** Rank, milestone, sprint, goal, “remove from board”: overlay only. Change story text, add/delete a scenario, retag epic: write `spec/`, stamp/stale rules apply, approve if the spec lock requires it.
14. **Jobs are durable on disk, executed in-process.** No daemon. `shalt job add` / the UI enqueue to `~/.shalt/jobs.json`. `shalt ui` and `shalt job run` both process the queue. SSE is how you watch.
15. **Sprints are optional.** The overlay always has the field. A project can hide sprints. The pipeline still reads: plan → tests → code.
16. **`verify` fails on overlay drift.** Dangling rid, missing project path, sprint/milestone/goal id that does not exist. An unscheduled rid (in spec, not on the board) is backlog, not drift.
17. **Onboard an existing repo.** `shalt onboard PATH` and the UI path field register the git workspace in place (they do not copy it into `~/.shalt/projects`). `ensure_workspace` writes `shalt.toml`, `spec/`, `steps/`, `contract/`, `.shalt/` only when missing, and never clobbers existing source. Author **reads** `src/` as well as `spec/`, and still **writes only** `spec/`. The prompt is: describe behaviour the code already implements; do not invent features. Stepwright still does not see `src/`.
18. **Ask with a guess, and a breakout chat.** `ask_human` always sends a concrete `guess`. The answer field is pre-filled with it. **Discuss** expands a chat on the ask card and opens the right-hand rail. **Add to spec** (and Continue) folds the accepted answer into the visible **plain language spec** (`job.prompt`) under a heading for that question, then continues the author. The chat is how the English spec is written; Gherkin is compiled from it. Voice in Ikonic is the same chat.
19. **Ikonic submodule, Linear glass.** The package at `ikonic/` embeds `shalt ui ?embed=1`. Ikonic is the host; the shalt surface is Linear-inspired (indigo, issue rows, light/dark/blue). `shalt ui` remains the engine.

## Architecture

```
 ~/.shalt/org.toml          org name + project paths
 ~/.shalt/board.json        org-level goals (span projects)
 ~/.shalt/jobs.json         durable job queue

 project/                   one git workspace per project
   spec/ steps/ contract/ src/
   .shalt/ledger.json       obligations (shalt.ledger/1)
   .shalt/board.json        rank, goal, milestone, sprint per rid

                    ┌─────────────────────────────────────┐
                    │              shalt binary           │
                    │  clap  ──┐                          │
                    │          ├── shalt_core             │
                    │  axum  ──┘  spec ledger integrity   │
                    │  + embed    roles runner mutate     │
                    │             org board jobs          │
                    └─────────────────────────────────────┘
```

### Crates

**`crates/shalt-core`** — no HTTP, no clap, no UI.

| module | Python origin | responsibility |
|---|---|---|
| `spec` | `spec.py` | Gherkin parse (`gherkin` crate), `@rid:` / `@holdout` / `@epic:`, canonical hash, holdout stripping |
| `narrative` | `narrative.py` | `As a / I want / So that` |
| `ledger` | `ledger.py` | `shalt.ledger/1`, status machine, regressions, mutation fields |
| `integrity` | `integrity.py` | zones, `READS`/`ZONES`, `GuardedTurn`, snapshot/diff, audit |
| `roles` | `roles.py` | stage outside workspace, `run_role`, mirror-back including deletions |
| `config` | `config.py` | `shalt.toml`, presets, `{spec}` `{steps}` `{report}` `{src}` |
| `reports` | `reports.py` | shalt JSON, Cucumber JSON (with and without leading `@` on tags), Cucumber Messages |
| `runner` | `runner.py` | invoke configured command, fold report, `[runner].env` with placeholder substitution |
| `backends` | `backends.py`, `api_backend.py` | fixture, OpenAI-compatible HTTP (Grok/OpenAI), `claude-cli` |
| `mutate` | `mutate.py` | python AST engine **and** text engine; restore-and-touch sources; re-run baseline; silence ≠ survived |
| `viz` | `viz.py` (data only) | tree model, use-case/breakdown/pipeline as data. Rendering moves to CLI and UI |
| `org` | new | org catalog, project membership |
| `board` | new | `shalt.board/1` overlay: goals, milestones, sprints, rank; bind by rid |
| `jobs` | new | durable queue, job kinds, status |

**`crates/shalt`** — binary.

- `cli` — clap, every current command, plus `ui`, `org`, `project`, `board`, `job`.
- `term` — colour, OSC 8, Gherkin highlighting. Labelled rows keep the label; standalone path references print the path.
- `server` — axum. REST for commands, SSE for jobs and runs. Serves embedded `ui/dist`; in dev, proxies to Vite.
- Exit codes stay as documented in `docs/cli.md` for existing commands. New commands use the same family (2 integrity, 4 spec parse).

Do not add a third crate until the port forces it.

### Hierarchy (what the UI organises)

```
Org
 └── Project          = one git workspace, one ledger
      ├── Goal        = overlay, may span projects
      ├── Milestone   = overlay, per project
      ├── [Sprint]    = overlay, per project, optional
      ├── Epic        = Gherkin `@epic:` or spec/ directory
      │    └── Story  = feature + As a / I want / So that
      │         └── Task = scenario, rid, ledger status
      └── Tests → Code = shalt steps / build against those rids
```

Roll-up: a parent is never greener than its children (already a ledger invariant). The dashboard applies that from task → story → epic → milestone/sprint → project → org. Overlay containers with no bound rids are empty, not green.

### UI (`ui/`)

Vite + TypeScript + Preact. Preact is the SPA runtime because the bundle is embedded; it is not a product identity.

Visual language is Linear-inspired (not a Linear clone). Shalt still loads as an Ikonic submodule; the glass inside the iframe is Linear.

- Inter / system sans. Tight tracking on titles. No serif, no LCARS uppercase capsules.
- Near-black surfaces, 8px radius, hairline borders, indigo `#5e6ad2`.
- Sidebar workspace (org), Inbox, project list. Jobs as issue rows: id, status dot, kind.
- Machine state (ids, hashes, rids, job ids) in a mono.
- Progress as discrete notches, one per task — not a smooth bar.
- Light, dark, and blue themes. Usable at phone width. Embeddable (`?embed=1`).

Layout: left sidebar, main pane, right rail for Discuss. Interview chat is a breakout from the ask card, not a shell log.

| route | job |
|---|---|
| `/` | org dashboard: every project’s notch meter, jobs in flight, next actions, drift warnings |
| `/projects/:id` | project dashboard: goals, milestones, sprint (if on), epic roll-up |
| `/projects/:id/board` | board: columns are ledger statuses (and a sprint swimlane if sprints are on). Rank is overlay. Drag to prioritize. Drag to a sprint/milestone is overlay. Drag to “delete from spec” is *not* a drag — that is a destructive confirm. |
| `/projects/:id/list` | Linear-style list: epic → story → task, filters for goal/milestone/sprint/holdout |
| `/projects/:id/spec` | the review gate: full Gherkin, holdouts marked, approve, write-through edit |
| `/projects/:id/stories` | who wants what; missing narratives named; use-case diagram derived from those clauses |
| `/projects/:id/run` | live job / build / steps; SSE transcript; integrity rejections |
| `/projects/:id/mutate` | campaign, vacuous ∪ blind_spot, survivors with file:line |
| `/projects/:id/ledger` | ledger, spec lock, regressions |
| `/goals/:id` | goal across projects: bound rids, roll-up |
| `/jobs` | queue: pending, running, done, failed; click to watch |

`shalt dashboard` still writes `docs/dashboard.html` *inside a project* as a snapshot for git. The live org UI replaces using that file as the working surface.

### Process model

```
$ shalt ui                 # org home, catalog at ~/.shalt/org.toml
$ shalt ui --root DIR      # single project (still valid)
$ shalt ui --port 7700 --no-open
```

- Binds `127.0.0.1` only. Not `0.0.0.0`.
- Opens the default browser unless `--no-open`.
- Blocks the terminal. Ctrl-C stops the server. Jobs that were running are left `interrupted` on disk and can be retried.
- API is same-origin. No auth on localhost. CORS is not opened to other origins.
- The CLI never talks to the UI server. Both talk to the library and the files on disk.

Ledger writes take an exclusive file lock on that project’s `.shalt/ledger.json`. Overlay writes lock `.shalt/board.json` or `~/.shalt/board.json`. Job queue writes lock `~/.shalt/jobs.json`. A second process waits or fails fast; it does not interleave JSON.

## Overlay schema (`shalt.board/1`)

Org file `~/.shalt/org.toml`:

```toml
name = "Rivlet"
[[projects]]
id = "invoice"
name = "Invoice"
path = "~/work/invoice"
```

`shalt org add PATH` registers an existing directory, scaffolds missing shalt files, and does not delete the git repo. `shalt onboard PATH [note]` does that and starts an author job that reads `src/` and writes Gherkin for behaviour already in the code. `shalt org rename ID NAME` changes the display name (id and jobs stay put). `shalt org remove ID` drops it from the catalog only. The UI can rename and remove from the project card and the project page.

Two overlay files, same schema name, different contents:

`~/.shalt/board.json` — org-level goals only (a goal may list several `project_ids`).

```json
{
  "schema": "shalt.board/1",
  "goals": [{ "id": "G-8f2a91c0", "title": "Exact billing", "project_ids": ["invoice"] }]
}
```

`<project>/.shalt/board.json` — that project’s milestones, sprints, and rid items. `goal_id` points at an org goal.

```json
{
  "schema": "shalt.board/1",
  "milestones": [{ "id": "M-1", "title": "v1 totals", "target": "2026-10-01" }],
  "sprints": [{ "id": "C-1", "title": "W38", "start": "2026-09-14", "end": "2026-09-20", "enabled": true }],
  "items": [{
    "rid": "S-b291b8fd",
    "rank": 100,
    "goal_id": "G-8f2a91c0",
    "milestone_id": "M-1",
    "sprint_id": null
  }]
}
```

Unknown fields tolerated, same rule as the ledger. After a successful author turn — in core, not in the agent stage — `sync_spec` plus overlay auto-add appends new rids at the end of the backlog (`rank` max+1). The agent cannot write `.shalt/board.json`. Deleting a scenario from the spec orphans the ledger entry; verify then fails until the overlay item is dropped or the scenario is restored.

**Remove from board** (UI/CLI): delete the overlay item. The scenario remains in the spec (unscheduled backlog). **Delete obligation**: remove the scenario from `spec/`, which is a spec write, goes stale/orphan, and requires the same confirmation as deleting from a feature file today.

## Jobs

`~/.shalt/jobs.json` — durable, versioned (`shalt.jobs/1`).

| kind | what it does | watches as |
|---|---|---|
| `author` | English → Gherkin under that project’s `spec/` | features appearing, then overlay items appearing, then use-case diagram updating |
| `steps` | stepwright turn | `steps/` + `contract/` |
| `build` | implementer loop + holdout verify | board statuses flipping, OVERFIT if it happens |
| `run` | suite once | ledger |
| `mutate` | oracle campaign | weak_oracles |
| `diagrams` | regenerate mermaid + dashboard snapshot | files under `docs/` |
| `verify` | integrity + overlay drift | problem list |

Approve is **not** a job. It is a human action on `/spec`.

Enqueue from the dashboard or `shalt job add <kind> --project invoice`. The UI shows a queue column: created → running → done/failed, with SSE events `job-start`, `job-log`, `board-sync`, `suite-result`, `integrity-violation`, `job-end`.

Watching “spec turn into stories + UX diagrams + a board” is exactly: an `author` job writes `spec/`; `sync_spec` + overlay auto-add; viz model refreshes; the board and the use-case diagram re-render from that model. No second generator.

## Workspace and runner contract

Unchanged on disk inside a project:

```
myproject/
  shalt.toml
  spec/
  steps/
  contract/
  src/
  .shalt/ledger.json
  .shalt/board.json
```

`shalt.toml` presets stay: python, javascript, go, java, ruby, rust, dotnet. The python preset **changes**: it does not import anything from the shalt binary. `shalt init --stack python` writes a **workspace-local** pytest plugin (a file under `steps/`, copied from a template in the binary) that emits shalt JSON to `{report}`. The invoice example uses that same template. Binding remains by `@rid:`. The plugin lives in the workspace so a shalt uninstall cannot break a project’s ability to run tests — only to fold them into the ledger.

The rust preset stays the rust-billing command (`cargo test --test cucumber` + `SHALT_REPORT`). Tag names are normalised by stripping a leading `@` before matching `rid:`.

Runner invocation uses `PATH` as found, not a hardcoded `python`. Placeholders: `{spec}` `{steps}` `{src}` `{report}`. `[runner].env` still substitutes `{report}`.

## Isolation

Three layers, all required, all tested through `run_role`:

1. Stage directory is created with `tempfile` **outside** the workspace. Relative traversal from the stage cannot reach `workspace/steps`.
2. After the backend returns, scan the stage: any file whose top-level component is not in the role’s write or read zones is an offence; any symlink anywhere is an offence.
3. `GuardedTurn` hashes the real workspace (all four zones + `.shalt/ledger.json`) before and after. Protected zones are copied to a backup with **content restored and mtimes bumped** on rollback/restore so compiled-language toolchains rebuild. A backend that writes by absolute path is still caught. `.shalt/board.json` is protected on agent turns the same way as the ledger: no role may write the overlay.

The API backend’s tools (`list_files`, `read_file`, `write_file`, `done`) resolve paths inside the stage: refuse absolute paths, refuse traversal, refuse any symlink component. Refusal is a tool result, not a crash.

`claude-cli` remains an untrusted, unverified backend. Docs keep saying so.

## Data flow

Every mutating command is: load spec → sync ledger → sync overlay → do the thing → save → return a structured result. The CLI prints it. The UI renders it. SSE is for jobs and other long operations.

```
UI or CLI
  → core::author | approve | steps | build | run | mutate | verify
  → core::board::{rank, assign, unschedule}
  → core::jobs::{enqueue, run}
      → backends::run(role, prompt, stage)
      → integrity::GuardedTurn
      → runner::run_suite
      → ledger::apply_run | apply_mutation | spec_lock
      → board::sync_new_rids
  → JSON result + optional SSE events
```

Approve is the human gate. The UI’s spec page is where it is meant to happen. `--yes` exists on the CLI for fixtures and CI; the UI never auto-approves.

Holdouts: stripped from the implementer’s staged spec; filtered from the failure digest; included in the final verification run. Visible-all-green + holdout-red = `OVERFIT`. Holdouts still appear on the board, marked.

## Error handling

| event | CLI | UI | stores |
|---|---|---|---|
| integrity violation | exit 2, offence list, nothing kept | modal + transcript | ledger/board unchanged |
| spec does not parse | exit 4 | inline errors per file | not written |
| not approved | refuse `steps` / `build` | spec page is the next action | — |
| collection failure | scenarios red with harness text | red, harness panel | red, not pending |
| runner missing / timeout | harness error | same | no fake green |
| mutate with no green baseline | refuse to score | refuse, explain | mutation section empty |
| mutate baseline does not reproduce | refuse to report a score | same | — |
| live API 4xx/5xx | error, list models if the name is wrong | same | — |
| overlay drift | `verify` non-zero, list | dashboard warning + list | not auto-repaired |
| missing project path | org command errors | project card “path missing” | catalog kept |
| job interrupted (UI quit) | `shalt job` shows interrupted | same on next open | retryable |

SSE events for a job: `job-start`, `job-log`, `board-sync`, `turn-start`, `turn-end`, `integrity-violation`, `suite-result`, `overfit`, `job-end`. A dropped SSE connection does not cancel the job; the UI reconnects to current ledger + overlay + job state.

## Install

`install.sh` at the repo root and served from the GitHub release:

1. Detect OS/arch (darwin/linux, amd64/arm64). Windows is documented as unsupported in v1, not silently attempted.
2. Download the matching `shalt` tarball from GitHub Releases (cargo-dist).
3. Install to `${SHALT_INSTALL_DIR:-$HOME/.local/bin}`.
4. Print `shalt --version`, `shalt ui --help`, and a PATH hint if the dest is not on PATH.

No root required. No package manager. No Node, no Python, no Rust toolchain for users.

Developers of shalt: Rust stable, Node LTS (UI build only), and for tests the toolchains of the examples they run (Python for invoice fixtures’ pytest-bdd steps, Rust for rust-billing).

## Testing

The port is not done until all of the following are green. Tests are written against public entry points.

**Invariant suite** (`shalt-core`):

- One test per name in `tests/test_isolation.py` and `tests/test_defects.py`, behaviour-named, hostile-backend pattern.
- Ledger invariants from `docs/ledger.md`.
- Reports: cucumber-rs tag without `@`; cucumber-jvm tag with `@`; zero binds when rids exist is a reported anomaly.
- Mutate: restore touches mtimes; post-campaign baseline must reproduce; silence is invalid; docstrings not mutated; `weak_oracles` is the union.
- Overlay: new rid auto-adds; remove-from-board does not touch spec; delete-obligation does; rank does not rewrite Gherkin; verify fails on a dangling rid; parent roll-up still never greener than children across overlay containers; agent turn cannot write `.shalt/board.json`.
- Jobs: enqueue is durable across process exit; `interrupted` is retryable; author job results in overlay items whose rids parse from spec.

**Adapter suite:**

- Wire-protocol mock of xAI chat-completions. Full author → approve → steps → build over real HTTP. Guard still rejects an API-driven out-of-zone write.

**Demos, not optional:**

- `examples/invoice/demo.sh` scenes 1–7.
- `examples/rust-billing` `shalt run`, `tree`, `verify`, `mutate --engine text`.
- A two-project org fixture: invoice + rust-billing registered, dashboard roll-up, one author job watched until a new story is on the board.

**UI:**

- Org dashboard, open a project, approve on `/spec`, queue a `build`, watch the board, cheat-fixture rejection. A screenshot is not the test. Ledger + overlay on disk after the session are the test.
- Prioritize (rank change) does not change canonical hashes. Delete-obligation does.

**Diagrams:** generated mermaid must parse.

Do not claim “tested against Grok.” Claim “plumbing proven against the wire mock; judgement unproven.”

## Migration of this repository

Order of landing, each mergeable:

1. Scaffold `crates/`, `ui/`, `install.sh`, workspace `Cargo.toml`. Python remains the working tool.
2. Port `spec` + `narrative` + `ledger` with their tests. Golden: parse the invoice features, hashes stable.
3. Port `integrity` + `roles` with the hostile-backend tests. This is the load-bearing slice.
4. Port `config` + `reports` + `runner`. rust-billing `shalt run` works from the Rust binary.
5. Port backends (fixture first, then HTTP mock, then claude-cli stub). Invoice demo scenes 1–3.
6. Port `mutate`. Demo scene 7 + rust-billing mutate.
7. Port CLI parity (`init` … `mutate`, `tree`, `stories`, `diagrams`, `dashboard`, `show`).
8. `org` + `board` + `jobs` modules, CLI, verify-drift. Two-project fixture.
9. UI: org dashboard, project board/list/spec, SSE jobs, approve gate, rank/assign/unschedule.
10. `install.sh` + cargo-dist release dry-run.
11. Delete `shalt/` (Python), `pyproject.toml`, Python-only tests; point README at the binary; `examples/invoice/demo.sh` uses `shalt` on PATH.

No commit in this sequence ships a `GuardedTurn` that `run_role` does not call. The first isolation test is written before the first backend is wired.

## CLI surface (parity)

All current commands remain, plus:

```
shalt ui [--port N] [--no-open]
shalt org [list|add PATH|remove ID]
shalt board --project ID            # list overlay items; required when the org has more than one project
shalt board rank RID --before RID
shalt board assign RID --goal|--milestone|--sprint ID
shalt board unschedule RID
shalt job add KIND --project ID
shalt job run
shalt job list
```

Existing commands, flags, and exit codes: as `docs/cli.md`. `--root` still means one project. Org-level commands default to `~/.shalt`.

`shalt dashboard` still writes the snapshot HTML inside a project. `shalt diagrams` still writes mermaid. Those are artefacts for git; the UI is for working.

## Limitations that carry over (stated, not “fixed by the rewrite”)

- Never run against a live model’s *judgement*. The mock is not the model.
- Gherkin does not express non-functional requirements.
- Holdouts and mutation sample; they do not prove.
- `claude-cli` backend unverified.
- No billing product. When the HTTP adapter returns token usage, CLI and UI display it on that turn. There is no accumulated cost ledger.
- Confidentiality is layer-one only. You cannot un-read a file.
- The overlay can drift. `verify` reports it; it does not merge.

## Open questions, closed in this spec

| question | decision |
|---|---|
| Node or Rust? | Rust engine + embedded TS UI |
| Install? | curl-to-binary |
| UI vs CLI? | UI is primary; CLI has full parity |
| v1 scope? | Full port of current behaviour, plus live org UI |
| Where? | This repo, in place |
| Daemon? | No. Jobs on disk, run in-process |
| Ledger schema bump? | No, unless a field meaning changes |
| pytest plugin? | Dropped from the package; workspace-local template remains |
| Org / Project / Goals / Milestones / Sprints? | Overlay (`shalt.board/1`). Epic/story/task stay Gherkin |
| One repo or many? | Org home, many local projects |
| Board vs spec? | Overlay ranks/schedules; spec holds obligations; two write paths |
| Remote Linear/GitHub? | Not v1 |
| Onboard existing repo? | Yes. In-place `org add` + author reads `src/`, writes only `spec/` |

No remaining open product questions for v1. Implementation sequencing is the plan, not this spec.
