"""Tests for mutation testing the oracle.

The campaign tests build a deliberately tiny workspace and use a small budget, because every
mutant costs a full suite run.
"""
import ast
from pathlib import Path

import pytest

from ratchet.config import Config, write_config
from ratchet.ledger import Ledger
from ratchet.mutate import (Mutant, MutationReport, SENTINEL, _docstring_nodes, _py_targets,
                            python_mutants, run_campaign, text_mutants)
from ratchet.spec import load_specs

FEATURE = '''\
@epic:maths
Feature: Doubling

  As an analyst
  I want numbers doubled
  So that I can scale a figure

  @rid:S-00000001
  Scenario: Two doubles to four
    Given the number 2
    When I double it
    Then the result is 4

  @rid:S-00000002
  Scenario: Zero doubles to zero
    Given the number 0
    When I double it
    Then the result is 0
'''

STEPS_REAL = '''\
from pytest_bdd import given, parsers, scenarios, then, when
from maths import double

scenarios("../spec/maths.feature")

@given(parsers.parse("the number {n:d}"), target_fixture="n")
def _n(n):
    return n

@when("I double it", target_fixture="result")
def _d(n):
    return double(n)

@then(parsers.parse("the result is {expected:d}"))
def _c(result, expected):
    assert result == expected
'''

STEPS_VACUOUS = STEPS_REAL.replace("assert result == expected",
                                   "assert result is not None  # asserts nothing")

SRC = '''\
"""Doubling."""


def double(n):
    return n * 2
'''


def _workspace(tmp_path: Path, steps: str = STEPS_REAL, src: str = SRC) -> Path:
    root = tmp_path / "ws"
    root.mkdir(parents=True)
    write_config(root, "python")
    for d in ("spec", "steps", "contract", "src", ".ratchet"):
        (root / d).mkdir(parents=True, exist_ok=True)
    (root / "spec" / "maths.feature").write_text(FEATURE, encoding="utf-8")
    (root / "steps" / "conftest.py").write_text(
        "import sys, pathlib\n"
        "sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / 'src'))\n")
    (root / "steps" / "test_maths.py").write_text(steps, encoding="utf-8")
    (root / "src" / "maths.py").write_text(src, encoding="utf-8")
    Ledger().save(root / ".ratchet" / "ledger.json")
    return root


# ------------------------------------------------------------------ operators
def test_docstrings_are_never_mutated():
    tree = ast.parse('"""Mod."""\ndef f():\n    """Fn."""\n    return 1 + 2\n')
    assert len(_docstring_nodes(tree)) == 2
    described = [t[2] for t in _py_targets(tree)]
    assert not any("Mod." in d or "Fn." in d for d in described), \
        "a mutated docstring always survives, so it is noise rather than a finding"


def test_python_engine_covers_the_operator_families(tmp_path):
    src = tmp_path / "src"
    src.mkdir()
    (src / "m.py").write_text(
        "def f(a, b):\n"
        "    if a == b and a > 0:\n"
        "        return a + 1\n"
        "    return 'fallback' if b else False\n")
    ops = {m.operator for m, _ in python_mutants(tmp_path, src)}
    assert {"comparison", "arithmetic", "boolean", "number", "string",
            "boolean-literal"} <= ops


def test_every_python_mutant_is_a_real_change(tmp_path):
    src = tmp_path / "src"
    src.mkdir()
    (src / "m.py").write_text("def f(x):\n    return x == 1\n")
    for mutant, text in python_mutants(tmp_path, src):
        assert text != (src / "m.py").read_text()
        assert ast.parse(text), "a mutant must still parse"


def test_text_engine_works_on_a_language_with_no_python_ast(tmp_path):
    src = tmp_path / "src"
    src.mkdir()
    (src / "m.go").write_text(
        "package main\n"
        "// a comment with == in it\n"
        "func Stage(d int) string {\n"
        "    if d >= 30 && d < 60 { return \"firm\" }\n"
        "    return \"none\"\n"
        "}\n")
    mutants = text_mutants(tmp_path, src)
    assert mutants, "the text engine must handle Go"
    assert all(m.line != 2 for m, _ in mutants), "comment lines must be skipped"
    assert {"comparison", "boolean"} <= {m.operator for m, _ in mutants}


def test_text_engine_ignores_files_it_cannot_reason_about(tmp_path):
    src = tmp_path / "src"
    src.mkdir()
    (src / "notes.md").write_text("this == that and true\n")
    (src / "data.bin").write_bytes(b"\x00\x01==\x02")
    assert text_mutants(tmp_path, src) == []


# ------------------------------------------------------------------ report
def _report(killed_by_map, survived_paths, baseline):
    r = MutationReport(baseline_green=list(baseline))
    for path, killers in killed_by_map:
        m = Mutant(path=path, line=1, operator="number", before="1", after="2",
                   killed_by=list(killers), status="killed")
        r.mutants.append(m)
        for rid in killers:
            r.kills[rid] = r.kills.get(rid, 0) + 1
    for path in survived_paths:
        r.mutants.append(Mutant(path=path, line=2, operator="number", before="1",
                                after="2", status="survived"))
    return r


def test_a_scenario_that_kills_nothing_is_vacuous():
    r = _report([("src/a.py", ["S-1"])], [], ["S-1", "S-2"])
    assert r.vacuous == ["S-2"]


def test_exercised_files_are_derived_from_kills():
    r = _report([("src/a.py", ["S-1"]), ("src/b.py", ["S-2"])], [], ["S-1", "S-2"])
    assert r.exercised == {"S-1": {"src/a.py"}, "S-2": {"src/b.py"}}


def test_blind_spots_only_implicate_scenarios_that_run_the_broken_file():
    r = _report([("src/a.py", ["S-1"]), ("src/b.py", ["S-2"])], ["src/a.py"],
                ["S-1", "S-2"])
    blind = r.blind_spots
    assert list(blind) == ["S-1"], \
        "S-2 never runs a.py, so a survivor there is not its blind spot"


def test_the_two_signals_are_complementary_and_the_union_is_what_callers_check():
    """A vacuous scenario cannot also be a blind spot: with no kills there is no evidence of
    which files it runs. So neither property alone catches every weak oracle."""
    only_vacuous = _report([("src/a.py", ["S-1"])], ["src/a.py"], ["S-1", "S-2"])
    assert only_vacuous.vacuous == ["S-2"]
    assert "S-2" not in only_vacuous.blind_spots
    assert set(only_vacuous.weak_oracles) == {"S-1", "S-2"}


def test_a_healthy_kill_count_does_not_clear_a_blind_spot():
    """The reason 'killed nothing' is too weak a test on its own: a vacuous assertion still
    catches mutations that crash, so it can post a good kill count and still be blind."""
    r = _report([("src/a.py", ["S-1"])] * 5, ["src/a.py"], ["S-1"])
    assert r.vacuous == [], "it killed plenty"
    assert "S-1" in r.blind_spots, "but it ran broken code and said nothing"


def test_score_ignores_invalid_mutants():
    r = _report([("src/a.py", ["S-1"])], ["src/a.py"], ["S-1"])
    r.mutants.append(Mutant(path="src/a.py", line=9, operator="number", before="1",
                            after="2", status="invalid"))
    assert r.score == 50.0, "a mutant that broke the suite proves nothing either way"


# ------------------------------------------------------------------ campaign
@pytest.mark.slow
def test_a_real_oracle_detects_mutations_and_leaves_the_source_untouched(tmp_path):
    root = _workspace(tmp_path)
    before = (root / "src" / "maths.py").read_text()
    report = run_campaign(root, Config.load(root), engine="python", budget=4, seed=1)
    assert not report.error, report.error
    assert sorted(report.baseline_green) == ["S-00000001", "S-00000002"]
    assert report.killed, "breaking `n * 2` must be noticed by a real assertion"
    assert (root / "src" / "maths.py").read_text() == before, \
        "the campaign must restore the implementation exactly"


@pytest.mark.slow
def test_a_vacuous_oracle_is_caught_as_a_blind_spot(tmp_path):
    root = _workspace(tmp_path, steps=STEPS_VACUOUS)
    report = run_campaign(root, Config.load(root), engine="python", budget=6, seed=1)
    assert not report.error, report.error
    assert report.survived, "a meaningless assertion cannot notice a changed value"
    # Which signal fires depends on whether the sampled mutations crash or merely change a
    # value, so the invariant is over the union, not either one.
    flagged = set(report.weak_oracles)
    assert flagged >= {"S-00000001", "S-00000002"}, report.to_dict()


@pytest.mark.slow
def test_the_campaign_refuses_to_run_without_a_green_baseline(tmp_path):
    root = _workspace(tmp_path, src='def double(n):\n    return "wrong"\n')
    report = run_campaign(root, Config.load(root), engine="python", budget=2)
    assert report.error
    assert report.mutants == []


@pytest.mark.slow
def test_the_ledger_records_oracle_strength(tmp_path):
    root = _workspace(tmp_path)
    led = Ledger()
    led.sync_spec(load_specs(root / "spec"))
    report = run_campaign(root, Config.load(root), engine="python", budget=4, seed=1)
    led.apply_mutation(report)
    assert led.mutation["score"] == report.score
    for rid in report.baseline_green:
        assert led.entries[rid].mutants_killed is not None
        assert led.entries[rid].blind_spots is not None


def test_an_unknown_engine_is_rejected(tmp_path):
    root = _workspace(tmp_path)
    with pytest.raises(ValueError, match="unknown mutation engine"):
        run_campaign(root, Config.load(root), engine="quantum")
