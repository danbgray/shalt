# CLI reference

```
shalt [--root DIR] [--backend NAME] [--fixtures DIR] [--model M] [--base-url URL] <command>
```

| global flag | default | meaning |
|---|---|---|
| `--root` | `.` | workspace root |
| `--backend` | `fixture` | `fixture`, `grok`, `qwen`, `openai` |
| `--fixtures` | — | fixture directory, required by the fixture backend |
| `--model` | per backend | override the backend's default model |
| `--base-url` | per backend | override the API base URL |
| `--color` | `auto` | `auto`, `always`, `never`. `NO_COLOR` always wins |
| `--tags` | — | Cucumber tag expression (`@wip`, `not @holdout`) |
| `--format` | `auto` | `auto`, `pretty`, `progress`, `play` |
| `--dry-run` | off | list matching scenarios; do not run the harness |
| `--editor` | auto | which editor a clicked path opens |

## Colour and clickable paths

Colour is **detected, never assumed**: it honours [`NO_COLOR`](https://no-color.org) and
`FORCE_COLOR`, and switches itself off when stdout is not a terminal, so piped output and CI logs
stay clean. Truecolor is used where `COLORTERM` advertises it, with 256- and 16-colour fallbacks.
The palette is the dashboard's dark theme, so a scenario that is amber on the web is amber in the
terminal.

**Every file reference is clickable, by one of two mechanisms.**

Terminals that support OSC 8 hyperlinks get a real link that opens the file at the right line:
iTerm2, VS Code, WezTerm, Kitty, Ghostty, Windows Terminal, and anything on VTE ≥ 0.50. **macOS
Terminal.app does not support OSC 8** — but it, and iTerm2, Command-click a plain `path:line`
string, so the fallback prints an absolute path rather than nothing.

Which editor opens is configurable, in precedence order: `--editor`, `$SHALT_EDITOR`, `[ui]
editor` in `shalt.toml`, then auto-detection (VS Code's integrated terminal is recognised).

| value | opens |
|---|---|
| `file` (default) | the system default for that file type |
| `vscode`, `cursor`, `windsurf` | VS Code and its forks, at the line |
| `zed`, `subl`, `idea`, `textmate` | Zed, Sublime, JetBrains, TextMate, at the line |

`SHALT_HYPERLINKS=0` or `=1` forces link emission off or on, for a terminal that is not detected
correctly.

One deliberate asymmetry in the no-OSC-8 fallback: a **standalone path reference** (the ledger, a
generated dashboard, a mutant's location) prints the absolute path, because the path is the
content and Command-click needs it visible. A **labelled row** (a scenario name in `status` or
`tree`) keeps its label, because substituting an absolute path for every scenario name turns a
readable list into a wall of paths. Legibility wins there; click-through is the thing given up.

## Syntax highlighting

`shalt approve` prints the full spec with Gherkin highlighting, because reading it *is* the
review gate and it is the only one. `--quiet` reverts to a list of scenario names.

Highlighting distinguishes the things that matter when reviewing a contract: feature and scenario
keywords, step keywords, `@epic:` and `@holdout` tags (the `@rid:` is muted — it is machinery,
not content), quoted values and `<placeholders>` (the concrete data), the user-story narrative in
italic, and docstring bodies as inert text rather than structure.

It is a strict no-op when colour is off, so `shalt show > out.feature` produces the feature file
and not escape codes.

## Commands

### `shalt login [--host https://shalt.dev]`
Opens GitHub via the Space host. Stores `~/.shalt/credentials.toml`. The token is never printed. `shalt whoami` / `shalt logout`. `shalt connect github` is the same grant.

### `shalt onboard PATH_OR_URL [-p NOTE] [--dir DIR]`
Wrap an existing codebase. `PATH` is a directory, or a GitHub URL (`github.com/org/repo`, HTTPS, or `git@github.com:…`). A URL is cloned with local `git` (your SSH keys), then wrapped. Agents keep using this CLI; they do not log into the website.

### `shalt init [--stack NAME] [--name N]`
Scaffolds the workspace: `spec/`, `contract/`, the configured steps and src zones, `.shalt/`,
an empty ledger, and `shalt.toml` from the chosen stack preset. **Rust and JavaScript are
supported.** Any cucumber-family language should *run* (`--stack python|go|…`). Support
order after rust and javascript: python, then the rest. Unsupported init prints a note; it does
not refuse.

### `shalt job list|show|waiting|ask|chat|decide|answer|pause|resume`

Jobs are the workers. When Play hits `ask_human`, the job is `waiting` — same state as the desk dialog.

```
shalt job waiting
shalt job ask                  # the only waiting job, or pass an id
shalt job chat J-xxx "yes, add is_manufacturable on the product"
shalt --backend grok job chat J-xxx "use ProductId, not &str"
shalt --backend grok job decide J-xxx
shalt job answer J-xxx "Add fn is_manufacturable(product) -> bool on ErpSystem. create_bom does not imply it."
```

`chat` and `decide` talk to the **answer agent** — `--backend` / `--model` pick who drafts; Play stays on the job's worker. `ANSWER:` is a draft. `answer` commits it and Play continues. Claude, Grok, Codex, and the UI share the same turn log. The desk dialog has the same agent picker and a **Decide** button.

### `shalt sprint open|list|assign|close|retro`

Sprints live on the overlay. Estimate tokens on each ticket, Play the loop, then close the sprint. The retro is estimate vs tokens spent (jobs billed to that sprint). The next ticket default is scaled by that accuracy.

```
shalt board estimate S-e6c9cbf6 120000
shalt sprint open W38
shalt sprint assign S-e6c9cbf6 C-1
shalt play
shalt sprint close
shalt sprint retro
```

### `shalt play [--max-steps N] [--yolo]`

Thin loop for agents and the terminal. Same engine as UI Play.

1. Registers `--root` in the local org if needed.
2. Pauses other projects so this one owns the model.
3. Writes tests (`steps`), runs them, writes code (`src`), runs them again until visible scenarios are green (or a stage fails).

`--yolo` turns on Yolo for this project: `ask_human` takes the model's guess and Play does not wait. Stays on until `shalt org yolo <id> off`. The desk has the same **Yolo** toggle.

Alias: `shalt loop`. Lines on stdout start with `PLAY` so Hermes / Claude / Codex skills can parse them. Does **not** start the UI.

```
shalt --root /path/to/workspace play
shalt --root /path/to/workspace play --yolo
shalt org play <project-id>
shalt org yolo <project-id> on
```

### `shalt journal`

Prints the project blog. Short **progress** posts (`JOURNAL:`) as work moves. Longer **features**
(`FEATURE:`) when something notable happens (first spec, storyboards, a journey going green).

```
shalt --root /path/to/workspace journal
```

### `shalt design`

Draws storyboards under `mockups/`: HTML/CSS/JS prototypes, one film per journey, hand-sketched until a scenario is green. Same job as desk Design. Play runs this after spec exists, before language/tests. API-only journeys can be `kind: none`.

```
shalt --root /path/to/workspace --backend grok design
```

### `shalt stop`

Stops every running shalt process: the UI, `play` / `loop`, and any other shalt binary. Parks running and pending jobs, pauses every project, then SIGTERM (and SIGKILL if needed). Waiting jobs stay waiting. Does not touch Ollama. Play after this is a deliberate start.

`shalt ui stop` still only stops the UI.

```
shalt stop
```

### `shalt author "<request>"`
Runs the author role. Writes Gherkin feature files and user stories into `spec/`. Guarded like
every other role — an author that writes outside `spec/` has its turn rejected.

### `shalt approve [--yes] [--by WHO]`
The human gate. Lists every scenario awaiting sign-off (marking holdouts), prompts unless
`--yes`, then:

- stamps a durable `@rid:` into every unstamped scenario;
- records the spec lock: who, when, and **a canonical hash per scenario**.

`--by` is recorded in the ledger. Nothing downstream runs until this has happened.

### `shalt steps`
Runs the stepwright role: step definitions into the steps zone, and the API surface it will call
into `contract/interface.md`. Refuses if the spec is not approved. Exits `2` on an integrity
violation.

### `shalt build [--max-turns N] [--strict]`
The implementer loop. Each turn: run the suite, fold results into the ledger, and if any
*visible* scenario is still failing, run one implementer turn with the failure digest. Holdouts
are stripped from its staged spec and filtered out of the digest.

Ends on all-visible-upheld or at `--max-turns` (default 6), then runs a **final verification
including holdouts** and reports overfitting if only the held-out ones fail. `--strict` aborts on
an integrity violation instead of continuing to the next turn.

### `shalt run [FEATURE…]` / Cucumber-style paths

Runs the suite (or a slice of it) and prints a Cucumber-shaped report. Feature paths on the
binary are the same command: `shalt spec/*.feature` becomes `shalt run spec/….feature`.
`shalt spec delete` is unchanged — `delete` is not a file.

```
shalt spec/*.feature
shalt spec/invoices.feature:12
shalt spec/*.feature --tags @wip
shalt run --tags "not @holdout"
shalt spec/*.feature --format progress
shalt spec/*.feature --dry-run
```

`--tags` is a Cucumber tag expression (`@wip`, `not @wip`, `@wip and not @holdout`, `@wip or @slow`,
comma as OR). `--format auto|pretty|progress|play` — `auto` is pretty on a tty, progress when
piped. `--dry-run` lists matching scenarios without executing the harness.

Pretty output is Feature / Scenario / steps, coloured by result, then:

```
1 scenario (1 failed)
3 steps (1 failed, 2 passed)
0m1.204s
```

Exit `1` if a selected scenario failed, `3` if the harness could not run. Updates the ledger for
whatever actually ran.

### `shalt status`
The spec as a Cucumber pretty (or progress) report from the ledger. Honours `--tags` and
`--format`. Pending is undefined (no test); stale is pending.

### `shalt tree`
The breakdown as a tree: epic → story → scenario, with each story's user story beneath it.

### `shalt stories`
Who wants what, from the narratives. Names any feature missing a complete user story, and prints
the expected grammar when none is found at all.

### `shalt diagrams [--stdout] [--diagram NAME]`
Writes `docs/diagrams/{use-cases,breakdown,pipeline}.{mmd,md}`. `--stdout` prints one instead of
writing.

### `shalt dashboard`
Writes `docs/dashboard.html` — one self-contained file, no network, no build step.

### `shalt show [target] [--status]`
Prints the spec with highlighting and line numbers. `target` is a scenario id (the scenario is
marked in the gutter), a feature file, or nothing for the whole spec. `--status` lists each
scenario's state underneath.

### `shalt verify`
The standing integrity audit. Reports: unstamped scenarios, duplicate or reused ids, orphans,
scenarios changed or added since approval, approved scenarios now missing, an absent spec lock, a
lock predating content hashing, and weak oracles from the last mutation run.

### `shalt mutate [--engine E] [--budget N] [--seed S] [--verbose]`
Mutation-tests the oracle. `--engine auto|python|text`, `--budget` default 30, `--seed` default 0
for reproducibility, `--verbose` shows each mutant as it runs and the weakest oracles.

## Exit codes

| code | meaning |
|---|---|
| `0` | success; for `verify` and `mutate`, nothing wrong found |
| `1` | a problem was found: unapproved spec, integrity problems, weak oracles, no user stories |
| `2` | an integrity violation rejected a turn, or a bad argument |
| `3` | the test harness itself could not run |
| `4` | the spec does not parse — the offending file is named |

## A full session

```bash
export XAI_API_KEY=...

shalt --root ./work init --name "Invoicing"
shalt --root ./work --backend grok author "Invoice totals, currency display, overdue reminders"

$EDITOR work/spec/*.feature          # the review gate, and the cheap one
shalt --root ./work approve --by you@example.com

shalt --root ./work --backend grok steps
shalt --root ./work --backend grok build --max-turns 8

shalt --root ./work verify
shalt --root ./work mutate --budget 30     # do the scenarios mean anything?
shalt --root ./work tree
shalt --root ./work dashboard
```

Reviewing one scenario, in the editor of your choice:

```bash
SHALT_EDITOR=cursor shalt --root ./work show S-b291b8fd
```

