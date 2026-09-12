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

# The ledger is the product. No role may write it, so it is protected on every turn regardless
# of which zones the role owns.
LEDGER_FILE = ".ratchet/ledger.json"

# What each role is allowed to READ. Enforced by staging: the role runs in a directory that
# physically contains only these zones, so isolation is not a matter of it choosing not to look.
#   stepwright  sees the spec, and nothing of the implementation.
#   implementer sees the spec and the interface contract, but never the step definitions
#               it must satisfy -- so it has to implement the behaviour, not the assertions.
READS: dict[str, tuple[str, ...]] = {
    "author": ("spec",),
    "stepwright": ("spec", "steps", "contract"),
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


def iter_files(zdir: Path):
    """Walk without following symlinks. Yields (path, is_symlink)."""
    if not zdir.exists():
        return
    stack = [zdir]
    while stack:
        d = stack.pop()
        try:
            entries = sorted(d.iterdir())
        except (NotADirectoryError, PermissionError, FileNotFoundError):
            continue
        for p in entries:
            if "__pycache__" in p.parts:
                continue
            if p.is_symlink():
                yield p, True
            elif p.is_dir():
                stack.append(p)
            elif p.is_file():
                yield p, False


def snapshot(root: Path, zones: tuple[str, ...] = ALL_ZONES) -> dict[str, dict[str, str]]:
    snap: dict[str, dict[str, str]] = {}
    for z in zones:
        files: dict[str, str] = {}
        for p, is_link in iter_files(root / z):
            rel = str(p.relative_to(root))
            files[rel] = "symlink:" + str(p.readlink()) if is_link else file_hash(p)
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
        led = self.root / LEDGER_FILE
        self._ledger_before = file_hash(led) if led.exists() else None
        self._ledger_bytes = led.read_bytes() if led.exists() else None
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
            # the turn failed or was rejected upstream; put the protected zones back as found
            self.restore()
            return False
        after = snapshot(self.root)
        d = diff(self._before, after)
        offences = {z: d.get(z, []) for z in self.protected if d.get(z)}
        led = self.root / LEDGER_FILE
        now_hash = file_hash(led) if led.exists() else None
        if now_hash != self._ledger_before:
            offences.setdefault("ledger", []).append(LEDGER_FILE)
        if offences:
            self.restore()
            raise IntegrityViolation(self.role, offences)
        return False

    def restore(self) -> None:
        if self._ledger_bytes is not None:
            led = self.root / LEDGER_FILE
            led.parent.mkdir(parents=True, exist_ok=True)
            led.write_bytes(self._ledger_bytes)
        for z in self.protected:
            tgt = self.root / z
            bak = self.backup_dir / z
            if tgt.exists():
                shutil.rmtree(tgt)
            if bak.exists():
                shutil.copytree(bak, tgt)


def audit(root: Path, ledger, features: list) -> list[str]:
    """Standing integrity checks that do not depend on a turn being in flight."""
    from .spec import duplicate_rids

    problems: list[str] = []
    for file, name in duplicate_rids(features):
        problems.append(f"duplicate scenario id: {file} :: {name}")
    unstamped = [(f.file, s.name) for f in features for s in f.scenarios if not s.rid]
    for file, name in unstamped:
        problems.append(f"unstamped scenario (run `ratchet approve`): {file} :: {name}")
    for e in ledger.by_status("orphan"):
        problems.append(f"orphan: ledger has '{e.name}' ({e.rid}) but the spec no longer does")
    for e in ledger.entries.values():
        if e.status == "green" and e.verified_spec_hash != e.spec_hash:
            problems.append(f"green but unverified against current spec: {e.rid} {e.name}")
    for e in ledger.entries.values():
        if e.status == "green" and e.mutants_killed == 0:
            problems.append(
                f"vacuous: {e.rid} '{e.name}' is green but detected no mutation — "
                f"its step definitions may not assert what the scenario says")
        elif e.status == "green" and e.blind_spots:
            problems.append(
                f"weak oracle: {e.rid} '{e.name}' ran {e.blind_spots} mutated version(s) "
                f"of code it executes without noticing")
    lock = ledger.spec_lock
    if not lock:
        problems.append("spec is not approved: no human sign-off recorded")
        return problems
    # the approved spec must still be the spec on disk
    approved: dict[str, str] = lock.get("scenario_hashes", {})
    if approved:
        current = {s.rid: s.spec_hash(f.background)
                   for f in features for s in f.scenarios if s.rid}
        for rid, h in approved.items():
            if rid not in current:
                problems.append(f"approved scenario {rid} is no longer in the spec")
            elif current[rid] != h:
                name = next((s.name for f in features for s in f.scenarios
                             if s.rid == rid), rid)
                problems.append(f"scenario changed since approval, unapproved: {rid} {name}")
        for rid in current:
            if rid not in approved:
                problems.append(f"scenario added since approval, unapproved: {rid}")
    else:
        problems.append("spec lock predates content hashing; re-run `ratchet approve`")
    return problems
