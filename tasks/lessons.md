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
