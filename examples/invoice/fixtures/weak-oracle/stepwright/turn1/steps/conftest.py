# workspace-local shalt reporter — not part of the shalt binary
import json, os, re, sys
from pathlib import Path

RID_RE = re.compile(r"^rid:(S-[0-9a-f]{8})$")
_STATE = {"node_to_rid": {}, "outcomes": {}, "report": None}

src = Path(__file__).resolve().parents[1] / "src"
if str(src) not in sys.path:
    sys.path.insert(0, str(src))

def pytest_configure(config):
    _STATE["report"] = os.environ.get("SHALT_REPORT")

def pytest_bdd_before_scenario(request, feature, scenario):
    for tag in getattr(scenario, "tags", None) or []:
        name = str(tag).strip().lstrip("@")
        m = RID_RE.fullmatch(name)
        if m:
            _STATE["node_to_rid"][request.node.nodeid] = m.group(1)
            return

def pytest_runtest_logreport(report):
    if report.when != "call" and not (report.when == "setup" and report.failed):
        return
    rid = _STATE["node_to_rid"].get(report.nodeid)
    if not rid:
        return
    outcome = "passed" if report.passed else "failed"
    detail = str(report.longrepr) if report.failed else ""
    prev = _STATE["outcomes"].get(rid)
    if prev is None or (prev["outcome"] == "passed" and outcome == "failed"):
        _STATE["outcomes"][rid] = {"outcome": outcome, "detail": detail, "nodeid": report.nodeid}

def pytest_sessionfinish(session, exitstatus):
    path = _STATE["report"]
    if not path:
        return
    p = Path(path)
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps({
        "results": _STATE["outcomes"],
        "exitstatus": int(exitstatus),
    }, indent=2), encoding="utf-8")
