"""Tests for the OpenAI-compatible backend (Grok and anything else on that protocol).

These prove the adapter and the sandbox. They cannot prove anything about a real model's
judgement -- that needs a key and network access to the provider.
"""
import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

from shalt.api_backend import OpenAICompatBackend, ToolPathError, _safe_path, _dispatch
from shalt.integrity import IntegrityViolation
from shalt.ledger import Ledger
from shalt.roles import run_role

from mock_llm import MockChat, fixture_responder, reply, role_of, tool_call

FIXTURES = Path(__file__).resolve().parents[1] / "examples" / "invoice" / "fixtures"


# --------------------------------------------------------------- path sandbox
@pytest.mark.parametrize("bad", [
    "../../../steps/test_x.py", "/etc/passwd", "src/../../steps/x.py",
    "..", "", "/", "./../outside.py",
])
def test_paths_that_leave_the_working_root_are_refused(tmp_path, bad):
    (tmp_path / "src").mkdir()
    with pytest.raises(ToolPathError):
        _safe_path(tmp_path, bad)


def test_symlinked_path_is_refused(tmp_path):
    (tmp_path / "src").mkdir()
    outside = tmp_path.parent / "outside"
    outside.mkdir(exist_ok=True)
    os.symlink(outside, tmp_path / "src" / "link")
    with pytest.raises(ToolPathError):
        _safe_path(tmp_path, "src/link/evil.py")


def test_ordinary_nested_paths_are_allowed(tmp_path):
    p = _safe_path(tmp_path, "src/pkg/mod.py")
    assert p == tmp_path / "src" / "pkg" / "mod.py"


def test_write_then_read_round_trips(tmp_path):
    assert "wrote" in _dispatch(tmp_path, "write_file",
                                {"path": "src/a.py", "content": "x = 1\n"})
    assert _dispatch(tmp_path, "read_file", {"path": "src/a.py"}) == "x = 1\n"
    assert "src/a.py" in _dispatch(tmp_path, "list_files", {})


def test_reading_a_missing_file_is_an_error_message_not_a_crash(tmp_path):
    assert "ERROR" in _dispatch(tmp_path, "read_file", {"path": "nope.py"})


# --------------------------------------------------------------- the tool loop
def _backend(base_url, **kw):
    return OpenAICompatBackend(base_url=base_url, model="grok-4", api_key="test",
                               name="grok", **kw)


def test_the_loop_writes_files_and_stops_on_done(tmp_path):
    def responder(payload, i):
        if i == 0:
            return reply([tool_call("c1", "write_file", path="src/m.py", content="ok\n")])
        return reply([tool_call("c2", "done", summary="finished")])

    with MockChat(responder) as srv:
        out = _backend(srv.base_url).run("implementer", "build it", tmp_path)
    assert (tmp_path / "src" / "m.py").read_text() == "ok\n"
    assert "done" in out
    assert len(srv.calls) == 2


def test_the_request_carries_the_model_tools_and_role_system_prompt(tmp_path):
    with MockChat(lambda p, i: reply([tool_call("c", "done", summary="x")])) as srv:
        _backend(srv.base_url).run("stepwright", "write steps", tmp_path)
    payload = srv.calls[0]
    assert payload["model"] == "grok-4"
    assert {t["function"]["name"] for t in payload["tools"]} == {
        "list_files", "read_file", "write_file", "done"}
    assert role_of(payload) == "stepwright"
    assert "STEPWRIGHT" in payload["messages"][0]["content"]


def test_a_refused_path_is_reported_back_to_the_model_not_raised(tmp_path):
    seen = {}

    def responder(payload, i):
        if i == 0:
            return reply([tool_call("c1", "write_file",
                                    path="../../../steps/test_x.py", content="assert False\n")])
        seen["tool_result"] = payload["messages"][-1]["content"]
        return reply([tool_call("c2", "done", summary="gave up")])

    with MockChat(responder) as srv:
        _backend(srv.base_url).run("implementer", "build it", tmp_path)
    assert seen["tool_result"].startswith("REFUSED")
    assert not (tmp_path.parent / "steps").exists()


def test_malformed_tool_arguments_do_not_crash_the_loop(tmp_path):
    def responder(payload, i):
        if i == 0:
            return reply([{"id": "c1", "type": "function",
                           "function": {"name": "write_file", "arguments": "{not json"}}])
        return reply([tool_call("c2", "done", summary="ok")])

    with MockChat(responder) as srv:
        out = _backend(srv.base_url).run("implementer", "go", tmp_path)
    assert "not valid JSON" in out


def test_transient_errors_are_retried(tmp_path):
    def responder(payload, i):
        if i == 0:
            return 429, {"error": "rate limited"}
        return reply([tool_call("c", "done", summary="ok")])

    with MockChat(responder) as srv:
        b = _backend(srv.base_url)
        b.run("implementer", "go", tmp_path)
    assert len(srv.calls) == 2, "a 429 must be retried, not surfaced"


def test_a_permanent_error_names_the_provider(tmp_path):
    with MockChat(lambda p, i: (401, {"error": "bad key"})) as srv:
        with pytest.raises(RuntimeError, match="grok API error HTTP 401"):
            _backend(srv.base_url).run("implementer", "go", tmp_path)


def test_the_loop_stops_at_max_steps(tmp_path):
    with MockChat(lambda p, i: reply([tool_call("c", "list_files")])) as srv:
        out = _backend(srv.base_url, max_steps=3).run("implementer", "go", tmp_path)
    assert "stopped after 3 steps" in out
    assert len(srv.calls) == 3


def test_usage_is_accumulated(tmp_path):
    with MockChat(lambda p, i: reply([tool_call("c", "done", summary="x")])) as srv:
        b = _backend(srv.base_url)
        b.run("implementer", "go", tmp_path)
    assert b.last_usage["total_tokens"] == 15


# ------------------------------------------------- the guard still governs it
def test_the_workspace_guard_still_applies_to_an_api_driven_role(tmp_path):
    """The tool sandbox is the first layer, not the guarantee. If a tool call ever landed
    outside the role's zone, run_role must still reject the turn."""
    for z in ("spec", "steps", "contract", "src", ".shalt"):
        (tmp_path / z).mkdir(parents=True, exist_ok=True)
    (tmp_path / "steps" / "test_x.py").write_text("def test_real():\n    assert True\n")
    Ledger().save(tmp_path / ".shalt" / "ledger.json")

    def responder(payload, i):
        if i == 0:
            # a path the sandbox allows but the implementer's zone does not
            return reply([tool_call("c1", "write_file", path="steps/test_x.py",
                                    content="assert False  # weakened\n")])
        return reply([tool_call("c2", "done", summary="x")])

    with MockChat(responder) as srv:
        with pytest.raises(IntegrityViolation) as ei:
            run_role(tmp_path, "implementer", "go", _backend(srv.base_url))
    assert "steps" in ei.value.offences
    assert "assert True" in (tmp_path / "steps" / "test_x.py").read_text()


# ------------------------------------------------- the whole pipeline over HTTP
def test_full_pipeline_runs_over_the_http_adapter(tmp_path):
    """author -> approve -> steps -> build, every model turn served over real HTTP."""
    root = tmp_path / "ws"
    env = dict(os.environ, XAI_API_KEY="test",
               PYTHONPATH=str(Path(__file__).resolve().parents[1]))

    with MockChat(fixture_responder(FIXTURES / "honest")) as srv:
        def cli(*args, expect=0):
            cmd = [sys.executable, "-m", "shalt.cli", "--root", str(root),
                   "--backend", "grok", "--base-url", srv.base_url, *args]
            p = subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=300)
            assert p.returncode == expect, p.stdout + p.stderr
            return p.stdout

        cli("init")
        cli("author", "Parse invoices and total them exactly.")
        assert (root / "spec" / "invoice.feature").exists()
        cli("approve", "--yes", "--by", "dan@rivlet.io")
        cli("steps")
        assert (root / "steps" / "test_invoice.py").exists()
        assert (root / "contract" / "interface.md").exists()
        out = cli("build", "--max-turns", "5")

    assert "100.0%" in out, out
    led = Ledger.load(root / ".shalt" / "ledger.json")
    assert led.summary()["green"] == led.summary()["total"] > 5
    assert led.spec_lock["approved_by"] == "dan@rivlet.io"
    assert srv.calls, "the model was actually called over HTTP"


def test_full_pipeline_over_http_still_catches_an_overfitting_model(tmp_path):
    root = tmp_path / "ws"
    env = dict(os.environ, XAI_API_KEY="test",
               PYTHONPATH=str(Path(__file__).resolve().parents[1]))
    responder = fixture_responder(FIXTURES / "honest")
    overfit = fixture_responder(FIXTURES / "overfit")

    def mixed(payload, i):
        return (overfit if role_of(payload) == "implementer" else responder)(payload, i)

    with MockChat(mixed) as srv:
        def cli(*args):
            cmd = [sys.executable, "-m", "shalt.cli", "--root", str(root),
                   "--backend", "grok", "--base-url", srv.base_url, *args]
            p = subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=300)
            return p.stdout + p.stderr

        cli("init"); cli("author", "x"); cli("approve", "--yes", "--by", "d"); cli("steps")
        out = cli("build", "--max-turns", "3")
    assert "OVERFIT" in out, out


def test_a_wrong_model_name_reports_what_is_available(tmp_path):
    def responder(payload, i):
        return 404, {"error": {"message": "The model `grok-4` does not exist"}}

    class Srv(MockChat):
        pass

    with MockChat(responder) as srv:
        b = _backend(srv.base_url)
        # serve GET /v1/models too
        import urllib.request
        real = b.available_models
        b.available_models = lambda: ["grok-4-fast", "grok-4-latest"]
        with pytest.raises(RuntimeError) as ei:
            b.run("implementer", "go", tmp_path)
    msg = str(ei.value)
    assert "grok-4-fast" in msg and "--model" in msg
