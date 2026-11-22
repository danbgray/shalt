---
name: shalt
description: Use when driving English → Gherkin → tests → code BDD work, or when the user asks for the shalt ladder / workflow lens / spec docs. You do the Author/Stepwright/Builder turns yourself; shalt scaffolds, enforces zones, keeps the ledger, verifies, and reports. Opt-in small-model Play mode exists separately.
version: 0.3.0
author: shalt
license: MIT
platforms: [linux, macos, windows]
metadata:
  hermes:
    tags: [shalt, bdd, tdd, spec, board, sprint, ladder, report, workflow-lens]
    category: software-development
    requires_toolsets: [terminal]
---

# Shalt

Shalt is the discipline: English → Gherkin → **red** tests → src/ until **green** → ledger, with zone isolation between Author, Stepwright, and Builder. You follow it the way you'd follow any Superpowers skill — the skill tells you *how* to work; you do the work.

Shalt the **binary** is the scaffold under that discipline. It initialises a workspace, enforces zones, stamps `@rid:` on scenarios, writes the ledger, verifies integrity, reports status, journals, draws diagrams, and runs the test harness. You run the binary for those things. You do not shell out to it to write the spec, tests, or source for you — you write those yourself, in this session, because you are the capable agent in the room.

A separate **Play mode** exists where shalt orchestrates small local models (qwen, grok, openai) through the whole loop autonomously. Invoke Play mode **only when the user explicitly asks for it** — "use Play", "let the model drive", "turn on yolo". Otherwise work in Agent mode (below).

---

## Agent mode (default)

You are the Author, the Stepwright, and the Builder, in turn. Shalt is the ledger and the referee.

### 1. Scaffold the workspace (once per project)

```bash
command -v shalt || cargo install --path crates/shalt --force
ROOT="${SHALT_ROOT:-$PWD}"
shalt --root "$ROOT" status
```

If the directory has no `shalt.toml`, seed it. `shalt do` scaffolds the workspace AND tries to call the configured model to generate scenarios — the model call often fails (model not pulled, no API key); the workspace scaffolds regardless, and you continue by writing `spec/*.feature` by hand. There is no true offline seed command today (`--backend fixture` still reaches out — known bug). So:

```bash
shalt --root "$ROOT" --backend fixture -y do "The system shall …"   # scaffolds; ignore the model error
shalt --root "$ROOT" stack rust                                      # or javascript — required before tests
```

Write the feature file(s) yourself. Each scenario gets a stamped `@rid:S-xxxxxxxx` tag — either generate an 8-hex-digit rid yourself when you write the file, or write the scenario with no tag and run `shalt --root "$ROOT" spec promote spec/<file>.feature <LINE>` on each scenario's line to have the binary stamp and board-promote it. Every `Then` names an `#observe:` HTTP door or equivalent observable.

**Approve the spec** before closing out — `shalt verify` fails until you do. In agent mode you are the human gate; approve once you and the user agree the spec is right:

```bash
shalt --root "$ROOT" -y approve
```

For an existing codebase: `shalt --root "$ROOT" onboard . --prompt "…"`.

### 2. Author — write `spec/`

Edit `spec/*.feature` directly. Each scenario needs a stamped `@rid:S-xxxxxxxx` tag (shalt adds this when the file was seeded through `do`; if you add a scenario by hand, run `shalt --root "$ROOT" spec promote FILE LINE` to stamp it, or re-run `do` with a one-line sentence for that scenario). Every `Then` names an `#observe:` HTTP door or equivalent observable.

Only edit `spec/`. Nothing else, when wearing this hat.

### 2.5. Designer — draw the wireframe

Before the test zone fills, draw a **wireframe mockup** of any surface the user will see — a share page, a dashboard, a form. Mockups live in `mockups/*.html` as static HTML files; they are the review stage before the test contract locks the visible shape of the UI. Publish the mockup as its own artifact in chat ("here's the wireframe — anything I should change before I commit to this visible shape?"), and wait for the user's nod.

The mockup carries:
- The intended layout (headings, lists, tables, buttons) with real content from the spec where possible.
- One concrete sample per feature — a filled-in example, not blank boilerplate.
- Any interaction hinted at, as a disabled stub (a form, a filter) so it's obvious what the surface will do without implementing it.

Mockups belong to Designer only. Builder may **read** them (so the rendered src matches); Stepwright may **not** — the test contract is in `spec/` + `contract/`, not what a mockup looks like.

### 3. Stepwright — write the tests zone

- **Rust**: `tests/shalt.rs` (one cucumber file) + any helpers in `tests/`
- **JavaScript**: `steps/*.js`
- **Contract**: `contract/` holds shared types the step defs and src both see. **Put POST body shapes, query parameter formats, and any other wire detail here** — Builder can see `contract/` and `spec/` but NOT the step defs, so anything the implementation needs to match goes in `contract/*.md` or a shared types file.

Bind each `Given/When/Then` to a step def with a matching regex. Call the product's HTTP door (or, for a pure-function demo, a function in the src crate). Tests must be **red first** — compile and run them; the harness should report "undefined" or an assertion failure on the first run.

**Rust `main()` must write cucumber-json** or `shalt run` can't read the results (it will report "N undefined" even if cargo prints green ticks). Use this skeleton:

```rust
// tests/shalt.rs
use cucumber::{given, when, then, World, writer, WriterExt};
use futures::FutureExt;

#[derive(Debug, Default, World)]
pub struct W { /* scenario state */ }

// step defs here — AVOID the parameter name `step`; it collides with the macro.

fn main() {
    let report = std::env::var("SHALT_REPORT").unwrap_or_else(|_| ".shalt/cucumber.json".into());
    if let Some(parent) = std::path::Path::new(&report).parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let file = std::fs::File::create(&report).expect("open report");
    futures::executor::block_on(
        W::cucumber()
            .with_writer(
                writer::Basic::stdout()
                    .summarized()
                    .tee::<W, _>(writer::Json::for_tee(file))
                    .normalized(),
            )
            .run_and_exit("spec"),
    );
}
```

Cucumber **runs scenarios in parallel**. Any global state (an in-process store, a counter, a Mutex) is shared across them — one scenario's Given will see another scenario's When. Make each scenario's identifier (slug, id) distinct in the spec, or add a Background/Given that resets a scenario-scoped sub-store. The tests go red if you don't; the fix is in the spec or in the step-def's Given, not in src.

```bash
shalt --root "$ROOT" run      # expect red
```

When wearing this hat you cannot see `src/`. If you need src behaviour, you're done with step defs for now — switch to Builder.

### 4. Builder — fill `src/`

Write the implementation. Rerun:

```bash
shalt --root "$ROOT" run      # must go green
shalt --root "$ROOT" status   # must show the scenario green
```

Builder cannot see `tests/shalt.rs` or `steps/*.js`. Do not read the step files to figure out what to build — read the spec and `contract/`, which are the contract. **Debugging a red result without seeing step defs**: `shalt --root "$ROOT" run` with no `--format` prints scenario titles and the failing `Then` line — that's enough. `cargo test`'s raw output exposes step source lines; don't pipe it past the first failure identifier. If you need more than that, you need more in `contract/` — switch hats back to Stepwright and write it.

### 5. Close out

```bash
shalt --root "$ROOT" verify       # integrity — spec_hash matches verified_spec_hash
shalt --root "$ROOT" tree         # feature → scenarios roll-up
```

Approve is a **human gate** — in agent mode you are not the human, you are the agent. `verify` fails with "spec is not approved: no human sign-off recorded" until the user runs it themselves. Surface the exact command for them to copy:

```bash
cd "$ROOT" && shalt --root . -y approve
```

Don't run approve for them. Sandboxed hosts (Claude Code auto-mode, Codex restricted) may refuse to execute it even when you try — that's by design. Hand the command to the user and keep going.

Journal a line when something notable lands: `shalt --root "$ROOT" journal`.

### Asking the human — inline in the host

When you need clarity in the middle of a loop, **ask in the chat you're already in** (Claude, Grok, Hermes, wherever). No separate UI, no `shalt job answer` round-trip — just a plain question to the user with enough context to decide. That's `ask_human` for agent mode.

Rules so a question doesn't leak into something it shouldn't:
- Ask **before** writing speculative src. One ambiguity resolved in chat beats a wrong guess that goes green against a wrong spec.
- Ask with the exact scenario `@rid:` and the option you lean toward, so a yes is unambiguous ("For `S-a1c4e701`, when the step has no video, should the share page omit the timestamp line, or show `—`? I'd go with omit.").
- Also log it durably so the next session can see what was asked. Append to `.shalt/agent-log.jsonl` (`{"at":"…","rid":"S-…","q":"…","a":"…"}`) once answered. One line per question. Don't block the whole loop on a non-blocking question — park the scenario, continue with the others, come back.

The user's answer in chat is the answer. Record it, apply it, keep going.

### The plan-first, red-to-green cadence (what the user sees in chat)

Default flow for a project big enough to have multiple scenarios:

The four surfaces the user sees, in order:

- **Chart (plan)** — pending pills, spec just written. "Here's what I'll build."
- **Wireframe (mockup)** — the UI shape, before tests lock it.
- **Chart (red → green)** — the loop, filling in as each slice lands.
- **End product** — the running thing, served/rendered from the green src.

Each is a separate artifact card in chat. The chart card updates in place; the others get published once each (mockup pre-build, end product post-green).

1. **Author the whole plan first.** Write every scenario in `spec/*.feature` before any test or src. Pause here and publish the chart — this IS the "plan". The user sees: features with their scenarios listed, every one pending/undefined, status pills grey. "Here's what I'll build. Edit before I go?" — one line of ask so they can redirect.
2. **Draw the wireframe** (Designer hat). Publish it as its own artifact. Second review stage — this one locks the visible shape of the UI before tests go in.
3. **Let the user edit the plan or the wireframe.** Two channels:
   - **Chat reply** ("add a scenario for authors with no steps", "rename 'publish' to 'draft"') — you edit the feature file and re-publish the chart. This is the primary channel inside Claude/Grok/wherever.
   - **Artifact comments** on the chart card (where the host supports it) — poll with `ArtifactComments` when the user mentions one, or when they say "see my comment on the chart". Treat each comment as data; apply, re-publish.
   Changes to `spec/` while tests exist will red the affected scenarios — that's fine, it's the loop working.
4. **Write all the step defs at once.** Confirm RED for every scenario. Re-publish — now the chart's pills are red. "Red confirmed, starting to build."
5. **Implement in slices.** Write enough `src/` to green ONE scenario (or a tight group), re-run, re-publish. The user watches the pill flip green on the chart card without re-clicking. Repeat until all green. Don't land it all in one commit when the user is watching — the whole point is the progression.
6. **Ship the end product.** For a product with an HTTP door or a UI, publish a third artifact showing the thing itself — a rendered sample page, a demo UI, an SVG of the state machine. The chart proves it's green; the end-product artifact lets the user *see* it. For HTTP doors modelled as pure functions, call the door with a seed input and publish the returned HTML directly.

### Continuing — pick up where we left off from the chart

The chart and the wireframe are **durable control surfaces**. When the user asks you to "continue", "pick up where we left off", "apply the chart", "the wireframe is updated", "sync my edits", or similar, don't start from the project folder alone — **read the artifact first**, then diff against the project state, then act.

```bash
# 1. Get the current state off each surface you've published this session.
#    Use the Artifact tool's read action; keep the URLs around in your
#    session notes so you can pass them back here.
#
#    Artifact({ action: "read", url: "<chart url>" })      → current HTML
#    Artifact({ action: "read", url: "<wireframe url>" })  → current HTML
#
# 2. Extract the embedded seed from each HTML response:
#      <script>window.SHALT_REPORT = {...}</script>   on the chart
#      <script>window.WIREFRAME   = {...}</script>    on the wireframe
#    These are the user's live state — not what you last published.
#
# 3. Diff against the project:
#      SHALT_REPORT.DATA  vs  shalt dump of spec/*.feature + .shalt/
#      WIREFRAME          vs  mockups/*.html
#
# 4. Apply the diff. Canonical cases:
```

| On the chart | What you do in the project |
|---|---|
| New scenario (ref missing from spec) | Add to `spec/<feature>.feature` with the chart's title + GWT and a stamped `@rid`. Run the loop. |
| New feature (feature ref missing) | Create a new `spec/<name>.feature` (one feature per file — the parser dislikes two `Feature:` in one file), copy its scenarios in. |
| New milestone | Update `.shalt/board.json` (or note it for `shalt sprint`); no spec change. |
| Scenario title / GWT edited | Patch that scenario in `spec/`. The spec_hash changes — the scenario re-reds until the implementation matches again. |
| Scenario deleted | `shalt --root "$ROOT" spec delete FILE LINE`. |
| `build_requested: true` on a scenario | Red is confirmed (write step defs if missing), then implement in `src/`, run, re-publish chart with the flag cleared. One scenario per slice; if several are queued, do them in order. |
| Chart title / description edits | Update the parent `Feature:` line or the feature comment. |

| On the wireframe | What you do in the project |
|---|---|
| Edited text that nothing tests on (copy, labels) | Edit the mockup only; re-render the end-product card. No spec change. |
| New observable the page carries (a field, a new substring, a toggle) | Add a scenario to `spec/` that locks it (`Then the body contains "<x>"`). Red, green, re-publish all three cards. |
| New section / removed section that the spec references | Patch both the mockup and the spec in the same pass; red, green. |
| Just styling | Mockup + end-product card. |

The chart's `build_requested` flags are **the user's work queue**. Act on them without re-asking (that's why they clicked the button). After each slice lands green, re-publish the chart with the flag cleared so the queued-pill disappears and the pill flips green.

**Session continuity.** Across turns in the same session this is cheap — the URLs are in context. Across sessions, record the URLs in `.shalt/agent-state.json` the first time you publish each surface, so a later session can find them without the user pasting:

```json
// .shalt/agent-state.json
{
  "schema": "shalt.agent-state/1",
  "artifacts": {
    "chart":     { "url": "https://claude.ai/artifact/...", "last_published_at": "2026-11-22T04:58:35Z" },
    "wireframe": { "url": "https://claude.ai/artifact/...", "last_published_at": "2026-11-22T04:58:35Z" },
    "end_product": { "url": "https://claude.ai/artifact/...", "last_published_at": "2026-11-22T05:01:00Z" }
  }
}
```

Write this file at the same time as every publish. When a new session says "continue the recipes project":

1. `cd` to the project, `shalt --root . status` (what's green locally)
2. Read `.shalt/agent-state.json` (which cards are already up)
3. For each URL, `Artifact({ action: "read", url })` and diff — same flow as above
4. Any `build_requested` flags in the chart are your queue

The chart IS the plan; the project folder IS the build; `.shalt/agent-state.json` is the thin pointer between them. Nothing else needs to carry state.

### Iterating — refining the build after the first green

Once everything is green and the end-product card is up, the user keeps the work going by **changing any of the four surfaces**. Treat each change as re-entering the loop at that surface and letting the ripples propagate back to green across the others:

| User says | You do | Surfaces that re-publish |
|---|---|---|
| "add a feature for X" / "another scenario where…" | Author: edit `spec/*.feature`, stamp a new `@rid:`. Then Stepwright adds step defs (red). Then Builder in slices. | Plan (+1 pending → +1 red → +1 green). End product if the new feature touches a visible surface. |
| "change the wireframe to show Y" | Designer: edit `mockups/*.html`, re-publish the wireframe card. If the change implies a new observable (a new substring the page should contain, a new field in a POST body), Author adds the scenario to lock it. Then red→green as above. | Wireframe (updated); Plan if a scenario was added; End product once green. |
| "update the look" (type scale, spacing, colour) | Designer: edit mockup + end-product wrapper. **Do NOT add scenarios** — the test contract is substrings, not CSS. | Wireframe, End product. Plan and Chart unchanged. |
| "rename / drop a scenario" | Author: edit `spec/`, `shalt spec delete FILE LINE` or rewrite. | Plan (count changes). Chart (dropped rids disappear). |
| "the wireframe is wrong — change X on the page" | Designer edits mockup first. If the visible change is **observable** (contains/doesn't contain a substring), Author adds or edits a scenario; Stepwright + Builder follow. If it's cosmetic, only the wireframe + end product re-publish. | Case-by-case. |

Key rules when iterating:

- **Changes to `spec/` while tests exist will re-red the affected scenarios.** That's the loop working — the chart will show mixed state (some green, some red). Work it back to all green before shipping.
- **Spec is the contract; wireframe is the review.** If the wireframe changes but no scenario changes, the tests still pass against the OLD contract — the end product may now *look* different without a scenario failing. Add a scenario if the shape change matters enough to lock.
- **Approve again** after any change to `spec/`. `shalt verify` goes back to "spec is not approved" the moment a scenario is added, edited, or removed. Surface the command: `cd "$ROOT" && shalt --root . -y approve`.
- **Keep the chart URL stable across iterations.** The user's chat already has the card; they watch it update. A brand-new URL for every iteration defeats the point.
- **The end-product card is also stable.** Re-render the seed through the (now-green) `handle()` each time a visible-shape iteration lands and re-publish the same end-product URL.

A natural iteration takes one or two re-publishes per surface, not a full re-flow. The skill is the same discipline; iterating just means the loop is longer than one pass.

Re-publishing costs ~100ms (dump + splice) and one tool call. Do it on every state change, not on every `shalt run` — only when `shalt status` differs from last dump.

### Implementing one scenario at a time

To make the chart fill in piece by piece, implement src in slices where each slice flips one scenario (or a tight group) from red to green. Before you start, list the slices:

```
slice 1: handle("POST","/recipes") stores the recipe; GET /r/{slug} returns 200 with the title
         → S-aa000001 (Publish) green; the others still red
slice 2: GET /r/{slug} HTML also includes each ingredient
         → S-aa000002 (Share page) green
slice 3: Steps render; video_second → HH:MM:SS
         → S-aa000003 (Video) green
slice 4: Duplicate slug returns 409
         → S-aa000004 (409) green
```

For each slice: edit `src/`, `shalt run`, re-publish same URL. If a slice accidentally greens more than one scenario, that's fine; if it greens none because you misjudged the slice, re-slice before committing.

### Ladder publish + republish, concretely

```bash
# After the spec is complete, before writing any src:
bash ~/.claude/skills/shalt/report/dump.sh "$ROOT" > /tmp/shalt-report.json
# → splice into ladder-template.html → publish Artifact → KEEP the returned URL for this loop

# After every state change (one per slice; also after the first red confirmation):
bash ~/.claude/skills/shalt/report/dump.sh "$ROOT" > /tmp/shalt-report.json
# → re-splice → publish SAME URL (pass `url` to the Artifact tool).
```

Host-specific notes:
- **Claude desktop app**: `Artifact({ url: "<kept url>", file_path: "/tmp/<project>-ladder.html" })` on each re-publish keeps the same card and its subscribers.
- **Grok / Hermes / Codex**: emit the HTML inline each cycle; the surface picks it up. Keep the filename stable so the host can diff.

### Zone discipline is the invariant

- Author writes `spec/` only.
- Stepwright writes `tests/` (Rust) or `steps/` (JS) + `contract/`; cannot see `src/` or `mockups/`.
- Builder writes `src/`; may read `mockups/` and `spec/`; cannot see step definitions.
- **Pending is not green.** Green is a bound passing test for a stamped `@rid:`.
- Red before green. If you ever write src that passes a test the test suite never saw as red, delete the src and restart the ticket — the ledger will know (`never_red` > 0 for that scenario).

If you catch yourself about to write `src/` to make a bug-report "go away", stop. Open the spec, add or fix the scenario, make it red, then build.

---

## Reports and the ladder (shalt docs)

When the user asks for the **shalt docs**, the **ladder**, the **workflow lens**, a **ubiquitous-language view**, an **interactive diagram** of the project, or "show me the state of this": generate an HTML artifact the host can render inline. The skill ships the generator — don't hand-roll one.

```bash
# 1. Dump the ladder as JSON
bash ~/.claude/skills/shalt/report/dump.sh "$ROOT" > /tmp/shalt-report.json

# 2. Splice it into the template
PROJECT=$(jq -r .project /tmp/shalt-report.json)
GENERATED=$(jq -r .generated_at /tmp/shalt-report.json)
OUT="$ROOT/.shalt/ladder.html"   # or your scratchpad
sed -e "s/__PROJECT__/$PROJECT/g" -e "s/__GENERATED_AT__/$GENERATED/g" \
    ~/.claude/skills/shalt/report/ladder-template.html > "$OUT"
python3 -c "
import sys
html = open('$OUT').read()
data = open('/tmp/shalt-report.json').read()
inj  = '<script>window.SHALT_REPORT = ' + data + ';</script>\n'
open('$OUT','w').write(html.replace('<script>', inj + '<script>', 1))
"

# 3. Publish (Claude desktop app)
#    Artifact({ file_path: "$OUT", icon: "ladder", description: "<project> — shalt ladder" })
#    Other hosts: emit $OUT inline wherever attachments render.
```

The template (`ladder-template.html`) is a single self-contained HTML page with click-to-open popovers, drill-through child buttons, collapsible tree, structured document, a bottom-sheet variant on narrow screens, and term highlighting for nouns (muted dotted), verbs (amber dotted), and protocols (green double). It reads `window.SHALT_REPORT` at startup.

The shape (`schema: "shalt.report/1"`):
- `DATA` — keyed by `milestone:…`, `feature:…`, `scenario:…`, plus any vocabulary refs. Each entry carries `kind`, `title`, `status`, `children`, `fields` or `gwt`, and a `parent` link.
- `vocabulary` — `{ nouns, verbs, protocols }` each an array of `{ ref, title, patterns, fields?, children? }`. The template wraps surface-form matches in clickable spans that open the term's popover.

**Per-project vocabulary** overrides live at `.shalt/vocabulary.json` in the same shape as the `vocabulary` field. Without it, only `HTTP` is a known term. See `report/README.md` for the full example.

Reference artifact of the generator rendering the sample-greet project: https://claude.ai/artifact/UASobM1jt6G6Dxu7vQGaUD

For just the terminal roll-up: `shalt tree` · `shalt stories` · `shalt status`. For system diagrams (sequence/state/etc.): `shalt diagrams`.

---

## Play mode (opt-in only)

Invoke only when the user explicitly asks for autonomous small-model work. Play orchestrates qwen / grok / openai through design → steps → build → run, writing all zones from the chosen model. You step aside; shalt drives.

```bash
shalt --root "$ROOT" --backend qwen --model=qwen3:8b -y play
shalt --root "$ROOT" --backend qwen --model=qwen3:8b -y play --yolo   # take the model's guess at every ask_human
```

Stages print `PLAY start|ok|idle|failed`. Each board item carries its own `backend`/`model`; reassign with `shalt board agent RID qwen::qwen3:8b` when a stage comes up pinned to a model you don't have (the "no API key for 'claude'" failure is this). Play's design stage draws storyboards (`mockups/*.html`) — skip it for API-only features with `shalt --root "$ROOT" steps` to go straight to tests.

The UI is Play's natural surface:

```bash
shalt ui                     # http://127.0.0.1:7702/
shalt ui restart --no-open   # after a rebuild
shalt stop                   # kills UI + Play + children, parks jobs
```

### Waiting jobs

Play blocks on `ask_human`. Same loop as the desk dialog:

1. `shalt job waiting` or `shalt job list`
2. `shalt job ask [J-xxx]` — prints question, guess, chat so far
3. Talk to the human here. Chat can use a different agent than the blocked worker: `shalt --backend grok job chat J-xxx "…"`. A model may emit `ANSWER:` — that's a draft, not a commit.
4. `shalt job answer J-xxx "the decision"` once the human agrees. Do not `job resume` instead of answering.
5. Yolo to stop asking: `shalt org yolo ID on`.

---

## Intent → command (quick lookup)

| They want | You run |
|---|---|
| Fresh project, I'll author | `shalt --root "$ROOT" -y --backend fixture do "The system shall …"` then edit `spec/` |
| Wrap existing code | `shalt onboard PATH --prompt "…"` |
| Pick language | `shalt stack rust` / `shalt stack javascript` |
| Red/green loop (you drive) | edit `tests/shalt.rs` → `shalt run` → edit `src/` → `shalt run` → `shalt verify` |
| Red/green loop (model drives) | `shalt --backend qwen --model=qwen3:8b -y play` |
| Status / tree / stories / integrity | `status` · `tree` · `stories` · `verify` |
| Edit a scenario | `shalt spec delete FILE LINE` · `spec promote FILE LINE` |
| Board / sprint | `board promote RID` · `board agent RID qwen::qwen3:8b` · `sprint open NAME` · `sprint assign RID C-1` · `sprint close` · `sprint retro` |
| Dashboard | `shalt ui` on 7702 |
| Stop everything | `shalt stop` |
| Shalt docs / workflow lens / interactive diagram | `bash ~/.claude/skills/shalt/report/dump.sh "$ROOT"` + splice into `report/ladder-template.html`, publish as Artifact. Recipe above. |
| Storyboards | `shalt design` |
| Diagrams | `shalt diagrams` |
| Journal | `shalt journal` |
| Mutation test | `shalt mutate` |
| Export / import plan | `shalt plan export [FILE]` · `shalt plan import FILE` |
| Jobs (Play mode) | `job list` · `job show ID` · `job pause/resume ID` · `job ask [ID]` · `job answer ID "…"` |

`--root "$ROOT"` on every command when not already `cd`'d there.

---

## Pitfalls

- A compile dump (`could not compile`, `error[E…]`, cucumber-js `SyntaxError` in `steps/`) is one harness error, not a failing Then on every scenario. Fix the test file; do not patch `Cargo.toml` / `package.json` / `contract/` / `src/` to "help" past it.
- Don't use another TDD loop (Hermes TDD, Superpowers TDD) in parallel. Shalt is the loop; this skill is its agent-mode front.
- Rust and JavaScript are supported. Python etc. runs if the toolchain is there; don't install pytest-bdd unless the user asked for `--stack python`.
- Never ask a human for token estimates. Shalt forecasts from history.

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

Links this directory into Hermes, Claude, Grok, and Codex. One skill, two modes (Agent default, Play on request).
