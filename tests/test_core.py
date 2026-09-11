"""Tests for the invariants the whole tool rests on."""
from pathlib import Path

import pytest

from ratchet.integrity import GuardedTurn, IntegrityViolation, snapshot, diff
from ratchet.ledger import GREEN, PENDING, RED, STALE, Ledger
from ratchet.spec import load_specs, stamp_rids, strip_holdouts, holdout_rids

FEATURE = """\
Feature: Money

  @billing
  Scenario: Add two amounts
    Given amounts "1.00" and "2.00"
    Then the total is "3.00"

  @holdout
  Scenario: Unseen amounts
    Given amounts "4.00" and "5.00"
    Then the total is "9.00"
"""


def write_spec(tmp_path: Path, text: str = FEATURE) -> Path:
    d = tmp_path / "spec"
    d.mkdir(parents=True, exist_ok=True)
    (d / "money.feature").write_text(text, encoding="utf-8")
    return d


# ---------------------------------------------------------------- identity
def test_rid_is_stamped_once_and_is_stable(tmp_path):
    d = write_spec(tmp_path)
    minted = stamp_rids(d)
    assert len(minted) == 2
    first = {s.name: s.rid for f in load_specs(d) for s in f.scenarios}
    stamp_rids(d)  # idempotent
    second = {s.name: s.rid for f in load_specs(d) for s in f.scenarios}
    assert first == second


def test_rid_survives_renaming_and_reordering(tmp_path):
    d = write_spec(tmp_path)
    stamp_rids(d)
    path = d / "money.feature"
    before = {s.rid for f in load_specs(d) for s in f.scenarios}
    path.write_text(path.read_text().replace("Add two amounts", "Sum two amounts"))
    after = {s.rid for f in load_specs(d) for s in f.scenarios}
    assert before == after


# ---------------------------------------------------------------- hashing
def _hash_of(d: Path, name: str) -> str:
    for f in load_specs(d):
        for s in f.scenarios:
            if s.name == name:
                return s.spec_hash(f.background)
    raise AssertionError(name)


def test_cosmetic_edits_do_not_change_the_spec_hash(tmp_path):
    d = write_spec(tmp_path)
    stamp_rids(d)
    h = _hash_of(d, "Add two amounts")
    p = d / "money.feature"
    p.write_text(p.read_text().replace('Given amounts "1.00" and "2.00"',
                                       '  Given amounts "1.00" and "2.00"   '))
    assert _hash_of(d, "Add two amounts") == h


def test_meaning_changes_do_change_the_spec_hash(tmp_path):
    d = write_spec(tmp_path)
    stamp_rids(d)
    h = _hash_of(d, "Add two amounts")
    p = d / "money.feature"
    p.write_text(p.read_text().replace('the total is "3.00"', 'the total is "4.00"'))
    assert _hash_of(d, "Add two amounts") != h


# ---------------------------------------------------------------- ledger
def _ledger_for(d: Path) -> tuple[Ledger, list]:
    features = load_specs(d)
    led = Ledger()
    led.sync_spec(features)
    return led, features


def test_a_scenario_with_no_test_is_pending_never_green(tmp_path):
    d = write_spec(tmp_path); stamp_rids(d)
    led, _ = _ledger_for(d)
    led.apply_run({}, "r1")
    assert all(e.status == PENDING for e in led.entries.values())
    assert led.summary()["completion_pct"] == 0.0


def test_green_goes_stale_when_its_scenario_changes_meaning(tmp_path):
    d = write_spec(tmp_path); stamp_rids(d)
    led, _ = _ledger_for(d)
    rid = next(r for r, e in led.entries.items() if e.name == "Add two amounts")
    led.apply_run({r: {"outcome": "passed", "detail": "", "nodeid": "n"}
                   for r in led.entries}, "r1")
    assert led.entries[rid].status == GREEN
    p = d / "money.feature"
    p.write_text(p.read_text().replace('the total is "3.00"', 'the total is "4.00"'))
    led.sync_spec(load_specs(d))
    assert led.entries[rid].status == STALE, "green must not survive a change of meaning"
    others = [e for r, e in led.entries.items() if r != rid]
    assert all(e.status == GREEN for e in others), "unrelated scenarios keep their green"


def test_green_to_red_is_recorded_as_a_regression(tmp_path):
    d = write_spec(tmp_path); stamp_rids(d)
    led, _ = _ledger_for(d)
    rid = next(iter(led.entries))
    led.apply_run({rid: {"outcome": "passed", "detail": "", "nodeid": "n"}}, "r1")
    out = led.apply_run({rid: {"outcome": "failed", "detail": "boom", "nodeid": "n"}}, "r2")
    assert len(out["regressions"]) == 1
    assert led.entries[rid].status == RED


def test_blocked_suite_reports_red_not_pending(tmp_path):
    d = write_spec(tmp_path); stamp_rids(d)
    led, _ = _ledger_for(d)
    led.apply_run({}, "r1", blocked="ImportError: no module named 'money'")
    assert all(e.status == RED for e in led.entries.values())


def test_ledger_round_trips(tmp_path):
    d = write_spec(tmp_path); stamp_rids(d)
    led, _ = _ledger_for(d)
    path = tmp_path / ".ratchet" / "ledger.json"
    led.save(path)
    again = Ledger.load(path)
    assert set(again.entries) == set(led.entries)


# ---------------------------------------------------------------- holdouts
def test_holdouts_are_stripped_for_the_implementer_but_stay_in_the_ledger(tmp_path):
    d = write_spec(tmp_path); stamp_rids(d)
    features = load_specs(d)
    assert len(holdout_rids(features)) == 1
    visible = strip_holdouts((d / "money.feature").read_text())
    assert "Unseen amounts" not in visible
    assert "Add two amounts" in visible
    assert "@holdout" not in visible


# ---------------------------------------------------------------- guards
def _workspace(tmp_path: Path) -> Path:
    for z in ("spec", "steps", "contract", "src"):
        (tmp_path / z).mkdir(parents=True, exist_ok=True)
    (tmp_path / "steps" / "test_x.py").write_text("assert True\n")
    (tmp_path / "spec" / "x.feature").write_text(FEATURE)
    return tmp_path


def test_implementer_editing_the_tests_is_rejected_and_rolled_back(tmp_path):
    root = _workspace(tmp_path)
    original = (root / "steps" / "test_x.py").read_text()
    with pytest.raises(IntegrityViolation) as ei:
        with GuardedTurn(root, "implementer", root / ".ratchet" / "backup"):
            (root / "src" / "x.py").write_text("ok\n")
            (root / "steps" / "test_x.py").write_text("assert False  # weakened\n")
    assert "steps" in ei.value.offences
    assert (root / "steps" / "test_x.py").read_text() == original, "tests must be restored"


def test_implementer_writing_only_to_src_is_allowed(tmp_path):
    root = _workspace(tmp_path)
    with GuardedTurn(root, "implementer", root / ".ratchet" / "backup"):
        (root / "src" / "x.py").write_text("ok\n")
    assert (root / "src" / "x.py").exists()


def test_stepwright_cannot_touch_the_spec(tmp_path):
    root = _workspace(tmp_path)
    with pytest.raises(IntegrityViolation):
        with GuardedTurn(root, "stepwright", root / ".ratchet" / "backup"):
            (root / "spec" / "x.feature").write_text("Feature: rewritten\n")


def test_snapshot_diff_detects_content_change(tmp_path):
    root = _workspace(tmp_path)
    a = snapshot(root)
    (root / "src" / "y.py").write_text("1\n")
    b = snapshot(root)
    assert diff(a, b)["src"] == ["src/y.py"]
