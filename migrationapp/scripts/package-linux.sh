#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/env.sh"
"$APP_ROOT/scripts/build-linux.sh"
APPDIR="$APP_ROOT/build/LoOS-Switch.AppDir"
rm -rf "$APPDIR"; mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/icons/hicolor/256x256/apps" "$APPDIR/usr/share/applications"
cp "$CARGO_TARGET_DIR/release/loos-switch" "$APPDIR/usr/bin/loos-switch"
cp "$APP_ROOT/assets/loos-switch.png" "$APPDIR/usr/share/icons/hicolor/256x256/apps/loos-switch.png"
cp "$APP_ROOT/assets/loos-switch.png" "$APPDIR/loos-switch.png"
cp "$APP_ROOT/assets/loos-switch.png" "$APPDIR/.DirIcon"
cp "$APP_ROOT/packaging/loos-switch.desktop" "$APPDIR/loos-switch.desktop"
cp "$APP_ROOT/packaging/AppRun" "$APPDIR/AppRun"
chmod +x "$APPDIR/AppRun" "$APPDIR/usr/bin/loos-switch"
mkdir -p "$APP_ROOT/build/tools"
TOOL="$APP_ROOT/build/tools/appimagetool-x86_64.AppImage"
if [ ! -x "$TOOL" ]; then
  echo "Downloading appimagetool (one-time packaging tool)…"
  curl -L --fail --retry 3 -o "$TOOL" https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-x86_64.AppImage
  chmod +x "$TOOL"
fi
mkdir -p "$APP_ROOT/dist"
OUT="$APP_ROOT/dist/LoOS-Switch-${VERSION:-0.1.0}-x86_64.AppImage"
ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL" --no-appstream "$APPDIR" "$OUT"
chmod +x "$OUT"
sha256sum "$OUT" > "$OUT.sha256"
echo "built $OUT"
