"""Reading test results back from any Cucumber-family runner.

Binding is by the `@rid:` tag. A tag written into the Gherkin survives into every report format
in the Cucumber family, which is what lets one ledger serve pytest-bdd, cucumber-js,
cucumber-jvm, godog, Reqnroll and the rest without a per-language shim.

A scenario is `passed` only if every one of its steps passed. Skipped, pending, undefined and
ambiguous all count as not-passed: a step nobody implemented is not evidence of success, and a
suite with undefined steps must never read as green.
"""
from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from .spec import RID_RE

PASSING = {"passed"}
# Everything else -- failed, skipped, pending, undefined, ambiguous, unknown -- is not a pass.


def _rid_from_tags(tags: list) -> str | None:
    for t in tags or []:
        name = t.get("name", "") if isinstance(t, dict) else str(t)
        if (m := RID_RE.fullmatch(name.strip())):
            return m.group(1)
    return None


def _merge(out: dict[str, dict[str, Any]], rid: str, outcome: str,
           detail: str, nodeid: str) -> None:
    """An outline produces one result per example row; the scenario is green only if all pass."""
    prev = out.get(rid)
    if prev is None or (prev["outcome"] == "passed" and outcome == "failed"):
        out[rid] = {"outcome": outcome, "detail": detail, "nodeid": nodeid}


def parse_cucumber_json(text: str) -> dict[str, dict[str, Any]]:
    """The classic Cucumber JSON report: a list of features, each with `elements`."""
    data = json.loads(text)
    results: dict[str, dict[str, Any]] = {}
    for feature in data if isinstance(data, list) else [data]:
        for el in feature.get("elements", []) or []:
            if el.get("type") not in (None, "scenario", "scenario_outline"):
                continue
            rid = _rid_from_tags(el.get("tags", []))
            if not rid:
                continue
            failures, statuses = [], []
            for step in el.get("steps", []) or []:
                res = step.get("result", {}) or {}
                status = (res.get("status") or "unknown").lower()
                statuses.append(status)
                if status not in PASSING:
                    kw = (step.get("keyword") or "").strip()
                    msg = res.get("error_message") or f"step status: {status}"
                    failures.append(f"{kw} {step.get('name', '')}\n    {msg}".strip())
            outcome = "passed" if statuses and all(s in PASSING for s in statuses) else "failed"
            if not statuses:
                failures.append("scenario reported no steps")
            nodeid = f"{feature.get('uri', '?')}::{el.get('name', '?')}"
            _merge(results, rid, outcome, "\n".join(failures)[:4000], nodeid)
    return results


def parse_cucumber_messages(text: str) -> dict[str, dict[str, Any]]:
    """Cucumber Messages NDJSON (`--format message`).

    Walks the envelope stream: a pickle carries the scenario's tags (and therefore its rid), a
    testCase points at its pickle, a testCaseStarted points at its testCase, and each
    testStepFinished carries a status.
    """
    pickle_rid: dict[str, str] = {}
    pickle_name: dict[str, str] = {}
    pickle_uri: dict[str, str] = {}
    case_pickle: dict[str, str] = {}
    started_case: dict[str, str] = {}
    step_status: dict[str, list[tuple[str, str]]] = {}

    for line in text.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            env = json.loads(line)
        except json.JSONDecodeError:
            continue
        if "pickle" in env:
            pk = env["pickle"]
            rid = _rid_from_tags(pk.get("tags", []))
            if rid:
                pickle_rid[pk["id"]] = rid
            pickle_name[pk["id"]] = pk.get("name", "?")
            pickle_uri[pk["id"]] = pk.get("uri", "?")
        elif "testCase" in env:
            tc = env["testCase"]
            case_pickle[tc["id"]] = tc.get("pickleId", "")
        elif "testCaseStarted" in env:
            tcs = env["testCaseStarted"]
            started_case[tcs["id"]] = tcs.get("testCaseId", "")
        elif "testStepFinished" in env:
            tsf = env["testStepFinished"]
            res = tsf.get("testStepResult", {}) or {}
            step_status.setdefault(tsf.get("testCaseStartedId", ""), []).append(
                ((res.get("status") or "UNKNOWN").lower(), res.get("message") or ""))

    results: dict[str, dict[str, Any]] = {}
    for started_id, statuses in step_status.items():
        pid = case_pickle.get(started_case.get(started_id, ""), "")
        rid = pickle_rid.get(pid)
        if not rid:
            continue
        outcome = "passed" if statuses and all(s in PASSING for s, _ in statuses) else "failed"
        detail = "\n".join(f"[{s}] {m}".strip() for s, m in statuses if s not in PASSING)
        nodeid = f"{pickle_uri.get(pid, '?')}::{pickle_name.get(pid, '?')}"
        _merge(results, rid, outcome, detail[:4000] or f"statuses: {statuses}", nodeid)
    return results


def parse_ratchet(text: str) -> dict[str, dict[str, Any]]:
    """ratchet's own pytest plugin report."""
    data = json.loads(text)
    return data.get("results", {})


PARSERS = {
    "ratchet": parse_ratchet,
    "cucumber-json": parse_cucumber_json,
    "cucumber-messages": parse_cucumber_messages,
}


def read_report(path: Path, fmt: str) -> dict[str, dict[str, Any]]:
    if fmt not in PARSERS:
        raise ValueError(f"unknown report format {fmt!r}")
    if not Path(path).exists():
        return {}
    text = Path(path).read_text(encoding="utf-8", errors="replace")
    if not text.strip():
        return {}
    return PARSERS[fmt](text)


def unbound_scenarios(report_results: dict, known_rids: set[str]) -> list[str]:
    return sorted(rid for rid in report_results if rid not in known_rids)
