"""Gherkin spec model.

The load-bearing idea: every scenario carries a durable identity (`@rid:S-xxxxxxxx`)
stamped into the feature file at approval time. Reordering, renaming, and rewording all
preserve that identity, so the ledger can track one scenario across spec revisions.

The second load-bearing idea: a scenario's *canonical hash* covers everything that changes
its meaning (steps, tables, docstrings, background, non-rid tags) and nothing that doesn't
(whitespace, ordering of tags, the rid itself). Green status in the ledger is always bound
to a canonical hash. Change the meaning, and green evaporates -- it does not silently carry
over. That is the shalt.
"""
from __future__ import annotations

import hashlib
import re
import secrets
from dataclasses import dataclass, field
from pathlib import Path

from gherkin.parser import Parser
from gherkin.token_scanner import TokenScanner

RID_PREFIX = "@rid:"
HOLDOUT_TAG = "@holdout"
EPIC_PREFIX = "@epic:"
EPIC_RE = re.compile(r"@epic:([A-Za-z0-9_.\-]+)")
RID_RE = re.compile(r"@rid:(S-[0-9a-f]{8})")


def new_rid() -> str:
    return "S-" + secrets.token_hex(4)


def _sha(text: str) -> str:
    return "sha256:" + hashlib.sha256(text.encode("utf-8")).hexdigest()[:32]


def _step_lines(steps: list[dict]) -> list[str]:
    out: list[str] = []
    for s in steps:
        line = f"{s['keyword'].strip()} {s['text'].strip()}"
        arg = s.get("docString")
        if arg:
            line += "\n<<<" + arg.get("content", "").strip() + ">>>"
        tbl = s.get("dataTable")
        if tbl:
            for row in tbl.get("rows", []):
                line += "\n|" + "|".join(c["value"].strip() for c in row["cells"]) + "|"
        out.append(line)
    return out


def _examples_lines(examples: list[dict]) -> list[str]:
    out: list[str] = []
    for ex in examples:
        head = ex.get("tableHeader")
        if head:
            out.append("|" + "|".join(c["value"].strip() for c in head["cells"]) + "|")
        for row in ex.get("tableBody", []):
            out.append("|" + "|".join(c["value"].strip() for c in row["cells"]) + "|")
    return out


@dataclass
class Scenario:
    rid: str | None
    name: str
    keyword: str
    tags: list[str]
    steps: list[str]
    examples: list[str]
    feature_name: str
    feature_file: str
    line: int
    tag_lines: list[int] = field(default_factory=list)
    rid_count: int = 0
    inherited_tags: list[str] = field(default_factory=list)

    @property
    def all_tags(self) -> list[str]:
        return list(self.tags) + [t for t in self.inherited_tags if t not in self.tags]

    @property
    def block_start(self) -> int:
        """First line of this scenario, including its tag block. 1-indexed."""
        return min(self.tag_lines) if self.tag_lines else self.line

    @property
    def is_holdout(self) -> bool:
        return any(t == HOLDOUT_TAG for t in self.tags)

    @property
    def epic(self) -> str:
        for t in self.all_tags:
            if (m := EPIC_RE.fullmatch(t)):
                return m.group(1)
        return ""

    @property
    def is_outline(self) -> bool:
        return "outline" in self.keyword.lower()

    def canonical(self, background: list[str]) -> str:
        """Meaning-bearing text of this scenario. Excludes @rid and cosmetic detail."""
        tags = sorted(t for t in self.tags if not t.startswith(RID_PREFIX))
        parts = [
            f"FEATURE:{self.feature_name.strip()}",
            "BACKGROUND:\n" + "\n".join(background),
            f"TAGS:{','.join(tags)}",
            f"{self.keyword.strip().upper()}:{self.name.strip()}",
            "STEPS:\n" + "\n".join(self.steps),
            "EXAMPLES:\n" + "\n".join(self.examples),
        ]
        return "\n".join(parts)

    def spec_hash(self, background: list[str]) -> str:
        return _sha(self.canonical(background))


@dataclass
class Feature:
    name: str
    file: str
    tags: list[str]
    background: list[str] = field(default_factory=list)
    scenarios: list[Scenario] = field(default_factory=list)
    rule_lines: list[int] = field(default_factory=list)
    description: str = ""

    @property
    def story(self):
        from .narrative import parse_story
        return parse_story(self.description)

    @property
    def epic(self) -> str:
        """The epic this story belongs to: an @epic: tag, else the directory under spec/."""
        for t in self.tags:
            if (m := EPIC_RE.fullmatch(t)):
                return m.group(1)
        parts = Path(self.file).parts
        return parts[0] if len(parts) > 1 else ""

    def blocks(self, total_lines: int) -> list[tuple[Scenario, int, int]]:
        """(scenario, first line, line after last) -- 1-indexed, end exclusive.

        Boundaries come from the parser, not from matching text, so a `Scenario:` or a `@tag`
        written inside a docstring or a table cell cannot be mistaken for structure.
        """
        anchors = sorted({sc.block_start for sc in self.scenarios} | set(self.rule_lines))
        out = []
        for sc in self.scenarios:
            start = sc.block_start
            nxt = [a for a in anchors if a > start]
            out.append((sc, start, min(nxt) if nxt else total_lines + 1))
        return out


def _walk_children(children: list[dict], feature_name: str, rel: str,
                   background: list[str], scenarios: list[Scenario],
                   rule_lines: list[int]) -> None:
    """Flatten Rule blocks; collect background steps and scenarios."""
    for child in children:
        if "background" in child:
            background.extend(_step_lines(child["background"].get("steps", [])))
        elif "rule" in child:
            rule_lines.append(child["rule"].get("location", {}).get("line", 0))
            _walk_children(child["rule"].get("children", []), feature_name, rel,
                           background, scenarios, rule_lines)
        elif "scenario" in child:
            sc = child["scenario"]
            rids = [m.group(1) for t in sc.get("tags", [])
                    if (m := RID_RE.fullmatch(t["name"]))]
            scenarios.append(
                Scenario(
                    rid=rids[0] if rids else None,
                    rid_count=len(rids),
                    tag_lines=[t.get("location", {}).get("line", 0)
                               for t in sc.get("tags", [])],
                    name=sc["name"],
                    keyword=sc["keyword"],
                    tags=[t["name"] for t in sc.get("tags", [])],
                    steps=_step_lines(sc.get("steps", [])),
                    examples=_examples_lines(sc.get("examples", [])),
                    feature_name=feature_name,
                    feature_file=rel,
                    line=sc.get("location", {}).get("line", 0),
                )
            )


def parse_feature(path: Path, root: Path) -> Feature | None:
    return parse_text(path.read_text(encoding="utf-8"), str(path.relative_to(root)))


def parse_text(source: str, rel: str) -> Feature | None:
    doc = Parser().parse(TokenScanner(source))
    feat = doc.get("feature")
    if not feat:
        return None
    background: list[str] = []
    scenarios: list[Scenario] = []
    rule_lines: list[int] = []
    _walk_children(feat.get("children", []), feat["name"], rel, background, scenarios,
                   rule_lines)
    feature_tags = [t["name"] for t in feat.get("tags", [])]
    # Gherkin tag inheritance: a feature-level tag applies to every scenario in it. This is how
    # one @epic: tag on the feature reaches each scenario without repeating it.
    for sc in scenarios:
        sc.inherited_tags = list(feature_tags)
    return Feature(
        name=feat["name"],
        file=rel,
        tags=feature_tags,
        background=background,
        scenarios=scenarios,
        rule_lines=rule_lines,
        description=feat.get("description", "") or "",
    )


class SpecParseError(Exception):
    def __init__(self, errors: dict[str, str]):
        self.errors = errors
        super().__init__("; ".join(f"{k}: {v}" for k, v in errors.items()))


def load_specs(spec_dir: Path, root: Path | None = None,
               strict: bool = True) -> list[Feature]:
    """Parse every feature file. A malformed file raises SpecParseError naming the file
    rather than surfacing a raw parser traceback from whichever command touched it."""
    root = root or spec_dir
    out, errors = [], {}
    for p in sorted(spec_dir.rglob("*.feature")):
        try:
            f = parse_feature(p, root)
        except Exception as exc:  # gherkin raises a composite parser exception
            errors[str(p.relative_to(root))] = str(exc).splitlines()[0][:200]
            continue
        if f:
            out.append(f)
    if errors and strict:
        raise SpecParseError(errors)
    return out


def stamp_rids(spec_dir: Path) -> dict[str, str]:
    """Give every unstamped scenario a durable rid, in place.

    Insertion points come from the parser, so a `Scenario:` line inside a docstring or a table
    cell is never mistaken for a scenario. Ids are unique across the whole spec directory, not
    just within one file, and the file's original line endings are preserved.
    """
    minted: dict[str, str] = {}
    paths = sorted(spec_dir.rglob("*.feature"))
    existing: set[str] = set()
    for path in paths:
        existing |= set(RID_RE.findall(path.read_text(encoding="utf-8")))
    for path in paths:
        raw = path.read_bytes()  # read_text normalises newlines; we must not lose CRLF
        newline = "\r\n" if b"\r\n" in raw else "\n"
        text = raw.decode("utf-8")
        lines = text.splitlines()
        feat = parse_text(text, str(path.relative_to(spec_dir)))
        if feat is None:
            continue
        # bottom-up, so line numbers of not-yet-processed scenarios stay valid
        for sc in sorted(feat.scenarios, key=lambda x: x.block_start, reverse=True):
            if sc.rid:
                continue
            rid = new_rid()
            while rid in existing:
                rid = new_rid()
            existing.add(rid)
            minted[rid] = sc.name
            idx = sc.block_start - 1
            line = lines[idx]
            indent = line[: len(line) - len(line.lstrip())]
            lines.insert(idx, f"{indent}{RID_PREFIX}{rid}")
        path.write_bytes((newline.join(lines) + newline).encode("utf-8"))
    return minted


def duplicate_rids(features: list[Feature]) -> list[tuple[str, str]]:
    """Scenarios carrying more than one @rid tag, plus any rid used twice."""
    problems = [(f.file, s.name) for f in features for s in f.scenarios if s.rid_count > 1]
    seen: dict[str, str] = {}
    for f in features:
        for s in f.scenarios:
            if s.rid and s.rid in seen:
                problems.append((f.file, f"{s.name} reuses id {s.rid} from {seen[s.rid]}"))
            elif s.rid:
                seen[s.rid] = s.name
    return problems


def index_scenarios(features: list[Feature]) -> dict[str, tuple[Feature, Scenario]]:
    idx: dict[str, tuple[Feature, Scenario]] = {}
    for f in features:
        for s in f.scenarios:
            if s.rid:
                idx[s.rid] = (f, s)
    return idx


def strip_holdouts(text: str) -> str:
    """Remove @holdout scenarios from feature-file text.

    Held-out scenarios are approved, and they run during verification, but they are never shown
    to the implementer. If the visible scenarios go green while the held-out ones stay red, the
    implementer overfitted to the examples it could see rather than implementing the behaviour.
    The write guard stops test *tampering*; holdouts are the defence against test *overfitting*.

    Line ranges come from the parser, so a docstring containing `@something` or `Scenario:`
    cannot cut the deletion short and leak part of a held-out scenario.
    """
    feat = parse_text(text, "<memory>")
    newline = "\r\n" if "\r\n" in text else "\n"
    lines = text.splitlines()
    if feat is None:
        return text
    ranges = [(start, end) for sc, start, end in feat.blocks(len(lines)) if sc.is_holdout]
    for start, end in sorted(ranges, reverse=True):
        del lines[start - 1:end - 1]
    return newline.join(lines) + newline


def holdout_rids(features: list[Feature]) -> set[str]:
    return {s.rid for f in features for s in f.scenarios if s.rid and s.is_holdout}
