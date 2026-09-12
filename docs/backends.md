# Backends

A backend is how a role's turn actually gets done. The protocol is deliberately tiny, because
everything that matters — isolation, rollback, attribution — lives outside it.

```python
class Backend(Protocol):
    name: str
    def run(self, role: str, prompt: str, stage: Path) -> str: ...
```

The backend receives a **staged** directory containing only the zones its role may read, does its
work in place, and returns a transcript. `roles.run_role` then verifies and mirrors back. A
backend cannot opt out of the guard: it is wrapped by the caller, not invoked by the callee.

| backend | selector | needs | used for |
|---|---|---|---|
| fixture | `--backend fixture --fixtures <dir>` | nothing | the test suite and the offline demo |
| Claude CLI | `--backend claude-cli` | `claude` on PATH | local development against Claude Code |
| Grok | `--backend grok` | `XAI_API_KEY` | xAI, via its OpenAI-compatible API |
| OpenAI | `--backend openai` | `OPENAI_API_KEY` | same adapter, different preset |

`--model` overrides the default; `--base-url` points the OpenAI-compatible adapter at any other
endpoint that speaks the protocol.

## Role system prompts

Held in `backends._ROLE_SYSTEM`, one per role, and shared by every backend. They state the
isolation as fact rather than as a request — the stepwright is told it sees no implementation
*because that is true*, which is a different kind of instruction from "please do not look."

The stepwright's prompt contains the one line that matters most in the whole system:

> Never weaken an assertion to make it easier to satisfy; you are the oracle, not the builder.

## FixtureBackend

Replays recorded turns from `fixtures/<role>/turn*/`, copying each turn directory over the stage
verbatim. Deterministic, offline, no key. Turn index advances per role and clamps at the last
recorded turn, so a build loop that runs longer than the fixture set simply replays the final
state.

An optional `_note.txt` in a turn directory is returned as the transcript instead of being
copied — used to label adversarial fixtures.

The project ships four fixture sets:

| set | models |
|---|---|
| `honest` | a normal build: turn 1 uses floats and fails the rounding scenario, turn 2 uses `Decimal` and passes |
| `cheating` | an implementer that writes into `steps/` to weaken the assertion |
| `overfit` | an implementer that hardcodes the visible examples |
| `weak-oracle` | a **stepwright** whose assertions do not check the value |

## ClaudeCLIBackend

Runs each role as one headless `claude -p` turn with the stage as `cwd`:

```
claude -p <prompt>
  --output-format text
  --permission-mode acceptEdits
  --allowedTools Read,Write,Edit,Glob,Grep
  --append-system-prompt <role system prompt>
```

`Bash` is deliberately absent from `--allowedTools`. Note the caveat in
[isolation.md](isolation.md#known-gaps): read confidentiality for this backend rests on Claude
Code's own permission model, and that has not been verified against a live run.

## OpenAICompatBackend

xAI's API is OpenAI-compatible, so one adapter covers Grok, OpenAI, and anything else on that
protocol. Standard library only — `urllib`, no SDK, no dependency.

The model works through **four scoped tools** rather than a shell:

| tool | arguments | behaviour |
|---|---|---|
| `list_files` | — | every file it can see, with sizes |
| `read_file` | `path` | up to 60 KB; a missing file returns an error string, not an exception |
| `write_file` | `path`, `content` | complete file content, up to 400 KB |
| `done` | `summary` | ends the turn |

### The path sandbox

Every path is resolved inside the stage before anything is opened. Refused:

- **absolute paths** — refused outright, *not* reinterpreted as stage-relative. Silently turning
  `/etc/passwd` into `<stage>/etc/passwd` would hide the intent, which was a real bug caught by
  `test_paths_that_leave_the_working_root_are_refused`;
- **traversal** — any `..` component;
- **symlinked components** — every ancestor of the target is checked, because a symlink inside
  the stage is a write to wherever it points;
- **a resolved parent outside the stage.**

A refusal is returned to the model **as a tool result** (`REFUSED: ...`), not raised. The model
can then correct course instead of losing the turn.
→ `test_a_refused_path_is_reported_back_to_the_model_not_raised`

This sandbox is layer one of three. If a tool call ever did land outside the role's zone, the
workspace guard still rejects and rolls back the turn.
→ `test_the_workspace_guard_still_applies_to_an_api_driven_role`

### The loop

Up to `max_steps` (default 40) exchanges. Tool calls are dispatched in order and each result
appended as a `role: "tool"` message. The loop ends on `done`, on a response with no tool calls,
or at the step cap. Malformed tool arguments become an error string rather than an exception.
→ `test_malformed_tool_arguments_do_not_crash_the_loop`, `test_the_loop_stops_at_max_steps`

### Errors and retries

`429`, `500`, `502`, `503`, `529` retry with exponential backoff, four attempts. Everything else
raises with the provider name and the response body. A `400`/`404` mentioning the model triggers
a `GET /models` lookup so the error names what the key can actually see:

```
grok API error HTTP 404: The model `grok-4` does not exist
  'grok-4' was rejected. Models this key can see: grok-4-fast, grok-4-latest
  Pass one with --model.
```

Token usage is accumulated across the turn in `last_usage`.

## Testing an API backend with no API

The adapter is fully exercised without any external call. `tests/mock_llm.py` is a real HTTP
server speaking the OpenAI/xAI chat-completions wire format, and the **entire pipeline** —
`init`, `author`, `approve`, `steps`, `build` — runs over it via subprocess, including the
overfit demo.

The important choice was to mock at the **protocol** boundary rather than the adapter boundary.
Mocking the adapter would have tested nothing; mocking the wire format tests request
construction, the tool loop, refusals, retries, usage accounting and the subprocess plumbing.

What it cannot test is judgement. Nothing in this repository demonstrates whether a real model
writes good Gherkin, whether the stepwright writes honest assertions, or how many turns a real
build takes.

## Adding a backend

One class with a `run(role, prompt, stage) -> str` method, registered in
`backends.make_backend` and added to `backends.BACKENDS`. It needs no knowledge of zones,
hashing, the ledger or rollback — those are applied around it.
