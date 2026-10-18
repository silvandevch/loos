#!/usr/bin/env bash
# 01-structure.sh — verify (and report) the assembled build layout.
# Does not modify anything; exits non-zero if a required piece is missing.
source "$(dirname "$0")/../env.sh"
set -uo pipefail
fails=0

check_dir()  { if [ -d "$2" ]; then log "OK   $1 -> $2"; else warn "MISS $1 -> $2"; fails=$((fails+1)); fi; }
check_file() { if [ -f "$2" ]; then log "OK   $1 -> $2"; else warn "MISS $1 -> $2"; fails=$((fails+1)); fi; }

log "== workspace =="
check_file "env"            "$LOOS_ROOT/env.sh"
log "== heavy assets (NVMe) =="
check_dir  "SDXROOT"        "$NT_SRC"
check_dir  "patchkit"       "$PATCHKIT"

log "== source tree landmarks =="
check_dir  "OOBE"           "$NT_SRC/base/ntsetup/oobe"
check_file "dirs"           "$NT_SRC/dirs"
check_file "razzle.cmd"     "$NT_SRC/tools/razzle.cmd"
check_file "razzle64.cmd"   "$NT_SRC/tools/razzle64.cmd"
check_file "prebuild.cmd"   "$NT_SRC/tools/prebuild.cmd"
check_file "missing.cmd"    "$NT_SRC/tools/missing.cmd"
check_file "postbuild.cmd"  "$NT_SRC/tools/postbuild.cmd"
check_file "certutil"       "$NT_SRC/certutil/generate.sh"

log "== x64 toolchains =="
check_dir  "x86 tools"      "$NT_SRC/tools/x86"
check_dir  "amd64 tools"    "$NT_SRC/tools/amd64"
check_dir  "win64 cross"    "$NT_SRC/tools/win64/x86/amd64"

if [ -d "$NT_SRC/.git" ]; then
    dirty=$(git -C "$NT_SRC" status --porcelain | wc -l)
    log "source tree modifications: $dirty"
fi

[ "$fails" -eq 0 ] && log "structure OK" || die "structure incomplete ($fails missing)"
