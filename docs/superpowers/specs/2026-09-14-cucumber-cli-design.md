# Shalt CLI — Cucumber-shaped invocation and report

**Date:** 2026-09-14
**Status:** approved
**Repo:** this repository (`github.com/rivletio/shalt`)

## Goal

A user who knows Cucumber can run shalt like Cucumber. Feature paths and `--tags` select work. The terminal report is Cucumber pretty (or progress when piped). Shalt remains shalt: zones, rids, pending ≠ green, `shalt play` is still the factory.

## Invocation

```
shalt spec/*.feature
shalt spec/invoices.feature:12
shalt spec/*.feature --tags @wip
shalt run --tags "not @holdout"
shalt spec/*.feature --format progress
shalt spec/*.feature --dry-run
```

Cucumber feature files are `.feature` (not `.features`). Globs that the shell does not expand are expanded by shalt (`spec/*.feature`).

If the first positional argument is a feature locator (ends with `.feature`, or `file.feature:line`), it is a **run**, not English `do`, and not `shalt spec delete`. `shalt spec delete|promote` is unchanged because `delete` is not a file.

`--tags` uses Cucumber tag expressions: `@wip`, `not @wip`, `@wip and not @holdout`, `@wip or @slow`, parentheses. Comma is OR (`@wip,@slow`). Tags match the Gherkin tag including `@` (`@holdout`, `@epic:billing`, `@rid:S-ab12cd34`).

`--format auto|pretty|progress|play`. `auto` is pretty on a tty, progress when stdout is not a terminal. `NO_COLOR` still wins. `--dry-run` prints the matching scenarios without executing the harness.

No files and no subcommand still prints help. `shalt run` with no paths runs all of `spec/`.

## Report

Pretty (tty default):

```
Feature: Invoice totals

  Scenario: Line items sum to the invoice total    # spec/invoices.feature:12
    Given an invoice with lines 10.00 and 2.50
    When I ask for the total
    Then the total is 12.50
      expected 12.50, got 12.00

Failing Scenarios:
shalt spec/invoices.feature:12

1 scenario (1 failed)
3 steps (1 failed, 2 passed)
0m1.204s
```

Progress: one character per scenario (`.`, `F`, `U`, `P`, `-`) then the same summary.

Ledger → Cucumber: green=passed, red=failed, no test=undefined, stale=pending, tag-filtered-out=skipped (omitted from pretty body). Without per-step harness data, a failed scenario counts 1 failed step and the rest skipped; a passed scenario counts every step passed.

Exit: harness error `3`; any selected red `1`; otherwise `0`.

## What does not change

- `shalt spec delete|promote`
- `shalt play` / `loop` (factory). Skills still parse `PLAY` lines on that command.
- Zones, rids, pending ≠ green, one project playing at a time.

## Files

- `crates/shalt-core/src/tags.rs` — tag expressions + scenario locators
- `crates/shalt/src/pretty.rs` — pretty / progress / summary
- `crates/shalt/src/main.rs` — inject `run` for feature paths; `--tags` `--format` `--dry-run`
- `docs/cli.md` — document the invocation
