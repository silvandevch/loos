#!/usr/bin/env bash
# 09-build-all.sh — Linux/Wine driver for the NT 5.2 x64 build.
#
# Upstream build (RAZZLE) is Windows-only: batch + PE tooling (cl.exe/link.exe/
# build.exe) driven by tools\razzle.cmd. This harness runs it through Wine's
# cmd.exe.  Wine reports PROCESSOR_ARCHITECTURE=AMD64, so razzle takes its
# Native Support path and builds amd64 directly — one pass.  In a 32-bit (x86)
# prefix razzle builds x86 natively, and amd64 is cross-built with "win64
# amd64"; set CFG_CROSS=1 to force that form.
#
# Full compiles take hours. Run with a generous timeout; per-pass output is
# tee'd to $LOGS_DIR.
source "$(dirname "$0")/../env.sh"
set -uo pipefail
require wine

WINEPREFIX="${WINEPREFIX:-$HOME/.wine}"
export WINEPREFIX

to_win() { winepath -w "$1" 2>/dev/null; }
SRC_W="$(to_win "$NT_SRC")"
ROOT_W="$(to_win "$LOOS_ROOT")"
[ -n "$SRC_W" ] || die "winepath failed for $NT_SRC"

HOSTARCH="$(wine cmd /c 'echo %PROCESSOR_ARCHITECTURE%' 2>/dev/null | tr -d '\r' | tr -d '\n')"
log "wineprefix : $WINEPREFIX"
log "host arch  : $HOSTARCH"
log "SDXROOT    : $SRC_W"
log "LOOS_ROOT  : $ROOT_W"

name="build-amd64-$(date +%Y%m%d-%H%M%S)"
logf="$LOGS_DIR/$name.log"
log "=== build : build-all.cmd  (log: $logf) ==="

( NT_SRC="$SRC_W" LOOS_ROOT="$ROOT_W" CFG_CROSS="${CFG_CROSS:-0}" \
      wine cmd /c "$ROOT_W\\scripts\\build-all.cmd" ) > "$logf" 2>&1
rc=$?
log "build rc=$rc (log: $logf)"
[ "$rc" -eq 0 ] || die "build failed (see $logf)"
log "build complete — binaries under $NT_SRC\\binaries.<arch><type>"
