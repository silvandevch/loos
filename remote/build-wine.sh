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
SRC_W="$(winepath -w "$NT_SRC")"
ROOT_W="$(winepath -w "$LOOS_ROOT")"

log "== phase 1/2: build (native AMD64 under Wine) =="
bash "$LOOS_ROOT/scripts/09-build-all.sh" || die "build failed"

log "== phase 2/2: ISO (oscdimg under Wine) =="
iso_log="$LOGS_DIR/iso-$(date +%Y%m%d-%H%M%S).log"
( VARIANT="$VARIANT" ISO_OUT="$ISO_OUT" \
  wine cmd /c "cd /d $SRC_W && tools\\razzle.cmd free offline no_certcheck exec \"$ROOT_W\\scripts\\_iso.cmd\"" ) \
  > "$iso_log" 2>&1
rc=$?
log "iso rc=$rc (log: $iso_log)"
[ "$rc" -eq 0 ] || die "iso build failed"

iso_win="$(winepath -u "$ISO_OUT" 2>/dev/null || echo "$ISO_OUT")"
if [ -f "$iso_win" ]; then
    log "ISO: $iso_win ($(du -h "$iso_win" | cut -f1))"
    log "boot it: remote/run-qemu.sh   (ISO=$iso_win)"
else
    warn "no ISO at $iso_win — postbuild layout may be incomplete (variant '$VARIANT')"
fi
