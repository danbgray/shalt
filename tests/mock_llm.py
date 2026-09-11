"""A mock OpenAI/xAI-compatible chat-completions server.

Lets the whole pipeline run through the real HTTP adapter -- request construction, tool-call
parsing, the multi-step loop, tool dispatch, retries -- without reaching an external API. It
proves the plumbing. It cannot prove anything about the model's judgement.
"""
from __future__ import annotations

import json
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path


def tool_call(cid: str, name: str, **args) -> dict:
    return {"id": cid, "type": "function",
            "function": {"name": name, "arguments": json.dumps(args)}}


def reply(tool_calls=None, content="") -> dict:
    msg = {"role": "assistant", "content": content}
    if tool_calls:
        msg["tool_calls"] = tool_calls
    return {"choices": [{"message": msg, "finish_reason":
                         "tool_calls" if tool_calls else "stop"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}}


class MockChat:
    """Serves scripted responses. `responder(payload, call_index) -> dict | (status, dict)`."""

    def __init__(self, responder):
        self.responder = responder
        self.calls: list[dict] = []
        self._server = None
        self._thread = None

    @property
    def base_url(self) -> str:
        host, port = self._server.server_address
        return f"http://127.0.0.1:{port}/v1"

    def __enter__(self):
        outer = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_POST(self):
                n = int(self.headers.get("Content-Length", 0))
                payload = json.loads(self.rfile.read(n) or b"{}")
                outer.calls.append(payload)
                out = outer.responder(payload, len(outer.calls) - 1)
                status, body = out if isinstance(out, tuple) else (200, out)
                raw = json.dumps(body).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(raw)))
                self.end_headers()
                self.wfile.write(raw)

        self._server = HTTPServer(("127.0.0.1", 0), Handler)
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)
        self._thread.start()
        return self

    def __exit__(self, *exc):
        self._server.shutdown()
        self._server.server_close()
        self._thread.join(timeout=5)
        return False


def role_of(payload: dict) -> str:
    """Recover which role is talking from the system prompt."""
    system = payload["messages"][0]["content"]
    for role, marker in (("author", "SPEC AUTHOR"), ("stepwright", "STEPWRIGHT"),
                         ("implementer", "IMPLEMENTER")):
        if marker in system:
            return role
    raise AssertionError("unknown role in system prompt")


def fixture_responder(fixtures: Path, turns: dict | None = None):
    """Replays the recorded fixture files, but delivered as real tool calls over HTTP.

    Same content the offline demo uses; entirely different code path.
    """
    fixtures = Path(fixtures)
    state: dict[str, int] = {}
    turns = turns or {}

    def responder(payload, i):
        role = role_of(payload)
        # one exchange per turn: emit every file, then done
        if payload["messages"][-1]["role"] == "tool":
            return reply(content=f"{role} finished")
        n = state.get(role, 0)
        state[role] = n + 1
        wanted = turns.get(role, n)
        candidates = sorted((fixtures / role).glob("turn*"))
        turn = candidates[min(wanted, len(candidates) - 1)]
        calls = []
        for f in sorted(turn.rglob("*")):
            if f.is_file() and f.name != "_note.txt":
                calls.append(tool_call(f"c{len(calls)}", "write_file",
                                       path=str(f.relative_to(turn)),
                                       content=f.read_text(encoding="utf-8")))
        calls.append(tool_call(f"c{len(calls)}", "done", summary=f"{role} turn {n}"))
        return reply(calls)

    return responder
