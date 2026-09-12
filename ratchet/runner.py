"""Drive whatever test runner the workspace declares, and fold results back into the ledger."""
from __future__ import annotations

import os
import subprocess
import time
from pathlib import Path

from .config import Config
from .reports import read_report


def run_suite(root: Path, cfg: Config | None = None) -> dict:
    root = Path(root)
    cfg = cfg or Config.load(root)
    report = root / cfg.report
    if report.exists():
        report.unlink()
    report.parent.mkdir(parents=True, exist_ok=True)

    env = dict(os.environ)
    env["PYTHONPATH"] = os.pathsep.join(
        [str(root), str(root / cfg.src), env.get("PYTHONPATH", "")]).strip(os.pathsep)
    env.update(cfg.env)

    started = time.time()
    try:
        if cfg.uses_shell:
            proc = subprocess.run(cfg.command.format(
                spec=str(root / "spec"), steps=str(root / cfg.steps),
                src=str(root / cfg.src), report=str(report), root=str(root)),
                shell=True, cwd=root, capture_output=True, text=True,
                env=env, timeout=cfg.timeout)
        else:
            proc = subprocess.run(cfg.argv(root), cwd=root, capture_output=True,
                                  text=True, env=env, timeout=cfg.timeout)
    except subprocess.TimeoutExpired as e:
        return {"results": {}, "harness_error": True, "collection_error": "",
                "stdout": (e.stdout or b"").decode(errors="replace")[-4000:]
                if isinstance(e.stdout, bytes) else (e.stdout or "")[-4000:],
                "stderr": f"runner timed out after {cfg.timeout}s",
                "run_id": time.strftime("run-%Y%m%d-%H%M%S", time.gmtime()),
                "duration": cfg.timeout}
    except FileNotFoundError as e:
        return {"results": {}, "harness_error": True, "collection_error": "",
                "stdout": "", "stderr": f"runner command not found: {e}. "
                f"Check [runner].command in ratchet.toml.",
                "run_id": "run-failed", "duration": 0}

    results = read_report(report, cfg.format)
    data = {
        "results": results,
        "stdout": proc.stdout[-8000:],
        "stderr": proc.stderr[-4000:],
        "run_id": time.strftime("run-%Y%m%d-%H%M%S", time.gmtime()),
        "duration": round(time.time() - started, 2),
        "returncode": proc.returncode,
    }
    # 3 = pytest internal error, 4 = bad usage: ours to fix, worth shouting about. Exit code 2
    # is usually a collection error -- typically the implementation does not exist yet -- which
    # is an ordinary red state for the implementer, not a broken harness.
    data["harness_error"] = proc.returncode in (3, 4)
    data["collection_error"] = ""
    if not results and proc.returncode != 0:
        data["collection_error"] = (proc.stdout or proc.stderr)[-3000:]
    return data


def harness_report(run: dict) -> str:
    return (run.get("stderr", "") or "")[-1800:] + "\n" + (run.get("stdout", "") or "")[-1800:]


def failure_digest(run: dict, allowed: set[str] | None = None, limit: int = 3) -> str:
    """Compact failing-test context to hand to the implementer.

    `allowed` restricts the digest to scenarios the role may see. Without it the assertion text
    of a held-out scenario -- including its expected value -- would go straight to the
    implementer, defeating the point of holding it out.
    """
    parts = []
    for rid, r in run.get("results", {}).items():
        if allowed is not None and rid not in allowed:
            continue
        if r["outcome"] == "failed":
            parts.append(f"--- {rid} ({r['nodeid']}) ---\n{r['detail'][:1500]}")
        if len(parts) >= limit:
            break
    if not parts:
        parts.append("no visible failing scenario detail available")
    return "\n\n".join(parts)
