# Lessons

## 2026-09-11 — the guard that was never wired in

Built a role-isolation mechanism (`GuardedTurn`) with tests proving it worked, and shipped a
pipeline that never called it. `grep -rn GuardedTurn` would have found only the test file. The
tests passed, the demo looked convincing, and the central claim of the project was false.

Compounding it: the staging directory was placed *inside* the workspace it was supposed to
isolate, so `../../../steps` reached the real tests from inside the sandbox.

**Rules for myself:**

1. When a security or integrity mechanism is added, grep for its production call sites before
   claiming it works. A passing test proves the mechanism works, not that it runs.
2. Test the guarantee through the public entry point, not the helper. `run_role(...)` with a
   hostile backend, never `GuardedTurn(...)` directly. Tests that call the helper cannot tell
   the difference between "wired in" and "dead code".
3. A sandbox nested inside the thing it protects is not a sandbox.
4. Path checks that classify by string prefix are defeated by symlinks. Walk without following
   links, and treat any symlink in a write zone as a write to its target.
5. For anything whose value *is* an adversarial guarantee, run a separate reviewer that has not
   seen me build it, and give it the specific escape routes to try. The review found 15 defects,
   including the one that invalidated the whole premise. My own testing found none of them.

## Ledger design

6. "Absence of evidence" states must never round toward success: a scenario with no test is
   `pending`, never green, and removing a failing scenario must not raise the completion figure.
   Any progress metric gets asked "how would I game this?" before it ships.

## 2026-09-11 — testing an API adapter without the API

Could not reach the provider (no key, and the egress gateway 403s the host). Rather than stop at
"blocked", built a mock that speaks the provider's wire protocol and ran the entire pipeline
over real HTTP through it. That covers request construction, the tool loop, path refusals,
retries, error surfacing and token accounting — everything except the model's judgement.

7. When an external dependency is unreachable, mock at the *protocol* boundary, not at the
   adapter boundary. Mocking the adapter tests nothing; mocking the wire protocol tests all of
   my code and only leaves the other party's behaviour unproven.
8. Say precisely what the mock does and does not establish. "Tested against Grok" would have
   been a lie; "the plumbing is proven, the judgement is not" is the honest claim.
9. A path sandbox must refuse, never silently reinterpret. `/etc/passwd` was being rewritten to
   `<stage>/etc/passwd` — no escape, but it hid intent. Refuse and say why.

## 2026-09-12 — "detected nothing" was the wrong test

Built a vacuous-oracle detector defined as "a green scenario that killed zero mutants", then
watched it clear a step definition whose entire assertion was `assert result is not None`. The
reason: a meaningless assertion still catches mutations that make the code *crash*, so it posts
a healthy kill count while being blind to every wrong value.

10. When measuring whether a test is meaningful, ask what the weak version still catches, not
    only what the strong version catches. The metric has to separate those two, or it certifies
    exactly the thing it was built to find.
11. Two partial signals that fire under different conditions need a named union, and the union
    is what callers check. I asserted the wrong one in a test and the test was right to fail.
12. Exclude findings nobody can act on. Mutated docstrings always survive; leaving them in put
    5 unactionable entries in a 6-entry survivor list, which teaches the reader to ignore it.
13. Read the line numbers before disbelieving the tool. I was sure a surviving mutant was a bug
    in my mutation engine; it was a real gap in a spec I had written and believed complete.

## 2026-09-13 — the language-agnostic claim was untested, and wrong twice

Shipped "works with any Cucumber-family runner" on the strength of a JSON payload I wrote myself
and handed to my own parser. The first real runner — cucumber-rs — broke it immediately, and
then broke mutation testing in a way that produced random scores.

14. **A parser tested only against a payload you authored is tested against your own
    assumptions.** cucumber-rs strips the `@` from tag names; cucumber-jvm keeps it; the format
    does not say. My synthetic fixture had the `@` because I wrote both sides. Test protocol
    adapters against a real implementation of the protocol, or do not claim compatibility.
15. **A silent bind failure looks like ordinary unfinished work.** Nothing errored — all nine
    scenarios just read `pending`, which is a perfectly normal state. A wrong answer that
    resembles a plausible one is far more dangerous than a crash. Where a binding step can bind
    *zero* things, that should be suspicious by construction, not silent.
16. **Restoring state is not the same as restoring behaviour.** `shutil.copytree` preserved
    mtimes, so restored sources looked older than artifacts built from a mutant and cargo skipped
    the rebuild. The *baseline* then ran mutated code. Same shape as the guard that was never
    wired in: the thing looked restored and was not.
17. **Check the result twice when the result is a measurement.** The campaign now re-runs the
    baseline afterwards and refuses to report a score if the workspace no longer reproduces it.
    I only found the bug because I distrusted a suspiciously round number — that instinct needs
    to be a mechanism, not a mood.
18. **Silence is not evidence.** A mutant run that reported nothing was being counted as
    "survived", manufacturing findings out of missing data. cucumber-rs exits 0 even when
    scenarios fail, so "no news" and "good news" were indistinguishable. Absence of evidence
    must never round toward a conclusion — the same rule the ledger already applies to
    `pending`, which I had failed to apply here.

## 2026-09-13 — presentation code can lie too

Added colour and clickable paths, and in doing so wrote two false statements into the product.

19. **A "helpful" substitution can destroy the content.** Making scenario names clickable meant
    that in terminals without OSC 8 the fallback printed an absolute path *instead of the
    scenario name* — every row became a path. The fix was to make the fallback a choice
    (`fallback="path"` where the path is the content, `"label"` where the label is), but the
    lesson is that a fallback needs designing, not defaulting.
20. **Do not let output claim more than the mechanism proves.** The blind-spot report said each
    scenario "provably executes the line that was broken". Attribution is file-granular; it
    proves the *file*. In a single-file Rust crate that difference is the whole finding. I only
    noticed because the Rust output attributed a reminder-logic survivor to the currency
    scenarios and I checked whether that could be true.
