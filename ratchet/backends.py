"""Pluggable model backends for the three agent roles.

A backend receives a *staged* workspace containing only the zones its role may read, does its
work, and the caller copies back only the zones its role may write. Isolation is therefore a
property of the filesystem the role runs in, not of the wording of its prompt.
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
from pathlib import Path
from typing import Protocol


class Backend(Protocol):
    name: str
    def run(self, role: str, prompt: str, stage: Path) -> str: ...


class FixtureBackend:
    """Replays recorded turns from a directory. Offline, deterministic, used by the test
    suite and the worked example. Each turn directory is copied over the stage verbatim."""
    name = "fixture"

    def __init__(self, fixtures: Path):
        self.fixtures = Path(fixtures)
        self._turn: dict[str, int] = {}

    def run(self, role: str, prompt: str, stage: Path) -> str:
        n = self._turn.get(role, 0)
        self._turn[role] = n + 1
        candidates = sorted((self.fixtures / role).glob("turn*")) if (self.fixtures / role).exists() else []
        if not candidates:
            raise FileNotFoundError(f"no fixtures for role {role!r} in {self.fixtures}")
        turn = candidates[min(n, len(candidates) - 1)]
        note = ""
        for item in sorted(turn.iterdir()):
            if item.name == "_note.txt":
                note = item.read_text(encoding="utf-8")
                continue
            dest = stage / item.name
            if item.is_dir():
                shutil.copytree(item, dest, dirs_exist_ok=True)
            else:
                dest.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(item, dest)
        return f"[fixture {role} {turn.name}] {note}".strip()


class ClaudeCLIBackend:
    """Runs each role as a headless `claude -p` turn inside the staged directory."""
    name = "claude-cli"

    def __init__(self, model: str | None = None, timeout: int = 900,
                 extra_args: list[str] | None = None):
        self.model = model
        self.timeout = timeout
        self.extra_args = extra_args or []

    def run(self, role: str, prompt: str, stage: Path) -> str:
        cmd = ["claude", "-p", prompt,
               "--output-format", "text",
               "--permission-mode", "acceptEdits",
               "--allowedTools", "Read,Write,Edit,Glob,Grep",
               "--append-system-prompt", _ROLE_SYSTEM[role]]
        if self.model:
            cmd += ["--model", self.model]
        cmd += self.extra_args
        env = dict(os.environ)
        proc = subprocess.run(cmd, cwd=stage, capture_output=True, text=True,
                              env=env, timeout=self.timeout)
        if proc.returncode != 0:
            raise RuntimeError(f"claude cli failed ({proc.returncode}): {proc.stderr[-1500:]}")
        return proc.stdout


_ROLE_SYSTEM = {
    "author": (
        "You are the SPEC AUTHOR in a BDD pipeline. You translate a plain-English request into "
        "Gherkin feature files under spec/. Write scenarios that a non-engineer stakeholder could "
        "read and approve. One behaviour per scenario. Prefer concrete example values over vague "
        "wording. Do not write code, tests, or step definitions. Only create files under spec/."
    ),
    "stepwright": (
        "You are the STEPWRIGHT in a BDD pipeline. You see the approved Gherkin spec and NOTHING "
        "of the implementation -- that is deliberate. Write pytest-bdd step definitions under "
        "steps/ that bind each scenario to the behaviour it describes, and declare the public API "
        "surface you call in contract/interface.md. Import only from that declared surface. Never "
        "weaken an assertion to make it easier to satisfy; you are the oracle, not the builder. "
        "Only create files under steps/ and contract/."
    ),
    "implementer": (
        "You are the IMPLEMENTER in a BDD pipeline. You see the spec, the interface contract, and "
        "the failing test output -- you do NOT see the step definitions, and you cannot edit them. "
        "Write code under src/ that satisfies the specified behaviour against the contract. Do not "
        "special-case test inputs or hard-code expected outputs; implement the behaviour. Only "
        "create files under src/."
    ),
}


def make_backend(spec: str, fixtures: Path | None = None) -> Backend:
    if spec == "fixture":
        if fixtures is None:
            raise ValueError("fixture backend requires a fixtures directory")
        return FixtureBackend(fixtures)
    if spec == "claude-cli":
        return ClaudeCLIBackend()
    raise ValueError(f"unknown backend {spec!r} (expected 'fixture' or 'claude-cli')")
