"""Regression tests for defects found by adversarial review of the first version."""
from pathlib import Path

import pytest

from ratchet.integrity import audit
from ratchet.ledger import GREEN, ORPHAN, PENDING, Ledger
from ratchet.spec import (SpecParseError, duplicate_rids, holdout_rids, load_specs,
                          stamp_rids, strip_holdouts)

DOCSTRING_HOLDOUT = '''\
Feature: Support

  @rid:S-aaaaaaaa
  Scenario: Visible
    Given a thing
    Then it works

  @holdout @rid:S-bbbbbbbb
  Scenario: SECRET
    Given the email body:
      """
@support please note
Scenario: this line is prose
the total must be 9.00
      """
    Then the total is "9.00"
'''


def _spec(tmp_path, text, name="f.feature"):
    d = tmp_path / "spec"
    d.mkdir(parents=True, exist_ok=True)
    (d / name).write_text(text, encoding="utf-8")
    return d


def test_holdout_containing_a_docstring_does_not_leak(tmp_path):
    visible = strip_holdouts(DOCSTRING_HOLDOUT)
    assert "SECRET" not in visible
    assert "9.00" not in visible, "the held-out expected value must not leak"
    assert "@support please note" not in visible
    assert "Visible" in visible
    # and what remains must still be parseable Gherkin
    d = _spec(tmp_path, visible)
    assert len(load_specs(d)[0].scenarios) == 1


def test_a_docstring_cannot_be_mistaken_for_a_scenario_when_stamping(tmp_path):
    d = _spec(tmp_path, '''\
Feature: Docs

  Scenario: Real
    Given a doc:
      """
Scenario: this is prose, not a scenario
      """
    Then it works
''')
    minted = stamp_rids(d)
    assert len(minted) == 1, "only the real scenario is stamped"
    assert "@rid" not in (d / "f.feature").read_text().split('"""')[1]


def test_holdout_tag_matching_is_exact(tmp_path):
    d = _spec(tmp_path, '''\
Feature: X

  @holdout_wip @rid:S-cccccccc
  Scenario: Not actually held out
    Given a thing
    Then it works
''')
    features = load_specs(d)
    assert holdout_rids(features) == set()
    assert "Not actually held out" in strip_holdouts((d / "f.feature").read_text()), \
        "a tag that merely starts with @holdout must not be stripped"


def test_duplicate_rids_are_reported(tmp_path):
    d = _spec(tmp_path, '''\
Feature: X

  @rid:S-dddddddd @rid:S-eeeeeeee
  Scenario: Two ids
    Given a thing
    Then it works

  @rid:S-dddddddd
  Scenario: Reused id
    Given a thing
    Then it works
''')
    problems = duplicate_rids(load_specs(d))
    assert len(problems) == 2


def test_crlf_line_endings_survive_stamping(tmp_path):
    d = _spec(tmp_path, "Feature: X\r\n\r\n  Scenario: A\r\n    Given a thing\r\n")
    stamp_rids(d)
    raw = (d / "f.feature").read_bytes()
    assert b"\r\n" in raw, "a CRLF feature file must not be silently rewritten to LF"
    assert raw.count(b"\n") == raw.count(b"\r\n"), "no mixed line endings"
    assert b"@rid:" in raw


def test_a_malformed_feature_file_names_itself(tmp_path):
    d = _spec(tmp_path, 'Feature: X\n\n  Scenario: A\n    Given a doc:\n      """\nunterminated\n')
    with pytest.raises(SpecParseError) as ei:
        load_specs(d)
    assert "f.feature" in ei.value.errors


# ------------------------------------------------------------------ ledger
SIMPLE = '''\
Feature: X

  @rid:S-00000001
  Scenario: A
    Given a thing
    Then it works

  @rid:S-00000002
  Scenario: B
    Given a thing
    Then it works
'''


def test_orphan_is_not_an_absorbing_state(tmp_path):
    d = _spec(tmp_path, SIMPLE)
    led = Ledger()
    led.sync_spec(load_specs(d))
    led.apply_run({"S-00000001": {"outcome": "passed", "detail": "", "nodeid": "n"},
                   "S-00000002": {"outcome": "failed", "detail": "x", "nodeid": "n"}}, "r1")
    # delete the failing scenario, then restore it verbatim
    only_a = SIMPLE.split("  @rid:S-00000002")[0]
    (d / "f.feature").write_text(only_a)
    led.sync_spec(load_specs(d))
    assert led.entries["S-00000002"].status == ORPHAN
    (d / "f.feature").write_text(SIMPLE)
    led.sync_spec(load_specs(d))
    assert led.entries["S-00000002"].status == PENDING, \
        "a restored scenario must prove itself again, not stay invisible"
    assert led.summary()["completion_pct"] != 100.0


def test_losing_the_test_that_proved_a_scenario_is_a_regression(tmp_path):
    d = _spec(tmp_path, SIMPLE)
    led = Ledger()
    led.sync_spec(load_specs(d))
    led.apply_run({r: {"outcome": "passed", "detail": "", "nodeid": "n"}
                   for r in led.entries}, "r1")
    assert all(e.status == GREEN for e in led.entries.values())
    out = led.apply_run({}, "r2")  # every test vanished
    assert len(out["regressions"]) == 2, "silent downgrade to pending is not acceptable"
    assert all(e.status == PENDING for e in led.entries.values())
    assert all(e.verified_spec_hash == "" for e in led.entries.values())


def test_ledger_tolerates_unknown_fields(tmp_path):
    d = _spec(tmp_path, SIMPLE)
    led = Ledger()
    led.sync_spec(load_specs(d))
    path = tmp_path / ".ratchet" / "ledger.json"
    led.save(path)
    import json
    raw = json.loads(path.read_text())
    raw["scenarios"]["S-00000001"]["owner"] = "dan"
    path.write_text(json.dumps(raw))
    assert set(Ledger.load(path).entries) == {"S-00000001", "S-00000002"}


# ------------------------------------------------------------------ approval
def test_verify_detects_an_edit_after_approval(tmp_path):
    d = _spec(tmp_path, SIMPLE)
    features = load_specs(d)
    led = Ledger()
    led.sync_spec(features)
    led.spec_lock = {"approved_by": "dan", "approved_at": "now",
                     "scenario_hashes": {s.rid: s.spec_hash(f.background)
                                         for f in features for s in f.scenarios}}
    assert audit(tmp_path, led, load_specs(d)) == []
    (d / "f.feature").write_text(SIMPLE.replace("Then it works", "Then it does something else", 1))
    led.sync_spec(load_specs(d))
    problems = audit(tmp_path, led, load_specs(d))
    assert any("unapproved" in p for p in problems)


def test_verify_detects_a_scenario_added_after_approval(tmp_path):
    d = _spec(tmp_path, SIMPLE)
    features = load_specs(d)
    led = Ledger()
    led.sync_spec(features)
    led.spec_lock = {"approved_by": "dan", "approved_at": "now",
                     "scenario_hashes": {s.rid: s.spec_hash(f.background)
                                         for f in features for s in f.scenarios}}
    (d / "f.feature").write_text(SIMPLE + '''
  @rid:S-00000003
  Scenario: C
    Given a thing
    Then it works
''')
    led.sync_spec(load_specs(d))
    problems = audit(tmp_path, led, load_specs(d))
    assert any("added since approval" in p for p in problems)


def test_same_basename_in_different_directories_does_not_collide(tmp_path):
    d = tmp_path / "spec"
    for sub, rid in (("billing", "S-0000000a"), ("refunds", "S-0000000b")):
        (d / sub).mkdir(parents=True)
        (d / sub / "m.feature").write_text(
            f"Feature: {sub}\n\n  @rid:{rid}\n  Scenario: Totals are correct\n"
            f"    Given a thing\n    Then it works\n")
    mapping = {}
    for f in load_specs(d):
        for s in f.scenarios:
            mapping[(f.file.replace("\\", "/"), s.name)] = s.rid
    assert len(mapping) == 2, "binding must key on the path relative to spec/, not the basename"
