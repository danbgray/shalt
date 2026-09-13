from __future__ import annotations

import argparse
import json
import shutil
import sys
import time
from pathlib import Path

from .backends import BACKENDS, make_backend
from .config import CONFIG_NAME, PRESETS, Config, write_config
from .narrative import parse_story
from .viz import (STATUS_LABEL, build_tree, actors, write_dashboard, write_mermaid,
                  mermaid_hierarchy, mermaid_pipeline, mermaid_usecase)
from .integrity import IntegrityViolation, audit
from .mutate import ENGINES, run_campaign
from .ledger import GREEN, PENDING, RED, STALE, ORPHAN, Ledger
from .roles import run_role
from .runner import failure_digest, harness_report, run_suite
from .spec import SpecParseError, holdout_rids, load_specs, stamp_rids
from .term import Term, highlight_gherkin, EDITORS

LEDGER_PATH = ".shalt/ledger.json"

# legacy names kept so call sites read the same; the palette now matches the dashboard
_NAMES = {"green": "ok", "red": "bad", "yellow": "warn", "dim": "muted",
          "bold": "bold", "accent": "accent", "orphan": "orphan"}
T = Term()


def _c(s: str, k: str) -> str:
    return T.s(s, _NAMES.get(k, k))


def _scenario_link(root: Path, entry, label: str) -> str:
    """A scenario's name, clickable through to its line in the feature file."""
    if not entry.feature_file:
        return label
    return T.path(root / "spec" / entry.feature_file, entry.line or None, label=label,
                  fallback="label")


def _root(args) -> Path:
    return Path(args.root).resolve()


def _backend(args):
    return make_backend(args.backend,
                        Path(args.fixtures) if args.fixtures else None,
                        model=args.model, base_url=args.base_url)


def _ledger(root: Path) -> Ledger:
    return Ledger.load(root / LEDGER_PATH)


def _sync(root: Path) -> tuple[Ledger, list]:
    try:
        features = load_specs(root / "spec")
    except SpecParseError as e:
        print(_c("spec does not parse:", "red"), file=sys.stderr)
        for f, msg in e.errors.items():
            print(f"  {f}: {msg}", file=sys.stderr)
        raise SystemExit(4)
    led = _ledger(root)
    led.sync_spec(features)
    led.save(root / LEDGER_PATH)
    return led, features


# ----------------------------------------------------------------- commands
def cmd_init(args) -> int:
    root = _root(args)
    root.mkdir(parents=True, exist_ok=True)
    preset = write_config(root, args.stack, name=args.name or root.name)
    cfg = Config.load(root)
    for d in ("spec", "contract", ".shalt", cfg.steps, cfg.src):
        (root / d).mkdir(parents=True, exist_ok=True)
    if args.stack == "python":
        (root / cfg.steps / "conftest.py").write_text(
            "import sys, pathlib\n"
            f"sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] "
            f"/ '{cfg.src}'))\n", encoding="utf-8")
    (root / ".shalt" / ".gitignore").write_text(
        "stage/\nbackup/\nlast_run.json\nmessages.ndjson\ncucumber.json\n",
        encoding="utf-8")
    Ledger().save(root / LEDGER_PATH)
    print(f"initialised shalt workspace at {root}  ({preset.label})")
    print(f"  spec/        Gherkin + user stories, written by the author, approved by you")
    print(f"  {cfg.steps + '/':<12} step definitions, written by the stepwright only")
    print(f"  contract/    the API surface the stepwright declares it will call")
    print(f"  {cfg.src + '/':<12} implementation, written by the implementer only")
    print(f"  {CONFIG_NAME}  runner command and report format — edit to suit your toolchain")
    if preset.note:
        print(f"\n  {preset.note}")
    return 0


def cmd_author(args) -> int:
    root = _root(args)
    backend = _backend(args)
    prompt = (f"Translate this request into Gherkin feature files under spec/.\n\n"
              f"REQUEST:\n{args.request}\n\n"
              f"Write one .feature file per coherent capability. Cover the happy path, the "
              f"edge cases a reviewer would ask about, and the failure modes. Use concrete "
              f"example values.")
    try:
        res = run_role(root, "author", prompt, backend)
    except IntegrityViolation as e:
        print(_c(f"turn rejected: {e}", "red"), file=sys.stderr)
        return 2
    print(f"author wrote {len(res.wrote)} file(s):")
    for w in res.wrote:
        print(f"  {w}")
    led, features = _sync(root)
    n = sum(len(f.scenarios) for f in features)
    print(f"\n{n} scenario(s) drafted. Review spec/ then run: shalt approve")
    return 0


def cmd_approve(args) -> int:
    root = _root(args)
    features = load_specs(root / "spec")
    if not features:
        print("no feature files in spec/", file=sys.stderr)
        return 1
    if args.quiet:
        print("Scenarios awaiting sign-off:\n")
        for f in features:
            print(_c(f"  {f.file}  ({f.name})", "bold"))
            for s in f.scenarios:
                mark = " [holdout]" if "@holdout" in s.tags else ""
                print(f"    - {s.name}{mark}")
    else:
        # This is the review gate, and the only one. Show the actual text being signed off.
        for f in features:
            path = root / "spec" / f.file
            print("\n" + T.path(path, label=f.file, style="bold") + "  "
                  + _c(f"({len(f.scenarios)} scenarios)", "dim"))
            print(T.rule())
            print(highlight_gherkin(path.read_text(encoding="utf-8"), T, number_from=1))
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
        # the content of what was signed off, so a later edit is detectable rather than
        # merely counted
        "scenario_hashes": {s.rid: s.spec_hash(f.background)
                            for f in features for s in f.scenarios if s.rid},
    }
    led.save(root / LEDGER_PATH)
    print(f"\napproved by {args.by}; {len(minted)} new scenario id(s) stamped into the spec.")
    print("next: shalt steps")
    return 0


def cmd_steps(args) -> int:
    root = _root(args)
    led = _ledger(root)
    if not led.spec_lock:
        print("spec is not approved yet -- run `shalt approve` first", file=sys.stderr)
        return 1
    backend = _backend(args)
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
    run = run_suite(root, Config.load(root))
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
        print("spec is not approved yet -- run `shalt approve` first", file=sys.stderr)
        return 1
    cfg = Config.load(root)
    backend = _backend(args)
    held = holdout_rids(features)
    live = [rid for rid, e in led.entries.items() if e.status != ORPHAN]
    visible = [rid for rid in live if rid not in held]
    if held:
        print(_c(f"{len(held)} scenario(s) held out from the implementer", "dim"))

    for turn in range(1, args.max_turns + 1):
        run = run_suite(root, cfg)
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
        # never show the implementer a held-out scenario's failure detail
        digest = run.get("collection_error") or failure_digest(run, allowed=set(visible))
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
    run = run_suite(root, cfg)
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


GLYPH = {"green": "+", "red": "x", "stale": "~", "pending": ".", "orphan": "?"}
COLOUR = {"green": "green", "red": "red", "stale": "yellow",
          "pending": "dim", "orphan": "yellow"}


def cmd_tree(args) -> int:
    """The breakdown as a tree: epic -> story -> scenario."""
    root = _root(args)
    led, features = _sync(root)
    tree = build_tree(list(led.entries.values()))
    if not tree:
        print("no scenarios yet — run `shalt author \"<what you want>\"`")
        return 0
    story_of = {f.file: f for f in features}
    for epic in tree:
        n = sum(len(st.children) for st in epic.children)
        g = sum(1 for st in epic.children for c in st.children if c.status == "green")
        print(f"\n{_c('EPIC', 'dim')} {_c(epic.label.upper(), 'bold')}  "
              f"{_c(f'{g}/{n} upheld', COLOUR[epic.status])}")
        for si, story in enumerate(epic.children):
            last_story = si == len(epic.children) - 1
            sbranch = "└──" if last_story else "├──"
            spipe = "   " if last_story else "│  "
            print(f" {sbranch} {_c('STORY', 'dim')} {story.label}")
            first = led.entries[story.children[0].key]
            st = parse_story("")
            narrative = ""
            if first.actor and first.capability:
                narrative = f"As a {first.actor}, I want {first.capability}"
                if first.benefit:
                    narrative += f", so that {first.benefit}"
            print(f" {spipe}      {_c(narrative or '(no user story on this feature)', 'dim')}")
            for ci, sc in enumerate(story.children):
                last = ci == len(story.children) - 1
                branch = "└──" if last else "├──"
                tag = " [holdout]" if "@holdout" in (led.entries[sc.key].tags or []) else ""
                entry = led.entries[sc.key]
                name = _scenario_link(root, entry, sc.label)
                print(f" {spipe} {branch} {_c(GLYPH[sc.status], COLOUR[sc.status])} "
                      f"{STATUS_LABEL[sc.status]:<9} {name}{tag} {_c(sc.key, 'dim')}")
    _print_status(led, root)
    return 0


def cmd_stories(args) -> int:
    """Who wants what, from the user-story narratives."""
    root = _root(args)
    led, features = _sync(root)
    acts = actors(list(led.entries.values()))
    if not acts:
        print("no user stories found. Add a narrative block under Feature::\n")
        print("  Feature: Invoice totals\n")
        print("    As a billing clerk")
        print("    I want invoice totals computed exactly")
        print("    So that customers are never billed the wrong amount")
        return 1
    for actor, items in acts.items():
        print(f"\n{_c(actor, 'bold')}")
        for it in items:
            print(f"  wants  {it['capability']}")
            if it["benefit"]:
                print(f"  {_c('so that ' + it['benefit'], 'dim')}")
    missing = [f.file for f in features if not f.story.complete]
    if missing:
        print(_c(f"\n{len(missing)} feature(s) without a complete user story:", "yellow"))
        for m in missing:
            print(f"  {m}")
    return 0


def cmd_diagrams(args) -> int:
    root = _root(args)
    led, _ = _sync(root)
    entries = list(led.entries.values())
    if args.stdout:
        which = {"use-cases": mermaid_usecase(entries),
                 "breakdown": mermaid_hierarchy(entries),
                 "pipeline": mermaid_pipeline()}
        print(which[args.diagram] if args.diagram else which["breakdown"], end="")
        return 0
    written = write_mermaid(root, entries)
    print(f"wrote {len(written)} file(s):")
    for w in written:
        print("  " + T.path(w, label=str(w.relative_to(root))))
    print(_c("\n  .mmd renders in GitHub, pull requests and most editors; the .md wrappers "
             "render inline.", "dim"))
    return 0


def cmd_dashboard(args) -> int:
    root = _root(args)
    led, _ = _sync(root)
    cfg = Config.load(root)
    out = write_dashboard(root, led, project=cfg.name or root.name,
                          stack=PRESETS.get(cfg.stack).label if cfg.stack in PRESETS else "")
    s = led.summary()
    print("wrote " + T.path(out, label=str(out.relative_to(root)))
          + f"  ({s['green']} upheld / {s['total']} scenarios, {s['completion_pct']}%)")
    print(_c("  one self-contained file: no network, no build step.", "dim"))
    return 0


def cmd_mutate(args) -> int:
    """Mutation-test the oracle: break the implementation, see whether the scenarios notice.

    The guards stop an implementer tampering with the tests, and holdouts catch it overfitting.
    Neither checks the stepwright. This does: a scenario that stays green while the behaviour it
    claims to verify is broken is not testing that behaviour.
    """
    root = _root(args)
    led, features = _sync(root)
    cfg = Config.load(root)
    print(f"mutating {cfg.src}/ — engine {args.engine}, budget {args.budget}, "
          f"seed {args.seed}")
    print(_c("  each mutant needs a full suite run; this takes a while.", "dim"))

    def progress(i, total, m):
        mark = {"killed": _c("killed", "green"), "survived": _c("SURVIVED", "red"),
                "invalid": _c("invalid", "dim")}[m.status]
        print(f"  [{i:>3}/{total}] {mark:<18} {m.describe()}")

    report = run_campaign(root, cfg, engine=args.engine, budget=args.budget,
                          seed=args.seed, progress=progress if args.verbose else None)
    if report.error:
        print(_c(f"\ncannot run: {report.error}", "red"), file=sys.stderr)
        return 1

    led.apply_mutation(report)
    led.save(root / LEDGER_PATH)

    print(f"\nmutation score {_c(str(report.score) + '%', 'bold')}  "
          f"({len(report.killed)} killed, {len(report.survived)} survived, "
          f"{len(report.invalid)} invalid)")
    if report.invalid:
        print(_c(f"  {len(report.invalid)} mutant(s) excluded: they broke the suite itself, "
                 f"so they prove nothing about the assertions.", "dim"))

    if report.survived:
        print(_c(f"\n{len(report.survived)} mutation(s) survived — no scenario noticed:",
                 "yellow"))
        for m in report.survived[:15]:
            print("  " + T.path(root / m.path, m.line,
                                label=f"{m.path}:{m.line}", style="warn")
                  + f"  {m.operator}  {m.before} -> {m.after}")
        if len(report.survived) > 15:
            print(_c(f"  ... and {len(report.survived) - 15} more", "dim"))
        print(_c("  Each is a question, not a proven defect: a survivor can mean a weak "
                 "assertion,\n  an unexercised line, or a mutation that changed nothing.",
                 "dim"))

    blind = report.blind_spots
    if blind:
        # grouped by mutant, not by scenario: one broken line missed by five scenarios is one
        # finding about those five, not five findings
        by_mutant: dict[str, list[str]] = {}
        for rid, ms in blind.items():
            for m in ms:
                by_mutant.setdefault(m.describe(), []).append(rid)
        print(_c(f"\nBLIND SPOTS — {len(by_mutant)} mutation(s) ran inside scenarios that "
                 f"stayed green:", "red"))
        for desc, rids in sorted(by_mutant.items(), key=lambda kv: -len(kv[1])):
            mu = next(m for ms in blind.values() for m in ms if m.describe() == desc)
            print("  " + T.path(root / mu.path, mu.line,
                                label=f"{mu.path}:{mu.line}", style="bad")
                  + f"  {mu.operator}  {mu.before} -> {mu.after}")
            for rid in sorted(rids)[:6]:
                e = led.entries.get(rid)
                print(_c(f"      missed by  {e.name if e else rid}", "dim"))
            if len(rids) > 6:
                print(_c(f"      ... and {len(rids) - 6} more", "dim"))
        print(_c("\n  Each of these scenarios provably executes the FILE that was broken and "
                 "stayed green.\n  Attribution is file-granular, so in a single-file project "
                 "this over-reaches: a\n  scenario may run the file without ever reaching the "
                 "mutated line. Read the line first.\n"
                 "  When it does hold, it is one of two defects, needing different fixes:\n"
                 "    - the assertion does not check the value  -> the step definitions are "
                 "weak\n"
                 "    - no scenario exercises the case the mutation changes  -> the spec is "
                 "missing a scenario", "dim"))

    if report.vacuous:
        print(_c(f"\nVACUOUS — {len(report.vacuous)} green scenario(s) detected no mutation "
                 f"at all:", "red"))
        for rid in report.vacuous:
            e = led.entries.get(rid)
            print(f"  {rid}  {e.name if e else '?'}")
        print(_c("  These pass regardless of what the implementation does. Read their step "
                 "definitions.", "dim"))
    elif not blind:
        print(_c("\nevery green scenario detected a mutation, and none stayed green while "
                 "code it runs was broken.", "green"))

    weakest = sorted(((report.kills.get(r, 0), r) for r in report.baseline_green))[:5]
    if weakest and args.verbose:
        print("\nweakest oracles (mutations detected):")
        for n, rid in weakest:
            e = led.entries.get(rid)
            print(f"  {n:>3}  {e.name if e else rid}")
    return 1 if report.weak_oracles else 0


def cmd_show(args) -> int:
    """Print the spec with Gherkin highlighting -- all of it, one feature, or one scenario."""
    root = _root(args)
    features = load_specs(root / "spec")
    if not features:
        print("no feature files in spec/", file=sys.stderr)
        return 1

    target = (args.target or "").strip()
    wanted, mark_rid = features, None
    if target:
        by_rid = {s.rid: (f, s) for f in features for s in f.scenarios if s.rid}
        if target in by_rid:
            f, sc = by_rid[target]
            wanted, mark_rid = [f], sc
        else:
            hit = [f for f in features if target in f.file or target.lower() in f.name.lower()]
            if not hit:
                print(f"no scenario id or feature matching {target!r}", file=sys.stderr)
                print(_c("  try `shalt status` for the ids", "dim"), file=sys.stderr)
                return 1
            wanted = hit

    led = _ledger(root)
    for f in wanted:
        path = root / "spec" / f.file
        text = path.read_text(encoding="utf-8")
        marks: set[int] = set()
        if mark_rid is not None:
            end = mark_rid.line + 1 + len(mark_rid.steps) + len(mark_rid.examples)
            marks = set(range(mark_rid.block_start, end))
        print("\n" + T.path(path, label=f.file, style="bold")
              + _c(f"  {len(f.scenarios)} scenarios", "dim"))
        print(T.rule())
        print(highlight_gherkin(text, T, number_from=1, marks=marks))
        if args.status:
            print()
            for sc in f.scenarios:
                e = led.entries.get(sc.rid)
                if not e:
                    continue
                print(f"  {_c(GLYPH[e.status], COLOUR[e.status])} "
                      f"{STATUS_LABEL[e.status]:<9} {_scenario_link(root, e, e.name)}  "
                      f"{_c(e.rid, 'dim')}")
    return 0


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
            name = _scenario_link(root, e, e.name)
            print(f"  {_c(glyph, colour)} {STATUS_LABEL[e.status]:<9} {name}{tag}  "
                  f"{_c(e.rid, 'dim')}")
    live = s["total"] - s[ORPHAN]
    bar_w = 28
    filled = int(bar_w * s[GREEN] / live) if live else 0
    bar = _c("#" * filled, "green") + _c("-" * (bar_w - filled), "dim")
    print(f"\n[{bar}] {s['completion_pct']}%  "
          f"{_c(str(s[GREEN]) + ' upheld', 'green')} / "
          f"{_c(str(s[RED]) + ' failing', 'red' if s[RED] else 'dim')} / "
          f"{_c(str(s[STALE]) + ' stale', 'yellow' if s[STALE] else 'dim')} / "
          f"{_c(str(s[PENDING]) + ' no test', 'dim')}")
    if s[ORPHAN]:
        print(_c(f"{s[ORPHAN]} scenario(s) removed from the spec are excluded from that "
                 f"figure -- run `shalt verify`", "yellow"))
    if led.regressions:
        print(_c(f"{len(led.regressions)} regression(s) recorded in the ledger", "yellow"))
    print(_c("ledger: ", "dim") + T.path(root / LEDGER_PATH, style="muted"))


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(prog="shalt", description="BDD for agentic workflows")
    p.add_argument("--root", default=".", help="workspace root")
    p.add_argument("--backend", default="fixture", choices=list(BACKENDS))
    p.add_argument("--fixtures", default=None, help="fixture directory (fixture backend)")
    p.add_argument("--model", default=None, help="override the backend's default model")
    p.add_argument("--base-url", default=None, help="override the API base url")
    p.add_argument("--color", default="auto", choices=["auto", "always", "never"],
                   help="colour output (NO_COLOR is always honoured)")
    p.add_argument("--editor", default=None, choices=sorted(EDITORS),
                   help="which editor clickable paths should open (default: $SHALT_EDITOR)")
    sub = p.add_subparsers(dest="cmd", required=True)

    i = sub.add_parser("init")
    i.add_argument("--stack", default="python", choices=list(PRESETS),
                   help="toolchain preset written into shalt.toml")
    i.add_argument("--name", default=None)
    i.set_defaults(fn=cmd_init)
    a = sub.add_parser("author"); a.add_argument("request"); a.set_defaults(fn=cmd_author)
    ap = sub.add_parser("approve")
    ap.add_argument("--yes", action="store_true"); ap.add_argument("--by", default="unknown")
    ap.add_argument("--quiet", action="store_true",
                    help="list scenario names instead of printing the spec")
    ap.set_defaults(fn=cmd_approve)
    sub.add_parser("steps").set_defaults(fn=cmd_steps)
    b = sub.add_parser("build")
    b.add_argument("--max-turns", type=int, default=6)
    b.add_argument("--strict", action="store_true", help="abort on an integrity violation")
    b.set_defaults(fn=cmd_build)
    sub.add_parser("run").set_defaults(fn=cmd_run)
    sub.add_parser("status").set_defaults(fn=cmd_status)
    sub.add_parser("verify").set_defaults(fn=cmd_verify)
    sub.add_parser("tree").set_defaults(fn=cmd_tree)
    sub.add_parser("stories").set_defaults(fn=cmd_stories)
    d = sub.add_parser("diagrams")
    d.add_argument("--stdout", action="store_true", help="print one diagram instead of writing")
    d.add_argument("--diagram", choices=["use-cases", "breakdown", "pipeline"], default=None)
    d.set_defaults(fn=cmd_diagrams)
    sub.add_parser("dashboard").set_defaults(fn=cmd_dashboard)
    sh = sub.add_parser("show")
    sh.add_argument("target", nargs="?", default=None,
                    help="a scenario id, a feature file, or nothing for the whole spec")
    sh.add_argument("--status", action="store_true", help="list each scenario's status after")
    sh.set_defaults(fn=cmd_show)
    m = sub.add_parser("mutate")
    m.add_argument("--engine", default="auto", choices=["auto"] + list(ENGINES),
                   help="auto picks the Python AST engine for a Python stack, else text")
    m.add_argument("--budget", type=int, default=30, help="how many mutants to try")
    m.add_argument("--seed", type=int, default=0, help="sampling seed, for reproducibility")
    m.add_argument("--verbose", action="store_true", help="show each mutant as it runs")
    m.set_defaults(fn=cmd_mutate)

    args = p.parse_args(argv)
    global T
    # the workspace may express a preference; an explicit flag always wins
    try:
        wcfg = Config.load(Path(args.root))
    except Exception:
        wcfg = None
    mode = args.color if args.color != "auto" else (wcfg.color if wcfg else "auto")
    editor = args.editor or (wcfg.editor if wcfg and wcfg.editor else None)
    T = Term(mode=mode, editor=editor)
    try:
        return args.fn(args)
    except (RuntimeError, ValueError) as e:
        print(_c(f"error: {e}", "red"), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
