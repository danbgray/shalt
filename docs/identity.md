# Scenario identity and canonical hashing

Two mechanisms make the ledger durable. Identity lets a scenario be tracked across edits;
canonical hashing makes an upheld status expire when the scenario's meaning changes.

## Durable ids

At approval, every unstamped scenario is given a durable id:

```gherkin
@epic:billing
Feature: Invoice totals

  As a billing clerk
  I want invoice totals computed exactly
  So that customers are never billed the wrong amount

  @rid:S-b291b8fd
  Scenario: An invoice with a single line item
    Given an invoice with lines:
      | description | amount |
      | Widget      | 100.00 |
    When I compute the invoice total
    Then the total is "100.00"
```

The id is `S-` plus 8 hex characters from `secrets.token_hex(4)`, unique across the whole
`spec/` directory rather than per file.

**Why a stamped tag rather than a derived identity.** The obvious alternatives both fail:

| scheme | fails because |
|---|---|
| position (`feature.feature:3`) | breaks on reorder or insertion |
| hash of the scenario text | breaks on every edit, including a typo fix |
| the scenario's name | breaks on rename, and names are not unique |
| **`@rid:` tag** | survives rename, rewording and reordering; deleting it is an explicit act |

Because the id is a Gherkin *tag*, it also survives into every Cucumber-family test report,
which is what makes result binding language-independent — see [runners.md](runners.md).

## Stamping is parser-driven

`spec.stamp_rids` inserts tags using line numbers from the Gherkin AST, never by matching text.
This matters more than it sounds. An earlier line-based implementation stamped a `Scenario:`
line that appeared *inside a docstring*:

```gherkin
  Scenario: Real
    Given a doc:
      """
      Scenario: this is prose, not a scenario
      """
```

Docstring content is part of a scenario's meaning, so that edit silently changed the spec and
reported a phantom scenario to the user. Regression test:
`test_a_docstring_cannot_be_mistaken_for_a_scenario_when_stamping`.

Stamping is idempotent (already-stamped scenarios are skipped), processes scenarios bottom-up so
earlier line numbers stay valid, and preserves CRLF line endings by reading bytes rather than
text.

`spec.duplicate_rids` reports scenarios carrying more than one `@rid` tag, and ids reused across
scenarios. `shalt verify` surfaces both.

## The canonical hash

A scenario's canonical form contains **everything that changes its meaning and nothing that
doesn't**:

| included | excluded |
|---|---|
| feature name | leading/trailing whitespace |
| background steps | indentation |
| tags, sorted — except `@rid` | tag order |
| the keyword (`Scenario` vs `Scenario Outline`) | the `@rid` tag itself |
| the scenario name | comments outside the scenario |
| every step: keyword + text | blank lines |
| docstrings | the feature's description block |
| data tables | |
| `Examples:` tables | |

Rendered as text, then `sha256`, truncated to 32 hex characters.

```python
# spec.py — Scenario.canonical
parts = [
    f"FEATURE:{feature_name}",
    "BACKGROUND:\n" + "\n".join(background),
    f"TAGS:{','.join(sorted(t for t in tags if not t.startswith('@rid:')))}",
    f"{keyword.upper()}:{name}",
    "STEPS:\n"    + "\n".join(steps),
    "EXAMPLES:\n" + "\n".join(examples),
]
```

Two deliberate choices worth challenging if you ever revisit this:

- **Background is included.** A change to the feature's `Background:` changes what every
  scenario in it means, so every one of them goes stale. That is correct but noisy.
- **The description block is excluded.** The user story is documentation, not behaviour, so
  rewording "So that customers are never billed wrong" does not expire anything. If you consider
  the narrative part of the contract, this is the line to move.

## Why status expires

An upheld status is always recorded *against* a canonical hash:

```json
"spec_hash":          "sha256:7f273b590236781891b588192789a900",
"verified_spec_hash": "sha256:7f273b590236781891b588192789a900"
```

`ledger.sync_spec` recomputes `spec_hash` on every command. If it no longer matches, a `green`
scenario becomes `stale`. The obligation was owed against particular words; reword it and nothing
has been discharged.

This is the rule that stops the oldest failure in spec-driven work — the spec drifted, the suite
still passes, nobody noticed. Verified behaviour:

- editing one scenario's expected value stales **exactly that scenario** and leaves its siblings
  upheld (`test_green_goes_stale_when_its_scenario_changes_meaning`);
- cosmetic edits change nothing (`test_cosmetic_edits_do_not_change_the_spec_hash`);
- a meaning change always changes the hash (`test_meaning_changes_do_change_the_spec_hash`).

## The spec lock

`shalt approve` records what was signed off:

```json
"spec_lock": {
  "approved_by": "dan@rivlet.io",
  "approved_at": "2026-09-12T15:44:20Z",
  "scenario_count": 14,
  "files": {"invoice.feature": 5, "currency.feature": 4, "reminders.feature": 5},
  "scenario_hashes": {"S-b291b8fd": "sha256:...", "...": "..."}
}
```

`scenario_hashes` is the part that does the work. `integrity.audit` compares it against the spec
on disk and reports three distinct problems: a scenario **changed** since approval, one
**added** since approval, and an approved scenario **no longer present**. An earlier version
stored only counts, which meant `shalt verify` reported "integrity ok" on a spec whose expected
values had been edited.
