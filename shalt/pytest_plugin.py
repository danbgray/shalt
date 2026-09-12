"""pytest plugin that maps pytest-bdd scenario outcomes back to shalt rids.

Binding is done on (feature file basename, scenario name) rather than on Gherkin tags, so it
does not depend on how the runner turns tags into markers. Scenario Outlines produce one test
per example row; the scenario is green only when every row passes.
"""
from __future__ import annotations

import json
from pathlib import Path

from shalt.spec import RID_RE

_STATE: dict = {"map": {}, "node_to_key": {}, "node_to_rid": {}, "outcomes": {},
                "report": None, "spec_dir": None}


def pytest_addoption(parser):
    g = parser.getgroup("shalt")
    g.addoption("--shalt-spec", default=None, help="shalt spec/ directory")
    g.addoption("--shalt-report", default=None, help="where to write the shalt run report")


def pytest_configure(config):
    spec = config.getoption("--shalt-spec")
    report = config.getoption("--shalt-report")
    _STATE["report"] = Path(report) if report else None
    _STATE["map"] = {}
    _STATE["node_to_key"] = {}
    _STATE["node_to_rid"] = {}
    _STATE["outcomes"] = {}
    if spec:
        from shalt.spec import load_specs
        spec_dir = Path(spec).resolve()
        _STATE["spec_dir"] = spec_dir
        for f in load_specs(spec_dir, strict=False):
            for s in f.scenarios:
                if s.rid:
                    # keyed on the path relative to spec/, so two features with the same
                    # basename in different directories do not collide
                    _STATE["map"][(f.file.replace("\\", "/"), s.name)] = s.rid


def _rid_from_scenario(scenario):
    """pytest-bdd exposes Gherkin tags as a set of names without the leading '@'."""
    for tag in getattr(scenario, "tags", None) or []:
        name = str(tag).strip().lstrip("@")
        if name.startswith("rid:") and RID_RE.fullmatch("@" + name):
            return name[4:]
    return None


def pytest_bdd_before_scenario(request, feature, scenario):
    rid = _rid_from_scenario(scenario)
    if rid:
        # binding by tag is exact and needs no filename matching at all
        _STATE["node_to_rid"][request.node.nodeid] = rid
        return
    spec_dir = _STATE.get("spec_dir")
    fname = Path(feature.filename).resolve()
    try:
        rel = str(fname.relative_to(spec_dir)).replace("\\", "/") if spec_dir else fname.name
    except ValueError:
        rel = fname.name
    _STATE["node_to_key"][request.node.nodeid] = (rel, scenario.name)


def pytest_runtest_logreport(report):
    if report.when != "call" and not (report.when == "setup" and report.failed):
        return
    rid = _STATE["node_to_rid"].get(report.nodeid)
    if rid is None:
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
                     if _STATE["map"].get(k) is None and n not in _STATE["node_to_rid"]]
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps({
        "results": _STATE["outcomes"],
        "bound": sorted(bound),
        "unbound_tests": sorted(unbound_nodes),
        "exitstatus": int(exitstatus),
    }, indent=2), encoding="utf-8")
