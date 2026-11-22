# shalt/report — generate the interactive ladder for a project

Two files, one recipe. The output is a single HTML page the host (Claude
desktop app, Grok, Hermes, Codex) can render inline — a click-driven
popover ladder with milestones → features → scenarios on the vertical
axis and nouns/verbs/protocols cross-cut on the horizontal.

## Files

- **`dump.sh`** — reads `.shalt/ledger.json`, `.shalt/board.json`, and
  `spec/*.feature` in a shalt project and prints the ladder as JSON.
  Shape: `{ schema, project, generated_at, DATA, vocabulary }`. The
  `DATA` object is keyed by `milestone:… / feature:… / scenario:…`;
  `vocabulary` holds `{nouns, verbs, protocols}` arrays, each with
  `{ ref, title, patterns, fields?, children? }`.
- **`ladder-template.html`** — the page. Reads `window.SHALT_REPORT`
  at startup; renders a structured document + click-anywhere popover
  with drill-through child buttons and vocabulary highlights. Theme
  follows the viewer (light/dark). Bottom-sheet variant on screens
  below 560px.

## Recipe (for any agent)

```bash
# 1. Dump the project's ladder
bash ~/.claude/skills/shalt/report/dump.sh "$PROJECT_DIR" > /tmp/shalt-report.json

# 2. Splice it into the template
PROJECT=$(jq -r .project /tmp/shalt-report.json)
GENERATED=$(jq -r .generated_at /tmp/shalt-report.json)
sed -e "s/__PROJECT__/$PROJECT/g" -e "s/__GENERATED_AT__/$GENERATED/g" \
    ~/.claude/skills/shalt/report/ladder-template.html > /tmp/shalt-ladder.html

# 3. Inject the data as a script tag right before the first <script>
python3 -c "
import sys, json
html = open('/tmp/shalt-ladder.html').read()
data = open('/tmp/shalt-report.json').read()
injection = f'<script>window.SHALT_REPORT = {data};</script>\n'
open('/tmp/shalt-ladder.html','w').write(html.replace('<script>', injection + '<script>', 1))
"
```

## Publishing

- **Claude desktop app**: publish `/tmp/shalt-ladder.html` with the
  Artifact tool (`icon: "ladder"`). Returns a shareable URL the user
  can open in the side panel.
- **Grok / Hermes / Codex**: emit the HTML inline wherever the host
  renders attachments. The template is self-contained — no external
  scripts or stylesheets beyond fonts.

## Extending the vocabulary

Create `.shalt/vocabulary.json` in the project to override the default
(which only knows `HTTP`):

```json
{
  "nouns": [
    { "ref": "noun:task", "title": "task",
      "patterns": ["\\btasks?\\b"],
      "fields": [["role","entity"], ["door","GET /tasks · POST /tasks"]] }
  ],
  "verbs":     [ { "ref": "verb:add",   "title": "add",   "patterns": ["\\badd(?:s|ing|ed)?\\b"] } ],
  "protocols": [ { "ref": "protocol:http", "title": "HTTP", "patterns": ["\\bHTTP\\b"] } ]
}
```

Patterns are JavaScript regex source (case-insensitive by default). Each
ref becomes a clickable popover showing its fields and back-references
to every scenario that mentions it.

## Session continuity — `agent-state.json`

Record the published URLs in `.shalt/agent-state.json` so a later
session can pick up where this one left off. Shape:

```json
{
  "schema": "shalt.agent-state/1",
  "artifacts": {
    "chart":             { "url": "https://claude.ai/artifact/...", "note": "static ladder",     "last_published_at": "..." },
    "chart_editable":    { "url": "https://claude.ai/artifact/...", "note": "editable ladder",   "last_published_at": "..." },
    "wireframe":         { "url": "https://claude.ai/artifact/...", "note": "static mockup",     "last_published_at": "..." },
    "wireframe_editable":{ "url": "https://claude.ai/artifact/...", "note": "editable mockup",   "last_published_at": "..." },
    "end_product":       { "url": "https://claude.ai/artifact/...", "note": "rendered sample",   "last_published_at": "..." }
  }
}
```

Write this file the first time you publish each surface, and update
`last_published_at` on every re-publish. A later session that gets
"continue the recipes project" reads the file, pulls each URL with
`Artifact({ action: "read", url })`, extracts the embedded seed (the
`<script>window.SHALT_REPORT = {...}</script>` or `window.WIREFRAME =
{...}`), diffs against the project, and acts on the diff and any
`build_requested` flags. See SKILL.md § "Continuing — pick up where we
left off from the chart" for the mapping table.

## The "build" button on the editable chart

Clicking `▶ build` on a scenario in the editable chart writes
`build_requested: true` into the chart's state and copies a ready-paste
prompt (`apply the chart and build S-xxxxxxxx (title)`) to the
viewer's clipboard. The flag is persistent, visible to any viewer, and
appears in every subsequent read of the chart. **It does not auto-spawn
an agent** — no artifact capability can start a filesystem-attached
Claude Code / Codex session today. The queue is a coordination surface;
an agent (a Claude Code session, a `shalt agent watch` loop you run
yourself) still has to pick it up. The clipboard copy + the per-session
polling pattern in the skill is the current bridge.

## Known gaps / TODO

- The dumper parses Gherkin with awk — tolerant but not a full parser.
  Scenario Outlines, Rules, and doc strings are passed through as
  plain GWT lines and may not fully parse. Fix: use a real Gherkin
  parser or extend `shalt verify` to emit GWT in the ledger.
- Mockups (`mockups/*.html`) are not yet threaded into the popover as
  "see also" links. When they're linked to scenario rids, add them.
- `shalt report --json` in the binary would obsolete `dump.sh`. Target.
- `shalt agent watch` — a local daemon that polls the chart's
  `build_requested` state via Artifact `read` and shells into Claude
  Code / Codex to run the loop. Would make the ▶ build button actually
  autonomous.
