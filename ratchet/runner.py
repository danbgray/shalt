"""Drive the test suite and fold results back into the ledger."""
from __future__ import annotations

import json
import os
import subprocess
import sys
import time
from pathlib import Path


def run_suite(root: Path, extra: list[str] | None = None) -> dict:
    report = root / ".ratchet" / "last_run.json"
    if report.exists():
        report.unlink()
    cmd = [sys.executable, "-m", "pytest", "-q", "--no-header",
           "-p", "ratchet.pytest_plugin",
           f"--ratchet-spec={root / 'spec'}",
           f"--ratchet-report={report}",
           str(root / "steps")]
    env = dict(os.environ)
    env["PYTHONPATH"] = os.pathsep.join(
        [str(root), str(root / "src"), env.get("PYTHONPATH", "")]).strip(os.pathsep)
    proc = subprocess.run(cmd, cwd=root, capture_output=True, text=True, env=env, timeout=600)
    data = {"results": {}, "unbound_tests": [], "exitstatus": proc.returncode}
    if report.exists():
        data = json.loads(report.read_text(encoding="utf-8"))
    data["stdout"] = proc.stdout[-8000:]
    data["stderr"] = proc.stderr[-4000:]
    data["run_id"] = time.strftime("run-%Y%m%d-%H%M%S", time.gmtime())
    # 3 = pytest internal error, 4 = bad usage. Those are ours to fix and are worth shouting
    # about. Exit code 2 (interrupted) is usually a *collection* error -- typically the
    # implementation does not exist yet -- which is an ordinary red state for the implementer
    # to work on, not a broken harness.
    data["harness_error"] = proc.returncode in (3, 4)
    data["collection_error"] = ""
    if proc.returncode == 2 or (proc.returncode == 5 and not data["results"]):
        data["collection_error"] = (proc.stdout or proc.stderr)[-3000:]
    return data


def harness_report(run: dict) -> str:
    return (run.get("stderr", "") or "")[-1800:] + "\n" + (run.get("stdout", "") or "")[-1800:]


def failure_digest(run: dict, limit: int = 3) -> str:
    """Compact failing-test context to hand to the implementer."""
    parts = []
    for rid, r in run.get("results", {}).items():
        if r["outcome"] == "failed":
            parts.append(f"--- {rid} ({r['nodeid']}) ---\n{r['detail'][:1500]}")
        if len(parts) >= limit:
            break
    if not parts:
        parts.append(run.get("stdout", "")[-2000:])
    return "\n\n".join(parts)
