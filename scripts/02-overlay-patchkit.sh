#!/usr/bin/env bash
# 02-overlay-patchkit.sh — merge the openxp/patchkit overlay into SDXROOT.
#
# patchkit ships 2046 files; relative to the anika tree they break down as:
#   ~1575 additive paths (new x64 sources/tools the tree lacks)
#   ~150  byte-identical
#   ~321  paths that would overwrite anika's modern-build patches
#
# Default policy is ADDITIVE: only missing paths are copied. The 321 conflicting
# files are left at their anika versions because anika's copies are the ones
# validated for a full modern build; use --force to take the patchkit versions
# (every overwritten file is copied to backups/ first).
#
# NOTE: these conflicts were subsequently resolved in favour of patchkit by
# scripts/11-resolve-patchkit-conflicts.sh (backup + line-ending normalise);
# re-running this script additive-only will not undo that.
source "$(dirname "$0")/../env.sh"
set -uo pipefail
require rsync cmp

FORCE=0
[ "${1:-}" = "--force" ] && FORCE=1

[ -d "$NT_SRC/.git" ] || die "source tree not found at $NT_SRC (run 00-fetch.sh)"
[ -d "$PATCHKIT" ]    || die "patchkit not found at $PATCHKIT (run 00-fetch.sh)"

stamp="$(date +%Y%m%d-%H%M%S)"
bk="$BACKUP_DIR/patchkit-overlay-$stamp"
added=0; same=0; overwritten=0; skipped=0

while IFS= read -r rel; do
    src="$PATCHKIT/$rel"
    dst="$NT_SRC/$rel"
    if [ ! -e "$dst" ]; then
        mkdir -p "$(dirname "$dst")"
        cp -p "$src" "$dst"
        added=$((added+1))
    elif cmp -s "$src" "$dst"; then
        same=$((same+1))
    else
        if [ "$FORCE" -eq 1 ]; then
            mkdir -p "$bk/$(dirname "$rel")"
            cp -p "$dst" "$bk/$rel"
            cp -p "$src" "$dst"
            overwritten=$((overwritten+1))
        else
            skipped=$((skipped+1))
        fi
    fi
done < <(cd "$PATCHKIT" && find . -path ./.git -prune -o -type f -print | sed 's|^\./||')

log "additive copies : $added"
log "identical       : $same"
log "conflict skipped: $skipped  (use --force to overwrite)"
[ "$FORCE" -eq 1 ] && { log "overwritten     : $overwritten"; log "backups         : $bk"; }

# Record the overlay result for provenance.
{ echo "overlay $stamp force=$FORCE added=$added same=$same skipped=$skipped overwritten=$overwritten"
  [ "$FORCE" -eq 1 ] && echo "backup=$bk"; } >> "$LOGS_DIR/overlay.log"

log "overlay complete"
