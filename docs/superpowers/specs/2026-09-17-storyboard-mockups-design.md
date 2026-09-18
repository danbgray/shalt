# Shalt — storyboards, HTML prototypes, sketch-to-built

**Date:** 2026-09-17
**Status:** draft, awaiting review
**Repo:** this repository (`github.com/rivletio/shalt`)

## Goal

A traditional UX path lives inside shalt, not beside it: user stories → story map → visual storyboard → clickable HTML prototype → tests → code.

The **visual storyboard is the top-level Plan**. Each journey is a film of screens. Previews are real HTML/CSS/JS you can click through, drawn in a hand-sketched style so they read as illustration. A design agent hones colour, shape, and layout. When a beat (or a region of a screen) is green in the ledger, that part locks to a polished look. One film shows what Play has proven and what is still a concept.

The spec still wins. The prototype is a designed reading of the spec, the same way the board is a ranking reading. Drawings do not invent behaviour tests will never see.

## Why

Plan already has journeys and a force-directed map of how they connect. Details already pins one scenario. What is missing is the middle of a UX process: storyboards and actual screens. Mermaid is a diagram. A sandboxed HTML prototype is something you can use. Generative raster is neither versionable nor a handoff the implementer can read.

## Non-goals (this slice)

- Figma, or any host besides `shalt ui` / the CLI.
- Image models, screenshots as the source of truth, or pixel-diff tests.
- A Mockups tab. Films live on Plan; the open frame’s preview lives on Details.
- Moving prototype files into `src/`. The implementer still writes `src/`. The storyboard is where sketch and built meet visually.
- Embedding the live production app in the iframe (later). This slice restyles the prototype from the ledger (`.built`), it does not mount `src/`.
- Blocking CLI/API-only journeys behind fake screens.
- Saying “Gherkin” in the desk (tabs, films, Play log, job prompts shown in the UI).

## Key decisions

1. **Journey → one storyboard. Scenario `@rid` → one frame → one HTML preview.** The user story (`As a` / `I want` / `So that`) captions the strip. Order is spec order. Two rids may share one HTML file.
2. **Plan is a wall of films.** Thin KPI row above. Force-directed journey graph is connections, under the films or behind a control — not the first thing on Plan.
3. **HTML/CSS/JS prototypes, not pictures.** Designer writes them under `mockups/`. Click-through links are the path through the journey.
4. **Sketch until green.** Default look is hand-drawn (wobbly strokes, paper, informal type). Design chat hones the drawing. Ledger green on a rid (or a `data-rid` region) applies a polished stylesheet. Pending is not green.
5. **Designer is a real role and a Play stage.** Writes only `mockups/`. Stepwright does not read mockups (oracle stays the spec). Implementer reads mockups the way they read `contract/`.
6. **CLI = UI.** `shalt design` is the same job as desk Design. Skills shell out; agents do not write `mockups/` with their own file tools.
7. **Empty frames exist as soon as the spec does.** Named boards with the scenario title, sketched. Design fills in HTML. The wall of films does not wait on the designer to exist.

## Mapping (identity)

Shalt’s three spec levels do not grow a fourth outline:

| Level | Spec | Storyboard |
|---|---|---|
| Journey | `@epic:` or folder under `spec/` | One film strip |
| User story | Feature + `As a` / `I want` / `So that` | Caption on that strip |
| Beat | Scenario with `@rid:` | One frame, one preview |

The pin is `@rid`. Same pin as Plan → Details → Tests → Code.

`storyboard.json` is a join table, not a copy of the stories:

```
mockups/
  tokens.sketch.css
  tokens.built.css
  preview.js
  journeys/<journey-slug>/
    storyboard.json
    inbox.html
    compose.html
```

```json
{
  "journey": "envelope-lifecycle",
  "kind": "ui",
  "spec_hash": "<hash of rids+scenario hashes in this journey>",
  "frames": [
    { "rid": "S-abc", "file": "inbox.html", "caption": "Sender opens the inbox" },
    { "rid": "S-def", "file": "compose.html", "caption": "Sender writes the envelope" }
  ]
}
```

- Journey slug is the id the desk already uses (`@epic:` value, else the first directory under `spec/`).
- `kind: "ui"` — film of previews.
- `kind: "none"` — compact “no screen” row (API/CLI journeys). No HTML required.
- Frame order in the json array is ignored at render time. The desk and CLI play frames in **spec scenario order** for that journey. Missing json still shows derived empty frames in spec order.
- Shared screen: two frames, same `file`, different `rid`. Mark regions in the HTML with `data-rid="S-…"`.

## Fidelity ladder

| Look | When | What you see |
|---|---|---|
| Sketch | No bound passing test for that rid (or region) | Hand-sketched HTML. Illustration, not the product. |
| Honed sketch | Design chat about colour, shape, layout | Still pencil. Layout is the agreed one. Behaviour unchanged unless it also lands in the spec. |
| Built | That rid (or `data-rid` region) is ledger **green** | Polished look on that part. Reads as product. |

One strip may mix all three.

Rules:

- Designer HTML always starts on `tokens.sketch.css`.
- `preview.js` (or the desk) adds class `built` on `[data-rid="S-…"]` when that rid is green. If a frame’s HTML has no `data-rid`, the whole document gets `built` when **that frame’s** rid is green.
- Red, pending, stale, orphan → sketch. Absence of a test never rounds to built.
- Design talk that only changes look writes `mockups/`. Talk that adds a control, a beat, or a path must fold into `spec/` (same rule as drawings elsewhere). The picture cannot grow obligations.

Sketch language: shalt ships a default sketch stylesheet (rough borders, slight rotation, paper, informal type, dashed annotation — no image API, no webfont CDN). The desk injects it on empty frames and as the base for prototype iframes. `mockups/tokens.sketch.css` overrides that default when present. Built language is `tokens.built.css` (same layout, clean rules). Designer’s HTML may link both; the desk still flips `.built` from the ledger.

## Desk

**Plan**

- Lead: one storyboard per journey, horizontal film, user-story caption.
- KPI row above (counts, spend, Now). Not a chart-first page.
- Click a frame → pin that rid: large sandboxed preview, then Details for that beat.
- API `kind: none` journeys: one compact row, not a fake film.
- Journey graph: how films connect; secondary.

**Details**

- Spec-as-page for the open beat, preview beside it (iframe). No extra tab.

**Tests / Code**

- Unchanged file tree + reader. Mockups are not shown as `src/`.

**Now bar**

- Design stage label: “Designing …”. Do not say “Gherkin”.

**Sandbox**

- Preview iframe: scripts allowed, same-origin and network not allowed. Prototype JS cannot touch the desk or the internet.
- Served by `shalt ui` from the project’s `mockups/` (existing project file API, scoped). Not `file://`.

## Pipeline and zones

```
English → Author (spec/) → Designer (mockups/) → Language → Stepwright (tests + contract) → Implementer (src/)
```

| Role | Writes | Reads |
|---|---|---|
| author | `spec/` | spec, src |
| designer | `mockups/` | spec, mockups |
| stepwright | steps, contract | spec, steps, contract — **not** mockups |
| implementer | `src/` | spec, contract, src, **mockups** |
| human | spec, steps, contract, src, mockups | same |

`ALL_ZONES` gains `mockups`. Hostile designer writing `src/` or `spec/` is rolled back. Hostile implementer writing `mockups/` is rolled back.

**Play:** after spec exists, one Design job for the project (epoch `project/design`, same shape as Author). Designer marks non-visual journeys `kind: none` and writes HTML for the rest. Then Language (if unset), then Steps / Run / Build as today.

Design does not need a stack. If spec exists and any journey still lacks a `storyboard.json` (or `spec_hash` is stale vs the spec), `next_stage` is Design before Language/Steps. Failed Design still chains Play (`play_chains_after`), same as a failed build — do not idle with an empty wall. Named empty frames remain if HTML is missing.

Now / job kind label: Designing. CLI: `shalt design`. Skill table includes it.

**Design talk:** chat on the Design job (same waiting / chat / answer / yolo dialog as author — no second popup type). A different agent may draft answers. Visual-only answers write `mockups/`. Behavioural answers go through the spec. Yolo: take the guess, keep drawing.

## Drift, verify, errors

- Spec hash on a journey moves → frames for those rids show a stale badge; Play queues Design. Spec wins; a Then is not overridden by a pretty screen.
- `storyboard.json` names a rid not in the spec → **orphan**; `verify` fails.
- Frame `file` missing on disk → `verify` fails (broken join).
- Journey with scenarios and no `storyboard.json` yet → derived empty frames; not a verify failure (Design has not run, or `kind` not written).
- Designer integrity violation → turn rejected wholesale (existing `GuardedTurn`).
- Prototype JS throws in the iframe → desk still shows the frame chrome and the spec; do not take the desk down.

## Data flow

1. Author writes feature files. Desk derives empty sketched frames per scenario, grouped by journey.
2. Design job writes `storyboard.json` + HTML/CSS/JS. Films fill in. Still sketch.
3. Human and design agent chat; mockups update; spec updates only when behaviour changes.
4. Stepwright binds tests to the spec, blind to mockups.
5. Implementer reads prototype + contract, writes `src/`.
6. Suite run. Green rids (and `data-rid` regions) get `.built` on Plan/Details from the ledger. Live job events that already patch KPI/journey status also flip `.built` without a full reload.

## Testing

Integrity (hostile backend, `run_role`, not a helper):

- Designer cannot write `src/` or `spec/`.
- Implementer cannot write `mockups/`.
- Stepwright staging has no `mockups/` tree.

Join and fidelity:

- Storyboard render order follows spec order, not json array order.
- Derived empty frames exist for a journey with spec and no json.
- `kind: none` emits no HTML requirement.
- Green rid → `built` on matching `data-rid` (and on the whole frame if no regions).
- Pending/red never → `built`.
- Orphan rid in json fails `verify`.
- Missing frame file fails `verify`.
- Stale `spec_hash` is a badge + Design re-queue, not a verify failure.

CLI / desk:

- `shalt design` is a known command (not rewritten to `do`).
- Desk copy for this surface does not include the word “Gherkin”.
- Preview URL is the shalt file API, iframe sandboxed.

## Files (implementation, after this spec is approved)

- `crates/shalt-core/src/integrity.rs` — zones / reads / `ALL_ZONES`
- `crates/shalt-core/src/jobs.rs` — `JobKind::Design`
- `crates/shalt-core/src/pipeline.rs` — `Stage::Design`, `next_stage`, chain
- `crates/shalt-core/src/roles.rs` / `api.rs` / `compose.rs` — designer prompts and tools
- `crates/shalt-core/src/uis.rs` or a small `mockups.rs` — parse `storyboard.json`, derive frames from spec, stale/orphan
- `crates/shalt/src/server.rs` — serve `mockups/`, design job, design chat
- `crates/shalt/src/ui.html` — Plan as films, Details preview, sketch/built class from ledger
- `crates/shalt/src/main.rs` — `shalt design`
- `docs/cli.md`, `docs/isolation.md`, `skills/shalt/SKILL.md`
- Fixture CSS: sketch + built tokens the designer may copy; desk can inject sketch on empty frames

## What does not change

- Spec wins. Pending is not green.
- Stepwright never sees `src/` or mockups.
- One binary, two fronts.
- `shalt stop` / `shalt ui stop` behaviour.
- Do not auto-Play ERP; do not invent spec; do not patch `src/` / `contract/` / `Cargo.toml` to “help.”
