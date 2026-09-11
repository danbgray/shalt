"""A backend that drives a role through an OpenAI-compatible chat-completions API.

xAI's API is OpenAI-compatible, so this one adapter covers Grok and any other endpoint that
speaks the same protocol. The model does its work through four scoped tools rather than a
shell, which makes the sandbox far tighter than a CLI: it can only name paths, and every path
is resolved inside the stage before anything is opened.

That scoping is convenience, not the guarantee. The real guarantee is still `roles.run_role`,
which hashes the workspace around the turn and rolls it back. This layer is the first of the
three, not a replacement for them.
"""
from __future__ import annotations

import json
import os
import time
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path

MAX_READ = 60_000
MAX_WRITE = 400_000


class ToolPathError(ValueError):
    pass


@dataclass
class Preset:
    base_url: str
    model: str
    key_env: str


PRESETS: dict[str, Preset] = {
    # Model names move fast; override with --model if xAI has renamed it.
    "grok": Preset("https://api.x.ai/v1", "grok-4", "XAI_API_KEY"),
    "openai": Preset("https://api.openai.com/v1", "gpt-4.1", "OPENAI_API_KEY"),
}

TOOLS = [
    {"type": "function", "function": {
        "name": "list_files",
        "description": "List every file you can see, with its size in bytes.",
        "parameters": {"type": "object", "properties": {}, "required": []}}},
    {"type": "function", "function": {
        "name": "read_file",
        "description": "Read one file.",
        "parameters": {"type": "object", "properties": {
            "path": {"type": "string", "description": "Path relative to your working root."}},
            "required": ["path"]}}},
    {"type": "function", "function": {
        "name": "write_file",
        "description": "Create or overwrite one file with the complete content.",
        "parameters": {"type": "object", "properties": {
            "path": {"type": "string", "description": "Path relative to your working root."},
            "content": {"type": "string", "description": "The entire file content."}},
            "required": ["path", "content"]}}},
    {"type": "function", "function": {
        "name": "done",
        "description": "Call this when the work is complete.",
        "parameters": {"type": "object", "properties": {
            "summary": {"type": "string"}}, "required": ["summary"]}}},
]


def _safe_path(stage: Path, raw: str) -> Path:
    """Resolve `raw` inside the stage, or refuse.

    Rejects absolute paths, traversal that climbs out, and any symlinked component -- a symlink
    inside the stage is a write to wherever it points.
    """
    cleaned = raw.strip() if raw else ""
    if not cleaned or cleaned in (".", "/"):
        raise ToolPathError("a file path is required")
    # An absolute path is refused outright rather than reinterpreted as root-relative:
    # silently turning "/etc/passwd" into "<stage>/etc/passwd" would hide the intent.
    if cleaned.startswith("/") or cleaned.startswith("\\") or Path(cleaned).is_absolute():
        raise ToolPathError(
            f"absolute paths are not allowed: {raw!r} -- use a path relative to your root")
    candidate = Path(cleaned)
    if any(part == ".." for part in candidate.parts):
        raise ToolPathError(f"path escapes the working root: {raw!r}")
    target = (stage / candidate)
    stage_res = stage.resolve()
    probe = target
    while True:
        if probe.is_symlink():
            raise ToolPathError(f"symlinked path is not allowed: {raw!r}")
        if probe == stage or probe.parent == probe:
            break
        probe = probe.parent
    try:
        resolved_parent = target.parent.resolve()
    except OSError as exc:
        raise ToolPathError(str(exc)) from exc
    if resolved_parent != stage_res and stage_res not in resolved_parent.parents:
        raise ToolPathError(f"path escapes the working root: {raw!r}")
    return target


def _tree(stage: Path) -> str:
    out = []
    for p in sorted(stage.rglob("*")):
        if p.is_file() and "__pycache__" not in p.parts:
            out.append(f"{p.relative_to(stage)}  ({p.stat().st_size} bytes)")
    return "\n".join(out) or "(no files yet)"


def _dispatch(stage: Path, name: str, args: dict) -> str:
    if name == "list_files":
        return _tree(stage)
    if name == "read_file":
        p = _safe_path(stage, args.get("path", ""))
        if not p.is_file():
            return f"ERROR: no such file: {args.get('path')!r}"
        return p.read_text(encoding="utf-8", errors="replace")[:MAX_READ]
    if name == "write_file":
        p = _safe_path(stage, args.get("path", ""))
        content = args.get("content", "")
        if len(content) > MAX_WRITE:
            return f"ERROR: content too large ({len(content)} bytes)"
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(content, encoding="utf-8")
        return f"wrote {p.relative_to(stage)} ({len(content)} bytes)"
    if name == "done":
        return "acknowledged"
    return f"ERROR: unknown tool {name!r}"


@dataclass
class OpenAICompatBackend:
    """Drives one role through an OpenAI-compatible /chat/completions endpoint."""
    base_url: str
    model: str
    api_key: str
    name: str = "openai-compat"
    max_steps: int = 40
    timeout: int = 180
    temperature: float = 0.0
    transport: object | None = None   # injectable for testing
    last_usage: dict = field(default_factory=dict)

    @classmethod
    def from_preset(cls, preset_name: str, model: str | None = None,
                    api_key: str | None = None, base_url: str | None = None,
                    **kw) -> "OpenAICompatBackend":
        p = PRESETS[preset_name]
        key = api_key or os.environ.get(p.key_env, "")
        if not key:
            raise RuntimeError(
                f"no API key for '{preset_name}': set {p.key_env} in the environment")
        return cls(base_url=base_url or p.base_url, model=model or p.model,
                   api_key=key, name=preset_name, **kw)

    # ------------------------------------------------------------------ http
    def _post(self, payload: dict) -> dict:
        if self.transport is not None:
            return self.transport(payload)
        body = json.dumps(payload).encode("utf-8")
        req = urllib.request.Request(
            f"{self.base_url.rstrip('/')}/chat/completions", data=body,
            headers={"Content-Type": "application/json",
                     "Authorization": f"Bearer {self.api_key}"})
        last: Exception | None = None
        for attempt in range(4):
            try:
                with urllib.request.urlopen(req, timeout=self.timeout) as r:
                    return json.loads(r.read().decode("utf-8"))
            except urllib.error.HTTPError as e:
                detail = e.read().decode("utf-8", "replace")[:600]
                if e.code in (429, 500, 502, 503, 529) and attempt < 3:
                    time.sleep(2 ** attempt)
                    last = RuntimeError(f"HTTP {e.code}: {detail}")
                    continue
                hint = ""
                if e.code in (400, 404) and "model" in detail.lower():
                    ids = self.available_models()
                    if ids:
                        hint = (f"\n  '{self.model}' was rejected. Models this key can see: "
                                f"{', '.join(ids[:12])}\n  Pass one with --model.")
                raise RuntimeError(
                    f"{self.name} API error HTTP {e.code}: {detail}{hint}") from e
            except urllib.error.URLError as e:
                if attempt < 3:
                    time.sleep(2 ** attempt)
                    last = RuntimeError(f"connection failed: {e.reason}")
                    continue
                raise RuntimeError(
                    f"could not reach {self.base_url}: {e.reason}. "
                    f"If this environment restricts outbound traffic, that host may be blocked."
                ) from e
        raise last or RuntimeError("request failed")

    def available_models(self) -> list[str]:
        """Best-effort model list, used to make a wrong --model self-correcting."""
        try:
            req = urllib.request.Request(
                f"{self.base_url.rstrip('/')}/models",
                headers={"Authorization": f"Bearer {self.api_key}"})
            with urllib.request.urlopen(req, timeout=30) as r:
                data = json.loads(r.read().decode("utf-8"))
            return sorted(m.get("id", "") for m in data.get("data", []) if m.get("id"))
        except Exception:
            return []

    # ------------------------------------------------------------------ loop
    def run(self, role: str, prompt: str, stage: Path) -> str:
        from .backends import _ROLE_SYSTEM
        stage = Path(stage)
        messages = [
            {"role": "system", "content": _ROLE_SYSTEM[role] +
             "\n\nYou work only through the provided tools. Every path is relative to your "
             "working root. Read what you need first, then write complete files -- never "
             "fragments or diffs. Call done() when finished."},
            {"role": "user", "content": f"{prompt}\n\nFiles you can see:\n{_tree(stage)}"},
        ]
        transcript: list[str] = []
        for step in range(self.max_steps):
            data = self._post({"model": self.model, "messages": messages,
                               "tools": TOOLS, "tool_choice": "auto",
                               "temperature": self.temperature})
            if data.get("usage"):
                for k, v in data["usage"].items():
                    if isinstance(v, int):
                        self.last_usage[k] = self.last_usage.get(k, 0) + v
            choices = data.get("choices") or []
            if not choices:
                raise RuntimeError(f"{self.name}: empty response: {str(data)[:400]}")
            msg = choices[0].get("message", {})
            calls = msg.get("tool_calls") or []
            messages.append({"role": "assistant",
                             "content": msg.get("content") or "",
                             **({"tool_calls": calls} if calls else {})})
            if not calls:
                transcript.append(msg.get("content") or "")
                break
            finished = False
            for call in calls:
                fn = call.get("function", {})
                fname = fn.get("name", "")
                try:
                    args = json.loads(fn.get("arguments") or "{}")
                except json.JSONDecodeError:
                    result = "ERROR: arguments were not valid JSON"
                else:
                    try:
                        result = _dispatch(stage, fname, args)
                    except ToolPathError as e:
                        # refused here, and refused again by the guard if it ever slipped
                        result = f"REFUSED: {e}"
                    except OSError as e:
                        result = f"ERROR: {e}"
                transcript.append(f"[{fname}] {result[:200]}")
                messages.append({"role": "tool", "tool_call_id": call.get("id", ""),
                                 "content": result[:MAX_READ]})
                if fname == "done":
                    finished = True
            if finished:
                break
        else:
            transcript.append(f"(stopped after {self.max_steps} steps)")
        return "\n".join(transcript)
