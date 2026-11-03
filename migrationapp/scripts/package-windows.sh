#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/env.sh"
"$APP_ROOT/scripts/build-windows.sh"
mkdir -p "$APP_ROOT/dist"
OUT="$APP_ROOT/dist/LoOS-Switch-${VERSION:-0.1.0}-windows-x64.exe"
cp "$CARGO_TARGET_DIR/x86_64-pc-windows-gnu/release/loos-switch.exe" "$OUT"
sha256sum "$OUT" > "$OUT.sha256"
echo "built $OUT (unsigned; UAC will ask for administrator)"
