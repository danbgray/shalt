# Agent mode

Shalt runs two ways:

- **Play mode** (classic): the shalt binary orchestrates small local models (qwen / grok / openai) through design → steps → build → run. Shalt writes the spec, the tests, and the code. You watch the desk, approve what it asks, and `yolo` the rest.
- **Agent mode** (this doc): a capable coding agent (Claude Code, Codex, a Hermes/Grok session) drives the loop themselves — writes `spec/`, writes the tests, writes `src/`. Shalt is the scaffold, the zone referee, the ledger, and the report generator. The agent runs commands, not sub-models.

Pick agent mode when the strongest model you have is the one in the chat. Pick Play mode when you want shalt to run a feature on local hardware overnight.

## The discipline

Same as Play's: Author → Stepwright (**red first**) → Builder (until **green**) → close out. Same zones:

- Author writes `spec/*.feature` only.
- Stepwright writes `tests/shalt.rs` (rust) or `steps/*.js` (javascript) plus `contract/`. Cannot see `src/` or `mockups/`.
- Builder writes `src/`. May read `mockups/` and `contract/`. Cannot see the step defs.
- Designer writes `mockups/*.html`. The wireframe locks the visible shape *before* the tests lock the observable contract.

Zone discipline in agent mode is on the honour system — there is no filesystem gate the way Play has. The skill says what each hat can see; you follow it.

## The four surfaces the user sees in chat

Agent mode is a conversation. The deliverables are **artifacts in the host** (Claude side-panel cards, Grok inline, whatever), updated as the loop moves:

| # | Surface | Published when |
|---|---|---|
| 1 | **Plan chart** — the spec as a ladder, scenarios pending | After Author writes `spec/*.feature`, before any test |
| 2 | **Wireframe** — the UI shape, Designer's review stage | After Designer writes `mockups/*.html` |
| 3 | **Red → green chart** — same URL as 1, status pills flipping | After each `shalt run` that changes any scenario's status |
| 4 | **End product** — rendered sample from the green `handle()` | After all scenarios green |

The chart card URL stays the same across the whole loop so the user watches it fill in without re-clicking.

## Editable surfaces (the control panel)

Surfaces 1 and 2 are also **editable**. The chart and wireframe templates declare the `artifact` capability so they persist their own edits by republishing themselves. The user:

- Adds / renames / deletes scenarios, features, milestones on the chart
- Edits text, adds/removes ingredients, toggles timestamps on the wireframe
- Clicks `▶ build` on any red / pending scenario to flag it `build_requested: true`

None of this edits `spec/` or `mockups/` directly. It edits the published artifact's state. When the user says "continue" / "apply the chart" / "the wireframe is updated", the agent reads the current artifact state (`Artifact action: "read"`), extracts the embedded `SHALT_REPORT` / `WIREFRAME` JSON, diffs against the project, and acts.

**Build queue is not autopilot.** Clicking `▶ build` persists intent and copies a ready-paste prompt to the clipboard — but no artifact capability can spawn a filesystem-attached Claude Code / Codex session. A human (or the agent in this conversation) still has to pick up the queue. A future `shalt agent watch` daemon could poll the chart and shell out; see TODO.

## Session continuity — `.shalt/agent-state.json`

Record the URLs of every published card so a later session can pick up without the user re-pasting:

```json
{
  "schema": "shalt.agent-state/1",
  "artifacts": {
    "chart":             { "url": "https://claude.ai/artifact/...", "last_published_at": "..." },
    "chart_editable":    { "url": "https://claude.ai/artifact/...", "last_published_at": "..." },
    "wireframe":         { "url": "https://claude.ai/artifact/...", "last_published_at": "..." },
    "wireframe_editable":{ "url": "https://claude.ai/artifact/...", "last_published_at": "..." },
    "end_product":       { "url": "https://claude.ai/artifact/...", "last_published_at": "..." }
  }
}
```

Write / update this file at every publish. A fresh session says `continue the recipes project`, reads this file, re-opens the cards, diffs, and resumes.

## The report generator

Agent mode ships with a ladder generator the skill invokes to produce surface 1 / 3:

- **`~/.claude/skills/shalt/report/dump.sh`** — reads `.shalt/ledger.json`, `.shalt/board.json`, and `spec/*.feature`, prints the ladder as JSON.
- **`~/.claude/skills/shalt/report/ladder-template.html`** — read-only ladder view that renders the JSON.
- **`~/.claude/skills/shalt/report/ladder-editable.html`** — the editable control-surface version with add / edit / delete / `▶ build`.
- **`~/.claude/skills/shalt/report/README.md`** — recipe to assemble and publish.

A future `shalt report --json` command in the binary would obsolete `dump.sh`.

## Running the loop

```bash
# Scaffold (shalt do's model call errors harmlessly if no model is pulled)
mkdir -p ~/tmp/my-project && cd ~/tmp/my-project
shalt --root . --backend fixture -y do "The system shall …"
shalt --root . stack rust      # or javascript

# Author writes spec/*.feature by hand (stamping @rid: tags).
# Designer writes mockups/*.html.
# Publish surfaces 1 and 2 as artifact cards (see report/README.md).

# Stepwright writes tests/shalt.rs (or steps/*.js).
# Builder writes src/lib.rs.
shalt --root . run              # confirm red; implement; shalt run again
shalt --root . status
# Re-publish surface 3 (same URL as 1) after each state change.

# Close out:
shalt --root . verify           # fails until human-approved
cd . && shalt --root . -y approve   # human gate; run this yourself
shalt --root . tree
```

See the [skill](../skills/shalt/SKILL.md) for the full agent-mode protocol, including the mapping tables for chart / wireframe edits and the one-at-a-time implementation cadence.

## When NOT to use agent mode

- You want the loop to run unattended overnight → Play mode.
- The project has a spec already full and green, you just want to run and verify → `shalt run`, no agent needed.
- The task is not software → shalt isn't the right tool.

## TODO

- `shalt report --json` in the binary.
- `shalt agent watch` daemon that polls editable chart state via `Artifact action: "read"` and picks up `build_requested` scenarios autonomously.
- Richer Gherkin parsing in `dump.sh` (Scenario Outline, Rule, doc strings).
- `.shalt/vocabulary.json` seeded by `shalt do` when new nouns / verbs show up in a spec.
