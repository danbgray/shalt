#!/usr/bin/env bash
# Runs all three demos end to end, offline, with no API key.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FIX="$HERE/fixtures"
WORK="${1:-/tmp/shalt-demo}"
R="python3 -m shalt.cli"

bootstrap () {  # $1 = workspace
  rm -rf "$1"
  $R --root "$1" init >/dev/null
  $R --root "$1" --fixtures "$FIX/honest" author "Parse invoices and total them exactly." >/dev/null
  $R --root "$1" --fixtures "$FIX/honest" approve --yes --by "demo@example.com" >/dev/null
  $R --root "$1" --fixtures "$FIX/honest" steps >/dev/null
}

echo "################ 1. the honest loop ################"
bootstrap "$WORK/honest"
$R --root "$WORK/honest" --fixtures "$FIX/honest" build --max-turns 5

echo
echo "################ 2. the implementer edits the tests ################"
bootstrap "$WORK/cheating"
$R --root "$WORK/cheating" --fixtures "$FIX/cheating" build --max-turns 2 || true
echo
if diff -q "$WORK/cheating/steps/test_invoice.py" \
           "$FIX/honest/stepwright/turn1/steps/test_invoice.py" >/dev/null; then
  echo "OK: steps/ is byte-identical to what the stepwright wrote."
fi

echo
echo "################ 3. the implementer overfits to what it saw ################"
bootstrap "$WORK/overfit"
$R --root "$WORK/overfit" --fixtures "$FIX/overfit" build --max-turns 3

echo
echo "################ 4. the breakdown ################"
$R --root "$WORK/honest" tree
echo
echo "################ 5. who wants what ################"
$R --root "$WORK/honest" stories
echo
echo "################ 6. diagrams and dashboard ################"
$R --root "$WORK/honest" diagrams
$R --root "$WORK/honest" dashboard

echo
echo "################ 7. does the oracle mean anything? ################"
echo "# a stepwright whose assertions do not check the value. every scenario goes green."
bootstrap_weak () {
  rm -rf "$1"
  $R --root "$1" init >/dev/null
  $R --root "$1" --fixtures "$FIX/honest" author "Invoicing" >/dev/null
  $R --root "$1" --fixtures "$FIX/honest" approve --yes --by "demo@example.com" >/dev/null
  $R --root "$1" --fixtures "$FIX/weak-oracle" steps >/dev/null
  $R --root "$1" --fixtures "$FIX/honest" build --max-turns 5 >/dev/null
}
bootstrap_weak "$WORK/weak"
$R --root "$WORK/weak" status | tail -2
echo
echo "# 100% green. now break the implementation and see which scenarios notice:"
$R --root "$WORK/weak" mutate --budget 30 || true
