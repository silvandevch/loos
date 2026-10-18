#!/usr/bin/env bash
# fix-wine-assoc.sh — stop Wine from opening .cmd/.bat in notepad.
#
# By default a bare `wine foo.cmd` runs foo.cmd through the Windows shell file
# association, which under a fresh prefix is often "notepad". Our harness never
# does that (it always uses `wine cmd /c`, and .cmd->.cmd `call`s execute inside
# cmd.exe), but this restores the correct association so nothing can fall
# through to notepad.
#
# Idempotent. Safe to re-run.
source "$(dirname "$0")/../env.sh"
set -uo pipefail
require wine

WINEPREFIX="${WINEPREFIX:-$HOME/.wine}"
export WINEPREFIX

set_assoc() {
    local ext="$1" cls="$2"
    wine reg add "HKLM\\Software\\Classes\\$cls\\shell\\open\\command" \
        /ve /d "\"cmd.exe\" /c \"%1\" %*" /f >/dev/null 2>&1
    wine reg add "HKLM\\Software\\Classes\\$ext" /ve /d "$cls" /f >/dev/null 2>&1
    # per-user hive too (Wine sometimes prefers HKCU)
    wine reg add "HKCU\\Software\\Classes\\$cls\\shell\\open\\command" \
        /ve /d "\"cmd.exe\" /c \"%1\" %*" /f >/dev/null 2>&1
    wine reg add "HKCU\\Software\\Classes\\$ext" /ve /d "$cls" /f >/dev/null 2>&1
}

log "fixing Wine associations in $WINEPREFIX"
set_assoc ".cmd" "cmdfile"
set_assoc ".bat" "batfile"
log "done — .cmd/.bat now run via cmd.exe"

# verify: a throwaway .cmd must execute, not open an editor
probe="$(mktemp -d)/assoc-check.cmd"
printf '@echo off\r\necho ASSOC-OK\r\n' > "$probe"
out="$(timeout 60 wine cmd /c "$(winepath -w "$probe")" 2>/dev/null | tr -d '\r')"
rm -rf "$(dirname "$probe")"
if printf '%s' "$out" | grep -q 'ASSOC-OK'; then
    log "verification: LOOS-assoc OK"
else
    warn "verification inconclusive (output: ${out:-<empty>}) — harness still uses 'wine cmd /c'"
fi
