"""Mutation testing the oracle, not the code.

The write guard stops an implementer tampering with the tests. Holdouts catch an implementer
overfitting to the examples it saw. Neither says anything about the *stepwright* -- nothing so
far checks that the generated step definitions actually assert what their scenario claims. A
step definition ending in `assert True` passes every time, and the ledger would show green.

The check runs in the opposite direction to the obvious one: rather than mutating the step
definitions, mutate the **implementation** and see whether the scenarios notice. Break the
rounding rule; if "a half-cent total rounds up" stays green, that scenario is not testing
rounding, whatever its name says.

Attribution is what makes this more useful than a suite-wide mutation score. For each mutant we
record exactly which scenarios went red, so every scenario gets a kill count. A green scenario
that kills nothing is *vacuous*: it passes no matter what the implementation does.

Two honest limitations, stated here because they bound what a score means:

1. **Equivalent mutants.** Some mutations do not change behaviour at all (`x * 1` vs `x / 1`
   on identity, an unreachable branch, a value never read). Those survive for reasons that have
   nothing to do with the oracle. A survivor is a question to answer, not a proven defect.
2. **Coverage confounds strength.** A mutant in a line no scenario exercises survives because
   nothing reaches it, not because the assertions are weak. Survivors are reported with their
   file and line so that distinction stays visible to a human.
"""
from __future__ import annotations

import ast
import random
import re
import shutil
import tempfile
from dataclasses import dataclass, field
from pathlib import Path

from .config import Config
from .runner import run_suite

# --------------------------------------------------------------------- model
@dataclass
class Mutant:
    path: str            # relative to the workspace root
    line: int
    operator: str
    before: str
    after: str
    killed_by: list[str] = field(default_factory=list)
    status: str = "pending"   # killed | survived | invalid

    def describe(self) -> str:
        return f"{self.path}:{self.line}  {self.operator}  {self.before} -> {self.after}"


@dataclass
class MutationReport:
    mutants: list[Mutant] = field(default_factory=list)
    baseline_green: list[str] = field(default_factory=list)
    kills: dict[str, int] = field(default_factory=dict)
    error: str = ""

    @property
    def killed(self) -> list[Mutant]:
        return [m for m in self.mutants if m.status == "killed"]

    @property
    def survived(self) -> list[Mutant]:
        return [m for m in self.mutants if m.status == "survived"]

    @property
    def invalid(self) -> list[Mutant]:
        return [m for m in self.mutants if m.status == "invalid"]

    @property
    def score(self) -> float:
        considered = len(self.killed) + len(self.survived)
        return round(100 * len(self.killed) / considered, 1) if considered else 0.0

    @property
    def vacuous(self) -> list[str]:
        """Scenarios green at baseline that no mutation could make fail at all."""
        return sorted(r for r in self.baseline_green if not self.kills.get(r))

    @property
    def exercised(self) -> dict[str, set[str]]:
        """scenario -> the implementation files it demonstrably reaches.

        Derived, not instrumented: if a scenario went red when a file was mutated, it
        provably executes that file. This gives coverage-like attribution without a
        language-specific coverage tool.
        """
        out: dict[str, set[str]] = {}
        for m in self.killed:
            for rid in m.killed_by:
                out.setdefault(rid, set()).add(m.path)
        return out

    @property
    def blind_spots(self) -> dict[str, list[Mutant]]:
        """scenario -> surviving mutants in files that scenario is known to execute.

        This is the finding that matters, and the reason "killed nothing" is too weak a test
        on its own. A vacuous assertion still catches mutations that *crash* the code, so a
        scenario with a meaningless assertion can post a healthy kill count. What it cannot
        catch is a well-formed but wrong value. A survivor in a file the scenario provably
        runs is exactly that: the scenario executed the broken code and said nothing.
        """
        ex = self.exercised
        out: dict[str, list[Mutant]] = {}
        for m in self.survived:
            for rid in self.baseline_green:
                if m.path in ex.get(rid, set()):
                    out.setdefault(rid, []).append(m)
        return out

    @property
    def weak_oracles(self) -> dict[str, str]:
        """scenario -> why its oracle is suspect. The union of the two signals.

        The two are complementary, and neither alone is sufficient:

        * `vacuous` catches a scenario that detected *nothing*. But attribution is impossible
          for it -- with no kills there is no evidence of which files it runs -- so it cannot
          also appear as a blind spot.
        * `blind_spots` catches a scenario with a healthy kill count that still ran broken
          code silently. A vacuous assertion still notices mutations that *crash*, so such a
          scenario can look well-armed on kill count alone.

        Which one fires depends on whether the mutations happen to crash or merely change a
        value, so callers should check this union rather than either property.
        """
        out: dict[str, str] = {}
        for rid in self.vacuous:
            out[rid] = "detected no mutation at all"
        for rid, ms in self.blind_spots.items():
            out[rid] = f"ran {len(ms)} mutated version(s) of code it executes without noticing"
        return out

    def to_dict(self) -> dict:
        return {
            "score": self.score,
            "killed": len(self.killed),
            "survived": len(self.survived),
            "invalid": len(self.invalid),
            "baseline_green": len(self.baseline_green),
            "kills": self.kills,
            "vacuous": self.vacuous,
            "weak_oracles": self.weak_oracles,
            "blind_spots": {rid: [m.describe() for m in ms]
                            for rid, ms in self.blind_spots.items()},
            "survivors": [m.describe() for m in self.survived],
        }


# ----------------------------------------------------------- python ast engine
_CMP_SWAP = {ast.Eq: ast.NotEq, ast.NotEq: ast.Eq, ast.Lt: ast.GtE, ast.GtE: ast.Lt,
             ast.Gt: ast.LtE, ast.LtE: ast.Gt}
_BIN_SWAP = {ast.Add: ast.Sub, ast.Sub: ast.Add, ast.Mult: ast.Div, ast.Div: ast.Mult}
_BOOL_SWAP = {ast.And: ast.Or, ast.Or: ast.And}
SENTINEL = "shalt-mutant"


def _docstring_nodes(tree: ast.AST) -> set[int]:
    """Docstrings cannot affect behaviour, so mutating one always survives.

    Including them would inflate the survivor list with findings no one can act on, which is
    worse than useless -- it teaches the reader to ignore the list.
    """
    out: set[int] = set()
    for node in ast.walk(tree):
        if isinstance(node, (ast.Module, ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            body = getattr(node, "body", None) or []
            if body and isinstance(body[0], ast.Expr) and \
                    isinstance(body[0].value, ast.Constant) and \
                    isinstance(body[0].value.value, str):
                out.add(id(body[0].value))
    return out


def _py_targets(tree: ast.AST) -> list[tuple[ast.AST, str, str, str]]:
    """Eligible nodes, each with the operator name and a before/after description."""
    out = []
    skip = _docstring_nodes(tree)
    for node in ast.walk(tree):
        if id(node) in skip:
            continue
        if isinstance(node, ast.Compare) and len(node.ops) == 1 \
                and type(node.ops[0]) in _CMP_SWAP:
            a, b = type(node.ops[0]), _CMP_SWAP[type(node.ops[0])]
            out.append((node, "comparison", a.__name__, b.__name__))
        elif isinstance(node, ast.BinOp) and type(node.op) in _BIN_SWAP:
            a, b = type(node.op), _BIN_SWAP[type(node.op)]
            out.append((node, "arithmetic", a.__name__, b.__name__))
        elif isinstance(node, ast.BoolOp) and type(node.op) in _BOOL_SWAP:
            a, b = type(node.op), _BOOL_SWAP[type(node.op)]
            out.append((node, "boolean", a.__name__, b.__name__))
        elif isinstance(node, ast.Constant):
            v = node.value
            if isinstance(v, bool):
                out.append((node, "boolean-literal", repr(v), repr(not v)))
            elif isinstance(v, int) and not isinstance(v, bool):
                out.append((node, "number", repr(v), repr(v + 1)))
            elif isinstance(v, float):
                out.append((node, "number", repr(v), repr(v + 1)))
            elif isinstance(v, str) and v and len(v) < 200 and v != SENTINEL:
                out.append((node, "string", repr(v), repr(SENTINEL)))
    return out


class _Mutator(ast.NodeTransformer):
    def __init__(self, target: ast.AST):
        self.target = target

    def visit(self, node):
        if node is self.target:
            if isinstance(node, ast.Compare):
                node.ops = [_CMP_SWAP[type(node.ops[0])]()]
            elif isinstance(node, ast.BinOp):
                node.op = _BIN_SWAP[type(node.op)]()
            elif isinstance(node, ast.BoolOp):
                node.op = _BOOL_SWAP[type(node.op)]()
            elif isinstance(node, ast.Constant):
                v = node.value
                node.value = (not v) if isinstance(v, bool) else (
                    SENTINEL if isinstance(v, str) else v + 1)
            return node
        return super().generic_visit(node) or node


def python_mutants(root: Path, src_dir: Path) -> list[tuple[Mutant, str]]:
    """(mutant, mutated file text) for every eligible node in every .py under src."""
    out = []
    for path in sorted(src_dir.rglob("*.py")):
        if "__pycache__" in path.parts:
            continue
        text = path.read_text(encoding="utf-8")
        try:
            base = ast.parse(text)
        except SyntaxError:
            continue
        n_targets = len(_py_targets(base))
        for i in range(n_targets):
            tree = ast.parse(text)
            targets = _py_targets(tree)
            node, op, before, after = targets[i]
            line = getattr(node, "lineno", 0)
            mutated = ast.unparse(_Mutator(node).visit(tree))
            if mutated == ast.unparse(ast.parse(text)):
                continue   # a no-op rewrite is not a mutant
            out.append((Mutant(path=str(path.relative_to(root)), line=line, operator=op,
                               before=before, after=after), mutated))
    return out


# --------------------------------------------------------------- text engine
# Deliberately crude, and deliberately language-agnostic: works on Go, JS, Java, Ruby, C#.
TEXT_OPS: list[tuple[str, str, str]] = [
    (r"(?<![=!<>])==(?!=)", "!=", "comparison"),
    (r"(?<![=!<>])!=(?!=)", "==", "comparison"),
    (r"(?<![<>=!])<=(?!=)", ">", "comparison"),
    (r"(?<![<>=!])>=(?!=)", "<", "comparison"),
    (r"(?<![<>=!+-])<(?![=<])", ">=", "comparison"),
    (r"(?<![<>=!+-])>(?![=>])", "<=", "comparison"),
    (r"\band\b", "or", "boolean"),
    (r"\bor\b", "and", "boolean"),
    (r"&&", "||", "boolean"),
    (r"\|\|", "&&", "boolean"),
    (r"\bTrue\b", "False", "boolean-literal"),
    (r"\bFalse\b", "True", "boolean-literal"),
    (r"\btrue\b", "false", "boolean-literal"),
    (r"\bfalse\b", "true", "boolean-literal"),
    (r"(?<![\w.])\+(?![+=])", "-", "arithmetic"),
    (r"(?<![\w.\s])-(?![-=>])", "+", "arithmetic"),
]
COMMENT_PREFIXES = ("#", "//", "--", "*", "/*")
TEXT_EXTENSIONS = {".py", ".js", ".mjs", ".ts", ".go", ".java", ".rb", ".cs", ".kt", ".rs"}


def text_mutants(root: Path, src_dir: Path) -> list[tuple[Mutant, str]]:
    out = []
    for path in sorted(src_dir.rglob("*")):
        if not path.is_file() or path.suffix not in TEXT_EXTENSIONS \
                or "__pycache__" in path.parts:
            continue
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
        for idx, line in enumerate(lines):
            stripped = line.strip()
            if not stripped or stripped.startswith(COMMENT_PREFIXES):
                continue
            for pattern, replacement, op in TEXT_OPS:
                for m in re.finditer(pattern, line):
                    new_line = line[:m.start()] + replacement + line[m.end():]
                    if new_line == line:
                        continue
                    mutated = list(lines)
                    mutated[idx] = new_line
                    out.append((Mutant(path=str(path.relative_to(root)), line=idx + 1,
                                       operator=op, before=m.group(0), after=replacement),
                                "\n".join(mutated) + "\n"))
    return out


ENGINES = {"python": python_mutants, "text": text_mutants}


# ------------------------------------------------------------------ campaign
def run_campaign(root: Path, cfg: Config | None = None, *, engine: str = "auto",
                 budget: int = 30, seed: int = 0,
                 progress=None) -> MutationReport:
    root = Path(root).resolve()
    cfg = cfg or Config.load(root)
    src_dir = root / cfg.src
    report = MutationReport()

    if engine == "auto":
        engine = "python" if cfg.stack == "python" else "text"
    if engine not in ENGINES:
        raise ValueError(f"unknown mutation engine {engine!r}")
    if not src_dir.exists():
        report.error = f"no implementation directory at {cfg.src}/"
        return report

    baseline = run_suite(root, cfg)
    if baseline.get("harness_error") or baseline.get("collection_error"):
        report.error = ("the suite does not run cleanly yet, so there is no baseline to "
                        "mutate against. Get it green first.")
        return report
    report.baseline_green = sorted(
        rid for rid, r in baseline.get("results", {}).items() if r["outcome"] == "passed")
    if not report.baseline_green:
        report.error = "no scenario is green, so nothing can be shown to detect a mutation."
        return report

    candidates = ENGINES[engine](root, src_dir)
    if not candidates:
        report.error = f"the {engine} engine found nothing to mutate under {cfg.src}/"
        return report
    rng = random.Random(seed)
    rng.shuffle(candidates)
    candidates = candidates[:budget]

    backup = Path(tempfile.mkdtemp(prefix="shalt-mutate-"))
    shutil.copytree(src_dir, backup / "src", ignore=shutil.ignore_patterns("__pycache__"))
    try:
        for i, (mutant, mutated_text) in enumerate(candidates, 1):
            target = root / mutant.path
            original = target.read_text(encoding="utf-8")
            try:
                target.write_text(mutated_text, encoding="utf-8")
                run = run_suite(root, cfg)
                if run.get("harness_error") or run.get("collection_error"):
                    # the mutant did not compile or the suite could not run: it proves
                    # nothing about the assertions, so it is excluded rather than counted
                    mutant.status = "invalid"
                else:
                    results = run.get("results", {})
                    killers = [rid for rid in report.baseline_green
                               if results.get(rid, {}).get("outcome") == "failed"]
                    mutant.killed_by = killers
                    mutant.status = "killed" if killers else "survived"
                    for rid in killers:
                        report.kills[rid] = report.kills.get(rid, 0) + 1
            finally:
                target.write_text(original, encoding="utf-8")
            report.mutants.append(mutant)
            if progress:
                progress(i, len(candidates), mutant)
    finally:
        if src_dir.exists():
            shutil.rmtree(src_dir)
        shutil.copytree(backup / "src", src_dir)
        shutil.rmtree(backup, ignore_errors=True)
    return report
