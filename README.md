# Shalt

English in. Spec, tests, and code out. **Green** means a bound test passed against the current wording of a scenario — not that an agent agreed with itself.

MIT. [github.com/danbgray/shalt](https://github.com/danbgray/shalt)

`shalt` is a Rust CLI. `shalt ui` is the same engine on `127.0.0.1` (default port 7700). Writers: Grok (`XAI_API_KEY`) or local Qwen via Ollama.

```bash
cargo test -p shalt-core --offline
cargo run -p shalt -- --help
shall --yes total invoices exactly in the customer's currency
shalt ui
```

`shall <sentence>` is `shalt do`. `--yes` accepts every scenario. `examples/invoice/demo.sh` is the offline loop (no API key). `examples/rust-billing` is cucumber-rs.

Docs: [architecture](docs/architecture.md) · [isolation](docs/isolation.md) · [ledger](docs/ledger.md) · [identity](docs/identity.md) · [runners](docs/runners.md) · [CLI](docs/cli.md) · [limitations](docs/limitations.md) · [the rest](docs/README.md)

## Why "shalt"

Requirements have used **"the system shall…"** for a long time. [RFC 2119](https://www.rfc-editor.org/rfc/rfc2119) makes SHALL an absolute requirement. [EARS](https://alistairmavin.com/ears/) is built on that verb.

This tool is that word aimed at the system. A spec is not a claim about what is true. It is an obligation. The ledger records which obligations still hold **against those words**. Reword the scenario and the old green is discharged — it goes stale.

## The problem

An agent that writes the spec, the tests, and the implementation can always go green. That is self-graded homework. Shalt takes the shortcuts away in the filesystem, not in the prompt.

| Failure | What it looks like | Defence |
|---|---|---|
| Tampering | Implementer edits the tests | Zones + write guard |
| Overfitting | Implementer special-cases the examples it saw | `@holdout` scenarios |
| Weak oracle | Then title is fine; the body does the When or `assert.ok(this.…)` | `#observe:` in the spec, Then-body lint, mutation |
| Drift | Spec moved; suite still green | Status bound to a canonical hash |

A human reading **only the sentences** is taking the spec's word. Review is sentences **plus** the Then bodies. Details shows both. `shalt verify` flags a Then that acts instead of observing.

## Zones

| Zone | Writer | Readers |
|---|---|---|
| `spec/` | author | everyone |
| `mockups/` | designer | designer, implementer, human |
| `steps/` | stepwright | test runner |
| `contract/` | stepwright | implementer |
| `src/` | implementer | implementer, test runner |

Each role runs in a **stage** that only contains what it may read. The stepwright cannot see `src/`. The implementer cannot see step definitions — it gets the spec, the contract, and failing output.

A turn that writes outside its zone is rejected and rolled back. Isolation is three layers: the stage is outside the workspace, the stage is scanned (including symlinks), and the real tree is hashed before and after. Tests go through `run_role` with a hostile backend, not the guard helper.

## Spec

At approval, every scenario is stamped:

```
  @rid:S-b291b8fd
  Scenario: An invoice with a single line item
    When I add a line of "10.00"
    Then the total is "10.00"
    #observe: printed total equals 10.00
```

`#observe:` is the door, locked before Play — same idea as a published hash vector. Changing the Then or the observe line after Play is an amendment, not a silent edit.

Statuses: `pending` (no bound test — **never green**), `red`, `green` (bound pass against current hash), `stale` (was green; wording changed), `orphan` (gone from the spec). First suite colour is recorded: red then green is evidence; a green that never went red is a suspicion.

## Language

The ledger and guards do not care what language the SUT is. The runner lives in `shalt.toml`:

```toml
[runner]
command = "npx cucumber-js {spec} --import {steps}/**/*.js --format message:{report}"
format  = "cucumber-messages"
report  = ".shalt/messages.ndjson"
```

`@rid:` is a tag. It survives into Cucumber-family reports, so binding a result does not need filename matching. Init defaults to **Rust** (cucumber-rs). **JavaScript** (cucumber-js) is supported. Other stacks may run if they emit Cucumber JSON or Messages; those presets are not finished.

`examples/rust-billing` is a real Cargo project on cucumber-rs. It found a tag-spelling miss (`rid:` vs `@rid:`) and a stale-binary mutation hazard. Both have regression tests.

A scenario is green only if **every** step passed. Skipped, pending, undefined, and ambiguous are not passes.

## Play

`shalt play` / `shalt org play ID` / the desk **Play** button: author → design → tests → run → build until the board is n/n. Then Play **parks** (`Done — n/n scenarios are green`). Sitting idle with Play still on was a loop bug.

Yolo (`shalt org yolo ID all`) takes guesses instead of asking. That is a setting, not a virtue.

## UI

One desk. If 7700 is taken, the next free port in 7700–7799. `--port` only when nothing is running. Logs: `~/.shalt/ui.log`.

```
shalt ui
shalt ui --foreground
shalt ui restart
shalt stop
```

Keys (Grok, OpenAI, Anthropic) live in the sidebar **Keys** panel → `~/.shalt/config.toml`. They are never printed. Env vars win if set.

macOS: `./scripts/macos-install.sh` puts Shalt.app in Applications and `shalt` on PATH. That is a window around the same desk, not a second product.

## Commands

```
shall --model                 list models / pick a default
shall --model=qwen3.5:2b …    pin this model
shall <sentence>              spec, y/n each scenario, then tests
shall --yes <sentence>        accept every scenario, then tests
shalt init [--stack NAME]
shalt author "…"
shalt approve --yes --by you  re-lock hashes after you edit the spec
shalt steps / shalt build / shalt run
shalt status / shalt verify / shalt tree / shalt stories
shalt diagrams / shalt dashboard / shalt mutate
shalt play
shalt ui / shalt org / shalt board / shalt job
```

## Backends

| | |
|---|---|
| `--backend fixture --fixtures DIR` | recorded turns; tests and `demo.sh` |
| `--backend grok` | xAI; `XAI_API_KEY` |
| `--backend qwen` | Ollama `http://127.0.0.1:11434` |
| `--backend openai` | `OPENAI_API_KEY` |

`--model` and `--base-url` override. Tools are `list_files`, `read_file`, `write_file`, `done` — no shell. Paths resolve inside the stage; absolute paths and traversal are refused.

## What this does not prove

See [limitations.md](docs/limitations.md). Short version: Feature/Scenario is a poor language for latency, cost, security, and how a UI feels. Holdouts and mutation **sample**. A Then that never went red is not evidence. Nobody reading the spec makes the human gate decorative.

## Layout

```
crates/shalt-core/      engine
crates/shalt/           CLI + desk HTML
crates/shalt-app/       optional macOS wrapper
examples/invoice/       offline fixture (Python is the SUT)
examples/rust-billing/  cucumber-rs
docs/
```

## License

[MIT](LICENSE). Copyright Daniel Gray.
