#!/bin/sh
# Point Hermes / Claude / Grok / Codex at the full shalt skill (not Play-only).
set -e
SKILL=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
link() {
    dest=$1
    mkdir -p "$dest"
    ln -sfn "$SKILL" "$dest/shalt"
    echo "linked $dest/shalt -> $SKILL"
}
link "${HERMES_HOME:-$HOME/.hermes}/skills/software-development"
link "${HOME}/.claude/skills"
link "${GROK_HOME:-$HOME/.grok}/skills"
link "${HOME}/.codex/skills"
echo "invoke: shalt --root <workspace> play"
