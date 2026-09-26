---
name: shalt
description: Use when driving shalt — spec, Gherkin, Play, board, sprints, waiting jobs, or the localhost UI. Agents shell out to shalt; do not run a second TDD loop.
version: 0.2.2
author: shalt
license: MIT
platforms: [linux, macos, windows]
metadata:
  hermes:
    tags: [shalt, bdd, tdd, spec, board, sprint, play]
    category: software-development
    requires_toolsets: [terminal]
---

# Shalt

You operate **shalt**, not a second TDD loop. Shalt is a complete inner-loop product: English → Gherkin → tests → code, plus org/project/board/sprints and a localhost UI. Hermes, Claude, Grok, and Codex are doorbells — they shell out to this binary. Play workers are grok / qwen / openai (not Codex). Shalt stands on its own (`shalt ui`, `shalt play`). For Daniel and Ikonic, shalt replaces Hermes as the work surface.

Do **not** write `spec/`, `steps/`, `contract/`, or `src/` with your own file tools unless the user is explicitly editing Gherkin by hand and asked you to. Prefer the binary. Zone violations are rolled back.

## When to Use

Any shalt work: new project, onboard a repo, reshape a spec, Play/Pause, ticket estimates, sprints, retros, status, verify, mutate, diagrams, UI.

Don't use when they want a throwaway prototype with no tests, or a non-software Hermes-style personal OS task (calendar, Telegram butler). That is not shalt.

## Always

```bash
command -v shalt || cargo install --path crates/shalt --force
ROOT="${SHALT_ROOT:-$PWD}"   # directory with shalt.toml or spec/
shalt --root "$ROOT" status
```

`--root` on every command unless you already `cd` there. Default backend in the CLI is `fixture`; live work needs `--backend qwen` (local Ollama) or `--backend grok`. UI Play uses the job's saved backend.

## Intent → command

| They want | You run |
|---|---|
| New system from English | `shalt --root "$ROOT" do "The system shall…"` or `author "…"` |
| Wrap existing code | `shalt onboard PATH --prompt "…"` |
| Dashboard | `shalt ui` on 7702 (Ikonic `?embed=1`). After a rebuild: `shalt ui restart --no-open`. One instance; `--port 7703` is refused while 7702 is up |
| Stop everything | `shalt stop` — UI, Play, and every other shalt process. Parks jobs. Does not kill Ollama. Play to start again |
| List / pause / play projects | `shalt org list` · `org pause ID` · `org play ID` |
| Don't ask — take guesses | `shalt org yolo ID on` · `play --yolo` (desk: **Yolo**) |
| Draw storyboards | `shalt --root "$ROOT" design` (HTML prototypes under `mockups/`) |
| Read the blog | `shalt --root "$ROOT" journal` — progress posts as work moves; longer features when something notable lands |
| Tests then code (the loop) | `shalt --root "$ROOT" play` (alias `loop`) |
| Only tests / only run / only implement | `steps` · `run` · `build` |
| Run feature files | `shalt spec/*.feature` (`--tags @wip`; `--format progress` when piped). That is `run`, not `spec delete` |
| Ledger / tree / stories | `status` · `tree` · `stories` |
| Integrity | `verify` |
| Edit a scenario | `shalt spec delete FILE LINE` · `spec promote FILE LINE` |
| Rank a ticket | `shalt board promote RID` · `board agent RID qwen::qwen3.8:27b-mlx` |
| Forecasts | shalt fills token + time forecasts from history. Do **not** ask a human for token estimates. Measure spent vs forecast. |
| Epic / agent | `shalt board epic Billing --agent qwen::qwen3.8:27b-mlx` (agent is qwen/grok/openai, then a model) |
| Sprint | `sprint open NAME` · `sprint assign RID C-1` · `sprint close` · `sprint retro` |
| Export / import a plan | `shalt plan export [FILE]` · `shalt plan import FILE [--dir DIR] [--name NAME]` (does not Play) |
| Build language | Plan first, then `shalt stack rust` or `shalt stack javascript` (or the UI dropdown). Play writes tests only after a language is picked. |
| Jobs | `job list` · `job show ID` · `job pause ID` · `job resume ID`. `job list` is the whole org — match the project id |
| Waiting on a human | `job waiting` · `job ask [ID]` (org-wide, no `--root`) · `job chat ID "…"` · `job decide ID` · `job answer ID "…"` · or `org yolo ID on` |
| Mutation / diagrams | `mutate` · `diagrams` |

Play stdout is prefixed `PLAY` (`start` / `ok` / `idle` / `failed`). Stages: `design` (storyboards) → `steps` (write tests) → `build` (run tests, write code, repeat) → `run`.

On `PLAY failed:` show the error. **Rust and JavaScript are the supported stacks**. Other languages should run if the toolchain is present; do not treat a python harness gap as a shalt bug, and do not install pytest-bdd unless the project is explicitly `--stack python`. Do not bypass the harness. A suite compile dump (`could not compile`, `error[E…]`, cucumber-js `SyntaxError` in `steps/`) is one harness error, not a failing Then on every scenario — do not patch `Cargo.toml` / `package.json` / `contract/` / `src/` to "help."

## UI (same engine)

`http://127.0.0.1:7702/` — hash routes: `#/` inbox, `#/new`, `#/p/{id}/plan`, `#/p/{id}/spec/{file}`, `#/p/{id}/tests`, `#/p/{id}/item/{rid}`, `#/jobs/{jid}`. Plan is charts + journeys (not a linear ticket list). Details is one feature file as the page (rail of features, the spec as a document) — not a dump of every journey. Do not say “Gherkin” in the desk. Code and Tests are a file tree beside one open file, not a flat dump of every path. While Play is on, the Now strip and Plan KPIs/journey bars update from the job stream (stage, ticket, elapsed, tokens, turn) — do not reload to see progress. Forecast vs spent is not a human token budget. Pause always says why. Export plan downloads a pack you can Import on New project. After Play has started, Pause, edit, Play to restart. Ikonic: panel type `shalt`, URL `http://127.0.0.1:7702/?embed=1`. Start with `shalt ui` if nothing is bound; `shalt ui restart --no-open` after a rebuild.

Talk-to-spec, plan, and file save go through `POST /api/project/{id}` (`chat`, `save_plan`, `save_file`). Prefer CLI when you are a terminal agent; use the API when the UI is already up and the user is looking at it.

## Waiting jobs (same loop as the desk dialog)

Play blocks when an author/implementer calls `ask_human`. The UI pops a dialog. **You are the same surface.** Do not invent the missing behaviour in `src/` or the spec.

1. `shalt job waiting` (or `job list`) — if any line is `waiting`, there is a question.
2. `shalt job ask` — print the question, guess, and chat so far.
3. Talk to **the human in this conversation**. Chat can use a **different** agent than the blocked worker: `shalt --backend grok job chat J-xxx "…"` or `shalt --backend grok job decide J-xxx`. The desk has the same picker. The model may emit `ANSWER:`; that is a draft, not a commit.
4. When the human agrees, `shalt job answer J-xxx "the decision"` — this unblocks Play. Do not `job resume` instead of answering.
5. If several jobs wait, pass the id.

If the human wants no more questions, `shalt org yolo ID on` (or desk **Yolo**). Each `ask_human` takes the model's guess and Play continues. `org yolo ID off` restores the dialog.

The desk, this skill, Claude, Grok, and Codex all write the same `turns` on the job. A chat in the UI is visible to `job ask`; an answer from the CLI is visible to the UI.

## Rules you must not break

- The spec wins. Drawings and chat fold into `spec/`.
- Author writes `spec/` only. Designer writes `mockups/` only. Stepwright writes the steps zone (`tests/shalt.rs` on rust, `steps/` on javascript) + `contract/` and cannot see `src/` or mockups. Implementer writes `src/`, may read mockups, and cannot see step definitions.
- Pending is not green. Green is a bound passing test against a stamped `@rid:`.
- One project plays at a time (limited local model). Pause others via `org play ID` / UI Play.
- Assign **agent and model** on each task (qwen, grok, openai — not grok-or-nothing, not Codex). Spend vs **forecast** per agent×epic is how shalt learns who is good at what and what the next forecast should be. Never ask a person to estimate tokens.

## Pitfalls

- Empty spec → `PLAY idle`. Run `author` or `do` first.
- UI restart parks running jobs. `shalt stop` parks jobs and kills every shalt process (UI, Play, children). `play` or `job resume` continues them. Does not kill Ollama. A failed build must not idle Play while tickets remain red — the desk chains the next job. A suite survey that binds no tests must go on to write tests — it must not enqueue Run forever.
- Do not patch `src/`, `contract/`, or `Cargo.toml` yourself to “help.” That skips the claim. A compile dump on every scenario is still one harness error.
- Do not use Hermes’s TDD skill (or Claude Superpowers TDD) in parallel. Shalt *is* the loop.

## Verification

```bash
shalt --root "$ROOT" status
shalt --root "$ROOT" verify
shalt --root "$ROOT" sprint retro
```

## Install

```bash
sh skills/shalt/install.sh
```

Links this directory into Hermes, Claude, Grok, and Codex. One skill, full shalt surface.
