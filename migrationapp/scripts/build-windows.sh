#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/env.sh"
export RUSTUP_HOME="${RUSTUP_HOME:-$APP_ROOT/build/rustup}"
if [ ! -x "$RUSTUP_HOME/toolchains/1.97.1-x86_64-unknown-linux-gnu/bin/rustc" ]; then
  echo "The workspace-local 1.97.1 toolchain is not installed." >&2
  echo "Run: RUSTUP_HOME=$RUSTUP_HOME rustup toolchain install 1.97.1 --profile minimal --target x86_64-pc-windows-gnu" >&2
  exit 1
fi
cd "$HOME"
RUSTUP_HOME="$RUSTUP_HOME" rustup run 1.97.1 cargo build --release \
  --manifest-path "$APP_ROOT/Cargo.toml" --target x86_64-pc-windows-gnu "$@"
echo "built $CARGO_TARGET_DIR/x86_64-pc-windows-gnu/release/loos-switch.exe"
