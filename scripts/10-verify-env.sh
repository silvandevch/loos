#!/usr/bin/env bash
# 10-verify-env.sh — pre-flight checks for building from this Linux host.
source "$(dirname "$0")/../env.sh"
set -uo pipefail
fails=0
ok()   { printf '  \033[1;32mOK\033[0m   %s\n' "$*"; }
bad()  { printf '  \033[1;31mFAIL\033[0m %s\n' "$*"; fails=$((fails+1)); }
note() { printf '  \033[1;33mNOTE\033[0m %s\n' "$*"; }

echo "== host =="
ok "arch $(uname -m), $(nproc) cpus"
echo "== tooling =="
for c in bash git rsync openssl perl python3 7z patch wine winepath; do
    command -v "$c" >/dev/null 2>&1 && ok "$c" || bad "$c missing"
done

echo "== wine prefix =="
if [ -d "${WINEPREFIX:-$HOME/.wine}" ]; then
    ok "WINEPREFIX=${WINEPREFIX:-$HOME/.wine}"
    if [ -f "${WINEPREFIX:-$HOME/.wine}/drive_c/windows/syswow64/cmd.exe" ]; then
        ok "32-bit cmd.exe present (WOW64)"
    else
        note "32-bit cmd.exe not seen; razzle/build tools are 32-bit PE and need a 32-bit-capable prefix"
    fi
else
    note "no wineprefix yet; run: WINEARCH=win32 WINEPREFIX=\$HOME/.wine wineboot -i"
fi

echo "== disk =="
df -h "$BUILD_ROOT" | awk 'NR==1 || /[0-9]%/{print "  "$0}'

echo "== source tree =="
[ -d "$NT_SRC/.git" ] && ok "SDXROOT present ($(find "$NT_SRC" -path "$NT_SRC/.git" -prune -o -type f -print | wc -l) files)" \
                     || bad "SDXROOT missing at $NT_SRC"

echo "== x64 cross toolchain =="
[ -f "$NT_SRC/tools/win64/x86/amd64/cl.exe" ] && ok "win64/x86/amd64 cl.exe" \
    || note "tools/win64/x86/amd64/cl.exe absent — the AMD64 pass needs it (see patchkit tools/amd64)"

echo
[ "$fails" -eq 0 ] && echo "pre-flight OK" || { echo "$fails hard failures"; exit 1; }
