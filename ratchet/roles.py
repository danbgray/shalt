"""Running one agent role as a guarded, staged turn.

Every turn is: stage the zones the role may read -> let the backend work -> verify it wrote
only where it was allowed -> copy back only its own zones. A turn that reaches outside its
zone is rejected wholesale; nothing it produced is kept.
"""
from __future__ import annotations

import shutil
from dataclasses import dataclass
from pathlib import Path

from .integrity import ALL_ZONES, READS, ZONES, IntegrityViolation
from .spec import strip_holdouts


@dataclass
class RoleResult:
    role: str
    transcript: str
    wrote: list[str]
    stage: Path


def _stage_for(root: Path, role: str, stage: Path, hide_holdouts: bool) -> None:
    if stage.exists():
        shutil.rmtree(stage)
    stage.mkdir(parents=True)
    for z in READS.get(role, ()):
        src = root / z
        if not src.exists():
            continue
        shutil.copytree(src, stage / z, ignore=shutil.ignore_patterns("__pycache__"))
    if hide_holdouts and (stage / "spec").exists():
        for p in (stage / "spec").rglob("*.feature"):
            p.write_text(strip_holdouts(p.read_text(encoding="utf-8")), encoding="utf-8")
    # make sure the role has somewhere to write
    for z in ZONES.get(role, ()):
        (stage / z).mkdir(parents=True, exist_ok=True)


def _stage_offences(stage: Path, role: str) -> dict[str, list[str]]:
    """Anything the role created outside its write zones, inside its own stage."""
    allowed = set(ZONES.get(role, ()))
    offences: dict[str, list[str]] = {}
    for p in sorted(stage.rglob("*")):
        if not p.is_file() or "__pycache__" in p.parts:
            continue
        rel = p.relative_to(stage)
        top = rel.parts[0] if len(rel.parts) > 1 else "<root>"
        if top in allowed:
            continue
        if top in READS.get(role, ()):
            # readable zone: only a *modification* is an offence, checked by hash below
            continue
        offences.setdefault(top, []).append(str(rel))
    return offences


def run_role(root: Path, role: str, prompt: str, backend, *,
             hide_holdouts: bool = False) -> RoleResult:
    from .integrity import snapshot, diff

    stage = root / ".ratchet" / "stage" / role
    _stage_for(root, role, stage, hide_holdouts)
    before = snapshot(stage, ALL_ZONES)

    transcript = backend.run(role, prompt, stage)

    after = snapshot(stage, ALL_ZONES)
    d = diff(before, after)
    offences = _stage_offences(stage, role)
    # a read-only zone that was modified in place is also an offence
    for z in READS.get(role, ()):
        if z not in ZONES.get(role, ()) and d.get(z):
            offences.setdefault(z, []).extend(d[z])
    if offences:
        raise IntegrityViolation(role, offences)

    wrote: list[str] = []
    for z in ZONES.get(role, ()):
        src = stage / z
        if not src.exists():
            continue
        dest = root / z
        dest.mkdir(parents=True, exist_ok=True)
        for p in sorted(src.rglob("*")):
            if p.is_file() and "__pycache__" not in p.parts:
                tgt = dest / p.relative_to(src)
                tgt.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(p, tgt)
                wrote.append(str(tgt.relative_to(root)))
    return RoleResult(role=role, transcript=transcript, wrote=wrote, stage=stage)
