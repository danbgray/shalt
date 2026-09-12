"""Visualising the breakdown: epic -> story -> scenario, and the use case diagram implied by it.

Nothing here is authored separately from the spec. Actors and capabilities come from the user
story in each feature's description block, epics from @epic: tags or the directory under spec/,
and status from the ledger. A derived diagram cannot drift away from the thing it describes --
which is the whole reason to derive it rather than draw it.

Two outputs:
  * Mermaid (.mmd) -- renders in GitHub, in pull requests, in most editors.
  * A single self-contained HTML dashboard -- no network, no build step, opens from a file.
"""
from __future__ import annotations

import html
import json
from collections import OrderedDict
from dataclasses import dataclass
from pathlib import Path

from .narrative import slug

STATUS_ORDER = ["green", "red", "stale", "pending", "orphan"]
STATUS_LABEL = {"green": "upheld", "red": "failing", "stale": "stale",
                "pending": "no test", "orphan": "removed"}
MERMAID_FILL = {"green": "#1f7a4c", "red": "#b3382e", "stale": "#a9761b",
                "pending": "#6b7684", "orphan": "#6e5a86"}
# status -> the CSS custom property that carries its colour
STATUS_VAR = {"green": "verified", "red": "failing", "stale": "stale",
              "pending": "pending", "orphan": "orphan"}


@dataclass
class Node:
    key: str
    label: str
    status: str = ""
    children: list = None

    def __post_init__(self):
        if self.children is None:
            self.children = []


def build_tree(entries: list) -> list[Node]:
    """epic -> story (feature) -> scenario, ordered by epic then feature then scenario name."""
    epics: OrderedDict[str, Node] = OrderedDict()
    for e in sorted(entries, key=lambda x: (x.epic or "~", x.feature_file, x.name)):
        ek = e.epic or "unassigned"
        epic = epics.setdefault(ek, Node(key=f"epic-{slug(ek)}", label=ek))
        story = next((c for c in epic.children if c.key == f"story-{slug(e.feature_file)}"), None)
        if story is None:
            story = Node(key=f"story-{slug(e.feature_file)}", label=e.feature)
            epic.children.append(story)
        story.children.append(Node(key=e.rid, label=e.name, status=e.status))
    for epic in epics.values():
        for story in epic.children:
            story.status = roll_up([c.status for c in story.children])
        epic.status = roll_up([s.status for s in epic.children])
    return list(epics.values())


def roll_up(statuses: list[str]) -> str:
    """A parent is only as good as its worst child. Green requires every child green --
    a rolled-up status must never be more optimistic than what it contains."""
    if not statuses:
        return "pending"
    for s in ("red", "stale", "pending"):
        if s in statuses:
            return s
    return "green" if all(s == "green" for s in statuses) else "orphan"


def actors(entries: list) -> "OrderedDict[str, list]":
    """actor -> the capabilities they want, from the 'As a ... I want ...' narratives."""
    out: OrderedDict[str, list] = OrderedDict()
    seen: set[tuple[str, str]] = set()
    for e in sorted(entries, key=lambda x: (x.actor or "~", x.feature_file)):
        if not e.actor or not e.capability:
            continue
        pair = (e.actor, e.capability)
        if pair in seen:
            continue
        seen.add(pair)
        out.setdefault(e.actor, []).append(
            {"capability": e.capability, "benefit": e.benefit,
             "story": e.feature, "epic": e.epic})
    return out


def _mid(s: str) -> str:
    """Mermaid label: quote it and strip the characters that break the parser."""
    return '"' + s.replace('"', "'").replace("\n", " ").strip() + '"'


# ------------------------------------------------------------------ mermaid
def mermaid_usecase(entries: list) -> str:
    """The use case diagram already present in the user stories.

    Mermaid has no UML use-case type, so actors are stadium nodes and use cases are rounded
    ones -- which is how a use case diagram reads anyway.
    """
    acts = actors(entries)
    if not acts:
        return ("graph LR\n  none[\"No user stories found.\"]\n"
                "  %% Add an 'As a ... / I want ... / So that ...' block under Feature:\n")
    lines = ["graph LR"]
    caps: OrderedDict[str, str] = OrderedDict()
    for actor, items in acts.items():
        aid = f"A_{slug(actor)}"
        lines.append(f"  {aid}([{_mid(actor)}])")
        for it in items:
            cid = f"U_{slug(it['capability'])[:40]}"
            caps[cid] = it["capability"]
            lines.append(f"  {aid} --- {cid}")
    lines.append("")
    for cid, cap in caps.items():
        lines.append(f"  {cid}({_mid(cap)})")
    lines.append("")
    for actor in acts:
        lines.append(f"  style A_{slug(actor)} fill:#3d4f7c,stroke:#2a3757,color:#ffffff")
    for cid in caps:
        lines.append(f"  style {cid} fill:#eef1f7,stroke:#3d4f7c,color:#1a1f26")
    return "\n".join(lines) + "\n"


def mermaid_hierarchy(entries: list) -> str:
    """epic -> story -> scenario, coloured by verified state."""
    tree = build_tree(entries)
    if not tree:
        return 'graph TD\n  none["No scenarios in the ledger yet."]\n'
    lines = ["graph TD"]
    styles = []
    for epic in tree:
        lines.append(f"  {epic.key}[{_mid(epic.label.upper())}]")
        styles.append(f"  style {epic.key} fill:#3d4f7c,stroke:#2a3757,color:#ffffff")
        for story in epic.children:
            lines.append(f"  {epic.key} --> {story.key}[{_mid(story.label)}]")
            styles.append(f"  style {story.key} fill:#eef1f7,stroke:#3d4f7c,color:#1a1f26")
            for sc in story.children:
                nid = sc.key.replace("-", "_")
                mark = {"green": "✓", "red": "✗", "stale": "~",
                        "pending": "·", "orphan": "?"}[sc.status]
                lines.append(f"  {story.key} --> {nid}({_mid(mark + '  ' + sc.label)})")
                styles.append(f"  style {nid} fill:{MERMAID_FILL[sc.status]},"
                              f"stroke:{MERMAID_FILL[sc.status]},color:#ffffff")
    return "\n".join(lines + [""] + styles) + "\n"


def mermaid_pipeline() -> str:
    """How a request becomes verified behaviour, and who is allowed to touch what."""
    return """graph LR
  req["Plain-English request"] --> author["Author agent"]
  author --> story["User story<br/>As a / I want / So that"]
  story --> gherkin["Gherkin scenarios"]
  gherkin --> gate{"Human<br/>approval"}
  gate -->|"stamps @rid, records<br/>content hashes"| locked["Approved spec"]
  locked --> sw["Stepwright agent<br/>sees spec only"]
  sw --> steps["Step definitions"]
  sw --> contract["Interface contract"]
  contract --> impl["Implementer agent<br/>never sees the steps"]
  locked --> impl
  impl --> src["Implementation"]
  steps --> run["Test run"]
  src --> run
  run --> ledger["Scenario ledger"]
  ledger -->|"still red"| impl
  ledger --> hold["Held-out scenarios<br/>verify, never shown"]

  style gate fill:#a9761b,stroke:#7d570f,color:#ffffff
  style locked fill:#3d4f7c,stroke:#2a3757,color:#ffffff
  style ledger fill:#1f7a4c,stroke:#155a38,color:#ffffff
  style hold fill:#6e5a86,stroke:#514166,color:#ffffff
  style sw fill:#eef1f7,stroke:#3d4f7c,color:#1a1f26
  style impl fill:#eef1f7,stroke:#3d4f7c,color:#1a1f26
  style author fill:#eef1f7,stroke:#3d4f7c,color:#1a1f26
"""


def write_mermaid(root: Path, entries: list) -> list[Path]:
    out_dir = Path(root) / "docs" / "diagrams"
    out_dir.mkdir(parents=True, exist_ok=True)
    written = []
    for name, text in (("use-cases", mermaid_usecase(entries)),
                       ("breakdown", mermaid_hierarchy(entries)),
                       ("pipeline", mermaid_pipeline())):
        p = out_dir / f"{name}.mmd"
        p.write_text(text, encoding="utf-8")
        written.append(p)
        md = out_dir / f"{name}.md"
        md.write_text(f"# {name.replace('-', ' ').title()}\n\n```mermaid\n{text}```\n",
                      encoding="utf-8")
        written.append(md)
    return written


# ------------------------------------------------------------------ dashboard
_CSS = """
:root{
  --ground:#f6f7f9; --surface:#ffffff; --surface-2:#eef1f7; --line:#d9dee8;
  --ink:#1a1f26; --ink-2:#4a5464; --ink-3:#6b7684;
  --accent:#3d4f7c; --accent-ink:#ffffff; --chip-ink:#ffffff;
  --verified:#1f7a4c; --failing:#b3382e; --stale:#a9761b;
  --pending:#6b7684; --orphan:#6e5a86;
  --detent-empty:#d9dee8;
  --shadow:0 1px 2px rgba(26,31,38,.07);
}
@media (prefers-color-scheme: dark){
  :root:not([data-theme="light"]){
    --ground:#13171c; --surface:#1a1f26; --surface-2:#222932; --line:#2f3844;
    --ink:#e6e9ef; --ink-2:#aab3c0; --ink-3:#8891a0;
    --accent:#8fa3d8; --accent-ink:#13171c; --chip-ink:#13171c;
    --verified:#4fb383; --failing:#e0776c; --stale:#d7a648;
    --pending:#8891a0; --orphan:#a992c4;
    --detent-empty:#2f3844;
    --shadow:0 1px 2px rgba(0,0,0,.4);
  }
}
:root[data-theme="dark"]{
  --ground:#13171c; --surface:#1a1f26; --surface-2:#222932; --line:#2f3844;
  --ink:#e6e9ef; --ink-2:#aab3c0; --ink-3:#8891a0;
  --accent:#8fa3d8; --accent-ink:#13171c; --chip-ink:#13171c;
  --verified:#4fb383; --failing:#e0776c; --stale:#d7a648;
  --pending:#8891a0; --orphan:#a992c4;
  --detent-empty:#2f3844;
  --shadow:0 1px 2px rgba(0,0,0,.4);
}
*{box-sizing:border-box}
body{
  margin:0; background:var(--ground); color:var(--ink);
  font-family:"IBM Plex Sans",-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;
  font-size:15px; line-height:1.5;
}
.wrap{max-width:1080px; margin:0 auto; padding-inline:20px; padding-block:40px 72px;}
.mono{font-family:"IBM Plex Mono",ui-monospace,SFMono-Regular,Menlo,monospace;
  font-variant-numeric:tabular-nums;}
.prose{font-family:"IBM Plex Serif",Georgia,serif;}

header.head{display:flex; flex-wrap:wrap; gap:16px 28px; align-items:baseline;
  border-bottom:2px solid var(--accent); padding-bottom:22px; margin-bottom:28px;}
h1{font-size:26px; font-weight:600; margin:0; letter-spacing:-.01em; text-wrap:balance;}
.head .sub{color:var(--ink-2); font-size:13.5px;}
.eyebrow{font-size:11px; text-transform:uppercase; letter-spacing:.1em;
  color:var(--ink-3); font-weight:600;}

.meter{margin-bottom:12px;}
.detents{display:flex; gap:3px; height:34px; align-items:stretch; margin:10px 0 8px;}
.detent{flex:1 1 0; min-width:3px; border-radius:2px; background:var(--detent-empty);
  position:relative;}
.detent.green{background:var(--verified)} .detent.red{background:var(--failing)}
.detent.stale{background:var(--stale)} .detent.pending{background:var(--pending)}
.detent.orphan{background:var(--orphan)}
.pawl{display:flex; justify-content:space-between; align-items:baseline; gap:16px;
  flex-wrap:wrap;}
.pct{font-size:40px; font-weight:600; letter-spacing:-.02em; line-height:1;}
.counts{display:flex; gap:18px; flex-wrap:wrap; font-size:13px;}
.count{display:flex; align-items:center; gap:7px; color:var(--ink-2);}
.swatch{width:10px; height:10px; border-radius:2px; flex:none;}

section{margin-top:40px;}
h2{font-size:17px; font-weight:600; margin:0 0 4px; letter-spacing:-.005em;}
.note{color:var(--ink-2); font-size:13.5px; margin:0 0 18px; max-width:64ch;}

.epic{margin-bottom:26px; background:var(--surface); border:1px solid var(--line);
  border-radius:6px; overflow:hidden; box-shadow:var(--shadow);}
.epic > .bar{display:flex; align-items:center; gap:12px; padding:10px 16px;
  background:var(--accent); color:var(--accent-ink);}
.epic > .bar .name{font-size:12px; font-weight:600; text-transform:uppercase;
  letter-spacing:.1em;}
.epic > .bar .tally{margin-left:auto; font-size:12px; opacity:.85;}

.story{border-top:1px solid var(--line); padding:16px;}
.story:first-of-type{border-top:none}
.story h3{font-size:15px; font-weight:600; margin:0 0 6px;}
.story .narrative{font-size:14.5px; color:var(--ink-2); margin:0 0 14px; max-width:66ch;}
.story .narrative em{font-style:normal; color:var(--ink); font-weight:600;}
.story .narrative.absent{color:var(--stale); font-family:"IBM Plex Sans",sans-serif;
  font-size:13px;}

.rows{display:flex; flex-direction:column; gap:1px; background:var(--line);
  border:1px solid var(--line); border-radius:4px; overflow:hidden;}
.row{display:flex; align-items:center; gap:12px; padding:9px 12px;
  background:var(--surface); flex-wrap:wrap;}
.row .stripe{width:3px; align-self:stretch; border-radius:2px; flex:none; margin:-9px 0;}
.chip{font-size:10.5px; font-weight:600; text-transform:uppercase; letter-spacing:.07em;
  padding:3px 7px; border-radius:3px; color:var(--chip-ink); flex:none; min-width:74px;
  text-align:center;}
.chip.green{background:var(--verified)} .chip.red{background:var(--failing)}
.chip.stale{background:var(--stale)} .chip.pending{background:var(--pending)}
.chip.orphan{background:var(--orphan)}
.row .what{flex:1 1 260px; min-width:0;}
.row .rid{font-size:11.5px; color:var(--ink-3); flex:none;}
.row .flag{font-size:10.5px; letter-spacing:.06em; text-transform:uppercase;
  color:var(--orphan); border:1px solid var(--orphan); border-radius:3px; padding:1px 5px;
  flex:none;}
.row .weak{font-size:10.5px; letter-spacing:.06em; text-transform:uppercase;
  color:var(--stale); border:1px solid var(--stale); border-radius:3px; padding:1px 5px;
  flex:none;}
.row .kills{font-size:11px; color:var(--ink-3); flex:none;}
.row .why{flex-basis:100%; font-size:12px; color:var(--failing); margin-top:2px;
  overflow-x:auto; white-space:pre-wrap;}

.usecase{background:var(--surface); border:1px solid var(--line); border-radius:6px;
  padding:8px; overflow-x:auto; box-shadow:var(--shadow);}
.usecase svg{display:block; max-width:100%; height:auto;}

.lock{display:flex; flex-wrap:wrap; gap:10px 26px; background:var(--surface-2);
  border:1px solid var(--line); border-left:3px solid var(--accent); border-radius:4px;
  padding:12px 16px; font-size:13px;}
.lock div{display:flex; gap:8px; align-items:baseline;}
.lock .k{color:var(--ink-3); font-size:11px; text-transform:uppercase; letter-spacing:.08em;}
footer{margin-top:48px; padding-top:16px; border-top:1px solid var(--line);
  color:var(--ink-3); font-size:12px;}
@media (max-width:520px){
  .pct{font-size:32px} h1{font-size:22px} .wrap{padding-block:28px 48px}
  .chip{min-width:0}
}
"""


def _usecase_svg(entries: list) -> str:
    """Actors on the left, the capabilities they want on the right, associations between.

    Hand-laid rather than delegated to a layout library, so the dashboard stays a single file
    that opens with no network and no build step.
    """
    acts = actors(entries)
    if not acts:
        return ('<p class="note">No user stories yet. Add an '
                '<span class="mono">As a / I want / So that</span> block under '
                '<span class="mono">Feature:</span> and this diagram appears.</p>')
    caps: list[str] = []
    for items in acts.values():
        for it in items:
            if it["capability"] not in caps:
                caps.append(it["capability"])

    pad, row_h, col_a, col_u, u_w = 24, 58, 210, 300, 330
    rows = max(len(acts), len(caps))
    height = pad * 2 + max(rows * row_h, row_h)
    width = pad * 2 + col_a + col_u + u_w
    a_y = {a: pad + row_h / 2 + i * row_h + (row_h * (rows - len(acts)) / 2)
           for i, a in enumerate(acts)}
    u_y = {c: pad + row_h / 2 + i * row_h + (row_h * (rows - len(caps)) / 2)
           for i, c in enumerate(caps)}

    parts = [f'<svg viewBox="0 0 {width:.0f} {height:.0f}" width="{width:.0f}" '
             f'role="img" aria-label="Use case diagram: actors and the capabilities '
             f'they want">']
    # associations first, so nodes sit on top of the lines
    x1, x2 = pad + col_a, pad + col_a + col_u
    for actor, items in acts.items():
        for it in items:
            y1, y2 = a_y[actor], u_y[it["capability"]]
            mx = (x1 + x2) / 2
            parts.append(
                f'<path d="M {x1:.0f} {y1:.0f} C {mx:.0f} {y1:.0f} {mx:.0f} {y2:.0f} '
                f'{x2:.0f} {y2:.0f}" fill="none" stroke="var(--line)" stroke-width="1.5"/>')
    for actor, y in a_y.items():
        label = html.escape(actor)
        parts.append(
            f'<g><rect x="{pad}" y="{y - 17:.0f}" width="{col_a - 10}" height="34" rx="17" '
            f'fill="var(--accent)"/>'
            f'<text x="{pad + (col_a - 10) / 2:.0f}" y="{y + 5:.0f}" text-anchor="middle" '
            f'font-size="13" font-weight="600" fill="var(--accent-ink)" '
            f'font-family="IBM Plex Sans, sans-serif">{label}</text></g>')
    for cap, y in u_y.items():
        text = cap if len(cap) <= 44 else cap[:42].rstrip() + "…"
        parts.append(
            f'<g><rect x="{x2}" y="{y - 17:.0f}" width="{u_w - 10}" height="34" rx="6" '
            f'fill="var(--surface-2)" stroke="var(--accent)" stroke-width="1.5"/>'
            f'<text x="{x2 + 14}" y="{y + 5:.0f}" font-size="13" fill="var(--ink)" '
            f'font-family="IBM Plex Sans, sans-serif">{html.escape(text)}</text></g>')
    parts.append("</svg>")
    return "".join(parts)


def terse_failure(detail: str, limit: int = 4) -> str:
    """Pull the assertion out of a runner's failure dump.

    A raw pytest longrepr opens with fixture objects and frame locals -- noise that buries the
    one line a reader wants. Lines pytest marks with a leading "E" carry the assertion; other
    runners put the message on its own line, so fall back to the tail.
    """
    lines = [ln.rstrip() for ln in (detail or "").splitlines()]
    marked = [ln[1:].strip() if ln[:1] == "E" else ln.strip()
              for ln in lines if ln[:1] == "E" or ln.lstrip().startswith("[")]
    chosen = marked or [ln.strip() for ln in lines if ln.strip()][-limit:]
    seen, out = set(), []
    for ln in chosen:
        if ln and ln not in seen:
            seen.add(ln)
            out.append(ln)
    return "\n".join(out[:limit])


def _narrative_html(entry) -> str:
    if entry.actor and entry.capability:
        bits = (f'As a <em>{html.escape(entry.actor)}</em>, I want '
                f'{html.escape(entry.capability)}')
        if entry.benefit:
            bits += f', so that {html.escape(entry.benefit)}'
        return f'<p class="narrative prose">{bits}.</p>'
    return ('<p class="narrative absent">No user story on this feature — add an '
            '“As a / I want / So that” block under <span class="mono">Feature:</span>.</p>')


def dashboard_html(ledger, project: str = "", stack: str = "") -> str:
    entries = list(ledger.entries.values())
    s = ledger.summary()
    tree = build_tree(entries)
    by_rid = {e.rid: e for e in entries}

    detents = "".join(
        f'<div class="detent {sc.status}" title="{html.escape(sc.label)} — '
        f'{STATUS_LABEL[sc.status]}"></div>'
        for epic in tree for st in epic.children for sc in st.children) or \
        '<div class="detent"></div>'

    counts = "".join(
        f'<span class="count"><span class="swatch" style="background:var(--{k})"></span>'
        f'<span class="mono">{s.get(key, 0)}</span> {STATUS_LABEL[key]}</span>'
        for key, k in (("green", "verified"), ("red", "failing"), ("stale", "stale"),
                       ("pending", "pending"), ("orphan", "orphan"))
        if s.get(key, 0) or key in ("green", "red"))

    epics_html = []
    for epic in tree:
        stories = []
        for story in epic.children:
            rows = []
            for sc in story.children:
                e = by_rid[sc.key]
                flag = ('<span class="flag">held out</span>'
                        if "@holdout" in (e.tags or []) else "")
                # oracle strength: green means nothing if the assertions test nothing
                if e.mutants_killed == 0:
                    flag += '<span class="weak" title="detected no mutation at all">' \
                            'vacuous</span>'
                elif e.blind_spots:
                    flag += (f'<span class="weak" title="ran {e.blind_spots} mutated '
                             f'version(s) of code it executes without noticing">weak '
                             f'oracle</span>')
                kills = (f'<span class="kills mono" title="mutations detected">'
                         f'{e.mutants_killed}&times;</span>'
                         if e.mutants_killed else "")
                terse = terse_failure(e.failure) if e.failure else ""
                why = (f'<div class="why mono">{html.escape(terse[:400])}</div>'
                       if e.status == "red" and terse else "")
                var = STATUS_VAR[sc.status]
                rows.append(
                    f'<div class="row">'
                    f'<span class="stripe" style="background:var(--{var})"></span>'
                    f'<span class="chip {sc.status}">{STATUS_LABEL[sc.status]}</span>'
                    f'<span class="what">{html.escape(sc.label)}</span>{flag}{kills}'
                    f'<span class="rid mono">{html.escape(sc.key)}</span>{why}</div>')
            n_green = sum(1 for c in story.children if c.status == "green")
            stories.append(
                f'<div class="story"><h3>{html.escape(story.label)}</h3>'
                f'{_narrative_html(by_rid[story.children[0].key])}'
                f'<div class="eyebrow" style="margin-bottom:6px">'
                f'{n_green} of {len(story.children)} scenarios verified</div>'
                f'<div class="rows">{"".join(rows)}</div></div>')
        total = sum(len(st.children) for st in epic.children)
        green = sum(1 for st in epic.children for c in st.children if c.status == "green")
        epics_html.append(
            f'<div class="epic"><div class="bar">'
            f'<span class="name">{html.escape(epic.label)}</span>'
            f'<span class="tally mono">{green}/{total}</span></div>'
            f'{"".join(stories)}</div>')

    mut = ledger.mutation or {}
    weak = mut.get("weak_oracles") or {}
    mut_html = ""
    if mut:
        tone = "--failing" if weak else "--verified"
        mut_html = (
            f'<div class="lock" style="border-left-color:var({tone})">'
            f'<div><span class="k">mutation score</span><span class="mono">'
            f'{mut.get("score", 0)}%</span></div>'
            f'<div><span class="k">detected</span><span class="mono">'
            f'{mut.get("killed", 0)}</span></div>'
            f'<div><span class="k">survived</span><span class="mono">'
            f'{mut.get("survived", 0)}</span></div>'
            f'<div><span class="k">weak oracles</span><span class="mono">'
            f'{len(weak)}</span></div></div>')

    lock = ledger.spec_lock or {}
    lock_html = (
        f'<div class="lock">'
        f'<div><span class="k">approved by</span><span class="mono">'
        f'{html.escape(str(lock.get("approved_by", "not approved")))}</span></div>'
        f'<div><span class="k">at</span><span class="mono">'
        f'{html.escape(str(lock.get("approved_at", "—")))}</span></div>'
        f'<div><span class="k">scenarios signed off</span><span class="mono">'
        f'{len(lock.get("scenario_hashes", {})) or lock.get("scenario_count", 0)}</span></div>'
        f'<div><span class="k">regressions on record</span><span class="mono">'
        f'{len(ledger.regressions)}</span></div></div>')

    title = f"{project} Scenario Ledger" if project else "Scenario Ledger"
    return f"""<title>{html.escape(title)}</title>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=IBM+Plex+Sans:wght@400;600&family=IBM+Plex+Mono:wght@400;600&family=IBM+Plex+Serif:ital@0;1&display=swap">
<style>{_CSS}</style>
<div class="wrap">
  <header class="head">
    <h1>{html.escape(project or "Scenario Ledger")}</h1>
    <span class="sub">{html.escape(stack or "")} &middot; every obligation bound to the
      wording it was upheld against</span>
  </header>

  <div class="meter">
    <div class="eyebrow">Obligations upheld</div>
    <div class="detents">{detents}</div>
    <div class="pawl">
      <span class="pct mono">{s.get("completion_pct", 0)}%</span>
      <span class="counts">{counts}</span>
    </div>
  </div>
  {lock_html}
  {mut_html}

  <section>
    <h2>Who wants what</h2>
    <p class="note">Derived from the “As a / I want / So that” block on each feature — the
      actors and capabilities are the spec, not a separate drawing, so this cannot drift.</p>
    <div class="usecase">{_usecase_svg(entries)}</div>
  </section>

  <section>
    <h2>Breakdown</h2>
    <p class="note">Epic, then story, then one scenario per row. A parent is never greener
      than its children, and a scenario with no test bound to it reads
      <span class="mono">pending</span> — never verified. The
      <span class="mono">n&times;</span> count is how many mutations of the implementation
      that scenario detected: green with a low count is a scenario that may not be testing
      what its name claims.</p>
    {"".join(epics_html) or '<p class="note">No scenarios yet.</p>'}
  </section>

  <footer>
    Generated by <span class="mono">shalt dashboard</span> from
    <span class="mono">.shalt/ledger.json</span>. Regenerate after any run; this file is a
    snapshot, the ledger is the record.
  </footer>
</div>
"""


def write_dashboard(root: Path, ledger, project: str = "", stack: str = "") -> Path:
    out = Path(root) / "docs" / "dashboard.html"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(dashboard_html(ledger, project, stack), encoding="utf-8")
    return out
