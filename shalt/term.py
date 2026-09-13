"""Terminal presentation: colour, clickable paths, and Gherkin highlighting.

Three decisions shape this file.

**Colour is detected, never assumed.** Honours NO_COLOR (https://no-color.org), FORCE_COLOR and
an explicit --color flag, and stays off when stdout is not a terminal so piped output and CI
logs stay clean.

**Paths are clickable two different ways, because terminals disagree.** Terminals that support
OSC 8 hyperlinks (iTerm2, VS Code, WezTerm, Kitty, Ghostty, Windows Terminal) get a real link
that opens the file at the right line in the configured editor. macOS Terminal.app does *not*
support OSC 8 — but it, and iTerm2, will Command-click a plain `path:line` string. So the
fallback is an absolute path rather than nothing, and stays useful.

**The palette matches the dashboard's dark theme**, so a scenario that is amber on the web is
amber in the terminal. One product, one set of colours.
"""
from __future__ import annotations

import os
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

# The dashboard's dark-theme tokens. Terminals are usually dark, and matching means a status
# reads the same in both places.
TRUECOLOR = {
    "accent": (143, 163, 216),   # --accent
    "ok": (79, 179, 131),        # --verified
    "bad": (224, 119, 108),      # --failing
    "warn": (215, 166, 72),      # --stale
    "orphan": (169, 146, 196),   # --orphan
    "muted": (136, 145, 160),    # --ink-3
    "ink": (230, 233, 239),      # --ink
}
ANSI256 = {"accent": 110, "ok": 78, "bad": 174, "warn": 179, "orphan": 140,
           "muted": 245, "ink": 253}
ANSI16 = {"accent": 34, "ok": 32, "bad": 31, "warn": 33, "orphan": 35,
          "muted": 90, "ink": 39}

ATTRS = {"bold": "1", "dim": "2", "italic": "3", "underline": "4", "reverse": "7"}

# Terminals known to render OSC 8. Apple_Terminal is deliberately absent: it does not.
LINK_PROGRAMS = {"iTerm.app", "vscode", "WezTerm", "ghostty", "Hyper", "rio", "Tabby"}

EDITORS = {
    "file": lambda p, l: f"file://{p}",
    "vscode": lambda p, l: f"vscode://file{p}" + (f":{l}" if l else ""),
    "cursor": lambda p, l: f"cursor://file{p}" + (f":{l}" if l else ""),
    "windsurf": lambda p, l: f"windsurf://file{p}" + (f":{l}" if l else ""),
    "zed": lambda p, l: f"zed://file{p}" + (f":{l}" if l else ""),
    "subl": lambda p, l: f"subl://open?url=file://{p}" + (f"&line={l}" if l else ""),
    "idea": lambda p, l: f"idea://open?file={p}" + (f"&line={l}" if l else ""),
    "textmate": lambda p, l: f"txmt://open?url=file://{p}" + (f"&line={l}" if l else ""),
}


def _truthy(v: str | None) -> bool:
    return (v or "").strip().lower() in {"1", "true", "yes", "always", "on"}


def _falsy(v: str | None) -> bool:
    return (v or "").strip().lower() in {"0", "false", "no", "never", "off"}


def detect_color(stream, env: dict, mode: str = "auto") -> tuple[bool, bool]:
    """(colour on, truecolor). `mode` is auto | always | never."""
    if mode == "never" or "NO_COLOR" in env:
        return False, False
    truecolor = (env.get("COLORTERM", "").lower() in {"truecolor", "24bit"}
                 or "truecolor" in env.get("TERM", ""))
    if mode == "always" or _truthy(env.get("FORCE_COLOR")) or _truthy(env.get("CLICOLOR_FORCE")):
        return True, truecolor
    if env.get("TERM", "") in {"dumb", ""} or not getattr(stream, "isatty", lambda: False)():
        return False, False
    return True, truecolor


def detect_links(env: dict) -> bool:
    """Whether the terminal renders OSC 8 hyperlinks."""
    if _falsy(env.get("SHALT_HYPERLINKS")):
        return False
    if _truthy(env.get("SHALT_HYPERLINKS")):
        return True
    if env.get("TERM_PROGRAM") in LINK_PROGRAMS:
        return True
    if "kitty" in env.get("TERM", "") or env.get("KITTY_WINDOW_ID"):
        return True
    if env.get("WT_SESSION"):
        return True
    try:
        if int(env.get("VTE_VERSION", "0")) >= 5000:
            return True
    except ValueError:
        pass
    return False   # includes Apple_Terminal, which Command-clicks plain paths instead


def detect_editor(env: dict) -> str:
    named = (env.get("SHALT_EDITOR") or "").strip().lower()
    if named in EDITORS:
        return named
    if env.get("TERM_PROGRAM") == "vscode":
        return "vscode"
    return "file"


@dataclass
class Term:
    stream: object = None
    mode: str = "auto"
    editor: str | None = None
    env: dict = field(default_factory=lambda: dict(os.environ))
    color: bool = False
    truecolor: bool = False
    links: bool = False

    def __post_init__(self):
        self.stream = self.stream or sys.stdout
        self.color, self.truecolor = detect_color(self.stream, self.env, self.mode)
        self.links = detect_links(self.env)
        self.editor = self.editor or detect_editor(self.env)

    # ------------------------------------------------------------------ colour
    def _seq(self, name: str) -> str:
        if name in ATTRS:
            return ATTRS[name]
        if self.truecolor and name in TRUECOLOR:
            r, g, b = TRUECOLOR[name]
            return f"38;2;{r};{g};{b}"
        if name in ANSI256:
            return f"38;5;{ANSI256[name]}"
        return str(ANSI16.get(name, 39))

    def s(self, text: str, *names: str) -> str:
        """Style `text`. A no-op when colour is off, so call sites stay unconditional."""
        if not self.color or not names:
            return text
        codes = ";".join(self._seq(n) for n in names)
        return f"\033[{codes}m{text}\033[0m"

    # ------------------------------------------------------------------ links
    def link(self, text: str, url: str) -> str:
        if not self.links:
            return text
        return f"\033]8;;{url}\033\\{text}\033]8;;\033\\"

    def file_url(self, path, line: int | None = None) -> str:
        p = Path(path).resolve()
        return EDITORS.get(self.editor, EDITORS["file"])(str(p), line)

    def path(self, path, line: int | None = None, label: str | None = None,
             style: str | tuple[str, ...] = (), fallback: str = "path") -> str:
        """A clickable reference to a file, optionally at a line.

        With OSC 8 the label can be short and the link does the work. Without it there is a
        real choice to make, which is what `fallback` selects:

        * `"path"` -- print the absolute `path:line`. Right when the path *is* the content (a
          mutant location, the ledger, a generated file), and it keeps Command-click working in
          iTerm2 and Terminal.app, which match on the visible string.
        * `"label"` -- print the label alone. Right when the label is the content: substituting
          an absolute path for every scenario name turns a readable list into a wall of paths.
          Click-through is lost in those terminals; legibility is worth more.
        """
        names = (style,) if isinstance(style, str) else tuple(style)
        p = Path(path)
        abs_p = p.resolve()
        if self.links:
            shown = label or (f"{p}:{line}" if line else str(p))
            return self.link(self.s(shown, *names) if names else shown,
                             self.file_url(abs_p, line))
        if fallback == "label" and label:
            return self.s(label, *names) if names else label
        shown = f"{abs_p}:{line}" if line else str(abs_p)
        return self.s(shown, *names) if names else shown

    def rule(self, width: int = 60) -> str:
        return self.s("─" * width, "muted")


# ---------------------------------------------------------------- Gherkin highlighting
FEATURE_KW = re.compile(
    r"^(\s*)(Feature|Rule|Background|Scenario Outline|Scenario Template|Scenario|Example|"
    r"Examples|Scenarios)(:)(.*)$")
STEP_KW = re.compile(r"^(\s*)(Given|When|Then|And|But|\*)(\s+)(.*)$")
TAG_LINE = re.compile(r"^\s*@[\w:.\-]+(\s+@[\w:.\-]+)*\s*$")
NARRATIVE = re.compile(
    r"^(\s*)(As an?|As the|I want|I need|I would like|I can|I do|So that|In order to)\b(.*)$",
    re.I)
COMMENT = re.compile(r"^(\s*)(#.*)$")
TABLE = re.compile(r"^(\s*)\|(.*)\|(\s*)$")
QUOTED = re.compile(r'("[^"]*"|<[^>]+>)')


def highlight_gherkin(text: str, term: "Term | None" = None,
                      number_from: int = 0, marks: set[int] | None = None) -> str:
    """Colourise a feature file for reading in a terminal.

    Line-oriented, because Gherkin is. `number_from` turns on line numbers (1-based), and
    `marks` highlights particular line numbers -- used to point at the scenario you asked for.
    """
    t = term or Term()
    marks = marks or set()
    out = []
    in_doc = False

    for i, line in enumerate(text.splitlines(), start=1):
        n = number_from + i - 1 if number_from else None
        body = line

        if line.strip().startswith('"""') or line.strip().startswith("'''"):
            in_doc = not in_doc
            body = t.s(line, "warn")
        elif in_doc:
            body = t.s(line, "warn")
        elif (m := COMMENT.match(line)):
            body = m.group(1) + t.s(m.group(2), "muted", "italic")
        elif TAG_LINE.match(line) and line.strip():
            body = _tags(line, t)
        elif (m := FEATURE_KW.match(line)):
            indent, kw, colon, rest = m.groups()
            body = (indent + t.s(kw + colon, "accent", "bold")
                    + t.s(rest, "ink", "bold"))
        elif (m := STEP_KW.match(line)):
            indent, kw, gap, rest = m.groups()
            body = indent + t.s(kw, "ok", "bold") + gap + _values(rest, t)
        elif (m := TABLE.match(line)):
            cells = m.group(2).split("|")
            body = (m.group(1) + t.s("|", "muted")
                    + t.s("|", "muted").join(_values(c, t) for c in cells)
                    + t.s("|", "muted") + m.group(3))
        elif (m := NARRATIVE.match(line)):
            indent, kw, rest = m.groups()
            body = indent + t.s(kw, "orphan", "italic") + t.s(rest, "muted", "italic")
        elif line.strip():
            body = t.s(line, "muted")

        if n is not None:
            gutter = t.s(f"{n:>4} ", "bad" if n in marks else "muted")
            body = gutter + (t.s("\u258e", "warn") + body if n in marks else "  " + body)
        out.append(body)
    return "\n".join(out)


def _tags(line: str, t: Term) -> str:
    def one(m):
        tag = m.group(0)
        if tag.startswith("@rid:"):
            return t.s(tag, "muted")
        if tag == "@holdout":
            return t.s(tag, "orphan", "bold")
        if tag.startswith("@epic:"):
            return t.s(tag, "accent", "bold")
        return t.s(tag, "accent")
    return re.sub(r"@[\w:.\-]+", one, line)


def _values(text: str, t: Term) -> str:
    """Quoted strings and <placeholders> are the concrete data in a scenario; lift them."""
    return QUOTED.sub(lambda m: t.s(m.group(0), "warn"), text)
