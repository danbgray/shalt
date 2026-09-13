"""Terminal presentation: colour detection, clickable paths, Gherkin highlighting."""
import io

import pytest

from shalt.term import (EDITORS, Term, detect_color, detect_editor, detect_links,
                        highlight_gherkin)


class FakeTTY(io.StringIO):
    def __init__(self, tty=True):
        super().__init__()
        self._tty = tty

    def isatty(self):
        return self._tty


# ---------------------------------------------------------------- colour detection
def test_colour_is_off_when_output_is_not_a_terminal():
    """Piped output and CI logs must stay free of escape codes."""
    on, _ = detect_color(FakeTTY(tty=False), {"TERM": "xterm-256color"})
    assert on is False


def test_no_color_wins_over_everything():
    """https://no-color.org — its presence at any value disables colour."""
    on, _ = detect_color(FakeTTY(), {"TERM": "xterm-256color", "NO_COLOR": "",
                                     "FORCE_COLOR": "1"})
    assert on is False
    on, _ = detect_color(FakeTTY(), {"NO_COLOR": "1"}, mode="always")
    assert on is False


def test_force_color_enables_colour_when_piped():
    on, _ = detect_color(FakeTTY(tty=False), {"FORCE_COLOR": "1"})
    assert on is True


def test_dumb_terminals_get_no_colour():
    on, _ = detect_color(FakeTTY(), {"TERM": "dumb"})
    assert on is False


def test_truecolor_is_detected_from_colorterm():
    on, true = detect_color(FakeTTY(), {"TERM": "xterm", "COLORTERM": "truecolor"})
    assert (on, true) == (True, True)
    on, true = detect_color(FakeTTY(), {"TERM": "xterm"})
    assert (on, true) == (True, False)


def test_styling_is_a_no_op_when_colour_is_off():
    t = Term(stream=FakeTTY(tty=False), env={})
    assert t.s("upheld", "ok", "bold") == "upheld"


def test_styling_emits_truecolor_matching_the_dashboard_palette():
    t = Term(stream=FakeTTY(), env={"COLORTERM": "truecolor"}, mode="always")
    # --verified in the dashboard's dark theme
    assert t.s("x", "ok") == "\033[38;2;79;179;131mx\033[0m"


# ---------------------------------------------------------------- hyperlinks
@pytest.mark.parametrize("env,expected", [
    ({"TERM_PROGRAM": "iTerm.app"}, True),
    ({"TERM_PROGRAM": "vscode"}, True),
    ({"TERM_PROGRAM": "WezTerm"}, True),
    ({"TERM_PROGRAM": "ghostty"}, True),
    ({"TERM": "xterm-kitty"}, True),
    ({"WT_SESSION": "abc"}, True),
    ({"VTE_VERSION": "6003"}, True),
    ({"TERM_PROGRAM": "Apple_Terminal"}, False),   # genuinely does not support OSC 8
    ({}, False),
])
def test_hyperlink_support_is_detected_per_terminal(env, expected):
    assert detect_links(env) is expected


def test_hyperlinks_can_be_forced_either_way():
    assert detect_links({"SHALT_HYPERLINKS": "1"}) is True
    assert detect_links({"TERM_PROGRAM": "iTerm.app", "SHALT_HYPERLINKS": "0"}) is False


def test_osc8_sequence_is_exactly_right():
    t = Term(stream=FakeTTY(), env={"SHALT_HYPERLINKS": "1"}, mode="never")
    assert t.link("click", "file:///x") == "\033]8;;file:///x\033\\click\033]8;;\033\\"


def test_link_is_dropped_when_the_terminal_cannot_render_it():
    t = Term(stream=FakeTTY(), env={"SHALT_HYPERLINKS": "0"}, mode="never")
    assert t.link("click", "file:///x") == "click"


# ---------------------------------------------------------------- editors
@pytest.mark.parametrize("editor,expected", [
    ("file", "file:///w/spec/a.feature"),
    ("vscode", "vscode://file/w/spec/a.feature:12"),
    ("cursor", "cursor://file/w/spec/a.feature:12"),
    ("zed", "zed://file/w/spec/a.feature:12"),
    ("subl", "subl://open?url=file:///w/spec/a.feature&line=12"),
    ("idea", "idea://open?file=/w/spec/a.feature&line=12"),
])
def test_editor_url_schemes(editor, expected):
    assert EDITORS[editor]("/w/spec/a.feature", 12) == expected


def test_vscode_terminal_is_auto_detected():
    assert detect_editor({"TERM_PROGRAM": "vscode"}) == "vscode"
    assert detect_editor({"SHALT_EDITOR": "zed"}) == "zed"
    assert detect_editor({"SHALT_EDITOR": "nonsense"}) == "file"
    assert detect_editor({}) == "file"


# ---------------------------------------------------------------- path fallbacks
def test_a_linking_terminal_shows_the_short_label(tmp_path):
    f = tmp_path / "spec" / "a.feature"
    f.parent.mkdir()
    f.write_text("Feature: x\n")
    t = Term(stream=FakeTTY(), env={"SHALT_HYPERLINKS": "1", "SHALT_EDITOR": "vscode"},
             mode="never")
    out = t.path(f, 12, label="A scenario name")
    assert "A scenario name" in out
    assert f"vscode://file{f}:12" in out


def test_without_links_a_bare_path_reference_prints_the_absolute_path(tmp_path):
    """Command-click in iTerm2 and Terminal.app matches the visible string, so the path has
    to be in the text for those terminals to be useful at all."""
    f = tmp_path / "out.html"
    f.write_text("x")
    t = Term(stream=FakeTTY(), env={"SHALT_HYPERLINKS": "0"}, mode="never")
    assert t.path(f, label="out.html") == str(f.resolve())


def test_without_links_a_labelled_row_keeps_its_label(tmp_path):
    """Substituting an absolute path for every scenario name would turn a readable list into a
    wall of paths. Legibility beats click-through where the label is the content."""
    f = tmp_path / "a.feature"
    f.write_text("Feature: x\n")
    t = Term(stream=FakeTTY(), env={"SHALT_HYPERLINKS": "0"}, mode="never")
    assert t.path(f, 12, label="A scenario name", fallback="label") == "A scenario name"


# ---------------------------------------------------------------- Gherkin highlighting
FEATURE = '''@epic:billing
Feature: Invoice totals

  As a billing clerk
  I want totals computed exactly
  So that nobody is overbilled

  # a comment
  @holdout @rid:S-00000001
  Scenario: A total
    Given an invoice with lines:
      | description | amount |
      | Widget      | 100.00 |
    When I compute the total
    Then the total is "100.00"
'''


def test_highlighting_is_plain_text_when_colour_is_off():
    """Piping `shalt show` into a file must produce the feature file, not escape codes."""
    t = Term(stream=FakeTTY(tty=False), env={})
    assert highlight_gherkin(FEATURE, t) == FEATURE.rstrip("\n")


def test_highlighting_never_changes_the_visible_characters():
    t = Term(stream=FakeTTY(), env={"COLORTERM": "truecolor"}, mode="always")
    out = highlight_gherkin(FEATURE, t)
    import re
    stripped = re.sub(r"\033\[[0-9;]*m", "", out)
    assert stripped == FEATURE.rstrip("\n"), "highlighting must not alter the text itself"


def test_the_pieces_that_matter_are_each_styled_differently():
    t = Term(stream=FakeTTY(), env={"COLORTERM": "truecolor"}, mode="always")
    out = highlight_gherkin(FEATURE, t)
    assert "\033[38;2;143;163;216;1m@epic:billing" in out      # epic tag, accent bold
    assert "\033[38;2;169;146;196;1m@holdout" in out           # holdout, orphan bold
    assert "\033[38;2;136;145;160m@rid:S-00000001" in out      # rid, muted: machinery
    assert "\033[38;2;143;163;216;1mFeature:" in out           # keyword
    assert "\033[38;2;79;179;131;1mGiven" in out               # step keyword
    assert '\033[38;2;215;166;72m"100.00"' in out              # the concrete value
    assert "\033[38;2;169;146;196;3mAs a" in out               # narrative, italic


def test_line_numbers_and_marks(tmp_path):
    t = Term(stream=FakeTTY(tty=False), env={})
    out = highlight_gherkin("a\nb\nc\n", t, number_from=1, marks={2})
    lines = out.splitlines()
    assert lines[0].startswith("   1 ")
    assert "▎" in lines[1], "a marked line gets a gutter bar"
    assert "▎" not in lines[0]


def test_a_docstring_body_is_not_parsed_as_gherkin():
    t = Term(stream=FakeTTY(), env={"COLORTERM": "truecolor"}, mode="always")
    src = '  Given a doc:\n    """\nScenario: prose, not a scenario\n    """\n'
    out = highlight_gherkin(src, t)
    assert "\033[38;2;143;163;216;1mScenario:" not in out, \
        "text inside a docstring is data, not structure"
