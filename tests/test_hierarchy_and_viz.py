"""Tests for the breakdown, the language-agnostic runner config, and the visual outputs."""
import json
from pathlib import Path

import pytest

from shalt.config import Config, PRESETS, write_config
from shalt.ledger import Ledger
from shalt.narrative import parse_story, slug
from shalt.reports import parse_cucumber_json, parse_cucumber_messages, read_report
from shalt.spec import load_specs, stamp_rids
from shalt.viz import (actors, build_tree, dashboard_html, mermaid_hierarchy,
                         mermaid_pipeline, mermaid_usecase, roll_up, terse_failure)

TWO_EPICS = {
    "billing.feature": '''\
@epic:billing
Feature: Invoice totals

  As a billing clerk
  I want invoice totals computed exactly
  So that customers are never billed the wrong amount

  @rid:S-00000001
  Scenario: One line
    Given a line
    Then the total is "1.00"

  @holdout @rid:S-00000002
  Scenario: Unseen
    Given a line
    Then the total is "2.00"
''',
    "collections.feature": '''\
@epic:collections
Feature: Overdue reminders

  As a credit controller
  I want reminders to escalate
  So that we chase while it is collectable

  @rid:S-00000003
  Scenario: A week late
    Given 7 days
    Then the reminder is "gentle"
''',
}


def _spec(tmp_path, files=None):
    d = tmp_path / "spec"
    d.mkdir(parents=True, exist_ok=True)
    for name, text in (files or TWO_EPICS).items():
        p = d / name
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text, encoding="utf-8")
    return d


def _ledger(tmp_path, files=None):
    d = _spec(tmp_path, files)
    led = Ledger()
    led.sync_spec(load_specs(d))
    return led, load_specs(d)


# ------------------------------------------------------------------ narrative
@pytest.mark.parametrize("text,actor,cap,benefit", [
    ("As a clerk\nI want totals\nSo that nobody is overbilled", "clerk", "totals",
     "nobody is overbilled"),
    ("In order to avoid disputes\nAs a finance lead\nI want exact totals", "finance lead",
     "exact totals", "avoid disputes"),
    ("As the auditor\nI need a trail\nSo that we pass review", "auditor", "a trail",
     "we pass review"),
])
def test_story_clause_orderings(text, actor, cap, benefit):
    st = parse_story(text)
    assert (st.actor, st.capability, st.benefit) == (actor, cap, benefit)


def test_prose_that_is_not_a_story_yields_nothing_and_says_what_is_missing():
    st = parse_story("Some general notes about billing.")
    assert not st.complete
    assert len(st.missing) == 3


def test_slug_is_stable_and_safe():
    assert slug("Billing Clerk!") == "billing-clerk"
    assert slug("") == "unassigned"


# ------------------------------------------------------------------ hierarchy
def test_epic_comes_from_the_feature_tag_and_is_inherited_by_scenarios(tmp_path):
    led, features = _ledger(tmp_path)
    assert {e.epic for e in led.entries.values()} == {"billing", "collections"}
    assert led.entries["S-00000001"].actor == "billing clerk"
    assert led.entries["S-00000003"].actor == "credit controller"


def test_epic_falls_back_to_the_directory_under_spec(tmp_path):
    files = {"payments/checkout.feature": '''\
Feature: Checkout

  As a shopper
  I want to pay
  So that I get my order

  @rid:S-000000aa
  Scenario: Card accepted
    Given a card
    Then it is accepted
'''}
    led, features = _ledger(tmp_path, files)
    assert features[0].epic == "payments"
    assert led.entries["S-000000aa"].epic == "payments"


def test_tree_nests_epic_story_scenario(tmp_path):
    led, _ = _ledger(tmp_path)
    tree = build_tree(list(led.entries.values()))
    assert [n.label for n in tree] == ["billing", "collections"]
    assert [c.label for c in tree[0].children] == ["Invoice totals"]
    assert len(tree[0].children[0].children) == 2


@pytest.mark.parametrize("children,expected", [
    (["green", "green"], "green"),
    (["green", "red"], "red"),
    (["green", "stale"], "stale"),
    (["green", "pending"], "pending"),
    (["red", "pending"], "red"),
    ([], "pending"),
])
def test_a_parent_is_never_greener_than_its_children(children, expected):
    assert roll_up(children) == expected


def test_rolled_up_status_is_not_optimistic_in_the_tree(tmp_path):
    led, _ = _ledger(tmp_path)
    led.apply_run({"S-00000001": {"outcome": "passed", "detail": "", "nodeid": "n"},
                   "S-00000002": {"outcome": "failed", "detail": "x", "nodeid": "n"},
                   "S-00000003": {"outcome": "passed", "detail": "", "nodeid": "n"}}, "r1")
    tree = build_tree(list(led.entries.values()))
    billing = next(n for n in tree if n.label == "billing")
    assert billing.status == "red", "one failing child must sink the epic"


def test_actors_map_to_the_capabilities_they_want(tmp_path):
    led, _ = _ledger(tmp_path)
    acts = actors(list(led.entries.values()))
    assert set(acts) == {"billing clerk", "credit controller"}
    assert acts["billing clerk"][0]["capability"] == "invoice totals computed exactly"
    assert len(acts["billing clerk"]) == 1, "one story, listed once, not once per scenario"


# ------------------------------------------------------------------ reports
def test_cucumber_json_binds_by_rid_tag_and_undefined_is_not_green():
    payload = json.dumps([{"uri": "f.feature", "elements": [
        {"type": "scenario", "tags": [{"name": "@rid:S-0000000a"}], "name": "ok",
         "steps": [{"result": {"status": "passed"}}]},
        {"type": "scenario", "tags": [{"name": "@rid:S-0000000b"}], "name": "undef",
         "steps": [{"keyword": "Given ", "name": "x", "result": {"status": "undefined"}}]},
        {"type": "scenario", "tags": [{"name": "@nope"}], "name": "unbound",
         "steps": [{"result": {"status": "passed"}}]},
    ]}])
    r = parse_cucumber_json(payload)
    assert r["S-0000000a"]["outcome"] == "passed"
    assert r["S-0000000b"]["outcome"] == "failed", "an unimplemented step is not a pass"
    assert len(r) == 2, "a scenario with no rid is simply not bound"


def test_cucumber_json_outline_is_green_only_if_every_row_passes():
    payload = json.dumps([{"uri": "f.feature", "elements": [
        {"type": "scenario", "tags": [{"name": "@rid:S-0000000c"}], "name": "row1",
         "steps": [{"result": {"status": "passed"}}]},
        {"type": "scenario", "tags": [{"name": "@rid:S-0000000c"}], "name": "row2",
         "steps": [{"result": {"status": "failed", "error_message": "nope"}}]},
    ]}])
    assert parse_cucumber_json(payload)["S-0000000c"]["outcome"] == "failed"


def test_cucumber_messages_walks_the_envelope_stream():
    msgs = [
        {"pickle": {"id": "p1", "name": "ok", "uri": "f.feature",
                    "tags": [{"name": "@rid:S-0000000d"}]}},
        {"testCase": {"id": "tc1", "pickleId": "p1"}},
        {"testCaseStarted": {"id": "s1", "testCaseId": "tc1"}},
        {"testStepFinished": {"testCaseStartedId": "s1",
                              "testStepResult": {"status": "PASSED"}}},
        {"testStepFinished": {"testCaseStartedId": "s1",
                              "testStepResult": {"status": "SKIPPED"}}},
    ]
    r = parse_cucumber_messages("\n".join(json.dumps(m) for m in msgs))
    assert r["S-0000000d"]["outcome"] == "failed", "a skipped step is not a pass"


def test_a_missing_or_empty_report_is_empty_not_an_error(tmp_path):
    assert read_report(tmp_path / "nope.json", "cucumber-json") == {}
    empty = tmp_path / "e.json"
    empty.write_text("")
    assert read_report(empty, "cucumber-json") == {}


# ------------------------------------------------------------------ config
def test_every_preset_writes_a_loadable_config(tmp_path):
    for stack in PRESETS:
        root = tmp_path / stack
        root.mkdir()
        write_config(root, stack)
        cfg = Config.load(root)
        assert cfg.stack == stack
        assert cfg.format in ("shalt", "cucumber-json", "cucumber-messages")
        # the runner has to be able to find out where to write its report. Most take it on the
        # command line; cargo has no such flag, so the rust preset passes it through the
        # environment instead. Either channel is fine; neither is not.
        reachable = "{report}" in cfg.command or any(
            "{report}" in v for v in cfg.env.values())
        assert reachable, f"{stack}: nothing tells the runner where to write its report"


def test_env_values_take_the_same_placeholders_as_the_command(tmp_path):
    """Without this the report path would have to be hardcoded identically in shalt.toml and in
    the test binary — two places that will drift."""
    write_config(tmp_path, "rust")
    cfg = Config.load(tmp_path)
    env = cfg.resolved_env(tmp_path)
    assert env["SHALT_REPORT"] == str(tmp_path / cfg.report)
    assert "{" not in env["SHALT_REPORT"]


def test_config_substitutes_workspace_paths(tmp_path):
    write_config(tmp_path, "python")
    cfg = Config.load(tmp_path)
    argv = cfg.argv(tmp_path)
    assert any(str(tmp_path / "spec") in a for a in argv)
    assert not any("{" in a for a in argv), "every placeholder must be substituted"


def test_an_unknown_report_format_is_rejected(tmp_path):
    (tmp_path / "shalt.toml").write_text(
        '[runner]\nformat = "junit-xml"\ncommand = "true"\n')
    with pytest.raises(ValueError, match="unknown runner format"):
        Config.load(tmp_path)


def test_a_redirecting_preset_is_marked_as_needing_a_shell(tmp_path):
    write_config(tmp_path, "go")
    assert Config.load(tmp_path).uses_shell


# ------------------------------------------------------------------ diagrams
def test_use_case_diagram_contains_every_actor_and_capability(tmp_path):
    led, _ = _ledger(tmp_path)
    m = mermaid_usecase(list(led.entries.values()))
    assert m.startswith("graph LR")
    assert "billing clerk" in m and "credit controller" in m
    assert "invoice totals computed exactly" in m


def test_diagrams_survive_labels_with_quotes_and_newlines(tmp_path):
    files = {"q.feature": '''\
Feature: Odd "quoted" name

  As a "power" user
  I want it to not break
  So that diagrams render

  @rid:S-000000ff
  Scenario: A "quoted" scenario
    Given a thing
    Then it works
'''}
    led, _ = _ledger(tmp_path, files)
    for m in (mermaid_usecase(list(led.entries.values())),
              mermaid_hierarchy(list(led.entries.values()))):
        assert '""' not in m, "a raw double quote inside a Mermaid label breaks the parser"


def test_diagrams_degrade_gracefully_when_empty():
    assert "No user stories found" in mermaid_usecase([])
    assert "No scenarios in the ledger" in mermaid_hierarchy([])
    assert "graph LR" in mermaid_pipeline()


# ------------------------------------------------------------------ dashboard
def test_dashboard_defines_every_colour_token_in_all_three_theme_states(tmp_path):
    led, _ = _ledger(tmp_path)
    doc = dashboard_html(led, project="Billing", stack="Python")
    root_block = doc.split(":root{", 1)[1].split("}", 1)[0]
    for token in ("--ground", "--surface", "--ink", "--accent", "--verified", "--failing",
                  "--stale", "--pending", "--orphan", "--chip-ink"):
        assert token in root_block, f"{token} must exist in the bare :root light palette"
    assert 'prefers-color-scheme: dark' in doc
    assert ':root:not([data-theme="light"])' in doc
    assert ':root[data-theme="dark"]' in doc
    assert "background:var(--ground)" in doc, "body must paint its own ground"


def test_dashboard_escapes_scenario_text(tmp_path):
    files = {"x.feature": '''\
Feature: Injection

  As a tester
  I want <script>alert(1)</script> handled
  So that the page is safe

  @rid:S-000000ee
  Scenario: A <script>alert(2)</script> scenario
    Given a thing
    Then it works
'''}
    led, _ = _ledger(tmp_path, files)
    doc = dashboard_html(led)
    assert "<script>alert(2)</script>" not in doc
    assert "&lt;script&gt;" in doc


def test_dashboard_reports_pending_and_never_inflates(tmp_path):
    led, _ = _ledger(tmp_path)
    led.apply_run({}, "r1")
    doc = dashboard_html(led)
    assert "0%" in doc
    assert "no test" in doc


def test_terse_failure_extracts_the_assertion_not_the_plumbing():
    raw = ("fixturefunc = <function _then at 0x7f00>\n"
           "request = <FixtureRequest for <Function test_x>>\n"
           "    def call_fixture_func(fixturefunc, request, kwargs):\n"
           "E       AssertionError: expected '1.01', got '1.00'\n")
    out = terse_failure(raw)
    assert out == "AssertionError: expected '1.01', got '1.00'"
    assert "fixturefunc" not in out


def test_terse_failure_falls_back_for_other_runners():
    assert "boom" in terse_failure("some runner\nsaid boom")


# ----------------------------------- real runner output, not a payload we wrote ourselves
# Captured verbatim from cucumber-rs 0.23 (`writer::Json`). The point of keeping it real is the
# tag spelling: cucumber-rs emits "rid:S-..." with NO leading "@", while cucumber-jvm and
# cucumber-js keep it. The JSON format does not settle the question. A parser that insists on
# one spelling binds nothing against half the ecosystem, and every scenario then reads
# "pending" — which looks like unfinished work rather than a bug. Found by running shalt
# against a real Rust project.
CUCUMBER_RS_REPORT = """[{
  "uri": "spec/reminders.feature",
  "keyword": "Feature",
  "name": "Overdue reminders",
  "tags": [{"name": "epic:collections", "line": 2}],
  "elements": [
    {
      "id": "overdue-reminders/an-invoice-that-is-not-yet-overdue-gets-no-reminder",
      "keyword": "Scenario", "line": 9, "type": "scenario",
      "name": "An invoice that is not yet overdue gets no reminder",
      "tags": [{"name": "rid:S-5d7d2848", "line": 9}],
      "steps": [
        {"keyword": "Given ", "name": "an invoice 0 days overdue",
         "result": {"status": "passed"}},
        {"keyword": "When ", "name": "I ask which reminder is due",
         "result": {"status": "passed"}},
        {"keyword": "Then ", "name": "the reminder is \\"none\\"",
         "result": {"status": "passed"}}
      ]
    },
    {
      "id": "overdue-reminders/two-months-late-gets-a-final-demand",
      "keyword": "Scenario", "line": 25, "type": "scenario",
      "name": "Two months late gets a final demand",
      "tags": [{"name": "rid:S-5d91b792", "line": 25}],
      "steps": [
        {"keyword": "Given ", "name": "an invoice 60 days overdue",
         "result": {"status": "passed"}},
        {"keyword": "When ", "name": "I ask which reminder is due",
         "result": {"status": "passed"}},
        {"keyword": "Then ", "name": "the reminder is \\"final\\"",
         "result": {"status": "failed", "error_message": "wrong reminder stage"}}
      ]
    }
  ]
}]"""


def test_a_tag_without_the_leading_at_sign_still_binds():
    r = parse_cucumber_json(CUCUMBER_RS_REPORT)
    assert set(r) == {"S-5d7d2848", "S-5d91b792"}, \
        "cucumber-rs strips the @ from tag names; binding must not depend on that spelling"
    assert r["S-5d7d2848"]["outcome"] == "passed"
    assert r["S-5d91b792"]["outcome"] == "failed"
    assert "wrong reminder stage" in r["S-5d91b792"]["detail"]


@pytest.mark.parametrize("tag,expected", [
    ({"name": "rid:S-0d41bae1"}, "S-0d41bae1"),       # cucumber-rs
    ({"name": "@rid:S-0d41bae1"}, "S-0d41bae1"),      # cucumber-jvm, cucumber-js
    ({"name": " @rid:S-0d41bae1 "}, "S-0d41bae1"),    # padded
    ("rid:S-0d41bae1", "S-0d41bae1"),                 # bare string, not an object
    ({"name": "epic:billing"}, None),
    ({"name": ""}, None),
])
def test_rid_extraction_tolerates_every_spelling_in_the_wild(tag, expected):
    from shalt.reports import _rid_from_tags
    assert _rid_from_tags([tag]) == expected
