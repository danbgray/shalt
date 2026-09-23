# Shalt — packed context and MoE write routing

**Date:** 2026-09-22
**Status:** draft, awaiting review
**Repo:** this repository (`github.com/rivletio/shalt`)

## Goal

Local Play produces a real oracle with a chain of small models, then a 27B audit, without dumping the workspace into a 4k window or falling off the write ladder into models that cannot call tools.

Shalt is the router and the scaffolder. Models fill holes.

## Why

Recipe share Play (2026-09-22) showed the current mix is fast at emitting tokens and slow at producing tests:

- First user message is: long system essay + journal sign-off + entire stage tree + tools JSON. Flash `num_ctx` is 4096. Most of the window is instructions, not the scenario.
- The JS prompt contains the cheat-sheet `function (not arrow) World;`. `qwen3.5:2b-mlx` pasted that into `steps/ingredients.steps.js`. The suite then reported 0 scenarios and exit 0.
- Write chain is `0.6b → 1.7b → 2b-mlx → 4b → 8b` then **`qwen2.5:0.5b → gemma3:1b → llama3.2:1b → gemma3:270m`**. Play died on `gemma3:1b does not support tools`. The log line said “next smaller coder.”
- Ticket pool (Grok / Claude / 27B-mtp) is ignored. `pin_inner_loop` overwrites it.
- 27B audits only after bind succeeds. Bind never succeeded, so the auditor never ran.
- Bind is phrase-matching. Copying `patronage.steps.js` to `patrons.steps.js` counted as 5/5 bound and made half the suite ambiguous.
- Board stayed 7/18. Gate stayed `tests`. Envelope stayed paused (correct).

We already scaffold `src/` from `contract/interface.md` (`apply_js_contract_stubs`). This slice does the same for the tests zone and makes the write pool match reality.

## Non-goals (this slice)

- Flash-as-classifier (`qwen3:0.6b` naming the gap without writing). Optional later. Flash *is* a writer now: three cheap tries, trash discarded.
- Implementer brief beyond the gate “Build does not start until tests audit PASS.” Failure-digest + one stub is a follow-on.
- Changing Envelope, auto-Play, yolo semantics (except: stepwright must not ask what step definitions look like).
- Desk copy except the lane log line. Do not say “Gherkin” in the desk.
- Doorbell agents writing `spec/`, `steps/`, `contract/`, or `src/` with file tools.
- Mutation testing, holdout policy, or zone rules. Zones do not change.
- Reloading Ollama’s 35B-128k session. Leave it alone.

## Key decisions

1. **Shalt emits stubs. Models fill bodies.** Signatures come from the spec phrases. Bodies start as `return 'pending'` (JS) or equivalent rust pending. The filler does not choose files, invent phrases, or rewrite a working oracle.
2. **Write pool is tool-capable only:** `qwen3:0.6b` → `qwen3:1.7b` → `qwen3.5:2b-mlx` → `qwen3:4b` → `qwen3:8b`. Never `gemma3:*`, `qwen2.5:0.5b`, `llama3.2:1b` (no tools). **N is measured:** `floor(auditor_pass / last_fill)` or `floor(writer_tok_s / auditor_tok_s)` — 1000 tok/s buys more fills than 200. Unknown → 2 so we measure. **Each failure hops up the chain.** Do not sit on 0.6B repeating. At 8B, leftover budget can repeat there. Trash is discarded. If none of the write pool is installed, the job fails.
3. **Packed brief, not a dump.** Size-class system prompt (~200 tokens at 2B, short essay at 8B). First user message is the brief. It does not include `Files you can see:` or the cheat-sheet `function (not arrow)`.
4. **Canned few-shot from shalt**, not a sibling file in the project. A corrupt `recipes.steps.js` must not become the example.
5. **Thin stage.** Stepwright sees: the one steps file, `world.js`, the one feature file, `contract/interface.md`. Other step files are not copied in.
6. **Ambiguous is not bound.** Two non-stub definitions for the same phrase do not count as bound. Pending / empty / `not implemented` is already not bound.
7. **Exact-duplicate step files are quarantined** to `.shalt/dup-steps/` (outside `steps/`, so `cucumber-js --import steps/**/*.js` cannot see them). Not deleted. `list_step_files` skips `dup-steps` / `.dup`. Rust: one harness file, no quarantine.
8. **27B test audit is fail-closed** and runs **before** any `src/` write. Timeout or missing review model: do not enqueue Run/Build. Gate stays `tests`.
9. **Escalate by failure class**, not by “wrote files, bound 0, try the next name on a list.” Stop after 8B.
10. **Board agents stay labels.** The inner loop is this chain. Do not pretend a ticket assigned `grok-4` is what writes tests.
11. **Native `/api/chat` + `options.num_ctx` stays.** Write models load at `local_num_ctx` (8192 for 2B/4B/8B). Do not leave `qwen3:8b` resident at 262144.
12. **JS is the live path (Recipe). Rust gets the same gates in this slice:** append-only pending steps in `tests/shalt.rs`, not a second file.

## Architecture

Two layers, never mixed:

| Layer | Owns | Does not |
|---|---|---|
| **Shalt (no LLM)** | pick the model, pack the window, emit stubs, parse-gate, bind-lock, quarantine duplicates, escalate by class | write behaviour |
| **Model** | fill pending bodies in one file; may append a missing contract export | choose files, change signatures, add extra step files |

Zones do not change. Stepwright still owns `steps/` + `contract/`. Shalt-the-binary may scaffold. Doorbell agents still do not.

A tests job:

```
focus journey
    → pin writer (0.6b, else 1.7b, else 2b-mlx, else 4b, else 8b)
    → quarantine exact-duplicate step files
    → stubber (signatures + pending bodies)
    → thin stage + brief
    → filler (bodies only)
    → parse → bind → 27B audit
    → PASS: job ok, Play may enqueue Run/Build
    → FAIL: escalate by class, at most to 8B, then fail closed
```

## Components

### Write pool (`crates/shalt-core/src/alloc.rs`)

Replace the inner-loop lists used by `pick_fast_model` / `pick_escalate_model`.

```
WRITE_MODELS = [
  "qwen3.5:2b-mlx",
  "qwen3:4b",
  "qwen3:8b",
  "qwen3:1.7b",   // only reached if none of the three above are installed
]
```

`pick_fast_model` returns the first installed name in that order, **skipping 1.7b when any of 2b-mlx / 4b / 8b is installed**.

`pick_escalate_model(current)` walks the same list and returns the next installed name that is **not** 1.7b unless it is the only remaining writer. After 8B: `None`.

`FLASH_MODELS` may remain for `local_num_ctx` / timeouts. Flash is not a writer. `FAST_MODELS` must not be the escalate tail.

`lane_for_role` / `lane_for_build_turn` stay Fast for stepwright and implementer, Review for author / designer / auditor / code_auditor.

### Step stubber (`crates/shalt-core/src/scaffold.rs`)

From the focus journey’s spec phrases, emit:

- JS: `steps/<journey>.steps.js` with cucumber-js ESM imports, `function` handlers (not arrows), `{string}` / `{int}` captures, bodies `return 'pending';`
- Rust: append pending `#[given]` / `#[when]` / `#[then]` functions to `tests/shalt.rs` if missing. Do not create a second harness file.

Rules:

- Dedupe phrases **globally**. If another step file already defines the phrase, do not emit it in this file.
- Never clobber a file that **parses and already binds** at least one scenario in this journey for real (non-stub, non-ambiguous).
- **Do** replace a file that does not parse (the 2026-09-22 `ingredients.steps.js` spec-paste).
- `write_js_world_if_missing` stays. World is not a model output.

Canonical JS path is `steps/<journey>.steps.js` where `<journey>` is the epic slug (`journey_slug`).

### Bind and duplicates (`crates/shalt-core/src/bindings.rs`)

Keep: pending / `todo!` / empty body / `not implemented` → `stub: true` → not bound.

New: a phrase with **two or more** non-stub definitions is **ambiguous**. `def_matches` for bind requires **exactly one** non-stub definition. Ambiguous ≠ bound.

Exact duplicate files (byte-for-byte, or the same non-empty phrase set as another step file) are moved to `.shalt/dup-steps/<filename>` **before** the stubber runs. Deterministic. No model. `list_step_files` and `load_step_defs` skip `.dup` / `dup-steps`. Putting copies under `steps/.dup/` would still be imported by cucumber-js.

`steps_needed` is true when the focus journey has any scenario that is not bound under these rules, including garbage and ambiguous files.

### Thin stage (`crates/shalt-core/src/roles.rs`)

For `stepwright` on a focused tests job, the stage is not the whole workspace. Copy in:

- `steps/<journey>.steps.js` (or `tests/shalt.rs` on rust) plus `steps/world.js` when JS
- the one feature file for that journey (not every file under `spec/`)
- `contract/interface.md`
- `shalt.toml` / `Cargo.toml` as today

Writable: the one steps file (and `tests/shalt.rs` on rust) and `contract/interface.md`. Extra step files in the stage are a zone-style offence and the turn is discarded.

Already-bound phrases from files **not** in the stage appear in the brief as a short list, not as files to edit.

### Brief packer (new `crates/shalt-core/src/brief.rs`)

`brief_for(role, stack, model, pack: BriefPack) -> (system, user)`

`BriefPack` for stepwright:

- current text of the one steps file
- unbound scenarios (rid, name, Given/When/Then lines), cap 14 as today
- contract export names already declared
- already-bound phrases (do not redefine)
- canned few-shot constant from shalt (a tiny legal `Given`/`Then` plus World) — not a project sibling

System prompt is size-classed:

- 2B / 1.7B: ~200 tokens. Role, zone, “fill pending bodies, do not change signatures, do not add files.”
- 8B: current short essay minus the copy-paste cheat-sheet.
- 27B auditor: unchanged PASS/FAIL contract.

`api.rs` `run()` for write-pool models uses this brief as the first user message. It does **not** append `Files you can see:\n{tree}`. `list_files` remains a tool; the model starts from the brief.

Journal `SIGN_OFF` stays on the system prompt, not duplicated in the user brief.

Filler policy: do not call `ask_human` to ask what step definitions look like. The stub is the answer. If that question appears, shalt answers with ASSUME_REPLY equivalent of “fill the pending bodies in the file you were given” without taking a yolo guess at prose.

Contract: filler may **append** a missing export to `contract/interface.md` when a step must call it and it is absent. It may not rewrite the whole contract on a 2B pass. It may not add other files.

### Gates (`crates/shalt-core/src/pipeline.rs`)

After the filler returns:

1. **Parse.** `steps_source_ok(stack, path, body)`. JS: `parse_step_defs` finds at least one step **or** the file is `world.js`; body must not contain `function (not arrow)`; if `node` is on PATH, `node --check` exits 0. Rust: at least one `#[given]`/`#[when]`/`#[then]` remains in `tests/shalt.rs`. Fail → discard the write, class `parse`.
2. **Bind.** `bound_count` after must be **greater** than before. Fail → class `bind`.
3. **27B audit** (`Subject::Tests`). PASS required. FAIL → class `audit`. Timeout / no review model → **fail closed**, class `audit`. Do not treat `Ok(None)` as skip-and-build.

Escalate:

| Class | Next writer |
|---|---|
| `parse` | 2B (stay if already 2B, one retry) |
| `bind` | 4B, then 8B |
| `audit` | 8B rewrite against findings, then 27B once more |
| `behaviour` | 8B (reserved for the later implementer brief) |

Parse: one retry at 2B, then fail. Bind: walk the remaining write pool (2B → 4B → 8B), then fail. Audit: one 8B rewrite and one more 27B pass, then fail. Never hop to a model outside `WRITE_MODELS`. Never hop to 1.7B after a larger writer.

Desk / job log: `lane · write · {model} · {class}` with `{parse, bind, behaviour, audit}`. Not “next smaller coder.”

Build / Run are not enqueued while `steps_needed` is true. Audit FAIL or timeout leaves gate `tests`.

`pin_inner_loop` uses `pick_fast_model` on `WRITE_MODELS`. If the job already holds a write-pool model, keep it. If it holds gemma / 0.5b / 27B, pin to the write pool (0.6B first when installed) before the first fill.

## Data flow

`execute_steps` becomes:

1. `start_running` / play lock (unchanged).
2. Pin writer from `WRITE_MODELS`.
3. Resolve focus journey (unchanged).
4. Stubber + quarantine duplicates.
5. Record `bound_before`.
6. Build brief + thin stage.
7. `run_role` stepwright with the brief.
8. Parse → bind → audit loop with classed escalate.
9. `apply_js_contract_stubs` only **after** audit PASS (src stubs are Build’s surface; do not grow `src/` during a failed tests job).
10. `finish_ok` with bind delta, or `finish_err` fail closed.

Play, pause, one-project lock, Envelope-not-auto-Play: unchanged.

## Error handling

| Fail | What shalt does |
|---|---|
| Target file does not parse | Replace with a stub, then fill. Do not keep garbage. |
| Filler output does not parse | Discard that write. Stay on / drop to 2B. One retry. |
| Wrote extra files or changed signatures | Discard. Same as a zone offence. |
| Bind did not rise | Escalate 2B → 4B → 8B. Stop. |
| Ambiguous phrases remain | Not bound. Quarantine exact dups first; leftover conflict fails bind. |
| 27B FAIL | One 8B rewrite against the findings, then 27B once more. Still FAIL → job failed, no Build. |
| 27B timeout / missing | Fail closed. Do not skip audit and enqueue Build. |
| No tool-capable writer installed | Fail the job. Do not fall through to gemma/0.5b. |
| Filler 400 / no-tools | Routing bug. Do not chain the next toy model. Fail the job. |
| 8B resident at 262144 ctx | Unload and reload at 8192 before the fill. Native `/api/chat` only. |
| Play / pause / Envelope | Unchanged. One project. Do not auto-Play Envelope. |

## Testing

All shalt-core. No live Ollama. Fixture backend. Do not mutate the Recipe project as a test fixture.

**Alloc**

- Installed `{0.6b, 2b-mlx, 8b, gemma3:1b}` → `pick_fast_model` is `qwen3:0.6b`
- Fill budget: unknown → walk the remaining write chain (do not strand on 1.7B). Measured: 5s vs 90s → 18; 1000 tok/s vs 25 → 40. Failures hop 0.6→1.7→2b→4b→8b. Stepwright does not get `ask_human`. Three mixed stage fails are not a Play spin — only the same failure three times.
- Same without 2b-mlx → `qwen3:4b`, else `qwen3:8b`, else `qwen3:1.7b`
- `pick_escalate_model(2b-mlx)` → 4b → 8b → `None`
- Only gemma / 0.5b installed → `pick_fast_model` is `None`

**Stubber**

- Missing `steps/ingredients.steps.js` → pending signatures for unique phrases
- File that does not parse → replaced
- File that parses and already binds → untouched
- Phrase already in `patronage.steps.js` → not emitted into `patrons.steps.js`
- Rust: pending `#[given]` appended to `tests/shalt.rs`, not a second file

**Bind**

- `return 'pending'` is not bound
- Two non-stub defs for the same phrase → not bound
- Exact-duplicate file → `.shalt/dup-steps/`, bind uses the original only

**Brief + stage**

- Stage listing is only: the one steps file, `world.js`, the one feature, `contract/interface.md`
- First user message does not contain `Files you can see:` or `function (not arrow)`
- Canned few-shot is a constant in shalt

**Pipeline**

- Parse fail discards the write and does not count as bind
- Bind did not rise → escalate then fail, no Run/Build
- 27B FAIL then FAIL → job failed, gate `tests`
- 27B timeout → fail closed, gate `tests`
- Audit PASS → Play may chain Run/Build as today

## Files

| File | Change |
|---|---|
| `crates/shalt-core/src/alloc.rs` | `WRITE_MODELS`, pick/escalate, no toy tail |
| `crates/shalt-core/src/scaffold.rs` | Step stubs, garbage replace, phrase dedupe |
| `crates/shalt-core/src/bindings.rs` | Ambiguous ≠ bound; skip `.dup/` |
| `crates/shalt-core/src/runner.rs` | `list_step_files` skips `.dup` / `dup-steps` |
| `crates/shalt-core/src/roles.rs` | Thin stage for focused stepwright |
| `crates/shalt-core/src/brief.rs` | New. Size-class prompts + packed user brief |
| `crates/shalt-core/src/api.rs` | Use brief instead of tree dump on write pool; keep native ctx |
| `crates/shalt-core/src/speed.rs` | Measured tok/s + fill secs; retry while another fill fits in one auditor pass |
| `crates/shalt-core/src/pipeline.rs` | Stubber, quarantine, measured retry, fail-closed audit, src stubs after PASS |
| `crates/shalt-core/src/audit.rs` | Timeout already returns err/skip — pipeline must not treat skip as PASS |
| `crates/shalt-core/tests/alloc.rs` or `alloc` module tests | Write pool |
| `crates/shalt-core/tests/scaffold.rs` | Step stubs |
| `crates/shalt-core/tests/bindings.rs` | Ambiguous + quarantine |
| `crates/shalt-core/tests/pipeline.rs` | Gates, no Build on audit fail/timeout |

## Invariants this slice must not break

- Spec wins. Pending is not green. Green is a bound passing test against a stamped `@rid`.
- Stepwright does not see `src/` or mockups. Implementer does not see step definitions.
- One project plays at a time. Do not auto-Play Envelope.
- `shalt stop` does not kill Ollama.
- Do not print keys. Do not install pytest-bdd unless the project is `--stack python`.
- Skills shell out to shalt. Doorbell agents do not write the zones with file tools.
- Forecast vs actual still measured. Do not ask a human for token estimates.

## Success

On a JS project like Recipe, a tests job either:

- raises bind on the focus journey with parse-valid, non-ambiguous, non-stub steps and a 27B PASS, or
- fails closed with a classed reason, gate still `tests`, no toy model, no Build.

It never pastes the prompt into `steps/`. It never loads gemma as a writer. It never skips 27B because bind was faked by a copied file.
