#!/usr/bin/env bash
# shalt/report/dump.sh — dumps the shalt ladder of a project as JSON.
#
# Usage:   bash dump.sh [PROJECT_DIR]   (defaults to $PWD)
# Output:  JSON on stdout, shape: { milestones, features, scenarios, nouns, verbs, protocols }
#          suitable for `window.SHALT_DATA` in ladder-template.html
#
# Reads:
#   .shalt/ledger.json      (status, feature, name, tags, spec_hash per rid)
#   .shalt/board.json       (milestones, sprints, items with rid/backend/model)
#   spec/*.feature          (Feature title, scenarios, Given/When/Then text)
#   .shalt/vocabulary.json  (optional — per-project nouns/verbs/protocols overrides)
#
# Requires: jq, awk, bash 4+.
set -euo pipefail

ROOT="${1:-$PWD}"
cd "$ROOT"

if [[ ! -d .shalt ]]; then
  echo "{\"error\":\"no .shalt dir in $ROOT — not a shalt project\"}" >&2
  exit 1
fi

LEDGER=".shalt/ledger.json"
BOARD=".shalt/board.json"
VOCAB=".shalt/vocabulary.json"

# ------------------------------------------------------------------
# 1. Parse spec/*.feature → {rid: {feature, title, gwt: [[kw, text], ...]}}
# ------------------------------------------------------------------
parse_specs() {
  local tmp
  tmp=$(mktemp)
  awk '
    BEGIN { feature=""; rid=""; title=""; split("", gwt) }
    /^Feature:/ { sub(/^Feature:[[:space:]]*/, ""); feature=$0; next }
    /@rid:/ {
      if (match($0, /@rid:S-[0-9a-f]+/)) rid=substr($0, RSTART+5, RLENGTH-5)
      next
    }
    /^[[:space:]]*Scenario:/ {
      sub(/^[[:space:]]*Scenario:[[:space:]]*/, ""); title=$0
      scn_rid=rid; scn_feature=feature; scn_title=title
      split("", gwt_arr); gwt_n=0
      next
    }
    /^[[:space:]]*(Given|When|Then|And|But)[[:space:]]/ {
      kw=$1; sub(/^[[:space:]]*(Given|When|Then|And|But)[[:space:]]+/, "")
      gwt_arr[++gwt_n] = kw "\t" $0
      next
    }
    /^[[:space:]]*$/ {
      if (scn_rid != "" && gwt_n > 0) {
        for (i=1; i<=gwt_n; i++) print scn_rid "\t" scn_feature "\t" scn_title "\t" gwt_arr[i]
        scn_rid=""; gwt_n=0
      }
    }
    END {
      if (scn_rid != "" && gwt_n > 0) {
        for (i=1; i<=gwt_n; i++) print scn_rid "\t" scn_feature "\t" scn_title "\t" gwt_arr[i]
      }
    }
  ' spec/*.feature 2>/dev/null > "$tmp" || true

  jq -Rn '
    [inputs | split("\t") | {rid:.[0], feature:.[1], title:.[2], kw:.[3], text:.[4]}]
    | group_by(.rid)
    | map({
        key: ("scenario:" + .[0].rid),
        value: {
          kind: "scenario",
          rid: .[0].rid,
          title: .[0].title,
          feature: .[0].feature,
          gwt: [.[] | [.kw, .text]]
        }
      })
    | from_entries
  ' < "$tmp"
  rm -f "$tmp"
}

SCENARIOS=$(parse_specs)

# ------------------------------------------------------------------
# 2. Merge ledger status into scenarios
# ------------------------------------------------------------------
if [[ -f $LEDGER ]]; then
  SCENARIOS=$(jq --argjson led "$(cat "$LEDGER")" '
    to_entries | map(
      .value as $s |
      ($led.scenarios[$s.rid] // {}) as $l |
      .value += {
        status: ($l.status // "undefined"),
        last_green_at: $l.last_green_at,
        epic: $l.epic,
        spec_hash: $l.spec_hash,
        parent: ("feature:" + ($s.feature | ascii_downcase | gsub("[^a-z0-9]+"; "-")))
      }
    ) | from_entries
  ' <<< "$SCENARIOS")
fi

# ------------------------------------------------------------------
# 3. Features, derived from scenarios' .feature field
# ------------------------------------------------------------------
FEATURES=$(jq '
  [to_entries[].value] | group_by(.feature) | map({
    key: ("feature:" + (.[0].feature | ascii_downcase | gsub("[^a-z0-9]+"; "-"))),
    value: {
      kind: "feature",
      title: .[0].feature,
      status: (if any(.[]; .status == "red") then "red"
               elif any(.[]; .status == "amber") then "amber"
               elif all(.[]; .status == "green") then "green"
               else "amber" end),
      children: [.[] | ("scenario:" + .rid)],
      childrenLabel: "scenarios",
      fields: [["scenarios", (length | tostring)]]
    }
  }) | from_entries
' <<< "$SCENARIOS")

# ------------------------------------------------------------------
# 4. Milestones, from board.json (or a synthetic one holding every feature)
# ------------------------------------------------------------------
MILESTONES=$(
  if [[ -f $BOARD ]] && jq -e '.milestones | length > 0' "$BOARD" > /dev/null 2>&1; then
    jq --argjson feats "$FEATURES" '
      .milestones | map({
        key: ("milestone:" + .id),
        value: {
          kind: "milestone",
          title: .title,
          childrenLabel: "features",
          children: [$feats | to_entries[] | .key],
          fields: []
        }
      }) | from_entries
    ' "$BOARD"
  else
    jq --argjson feats "$FEATURES" -n '
      {"milestone:project": {
         kind: "milestone",
         title: "Project",
         childrenLabel: "features",
         children: [$feats | to_entries[] | .key],
         fields: []
      }}
    '
  fi
)

# ------------------------------------------------------------------
# 5. Vocabulary — nouns / verbs / protocols
#    Start from .shalt/vocabulary.json if present, otherwise a tiny default.
# ------------------------------------------------------------------
if [[ -f $VOCAB ]]; then
  VOCABULARY=$(cat "$VOCAB")
else
  VOCABULARY='{"nouns":[],"verbs":[],"protocols":[{"ref":"protocol:http","title":"HTTP","patterns":["\\bHTTP\\b"]}]}'
fi

# ------------------------------------------------------------------
# 6. Emit the combined blob
# ------------------------------------------------------------------
jq -n \
  --argjson milestones "$MILESTONES" \
  --argjson features   "$FEATURES" \
  --argjson scenarios  "$SCENARIOS" \
  --argjson vocabulary "$VOCABULARY" \
  --arg     generated_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg     project "$(basename "$ROOT")" \
  '{
    schema: "shalt.report/1",
    project: $project,
    generated_at: $generated_at,
    DATA: ($milestones + $features + $scenarios),
    vocabulary: $vocabulary
  }'
