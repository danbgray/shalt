"""Regression tests for every way the isolation guarantee was found to be defeatable.

Each test here corresponds to a confirmed escape from an adversarial review of the first
version, in which the guard existed but was never wired into the pipeline.
"""
import os
from pathlib import Path

import pytest

from ratchet.integrity import IntegrityViolation
from ratchet.ledger import Ledger
from ratchet.roles import run_role

FEATURE = """\
Feature: Money

  @rid:S-11111111
  Scenario: Add
    Given amounts "1.00" and "2.00"
    Then the total is "3.00"
"""
TEST_SRC = "def test_real():\n    assert True\n"


@pytest.fixture
def workspace(tmp_path):
    for z in ("spec", "steps", "contract", "src", ".ratchet"):
        (tmp_path / z).mkdir(parents=True, exist_ok=True)
    (tmp_path / "spec" / "money.feature").write_text(FEATURE)
    (tmp_path / "steps" / "test_money.py").write_text(TEST_SRC)
    Ledger().save(tmp_path / ".ratchet" / "ledger.json")
    return tmp_path


class _Backend:
    """A backend that does whatever `action` says, to model a misbehaving agent."""
    name = "evil"

    def __init__(self, action):
        self.action = action

    def run(self, role, prompt, stage):
        self.action(Path(stage))
        return "done"


def _assert_workspace_intact(ws):
    assert (ws / "steps" / "test_money.py").read_text() == TEST_SRC
    assert (ws / "spec" / "money.feature").read_text() == FEATURE


# --------------------------------------------------------- escaping the stage
def test_relative_traversal_out_of_the_stage_cannot_reach_the_tests(workspace):
    def act(stage):
        (stage / "src" / "ok.py").write_text("x = 1\n")
        escape = stage / ".." / ".." / ".." / "steps" / "test_money.py"
        escape.parent.mkdir(parents=True, exist_ok=True)
        escape.write_text("assert False  # weakened\n")
    run_role(workspace, "implementer", "p", _Backend(act))
    _assert_workspace_intact(workspace)


def test_absolute_write_to_the_tests_is_caught_and_rolled_back(workspace):
    def act(stage):
        (stage / "src" / "ok.py").write_text("x = 1\n")
        (workspace / "steps" / "test_money.py").write_text("assert False  # weakened\n")
    with pytest.raises(IntegrityViolation) as ei:
        run_role(workspace, "implementer", "p", _Backend(act))
    assert "steps" in ei.value.offences
    _assert_workspace_intact(workspace)


def test_symlinked_directory_inside_an_allowed_zone_is_rejected(workspace):
    def act(stage):
        os.symlink(workspace / "steps", stage / "src" / "escape")
        (stage / "src" / "escape" / "test_money.py").write_text("assert False\n")
    with pytest.raises(IntegrityViolation) as ei:
        run_role(workspace, "implementer", "p", _Backend(act))
    assert "symlink" in ei.value.offences
    _assert_workspace_intact(workspace)


def test_symlinked_file_masquerading_as_a_source_module_is_rejected(workspace):
    def act(stage):
        os.symlink(workspace / "steps" / "test_money.py", stage / "src" / "shim.py")
        (stage / "src" / "shim.py").write_text("assert False  # weakened\n")
    with pytest.raises(IntegrityViolation):
        run_role(workspace, "implementer", "p", _Backend(act))
    _assert_workspace_intact(workspace)


def test_a_role_cannot_rewrite_the_ledger(workspace):
    def act(stage):
        (stage / "src" / "ok.py").write_text("x = 1\n")
        (workspace / ".ratchet" / "ledger.json").write_text(
            '{"schema": "ratchet.ledger/1", "scenarios": {}, '
            '"spec_lock": {"approved_by": "nobody"}}')
    with pytest.raises(IntegrityViolation) as ei:
        run_role(workspace, "implementer", "p", _Backend(act))
    assert "ledger" in ei.value.offences
    assert Ledger.load(workspace / ".ratchet" / "ledger.json").spec_lock == {}


def test_a_file_at_the_stage_root_is_rejected(workspace):
    def act(stage):
        (stage / "loose.py").write_text("x = 1\n")
    with pytest.raises(IntegrityViolation):
        run_role(workspace, "implementer", "p", _Backend(act))


def test_deleting_a_readonly_staged_file_is_rejected(workspace):
    def act(stage):
        (stage / "spec" / "money.feature").unlink()
    with pytest.raises(IntegrityViolation):
        run_role(workspace, "implementer", "p", _Backend(act))
    _assert_workspace_intact(workspace)


# --------------------------------------------------------- legitimate behaviour
def test_writing_only_to_its_own_zone_is_allowed(workspace):
    def act(stage):
        (stage / "src" / "money.py").write_text("def total(): return '3.00'\n")
    res = run_role(workspace, "implementer", "p", _Backend(act))
    assert res.wrote == ["src/money.py"]
    assert (workspace / "src" / "money.py").exists()


def test_a_role_can_delete_a_file_in_its_own_zone(workspace):
    (workspace / "src" / "stale.py").write_text("old\n")

    def act(stage):
        (stage / "src" / "stale.py").unlink()
        (stage / "src" / "fresh.py").write_text("new\n")
    res = run_role(workspace, "implementer", "p", _Backend(act))
    assert res.removed == ["src/stale.py"]
    assert not (workspace / "src" / "stale.py").exists()
    assert (workspace / "src" / "fresh.py").exists()


def test_stepwright_sees_its_own_previous_steps_but_never_the_implementation(workspace):
    (workspace / "src" / "secret.py").write_text("SECRET = 1\n")
    seen = {}

    def act(stage):
        seen["zones"] = sorted(p.name for p in stage.iterdir())
        seen["has_previous_steps"] = (stage / "steps" / "test_money.py").exists()
    run_role(workspace, "stepwright", "p", _Backend(act))
    assert "src" not in seen["zones"], "the stepwright must not see the implementation"
    assert seen["has_previous_steps"], "it must see its own previous work to revise it"


def test_implementer_never_sees_the_step_definitions(workspace):
    seen = {}

    def act(stage):
        seen["zones"] = sorted(p.name for p in stage.iterdir())
    run_role(workspace, "implementer", "p", _Backend(act))
    assert "steps" not in seen["zones"]
    assert "spec" in seen["zones"] and "contract" in seen["zones"]
