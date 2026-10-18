#!/usr/bin/env bash
# _iso-run.sh — (re)make the bootable ISO under Wine, using the tree's oscdimg.
# Requires an existing build (postbuild layout). `loos iso` calls this.
source "$(dirname "$0")/../env.sh"
set -uo pipefail
require wine winepath

WINEPREFIX="${WINEPREFIX:-$HOME/.wine}"; export WINEPREFIX
VARIANT="${VARIANT:-srv}"
ISO_OUT="${ISO_OUT:-$NT_OUT/loos.iso}"
SRC_W="$(winepath -w "$NT_SRC")"
ROOT_W="$(winepath -w "$LOOS_ROOT")"

log "== ISO (variant=$VARIANT, out=$ISO_OUT) =="
iso_log="$LOGS_DIR/iso-$(date +%Y%m%d-%H%M%S).log"
( VARIANT="$VARIANT" ISO_OUT="$ISO_OUT" \
  wine cmd /c "cd /d $SRC_W && tools\\razzle.cmd free offline no_certcheck exec \"$ROOT_W\\scripts\\_iso.cmd\"" ) \
  > "$iso_log" 2>&1
rc=$?
log "iso rc=$rc (log: $iso_log)"
[ "$rc" -eq 0 ] || die "iso build failed"
iso_win="$(winepath -u "$ISO_OUT" 2>/dev/null || echo "$ISO_OUT")"
[ -f "$iso_win" ] && log "ISO: $iso_win ($(du -h "$iso_win" | cut -f1))" \
                  || warn "no ISO produced — is the postbuild layout complete?"
