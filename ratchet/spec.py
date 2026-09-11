"""Gherkin spec model.

The load-bearing idea: every scenario carries a durable identity (`@rid:S-xxxxxxxx`)
stamped into the feature file at approval time. Reordering, renaming, and rewording all
preserve that identity, so the ledger can track one scenario across spec revisions.

The second load-bearing idea: a scenario's *canonical hash* covers everything that changes
its meaning (steps, tables, docstrings, background, non-rid tags) and nothing that doesn't
(whitespace, ordering of tags, the rid itself). Green status in the ledger is always bound
to a canonical hash. Change the meaning, and green evaporates -- it does not silently carry
over. That is the ratchet.
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


def _walk_children(children: list[dict], feature_name: str, rel: str,
                   background: list[str], scenarios: list[Scenario]) -> None:
    """Flatten Rule blocks; collect background steps and scenarios."""
    for child in children:
        if "background" in child:
            background.extend(_step_lines(child["background"].get("steps", [])))
        elif "rule" in child:
            _walk_children(child["rule"].get("children", []), feature_name, rel,
                           background, scenarios)
        elif "scenario" in child:
            sc = child["scenario"]
            scenarios.append(
                Scenario(
                    rid=next((m.group(1) for t in sc.get("tags", [])
                              if (m := RID_RE.fullmatch(t["name"]))), None),
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
    doc = Parser().parse(TokenScanner(path.read_text(encoding="utf-8")))
    feat = doc.get("feature")
    if not feat:
        return None
    rel = str(path.relative_to(root))
    background: list[str] = []
    scenarios: list[Scenario] = []
    _walk_children(feat.get("children", []), feat["name"], rel, background, scenarios)
    return Feature(
        name=feat["name"],
        file=rel,
        tags=[t["name"] for t in feat.get("tags", [])],
        background=background,
        scenarios=scenarios,
    )


def load_specs(spec_dir: Path, root: Path | None = None) -> list[Feature]:
    root = root or spec_dir
    out = []
    for p in sorted(spec_dir.rglob("*.feature")):
        f = parse_feature(p, root)
        if f:
            out.append(f)
    return out


def stamp_rids(spec_dir: Path) -> dict[str, str]:
    """Give every unstamped scenario a durable rid, in place. Returns {rid: scenario name}."""
    minted: dict[str, str] = {}
    for path in sorted(spec_dir.rglob("*.feature")):
        lines = path.read_text(encoding="utf-8").splitlines()
        out: list[str] = []
        existing = set(RID_RE.findall("\n".join(lines)))
        for i, line in enumerate(lines):
            stripped = line.strip()
            is_scenario = re.match(r"^(Scenario Outline|Scenario Template|Scenario|Example):",
                                   stripped)
            if is_scenario:
                # Look back over the contiguous tag block directly above.
                j = len(out) - 1
                tagline = None
                while j >= 0 and out[j].strip().startswith("@"):
                    if RID_RE.search(out[j]):
                        tagline = -1  # already stamped
                        break
                    tagline = j
                    j -= 1
                if tagline != -1:
                    rid = new_rid()
                    while rid in existing:
                        rid = new_rid()
                    existing.add(rid)
                    minted[rid] = stripped.split(":", 1)[1].strip()
                    indent = line[: len(line) - len(line.lstrip())]
                    if tagline is None:
                        out.append(f"{indent}{RID_PREFIX}{rid}")
                    else:
                        out[tagline] = out[tagline].rstrip() + f" {RID_PREFIX}{rid}"
            out.append(line)
        path.write_text("\n".join(out) + "\n", encoding="utf-8")
    return minted


def index_scenarios(features: list[Feature]) -> dict[str, tuple[Feature, Scenario]]:
    idx: dict[str, tuple[Feature, Scenario]] = {}
    for f in features:
        for s in f.scenarios:
            if s.rid:
                idx[s.rid] = (f, s)
    return idx


HOLDOUT_TAG = "@holdout"


def strip_holdouts(text: str) -> str:
    """Remove @holdout scenarios from feature-file text.

    Held-out scenarios are approved, and they run during verification, but they are never shown
    to the implementer. If the visible scenarios go green while the held-out ones stay red, the
    implementer overfitted to the examples it could see rather than implementing the behaviour.
    The write guard stops test *tampering*; holdouts are the defence against test *overfitting*.
    """
    lines = text.splitlines()
    out: list[str] = []
    i = 0
    scenario_re = re.compile(r"^(Scenario Outline|Scenario Template|Scenario|Example):")
    while i < len(lines):
        # gather a contiguous tag block
        start = i
        tags: list[str] = []
        while i < len(lines) and lines[i].strip().startswith("@"):
            tags.append(lines[i])
            i += 1
        if i < len(lines) and scenario_re.match(lines[i].strip()) and \
                any(HOLDOUT_TAG in t for t in tags):
            indent = len(lines[i]) - len(lines[i].lstrip())
            i += 1
            while i < len(lines):
                nxt = lines[i]
                s = nxt.strip()
                if s and (len(nxt) - len(nxt.lstrip())) <= indent and \
                        (scenario_re.match(s) or s.startswith("@") or
                         s.startswith("Rule:") or s.startswith("Feature:")):
                    break
                i += 1
            continue
        out.extend(lines[start:i])
        if i < len(lines):
            out.append(lines[i])
            i += 1
    return "\n".join(out) + "\n"


def holdout_rids(features: list[Feature]) -> set[str]:
    return {s.rid for f in features for s in f.scenarios
            if s.rid and HOLDOUT_TAG in s.tags}
