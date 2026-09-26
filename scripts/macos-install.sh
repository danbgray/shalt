#!/usr/bin/env bash
# Build Shalt.app and copy it into /Applications.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if ! command -v cargo-tauri >/dev/null 2>&1; then
  echo "installing tauri-cli…"
  cargo install tauri-cli --locked
fi

# cargo-tauri wants to run from the app crate; the workspace target is at repo root.
cd "$ROOT/crates/shalt-app"
cargo tauri build --bundles app

APP="$ROOT/target/release/bundle/macos/Shalt.app"
if [[ ! -d "$APP" ]]; then
  # tauri 2 sometimes writes next to the crate
  APP="$ROOT/crates/shalt-app/target/release/bundle/macos/Shalt.app"
fi
if [[ ! -d "$APP" ]]; then
  echo "build finished but Shalt.app was not where we expected" >&2
  find "$ROOT" -name 'Shalt.app' -type d | head
  exit 1
fi

if command -v codesign >/dev/null 2>&1; then
  codesign --force --deep --sign - "$APP" 2>/dev/null || true
fi

dest="/Applications/Shalt.app"
rm -rf "$dest"
cp -R "$APP" "$dest"
xattr -cr "$dest" 2>/dev/null || true
echo "Installed $dest"

install_cli() {
  local src="$1"
  local dest="$2"
  mkdir -p "$(dirname "$dest")"
  cp "$src" "$dest"
  chmod +x "$dest"
}

CLI=""
if [[ -x "$dest/Contents/MacOS/shalt-cli" ]]; then
  CLI="$dest/Contents/MacOS/shalt-cli"
else
  CLI="$(ls "$dest/Contents/MacOS"/shalt-cli-* 2>/dev/null | head -1 || true)"
fi
if [[ -n "$CLI" && -x "$CLI" ]]; then
  install_cli "$CLI" "$HOME/.local/bin/shalt"
  if [[ -d "$HOME/.cargo/bin" ]]; then
    install_cli "$CLI" "$HOME/.cargo/bin/shalt"
  fi
  for d in /opt/homebrew/bin /usr/local/bin; do
    if [[ -d "$d" && -w "$d" ]]; then
      install_cli "$CLI" "$d/shalt"
    fi
  done
  MARKER='# shalt (Shalt.app)'
  if ! grep -qF "$MARKER" "$HOME/.zprofile" 2>/dev/null \
    && ! grep -qE '\$HOME/\.local/bin|~/\.local/bin' "$HOME/.zprofile" 2>/dev/null \
    && ! grep -qE '\$HOME/\.local/bin|~/\.local/bin' "$HOME/.zshrc" 2>/dev/null; then
    printf '\n%s\nexport PATH="$HOME/.local/bin:$PATH"\n' "$MARKER" >> "$HOME/.zprofile"
  fi
  echo "Installed shalt CLI → $HOME/.local/bin/shalt"
  echo "New terminals will see it. This one: export PATH=\"\$HOME/.local/bin:\$PATH\""
fi
echo "Open it from Applications, or: open -a Shalt"
