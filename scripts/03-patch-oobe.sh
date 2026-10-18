#!/usr/bin/env bash
# 03-patch-oobe.sh — apply the openxp/patchkit OOBE build fixes to SDXROOT.
#
# NOTE ON PATHS: the request referred to "enduser/oobe". No such directory
# exists; OOBE lives at base/ntsetup/oobe. This script patches the real path.
#
# The three fixes patchkit carries for OOBE (all verified against this tree):
#   base/ntsetup/oobe/idl/sources        - drop `licdlli.c` (source not shipped)
#   base/ntsetup/oobe/msobmain/msobmain.h  - disable #include "licdll.h" and the
#                                            non-existent "setup.h"; drop the
#                                            ICOMLicenseAgent member
#   base/ntsetup/oobe/msobmain/msobmain.cpp - patched implementation (new file)
# plus the OOBE html image resources the tree lacks.
#
# Every file that already exists is copied to backups/ before replacement.
source "$(dirname "$0")/../env.sh"
set -uo pipefail
require cp diff

OOBE_SRC="$PATCHKIT/base/ntsetup/oobe"
OOBE_DST="$NT_SRC/base/ntsetup/oobe"
[ -d "$OOBE_SRC" ] || die "patchkit OOBE payload missing at $OOBE_SRC"
[ -d "$OOBE_DST" ] || die "target OOBE tree missing at $OOBE_DST"

stamp="$(date +%Y%m%d-%H%M%S)"
bk="$BACKUP_DIR/oobe-$stamp"
changed=0; backed=0

log "patching OOBE: $OOBE_DST"
while IFS= read -r rel; do
    src="$OOBE_SRC/$rel"
    dst="$OOBE_DST/$rel"
    if [ -e "$dst" ] && cmp -s "$src" "$dst"; then
        continue
    fi
    if [ -e "$dst" ]; then
        mkdir -p "$bk/$(dirname "$rel")"
        cp -p "$dst" "$bk/$rel"
        backed=$((backed+1))
    fi
    mkdir -p "$(dirname "$dst")"
    cp -p "$src" "$dst"
    log "  ${dst#$NT_SRC/}  $([ -e "$bk/$rel" ] && echo '(replaced)' || echo '(added)')"
    changed=$((changed+1))
done < <(cd "$OOBE_SRC" && find . -type f | sed 's|^\./||')

# The patched msobmain.cpp references UpdateMuiFiles()/RemovePersistData() that
# the pre-patch header lacked; confirm the header now declares what cpp uses.
hdr="$OOBE_DST/msobmain/msobmain.h"
grep -q 'void UpdateMuiFiles()' "$hdr" || warn "msobmain.h missing UpdateMuiFiles() decl"
grep -q '//#include "licdll.h"' "$hdr" || warn "msobmain.h still includes licdll.h"
if grep -qE '^[[:space:]]+licdlli\.c[[:space:]]*\\?$' "$OOBE_DST/idl/sources"; then
    warn "idl/sources still lists licdlli.c in SOURCES"
fi

{ echo "oobe $stamp changed=$changed backed_up=$backed"
  [ "$backed" -gt 0 ] && echo "backup=$bk"; } >> "$LOGS_DIR/oobe.log"

log "OOBE patched ($changed files, $backed backed up)"
if [ "$backed" -gt 0 ]; then log "backups: $bk"; fi
exit 0
