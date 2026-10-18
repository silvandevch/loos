#!/usr/bin/env bash
# build-wine.sh — build + make the ISO entirely on this host under Wine.
# No VM needed. Produces an ISO with the tree's own oscdimg.exe.
#
#   scripts/../remote/build-wine.sh [variant]
#
# The heavy lifting is scripts/09-build-all.sh (razzle prebuild + build). Then a
# second razzle session runs _iso.cmd. Env: WINEPREFIX, VARIANT, ISO_OUT.
source "$(dirname "$0")/../env.sh"
set -uo pipefail
require wine winepath

WINEPREFIX="${WINEPREFIX:-$HOME/.wine}"; export WINEPREFIX
VARIANT="${1:-${VARIANT:-srv}}"
ISO_OUT="${ISO_OUT:-$NT_OUT/loos.iso}"
log "== phase 1/2: build (native AMD64 under Wine) =="
bash "$LOOS_ROOT/scripts/09-build-all.sh" || die "build failed"

log "== phase 2/2: ISO (oscdimg under Wine) =="
VARIANT="$VARIANT" ISO_OUT="$ISO_OUT" bash "$LOOS_ROOT/scripts/_iso-run.sh"
