#!/usr/bin/env bash
# Shared build environment. The LoOS workspace replaces crates.io with a
# vendor directory under ../crates/vendor, so the app builds from $HOME using
# its own CARGO_HOME. All artifacts still land in migrationapp/target.
set -euo pipefail
APP_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
export APP_ROOT
export CARGO_HOME="${CARGO_HOME:-$APP_ROOT/.cargo-home}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$APP_ROOT/target}"
mkdir -p "$CARGO_HOME" "$CARGO_TARGET_DIR"
