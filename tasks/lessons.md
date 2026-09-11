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
