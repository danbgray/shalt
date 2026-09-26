#!/usr/bin/env bash
# Build the shalt CLI sidecar. Invoked by Tauri beforeBuildCommand (cwd may vary).
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$HERE"
while [[ "$ROOT" != "/" ]]; do
  if [[ -f "$ROOT/Cargo.toml" ]] && grep -q 'crates/shalt' "$ROOT/Cargo.toml"; then
    break
  fi
  ROOT="$(cd "$ROOT/.." && pwd)"
done
cd "$ROOT"
cargo build -p shalt --release
HOST="$(rustc -vV | awk '/^host:/{print $2}')"
DEST_DIR="$HERE/binaries"
mkdir -p "$DEST_DIR"
cp "$ROOT/target/release/shalt" "$DEST_DIR/shalt-cli-$HOST"
chmod +x "$DEST_DIR/shalt-cli-$HOST"
echo "sidecar $DEST_DIR/shalt-cli-$HOST"
