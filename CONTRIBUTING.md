# Contributing

## Licensing status — read this first

This repository is **private and not yet licensed**. No open-source licence has been granted, so
all rights are reserved by default.

The intention is to release it under the **MIT License**. Everything here is set up to keep that
option open, and the thing that most often destroys it is contributions whose copyright holders
never agreed to the licence. So:

**Every contribution must be signed off under the [Developer Certificate of
Origin](https://developercertificate.org/) (DCO) v1.1.** Add a `Signed-off-by` line to each
commit:

```bash
git commit -s -m "your message"
```

By signing off you certify that you wrote the contribution or otherwise have the right to submit
it under the project's licence, **including a future MIT release**. Contributions that are not
signed off cannot be merged, because they would block relicensing.

If you are not able to sign off — for example your employer owns the copyright — say so in the
pull request before writing any code.

## Before the licence flips

A checklist for whoever makes this public, kept here so it does not have to be reconstructed:

- [ ] Every commit in history is signed off, or its author has agreed in writing to MIT
- [ ] Decide and record the copyright holder in the `LICENSE` file
- [ ] Re-check that every runtime dependency is MIT-compatible (see below)
- [ ] Scan the full history — not just the tip — for credentials: `git log -p | grep -iE 'api[_-]?key|secret|token'`
- [ ] Remove or generalise anything internal to one organisation
- [ ] Confirm the package name is free on the registry you are publishing to

### Dependency licences

Runtime crates are MIT or MIT-compatible (`gherkin`, `serde`, `clap`, `axum`, …).
The invoice example’s *project under test* still uses pytest-bdd (MIT) as its runner.

## Working on it

```bash
cargo test --workspace
examples/invoice/demo.sh            # needs python3 + pytest + pytest-bdd for the SUT
```

Two rules that matter more than style:

1. **Test integrity guarantees through the public entry point.** A test that calls a guard
   helper directly cannot tell the difference between "wired in" and "dead code". This has
   already happened once — see `tasks/lessons.md`.
2. **Never let an absence of evidence round toward success.** A scenario with no test is
   `pending`, never green. Any new status or metric gets asked "how would I game this?" before
   it ships.
