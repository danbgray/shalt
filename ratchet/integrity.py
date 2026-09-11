"""Write guards and turn verification.

If the same model writes the spec, the tests, and the implementation, a green suite proves
internal consistency rather than correctness. The mitigation is structural, not procedural:
each role gets a zone it may write, and a turn that touches anything outside its zone is
rejected and rolled back. "The implementer cannot edit the tests" is enforced here, not asked
for politely in a prompt.
"""
from __future__ import annotations

import hashlib
import shutil
from dataclasses import dataclass
from pathlib import Path

# role -> zones the role may write
ZONES: dict[str, tuple[str, ...]] = {
    "author": ("spec",),
    "stepwright": ("steps", "contract"),
    "implementer": ("src",),
    "human": ("spec", "steps", "contract", "src"),
}

ALL_ZONES = ("spec", "steps", "contract", "src")

# What each role is allowed to READ. Enforced by staging: the role runs in a directory that
# physically contains only these zones, so isolation is not a matter of it choosing not to look.
#   stepwright  sees the spec, and nothing of the implementation.
#   implementer sees the spec and the interface contract, but never the step definitions
#               it must satisfy -- so it has to implement the behaviour, not the assertions.
READS: dict[str, tuple[str, ...]] = {
    "author": ("spec",),
    "stepwright": ("spec",),
    "implementer": ("spec", "contract", "src"),
}


class IntegrityViolation(RuntimeError):
    def __init__(self, role: str, offences: dict[str, list[str]]):
        self.role = role
        self.offences = offences
        detail = "; ".join(f"{z}: {', '.join(sorted(p)[:6])}" for z, p in offences.items() if p)
        super().__init__(f"role '{role}' wrote outside its zone -> {detail}")


def file_hash(p: Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest()[:16]


def snapshot(root: Path, zones: tuple[str, ...] = ALL_ZONES) -> dict[str, dict[str, str]]:
    snap: dict[str, dict[str, str]] = {}
    for z in zones:
        zdir = root / z
        files: dict[str, str] = {}
        if zdir.exists():
            for p in sorted(zdir.rglob("*")):
                if p.is_file() and "__pycache__" not in p.parts:
                    files[str(p.relative_to(root))] = file_hash(p)
        snap[z] = files
    return snap


def diff(before: dict[str, dict[str, str]], after: dict[str, dict[str, str]]) -> dict[str, list[str]]:
    out: dict[str, list[str]] = {}
    for z in set(before) | set(after):
        b, a = before.get(z, {}), after.get(z, {})
        changed = [p for p in set(b) | set(a) if b.get(p) != a.get(p)]
        out[z] = sorted(changed)
    return out


@dataclass
class GuardedTurn:
    """Context manager enforcing that `role` only wrote inside its own zone.

    Protected zones are backed up on entry and restored on violation, so a cheating turn
    leaves the workspace exactly as it found it.
    """
    root: Path
    role: str
    backup_dir: Path

    def __post_init__(self) -> None:
        self.allowed = ZONES.get(self.role, ())
        self.protected = tuple(z for z in ALL_ZONES if z not in self.allowed)

    def __enter__(self) -> "GuardedTurn":
        self._before = snapshot(self.root)
        if self.backup_dir.exists():
            shutil.rmtree(self.backup_dir)
        self.backup_dir.mkdir(parents=True, exist_ok=True)
        for z in self.protected:
            src = self.root / z
            if src.exists():
                shutil.copytree(src, self.backup_dir / z,
                                ignore=shutil.ignore_patterns("__pycache__"))
        return self

    def __exit__(self, exc_type, exc, tb) -> bool:
        if exc_type is not None:
            return False
        after = snapshot(self.root)
        d = diff(self._before, after)
        offences = {z: d.get(z, []) for z in self.protected if d.get(z)}
        if offences:
            self.restore()
            raise IntegrityViolation(self.role, offences)
        return False

    def restore(self) -> None:
        for z in self.protected:
            tgt = self.root / z
            bak = self.backup_dir / z
            if tgt.exists():
                shutil.rmtree(tgt)
            if bak.exists():
                shutil.copytree(bak, tgt)


def audit(root: Path, ledger, features: list) -> list[str]:
    """Standing integrity checks that do not depend on a turn being in flight."""
    problems: list[str] = []
    unstamped = [(f.file, s.name) for f in features for s in f.scenarios if not s.rid]
    for file, name in unstamped:
        problems.append(f"unstamped scenario (run `ratchet approve`): {file} :: {name}")
    for e in ledger.by_status("orphan"):
        problems.append(f"orphan: ledger has '{e.name}' ({e.rid}) but the spec no longer does")
    for e in ledger.entries.values():
        if e.status == "green" and e.verified_spec_hash != e.spec_hash:
            problems.append(f"green but unverified against current spec: {e.rid} {e.name}")
    lock = ledger.spec_lock
    if not lock:
        problems.append("spec is not approved: no human sign-off recorded")
    return problems
