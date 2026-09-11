"""pytest plugin that maps pytest-bdd scenario outcomes back to ratchet rids.

Binding is done on (feature file basename, scenario name) rather than on Gherkin tags, so it
does not depend on how the runner turns tags into markers. Scenario Outlines produce one test
per example row; the scenario is green only when every row passes.
"""
from __future__ import annotations

import json
from pathlib import Path

_STATE: dict = {"map": {}, "node_to_key": {}, "outcomes": {}, "report": None}


def pytest_addoption(parser):
    g = parser.getgroup("ratchet")
    g.addoption("--ratchet-spec", default=None, help="ratchet spec/ directory")
    g.addoption("--ratchet-report", default=None, help="where to write the ratchet run report")


def pytest_configure(config):
    spec = config.getoption("--ratchet-spec")
    report = config.getoption("--ratchet-report")
    _STATE["report"] = Path(report) if report else None
    _STATE["map"] = {}
    _STATE["node_to_key"] = {}
    _STATE["outcomes"] = {}
    if spec:
        from ratchet.spec import load_specs
        spec_dir = Path(spec)
        for f in load_specs(spec_dir):
            for s in f.scenarios:
                if s.rid:
                    _STATE["map"][(Path(f.file).name, s.name)] = s.rid


def pytest_bdd_before_scenario(request, feature, scenario):
    key = (Path(feature.filename).name, scenario.name)
    _STATE["node_to_key"][request.node.nodeid] = key


def pytest_runtest_logreport(report):
    if report.when != "call" and not (report.when == "setup" and report.failed):
        return
    key = _STATE["node_to_key"].get(report.nodeid)
    if key is None:
        return
    rid = _STATE["map"].get(key)
    if rid is None:
        return
    detail = ""
    if report.failed:
        detail = str(report.longrepr)
    prev = _STATE["outcomes"].get(rid)
    outcome = "passed" if report.passed else "failed"
    # An outline is green only if every example row passes.
    if prev is None or (prev["outcome"] == "passed" and outcome == "failed"):
        _STATE["outcomes"][rid] = {"outcome": outcome, "detail": detail,
                                   "nodeid": report.nodeid}


def pytest_sessionfinish(session, exitstatus):
    path = _STATE["report"]
    if not path:
        return
    bound = set(_STATE["outcomes"])
    unbound_nodes = [n for n, k in _STATE["node_to_key"].items()
                     if _STATE["map"].get(k) is None]
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps({
        "results": _STATE["outcomes"],
        "bound": sorted(bound),
        "unbound_tests": sorted(unbound_nodes),
        "exitstatus": int(exitstatus),
    }, indent=2), encoding="utf-8")
