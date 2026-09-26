# Storyboards and sketched HTML prototypes

> **For agentic workers:** Implement in this session. TDD for engine (zones, films, Design stage). Desk after the engine is green.

**Goal:** Plan is a wall of storyboard films. Frames are sandboxed HTML prototypes, sketched until the ledger is green, then polished. Designer role writes `mockups/`. Spec still wins.

**Architecture:** `mockups.rs` derives films from spec + `storyboard.json`. Play grows `Stage::Design` / `JobKind::Design` before Language. Desk renders films on Plan and a preview on Details. Iframe sandbox: scripts yes, same-origin and network no.

**Tech Stack:** shalt-core + shalt (axum, ui.html). No image API.

## Global Constraints

- Spec wins. Pending is not green. Do not say “Gherkin” in the desk.
- Designer writes only `mockups/`. Stepwright does not read mockups. Implementer reads mockups.
- CLI = UI (`shalt design`).
- Do not auto-Play ERP. Do not invent spec. Do not patch `src/` / `contract/` / `Cargo.toml`.
- Empty frames exist as soon as the spec does.

## Tasks

1. `mockups.rs` — films, spec-order frames, kind none, stale, orphan, built from ledger green, `design_needed`, `verify_mockups`.
2. Integrity — `ALL_ZONES` + designer write/read maps; hostile tests.
3. `JobKind::Design`, `Stage::Design`, `next_stage`, `play_chains_after`, epoch `project/design`, `execute_design`, ROLE_SYSTEM, `shalt design`.
4. Project API `films`, GET `/api/project/{id}/mockups/{*path}` with sketch CSS + `.built` injection.
5. Plan wall of films + Details preview iframe; kindLabel Designing; mermaid pipeline.
6. Docs: cli.md, isolation.md, skill.
