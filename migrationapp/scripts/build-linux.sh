#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/env.sh"
cd "$HOME"
cargo build --release --manifest-path "$APP_ROOT/Cargo.toml" "$@"
echo "built $CARGO_TARGET_DIR/release/loos-switch"
