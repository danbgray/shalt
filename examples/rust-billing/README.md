# Rust example — cucumber-rs

A real Cargo project, driven by the same `shalt` workspace model as the Python example. It
exists to test the language-agnosticism claim against a real runner rather than a payload we
wrote ourselves — and it immediately found two bugs, both recorded below.

```bash
cd examples/rust-billing
shalt --root . run           # 9 scenarios, via cargo test + cucumber-rs
shalt --root . tree
shalt --root . verify
shalt --root . mutate --engine text --budget 8
```

Requires a Rust toolchain. Unlike `examples/invoice`, this one is **not** part of `demo.sh`,
which stays dependency-free.

## How it maps onto Cargo

`shalt.toml` points the zones at Cargo's own layout, so nothing has to be rearranged:

| shalt zone | Cargo directory |
|---|---|
| `spec/` | `spec/` — feature files, passed to `cucumber::run("spec")` |
| `steps/` → `tests/` | the `[[test]] harness = false` target |
| `src/` | the crate |

`cargo test` has no flag for a report path, so the preset passes it through `[runner].env` as
`SHALT_REPORT`, which the test binary reads. Environment values take the same `{report}`
placeholder the command does, so the path is written once.

## What this example found

**1. Tag spelling is not portable.** cucumber-rs emits `{"name": "rid:S-0d41bae1"}` — with **no
leading `@`** — while cucumber-jvm and cucumber-js keep it. The Cucumber JSON format does not
settle the question. shalt's parser matched only `@rid:`, so it bound nothing: all nine
scenarios read `pending`, which looks like unfinished work rather than a bug. The fix normalises
the tag before matching. Regression test: `test_a_tag_without_the_leading_at_sign_still_binds`,
using output captured verbatim from this project.

**2. Mutation testing was silently wrong for compiled languages.** The campaign restored `src/`
with `shutil.copytree`, which *preserves mtimes* — so the restored source looked older than
artifacts built from a mutant, cargo skipped the rebuild, and the next campaign's **baseline ran
a binary built from the previous campaign's last mutant**. Scenarios wrongly failed at baseline,
so mutants affecting them could not be killed and were reported as survivors. Identical input
gave 100%, then 25%, then 25%.

Two fixes: restored files are stamped as modified now, and the campaign re-runs the baseline
afterwards and refuses to report a score if the workspace no longer reproduces it. A run that
reports nothing about any measured scenario is now `invalid` rather than `survived` — silence is
not evidence of survival, and cucumber-rs exits `0` even when scenarios fail.

## The spec

Two epics, two stories, nine scenarios, two of them held out:

- `spec/reminders.feature` — `@epic:collections`, escalation thresholds; the holdout checks the
  day before escalation.
- `spec/currency.feature` — `@epic:billing`, symbol and grouping from minor units; the holdout
  checks that yen rounds rather than truncates.

Amounts are integer minor units so the crate needs no decimal dependency.
