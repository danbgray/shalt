"""The scenario ledger.

One artifact that is simultaneously the requirement, the test binding, the ticket, and the
progress bar. Portable, versioned JSON -- deliberately not tied to any runner or vendor.

Status meanings, which the whole product rests on:
  pending  approved scenario with no test bound to it   (NOT green; absence of a test is not success)
  red      bound test exists and fails
  green    bound test passes *against the spec hash recorded alongside it*
  stale    was green, but the scenario's meaning changed since -- green does not carry over
  orphan   a test claims an rid that no longer exists in the spec

The ratchet invariant: green is always relative to a spec hash. Edit the spec and the green
evaporates rather than silently persisting. This is what stops the oldest fraud in test-driven
work -- the spec drifted, the suite still passes, and nobody noticed.
"""
from __future__ import annotations

import json
import time
from dataclasses import dataclass, field, asdict
from pathlib import Path
from typing import Any

SCHEMA = "ratchet.ledger/1"

PENDING, RED, GREEN, STALE, ORPHAN = "pending", "red", "green", "stale", "orphan"


@dataclass
class Entry:
    rid: str
    name: str
    feature: str
    feature_file: str
    tags: list[str] = field(default_factory=list)
    epic: str = ""
    actor: str = ""
    capability: str = ""
    benefit: str = ""
    status: str = PENDING
    spec_hash: str = ""
    verified_spec_hash: str = ""   # the hash this scenario was last GREEN against
    last_green_at: str | None = None
    last_run_at: str | None = None
    failure: str | None = None
    history: list[dict[str, Any]] = field(default_factory=list)

    def record(self, event: str, **kw: Any) -> None:
        self.history.append({"at": _now(), "event": event, **kw})
        if len(self.history) > 50:
            self.history = self.history[-50:]


def _now() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


@dataclass
class Ledger:
    entries: dict[str, Entry] = field(default_factory=dict)
    spec_lock: dict[str, Any] = field(default_factory=dict)
    regressions: list[dict[str, Any]] = field(default_factory=list)

    # ---------- persistence ----------
    @classmethod
    def load(cls, path: Path) -> "Ledger":
        if not path.exists():
            return cls()
        raw = json.loads(path.read_text(encoding="utf-8"))
        if raw.get("schema") != SCHEMA:
            raise ValueError(f"unsupported ledger schema: {raw.get('schema')!r}")
        known = set(Entry.__dataclass_fields__)
        return cls(
            entries={k: Entry(**{f: v for f, v in e.items() if f in known})
                     for k, e in raw.get("scenarios", {}).items()},
            spec_lock=raw.get("spec_lock", {}),
            regressions=raw.get("regressions", []),
        )

    def save(self, path: Path) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        payload = {
            "schema": SCHEMA,
            "generated_at": _now(),
            "summary": self.summary(),
            "spec_lock": self.spec_lock,
            "regressions": self.regressions,
            "scenarios": {k: asdict(v) for k, v in sorted(self.entries.items())},
        }
        path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")

    # ---------- sync with the spec ----------
    def sync_spec(self, features: list) -> dict[str, int]:
        """Reconcile the ledger against the current spec. Green that no longer matches
        its scenario's meaning becomes stale."""
        stats = {"added": 0, "staled": 0, "orphaned": 0, "unchanged": 0, "restored": 0}
        seen: set[str] = set()
        for f in features:
            for s in f.scenarios:
                if not s.rid:
                    continue
                seen.add(s.rid)
                h = s.spec_hash(f.background)
                e = self.entries.get(s.rid)
                if e is None:
                    st = f.story
                    e = Entry(rid=s.rid, name=s.name, feature=f.name,
                              feature_file=f.file, tags=list(s.all_tags), spec_hash=h,
                              epic=s.epic or f.epic, actor=st.actor,
                              capability=st.capability, benefit=st.benefit)
                    e.record("added", spec_hash=h)
                    self.entries[s.rid] = e
                    stats["added"] += 1
                    continue
                e.name, e.feature, e.feature_file = s.name, f.name, f.file
                e.tags = list(s.all_tags)
                st = f.story
                e.epic, e.actor = s.epic or f.epic, st.actor
                e.capability, e.benefit = st.capability, st.benefit
                if e.status == ORPHAN:
                    # orphan is not an absorbing state; a restored scenario must prove itself
                    e.status = PENDING
                    e.verified_spec_hash = ""
                    e.record("restored_to_spec")
                    stats["restored"] = stats.get("restored", 0) + 1
                if e.spec_hash != h:
                    prev = e.status
                    e.spec_hash = h
                    if prev == GREEN:
                        e.status = STALE
                        e.record("spec_changed", was=prev, spec_hash=h)
                        stats["staled"] += 1
                    else:
                        e.record("spec_changed", was=prev, spec_hash=h)
                else:
                    stats["unchanged"] += 1
        for rid, e in self.entries.items():
            if rid not in seen and e.status != ORPHAN:
                e.status = ORPHAN
                e.record("removed_from_spec")
                stats["orphaned"] += 1
        return stats

    # ---------- sync with a test run ----------
    def apply_run(self, results: dict[str, dict[str, Any]], run_id: str,
                  blocked: str = "") -> dict[str, Any]:
        """results: {rid: {"outcome": "passed"|"failed", "detail": str, "nodeid": str}}

        `blocked` carries a collection/import error. When the suite could not be collected we
        cannot honestly claim "no test is bound to this scenario" -- we only know the suite did
        not run -- so unresolved scenarios go red with that error rather than quietly pending.
        """
        now = _now()
        new_regressions: list[dict[str, Any]] = []
        for rid, e in self.entries.items():
            if e.status == ORPHAN:
                continue
            r = results.get(rid)
            if r is None:
                if blocked:
                    if e.status == GREEN:
                        reg = {"at": now, "rid": rid, "name": e.name, "run": run_id,
                               "detail": "suite stopped collecting"}
                        self.regressions.append(reg)
                        new_regressions.append(reg)
                        e.record("REGRESSION", run=run_id)
                    e.status = RED
                    e.failure = blocked[:2000]
                    e.last_run_at = now
                    continue
                # No test bound to this scenario. Never green.
                if e.status == GREEN:
                    reg = {"at": now, "rid": rid, "name": e.name, "run": run_id,
                           "detail": "the test that proved this scenario is gone"}
                    self.regressions.append(reg)
                    new_regressions.append(reg)
                    e.record("REGRESSION", run=run_id, why="test_unbound")
                elif e.status in (STALE, RED):
                    e.record("test_unbound", was=e.status)
                e.verified_spec_hash = ""
                e.status = PENDING
                e.failure = "no test bound to this scenario"
                continue
            e.last_run_at = now
            if r["outcome"] == "passed":
                if e.status == RED:
                    e.record("fixed", run=run_id)
                e.status = GREEN
                e.verified_spec_hash = e.spec_hash
                e.last_green_at = now
                e.failure = None
            else:
                was = e.status
                if was == GREEN:
                    reg = {"at": now, "rid": rid, "name": e.name, "run": run_id,
                           "detail": r.get("detail", "")[:400]}
                    self.regressions.append(reg)
                    new_regressions.append(reg)
                    e.record("REGRESSION", run=run_id)
                e.status = RED
                e.failure = r.get("detail", "")[:2000]
        # tests claiming rids we do not know about
        unknown = [rid for rid in results if rid not in self.entries]
        return {"regressions": new_regressions, "unknown_rids": unknown,
                "summary": self.summary()}

    # ---------- views ----------
    def summary(self) -> dict[str, int]:
        out = {GREEN: 0, RED: 0, PENDING: 0, STALE: 0, ORPHAN: 0}
        for e in self.entries.values():
            out[e.status] = out.get(e.status, 0) + 1
        out["total"] = len(self.entries)
        live = out["total"] - out[ORPHAN]
        out["completion_pct"] = round(100 * out[GREEN] / live, 1) if live else 0.0
        return out

    def by_status(self, status: str) -> list[Entry]:
        return [e for e in self.entries.values() if e.status == status]
