#!/usr/bin/env bash
# Package Shalt.app as a zip + dmg a friend can open.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VER="${SHALT_VERSION:-0.1.0}"
ARCH="$(uname -m)"
case "$ARCH" in
  arm64) ARCH_TAG=arm64 ;;
  x86_64) ARCH_TAG=x86_64 ;;
  *) ARCH_TAG="$ARCH" ;;
esac
NAME="Shalt-${VER}-macos-${ARCH_TAG}"
DIST="$ROOT/dist"
APP="$ROOT/target/release/bundle/macos/Shalt.app"

if [[ ! -d "$APP" ]] || [[ "${1:-}" == "--build" ]]; then
  bash "$ROOT/scripts/macos-install.sh"
  APP="$ROOT/target/release/bundle/macos/Shalt.app"
fi
if [[ ! -d "$APP" ]]; then
  echo "no Shalt.app to pack" >&2
  exit 1
fi
if [[ ! -x "$APP/Contents/MacOS/shalt-cli" ]]; then
  echo "Shalt.app is missing shalt-cli — rebuild with macos-install.sh" >&2
  exit 1
fi

if command -v codesign >/dev/null 2>&1; then
  codesign --force --deep --sign - "$APP" 2>/dev/null || true
fi

rm -rf "$DIST"
mkdir -p "$DIST/stage"
cp -R "$APP" "$DIST/stage/Shalt.app"
cat > "$DIST/stage/How to install.txt" <<'EOF'
Shalt
=====

1. Drag Shalt.app to Applications.
2. First open: right-click the app → Open (macOS will warn because it is
   not from the App Store). After that, double-click is fine.
3. Opening the app also installs the `shalt` command for new terminals
   (~/.local/bin/shalt).

Keys: in the sidebar, Keys. Saved on this Mac only (~/.shalt/config.toml).
Play: pick a project, then Play. Local models need Ollama; cloud needs a key.

This is the same desk as `shalt ui`. It is a shareable window, not a
different product. Inner-loop work can also live in Ikonic.
EOF
ln -s /Applications "$DIST/stage/Applications"

ZIP="$DIST/${NAME}.zip"
DMG="$DIST/${NAME}.dmg"
ditto -c -k --keepParent "$DIST/stage/Shalt.app" "$ZIP"

rm -f "$DMG"
hdiutil create -volname "Shalt" -srcfolder "$DIST/stage" -ov -format UDZO "$DMG" >/dev/null

xattr -cr "$ZIP" "$DMG" 2>/dev/null || true
echo "Share either of these:"
echo "  $ZIP"
echo "  $DMG"
ls -lh "$ZIP" "$DMG"
