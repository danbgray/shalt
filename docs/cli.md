# CLI reference

```
shalt [--root DIR] [--backend NAME] [--fixtures DIR] [--model M] [--base-url URL] <command>
```

| global flag | default | meaning |
|---|---|---|
| `--root` | `.` | workspace root |
| `--backend` | `fixture` | `fixture`, `claude-cli`, `grok`, `openai` |
| `--fixtures` | — | fixture directory, required by the fixture backend |
| `--model` | per backend | override the backend's default model |
| `--base-url` | per backend | override the API base URL |
| `--color` | `auto` | `auto`, `always`, `never`. `NO_COLOR` always wins |
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

### `shalt init [--stack NAME] [--name N]`
Scaffolds the workspace: `spec/`, `contract/`, the configured steps and src zones, `.shalt/`,
an empty ledger, and `shalt.toml` from the chosen stack preset. Stacks: `python` (default),
`javascript`, `go`, `java`, `ruby`, `dotnet`.

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

### `shalt run`
Runs the suite once and updates the ledger. Reports regressions by name and any test not bound to
a scenario.

### `shalt status`
The ledger as a progress view: every scenario by feature with status, holdout marker and id, then
the bar and counts. Warns when removed scenarios are excluded from the figure.

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

shalt --root ./work init --stack python --name "Invoicing"
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

