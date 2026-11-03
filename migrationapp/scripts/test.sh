#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/env.sh"
cd "$HOME"
cargo test --manifest-path "$APP_ROOT/Cargo.toml"
"$CARGO_TARGET_DIR/debug/loos-switch" --self-test
"$CARGO_TARGET_DIR/debug/loos-switch" --list-disks
