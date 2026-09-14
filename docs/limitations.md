# Limitations, non-goals and known risks

This page exists because a tool whose entire pitch is "your green means something" has to be
straight about where its own guarantees stop.

## Live models vs the mock

`shalt --backend grok` (xAI, `XAI_API_KEY`) and `--backend openai` are the live adapters.
Path refusals are tested without a network. Fixtures remain the offline demo.

A live green suite still needs `shalt mutate` afterwards: the model that wrote the steps is
not a proof that the steps assert anything. The `claude-cli` backend is written and unverified.

## Gherkin's expressiveness ceiling

Gherkin is bad at latency, throughput, cost, security posture, accessibility, concurrency,
resource limits and anything about how a UI feels. There is currently **no escape hatch**, which
means a specification written only in Gherkin is a lie by omission: it looks complete and
silently omits every non-functional requirement.

The honest framing today is that `shalt` verifies functional behaviour and says nothing about the
rest. A future version would need a way to attach non-functional obligations to a scenario and
check them — probably a separate obligation type with its own runner, not a Gherkin extension.

## What the guards do not cover

- **Confidentiality is layer-one only.** You cannot un-read a file. The workspace guard detects
  and rolls back writes, but if a backend can read outside its stage, the isolation is already
  gone. For the API backend this is enforced properly; for `claude-cli` it depends on Claude
  Code's own permission model, unverified.
- **Inference from failure output.** The digest is filtered to the role's visible scenarios, but
  within those it is verbatim, so an implementer can reverse-engineer assertions from error text.
- **Resource exhaustion.** Beyond the runner timeout there is no memory, disk or wall-clock cap.
  A runaway agent can fill the disk.
- **A malicious backend implementation.** The protocol trusts the backend to run in the stage.
  A deliberately hostile backend that shells out arbitrarily is outside the threat model — the
  adversary here is a model taking shortcuts, not an attacker with code execution.
- **The interface contract is unchecked.** A stepwright that declares the wrong API surface will
  be satisfied faithfully and wrongly, and nothing notices.

## Statistical, not proven

Holdouts and mutation testing both *sample*:

- **Holdouts** catch crude overfitting. A sufficiently capable implementer generalises just far
  enough to pass them — and every holdout you spend is a scenario you are not using to guide the
  build.
- **Mutation testing** is sampled, slow (one full suite run per mutant), and produces survivors
  that need human triage to separate a weak assertion from a missing scenario from an equivalent
  mutant.

Neither produces a guarantee. They raise the cost of a bad oracle from zero to non-zero.

## Operational gaps

| gap | consequence |
|---|---|
| no cost or token accounting in the build loop | a 40-turn build does not report what it spent |
| no concurrency control on the ledger | two `shalt` commands at once lose one set of writes |
| no `shalt undo` | rolling back an approval means editing the ledger by hand |
| no incremental runs | every turn runs the whole suite |
| no CI recipe shipped | exit codes are stable and documented, but there is no example workflow |
| the dashboard is a snapshot | it does not update itself; regenerate after each run |

## Non-goals

- **Not a test framework.** It drives yours. The runner is a commodity and deliberately replaceable.
- **Not a Gherkin dialect.** Plain Gherkin, plus two conventions (`@rid:`, `@epic:`) and one
  optional tag (`@holdout`). Any Cucumber-family tool can read the spec without `shalt`.
- **Not a proof system.** "Upheld" means a test passed against a recorded meaning. It is not a
  correctness proof and the docs should never imply otherwise.
- **Not a replacement for review.** The human gate at the Gherkin is load-bearing. A tool that
  let you skip it would be solving a different, easier, less useful problem.

## Where it would break first

If you are looking for the weakest link, in order:

1. **The stepwright's judgement.** It is the oracle. Mutation testing samples it; nothing proves
   it. Everything downstream inherits its mistakes.
2. **The approval gate becoming a rubber stamp.** If nobody actually reads the Gherkin, the whole
   chain of custody is decorative. Nothing in the tool can detect this.
3. **Non-functional requirements.** The first production incident that is not a functional bug
   will be one this tool could never have caught, and the gap should be stated before then rather
   than after.
