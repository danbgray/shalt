from __future__ import annotations

import argparse
import json
import shutil
import sys
import time
from pathlib import Path

from .backends import make_backend
from .integrity import IntegrityViolation, audit
from .ledger import GREEN, PENDING, RED, STALE, ORPHAN, Ledger
from .roles import run_role
from .runner import failure_digest, harness_report, run_suite
from .spec import holdout_rids, load_specs, stamp_rids

LEDGER_PATH = ".ratchet/ledger.json"

C = {"green": "\033[32m", "red": "\033[31m", "yellow": "\033[33m",
     "dim": "\033[2m", "bold": "\033[1m", "reset": "\033[0m"}


def _c(s: str, k: str) -> str:
    return f"{C[k]}{s}{C['reset']}" if sys.stdout.isatty() else s


def _root(args) -> Path:
    return Path(args.root).resolve()


def _ledger(root: Path) -> Ledger:
    return Ledger.load(root / LEDGER_PATH)


def _sync(root: Path) -> tuple[Ledger, list]:
    features = load_specs(root / "spec")
    led = _ledger(root)
    led.sync_spec(features)
    led.save(root / LEDGER_PATH)
    return led, features


# ----------------------------------------------------------------- commands
def cmd_init(args) -> int:
    root = _root(args)
    for d in ("spec", "steps", "contract", "src", ".ratchet"):
        (root / d).mkdir(parents=True, exist_ok=True)
    (root / "steps" / "conftest.py").write_text(
        "import sys, pathlib\n"
        "sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / 'src'))\n",
        encoding="utf-8")
    (root / ".ratchet" / ".gitignore").write_text("stage/\nbackup/\nlast_run.json\n",
                                                  encoding="utf-8")
    Ledger().save(root / LEDGER_PATH)
    print(f"initialised ratchet workspace at {root}")
    print("  spec/      Gherkin, written by the author role, approved by you")
    print("  steps/     step definitions, written by the stepwright role only")
    print("  contract/  the API surface the stepwright declares it will call")
    print("  src/       implementation, written by the implementer role only")
    return 0


def cmd_author(args) -> int:
    root = _root(args)
    backend = make_backend(args.backend, Path(args.fixtures) if args.fixtures else None)
    prompt = (f"Translate this request into Gherkin feature files under spec/.\n\n"
              f"REQUEST:\n{args.request}\n\n"
              f"Write one .feature file per coherent capability. Cover the happy path, the "
              f"edge cases a reviewer would ask about, and the failure modes. Use concrete "
              f"example values.")
    res = run_role(root, "author", prompt, backend)
    print(f"author wrote {len(res.wrote)} file(s):")
    for w in res.wrote:
        print(f"  {w}")
    led, features = _sync(root)
    n = sum(len(f.scenarios) for f in features)
    print(f"\n{n} scenario(s) drafted. Review spec/ then run: ratchet approve")
    return 0


def cmd_approve(args) -> int:
    root = _root(args)
    features = load_specs(root / "spec")
    if not features:
        print("no feature files in spec/", file=sys.stderr)
        return 1
    print("Scenarios awaiting sign-off:\n")
    for f in features:
        print(_c(f"  {f.file}  ({f.name})", "bold"))
        for s in f.scenarios:
            mark = " [holdout]" if "@holdout" in s.tags else ""
            print(f"    - {s.name}{mark}")
    if not args.yes:
        reply = input("\nApprove this spec as the contract to build against? [y/N] ").strip().lower()
        if reply != "y":
            print("not approved; nothing locked.")
            return 1
    minted = stamp_rids(root / "spec")
    led, features = _sync(root)
    led.spec_lock = {
        "approved_by": args.by,
        "approved_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "scenario_count": sum(len(f.scenarios) for f in features),
        "files": {f.file: len(f.scenarios) for f in features},
    }
    led.save(root / LEDGER_PATH)
    print(f"\napproved by {args.by}; {len(minted)} new scenario id(s) stamped into the spec.")
    print("next: ratchet steps")
    return 0


def cmd_steps(args) -> int:
    root = _root(args)
    led = _ledger(root)
    if not led.spec_lock:
        print("spec is not approved yet -- run `ratchet approve` first", file=sys.stderr)
        return 1
    backend = make_backend(args.backend, Path(args.fixtures) if args.fixtures else None)
    prompt = (
        "Write pytest-bdd step definitions under steps/ for every scenario in spec/.\n\n"
        "Rules:\n"
        "- one module per feature file, named test_<feature>.py\n"
        "- bind scenarios with `scenarios('<relative path to .feature>')`\n"
        "- assert on real observable behaviour; do not weaken assertions\n"
        "- import only from the public API surface you declare\n"
        "- write that surface to contract/interface.md: module names, function signatures, "
        "types, and what each is expected to do\n"
    )
    try:
        res = run_role(root, "stepwright", prompt, backend)
    except IntegrityViolation as e:
        print(_c(f"turn rejected: {e}", "red"), file=sys.stderr)
        return 2
    print(f"stepwright wrote {len(res.wrote)} file(s):")
    for w in res.wrote:
        print(f"  {w}")
    return 0


def cmd_run(args) -> int:
    root = _root(args)
    led, features = _sync(root)
    run = run_suite(root)
    if run.get("harness_error"):
        print(_c("the test harness failed to run -- this is not 'pending' work:", "red"),
              file=sys.stderr)
        print(harness_report(run), file=sys.stderr)
        return 3
    out = led.apply_run(run.get("results", {}), run.get("run_id", "run"),
                        blocked=run.get("collection_error", ""))
    led.save(root / LEDGER_PATH)
    if out["regressions"]:
        print(_c(f"{len(out['regressions'])} REGRESSION(S): previously green, now red", "red"))
        for r in out["regressions"]:
            print(f"  {r['rid']}  {r['name']}")
    if run.get("unbound_tests"):
        print(_c(f"{len(run['unbound_tests'])} test(s) not bound to any scenario", "yellow"))
    _print_status(led, root)
    return 0


def cmd_build(args) -> int:
    root = _root(args)
    led, features = _sync(root)
    if not led.spec_lock:
        print("spec is not approved yet -- run `ratchet approve` first", file=sys.stderr)
        return 1
    backend = make_backend(args.backend, Path(args.fixtures) if args.fixtures else None)
    held = holdout_rids(features)
    visible = [rid for rid in led.entries if rid not in held]
    if held:
        print(_c(f"{len(held)} scenario(s) held out from the implementer", "dim"))

    for turn in range(1, args.max_turns + 1):
        run = run_suite(root)
        if run.get("harness_error"):
            print(_c(f"\nturn {turn}: the test harness failed to run", "red"))
            print(harness_report(run))
            return 3
        led.apply_run(run.get("results", {}), run.get("run_id", f"turn{turn}"),
                      blocked=run.get("collection_error", ""))
        led.save(root / LEDGER_PATH)
        red_visible = [r for r in visible if led.entries[r].status in (RED, PENDING, STALE)]
        print(f"\nturn {turn}: {len(visible) - len(red_visible)}/{len(visible)} visible green")
        if not red_visible:
            break
        digest = run.get("collection_error") or failure_digest(run)
        wanted = "\n".join(f"- {led.entries[r].name} ({r}): {led.entries[r].status}"
                           for r in red_visible[:12])
        prompt = (
            "Make the failing scenarios pass by implementing the behaviour under src/.\n\n"
            "You cannot see or edit the step definitions, and you cannot edit the spec.\n"
            "Implement against contract/interface.md.\n\n"
            f"STILL FAILING:\n{wanted}\n\nTEST OUTPUT:\n{digest}\n"
        )
        try:
            res = run_role(root, "implementer", prompt, backend, hide_holdouts=True)
        except IntegrityViolation as e:
            print(_c(f"\nturn {turn} REJECTED -- {e}", "red"))
            print(_c("  nothing from this turn was kept; the spec and tests are untouched.", "dim"))
            if args.strict:
                return 2
            continue
        print(_c(f"  implementer wrote: {', '.join(res.wrote) or '(nothing)'}", "dim"))
    else:
        print(_c(f"\nstopped after {args.max_turns} turns", "yellow"))

    # final verification, holdouts included
    run = run_suite(root)
    if run.get("harness_error"):
        print(_c("final verification could not run", "red"))
        print(harness_report(run))
        return 3
    led.apply_run(run.get("results", {}), "verify",
                  blocked=run.get("collection_error", ""))
    led.save(root / LEDGER_PATH)
    held_red = [r for r in held if led.entries.get(r) and led.entries[r].status != GREEN]
    vis_red = [r for r in visible if led.entries[r].status != GREEN]
    if held and held_red and not vis_red:
        print(_c("\nOVERFIT: every visible scenario is green but held-out scenarios fail.", "red"))
        for r in held_red:
            print(f"  {r}  {led.entries[r].name}")
        print(_c("  the implementation satisfies the examples it saw, not the behaviour.", "dim"))
    _print_status(led, root)
    return 0


def cmd_status(args) -> int:
    root = _root(args)
    led, _ = _sync(root)
    _print_status(led, root)
    return 0


def cmd_verify(args) -> int:
    root = _root(args)
    led, features = _sync(root)
    problems = audit(root, led, features)
    if not problems:
        print(_c("integrity ok", "green"))
        print(f"  spec approved by {led.spec_lock.get('approved_by')} "
              f"at {led.spec_lock.get('approved_at')}")
        return 0
    print(_c(f"{len(problems)} integrity problem(s):", "red"))
    for p in problems:
        print(f"  - {p}")
    return 1


def _print_status(led: Ledger, root: Path) -> None:
    s = led.summary()
    print()
    order = [(GREEN, "green"), (RED, "red"), (STALE, "yellow"),
             (PENDING, "dim"), (ORPHAN, "yellow")]
    for f in sorted({e.feature_file for e in led.entries.values()}):
        print(_c(f"{f}", "bold"))
        for e in sorted(led.entries.values(), key=lambda x: x.name):
            if e.feature_file != f:
                continue
            colour = dict(order).get(e.status, "dim")
            glyph = {"green": "+", "red": "x", "stale": "~",
                     "pending": ".", "orphan": "?"}[e.status]
            tag = " [holdout]" if "@holdout" in e.tags else ""
            print(f"  {_c(glyph, colour)} {e.status:<8} {e.name}{tag}  {_c(e.rid, 'dim')}")
    live = s["total"] - s[ORPHAN]
    bar_w = 28
    filled = int(bar_w * s[GREEN] / live) if live else 0
    bar = _c("#" * filled, "green") + _c("-" * (bar_w - filled), "dim")
    print(f"\n[{bar}] {s['completion_pct']}%  "
          f"{s[GREEN]} green / {s[RED]} red / {s[STALE]} stale / {s[PENDING]} pending")
    if led.regressions:
        print(_c(f"{len(led.regressions)} regression(s) recorded in the ledger", "yellow"))
    print(_c(f"ledger: {root / LEDGER_PATH}", "dim"))


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(prog="ratchet", description="BDD for agentic workflows")
    p.add_argument("--root", default=".", help="workspace root")
    p.add_argument("--backend", default="fixture", choices=["fixture", "claude-cli"])
    p.add_argument("--fixtures", default=None, help="fixture directory (fixture backend)")
    sub = p.add_subparsers(dest="cmd", required=True)

    sub.add_parser("init").set_defaults(fn=cmd_init)
    a = sub.add_parser("author"); a.add_argument("request"); a.set_defaults(fn=cmd_author)
    ap = sub.add_parser("approve")
    ap.add_argument("--yes", action="store_true"); ap.add_argument("--by", default="unknown")
    ap.set_defaults(fn=cmd_approve)
    sub.add_parser("steps").set_defaults(fn=cmd_steps)
    b = sub.add_parser("build")
    b.add_argument("--max-turns", type=int, default=6)
    b.add_argument("--strict", action="store_true", help="abort on an integrity violation")
    b.set_defaults(fn=cmd_build)
    sub.add_parser("run").set_defaults(fn=cmd_run)
    sub.add_parser("status").set_defaults(fn=cmd_status)
    sub.add_parser("verify").set_defaults(fn=cmd_verify)

    args = p.parse_args(argv)
    return args.fn(args)


if __name__ == "__main__":
    raise SystemExit(main())
