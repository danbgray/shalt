# shalt docs

Shalt turns an English request into Feature/Scenario obligations, then drives isolated roles until those obligations hold — against the current wording, with a bound test. Green is not "the model said so."

**Start here:** [manual.md](manual.md) (screenshots + how to run the desk).

## Read in this order

| | |
|---|---|
| [architecture.md](architecture.md) | problem, zones, pipeline |
| [isolation.md](isolation.md) | threat model and the three guards |
| [ledger.md](ledger.md) | status machine and schema |
| [identity.md](identity.md) | `@rid:` and why green expires |
| [runners.md](runners.md) | `shalt.toml` runner, adding a stack |
| [hierarchy.md](hierarchy.md) | epic → story → scenario, overlay board |
| [mutation.md](mutation.md) | does the oracle notice a broken `src/`? |
| [backends.md](backends.md) | fixture / grok / qwen / openai |
| [cli.md](cli.md) | commands and exit codes |
| [limitations.md](limitations.md) | where the guarantees stop |

`shalt diagrams` writes [diagrams/](diagrams/).

`examples/invoice` is the offline loop. `examples/rust-billing` is cucumber-rs.

## Short

Five write zones (`spec/`, `mockups/`, `steps/`, `contract/`, `src/`). One writer each. Roles run in a stage that only contains what they may read. A turn that writes outside its zone is rolled back. Every scenario has an `@rid:` and a hash of its meaning. No bound test → not green. Every Then needs `#observe:` before Play. `shalt verify` reads Then **bodies**, not just titles. `shalt mutate` breaks `src/` to see if the scenarios notice.
