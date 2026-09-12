"""Running one agent role as a guarded, staged turn.

Every turn is: stage the zones the role may read into a directory *outside* the workspace ->
let the backend work -> verify it wrote only where it was allowed -> mirror back only its own
zones. A turn that reaches outside its zone is rejected wholesale and the workspace is restored.

Three things are checked, because any one of them alone is defeatable:

1. The stage lives outside the workspace, so relative traversal out of it (`../../steps`) lands
   in a scratch directory rather than in the real spec or tests.
2. The stage is scanned for files outside the role's write zones, and for symlinks anywhere --
   a symlink inside an allowed zone is a write to wherever it points.
3. The real workspace is hashed before and after under a GuardedTurn, so a backend that writes
   by absolute path is still caught, and the protected zones are restored.
"""
from __future__ import annotations

import shutil
import tempfile
from dataclasses import dataclass
from pathlib import Path

from .integrity import (ALL_ZONES, READS, ZONES, GuardedTurn, IntegrityViolation,
                        diff, iter_files, snapshot)
from .spec import strip_holdouts


@dataclass
class RoleResult:
    role: str
    transcript: str
    wrote: list[str]
    removed: list[str]


def _stage_for(root: Path, role: str, stage: Path, hide_holdouts: bool) -> None:
    stage.mkdir(parents=True, exist_ok=True)
    for z in sorted(set(READS.get(role, ())) | set(ZONES.get(role, ()))):
        src = root / z
        (stage / z).mkdir(parents=True, exist_ok=True)
        if not src.exists():
            continue
        for p, is_link in iter_files(src):
            if is_link:
                continue  # never propagate a symlink into a stage
            tgt = stage / z / p.relative_to(src)
            tgt.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(p, tgt)
    if hide_holdouts and (stage / "spec").exists():
        for p in (stage / "spec").rglob("*.feature"):
            raw = p.read_bytes().decode("utf-8")
            p.write_bytes(strip_holdouts(raw).encode("utf-8"))


def _stage_offences(stage: Path, role: str) -> dict[str, list[str]]:
    """Files the role created outside its write zones, and any symlink anywhere."""
    allowed = set(ZONES.get(role, ()))
    readable = set(READS.get(role, ()))
    offences: dict[str, list[str]] = {}
    for p, is_link in iter_files(stage):
        rel = p.relative_to(stage)
        top = rel.parts[0] if len(rel.parts) > 1 else "<stage root>"
        if is_link:
            offences.setdefault("symlink", []).append(f"{rel} -> {p.readlink()}")
            continue
        if top in allowed or top in readable:
            continue  # readable-but-not-writable zones are checked by hash instead
        offences.setdefault(top, []).append(str(rel))
    return offences


def _mirror_back(stage: Path, root: Path, zones: tuple[str, ...]) -> tuple[list[str], list[str]]:
    """Make the role's own zones in the workspace match its stage exactly, deletions included.
    Without the deletions a role could never remove a file it had superseded, and stale modules
    would accumulate and keep being collected by the test runner."""
    wrote: list[str] = []
    removed: list[str] = []
    for z in zones:
        src, dest = stage / z, root / z
        dest.mkdir(parents=True, exist_ok=True)
        staged = {str(p.relative_to(src)): p for p, link in iter_files(src) if not link}
        present = {str(p.relative_to(dest)): p for p, link in iter_files(dest) if not link}
        for rel, p in sorted(staged.items()):
            tgt = dest / rel
            tgt.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(p, tgt)
            wrote.append(str(tgt.relative_to(root)))
        for rel, p in sorted(present.items()):
            if rel not in staged:
                p.unlink()
                removed.append(str(p.relative_to(root)))
    return wrote, removed


def run_role(root: Path, role: str, prompt: str, backend, *,
             hide_holdouts: bool = False) -> RoleResult:
    root = Path(root).resolve()
    parent = Path(tempfile.mkdtemp(prefix="shalt-stage-"))
    stage = parent / role
    try:
        _stage_for(root, role, stage, hide_holdouts)
        before = snapshot(stage, ALL_ZONES)
        with GuardedTurn(root, role, root / ".shalt" / "backup"):
            transcript = backend.run(role, prompt, stage)

            offences = _stage_offences(stage, role)
            d = diff(before, snapshot(stage, ALL_ZONES))
            for z in READS.get(role, ()):
                if z not in ZONES.get(role, ()) and d.get(z):
                    offences.setdefault(z, []).extend(d[z])
            if offences:
                raise IntegrityViolation(role, offences)

            wrote, removed = _mirror_back(stage, root, ZONES.get(role, ()))
        return RoleResult(role=role, transcript=transcript, wrote=wrote, removed=removed)
    finally:
        shutil.rmtree(parent, ignore_errors=True)
