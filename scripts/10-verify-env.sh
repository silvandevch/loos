#!/usr/bin/env bash
# 10-verify-env.sh — pre-flight checks for building LoOS on Linux.
source "$(dirname "$0")/../env.sh"
set -uo pipefail
fails=0
ok()   { printf '  \033[1;32mOK\033[0m   %s\n' "$*"; }
bad()  { printf '  \033[1;31mFAIL\033[0m %s\n' "$*"; fails=$((fails+1)); }
note() { printf '  \033[1;33mNOTE\033[0m %s\n' "$*"; }

echo "== host =="
ok "arch $(uname -m), $(nproc) cpus"
echo "== LoOS build tooling =="
for c in bash cmake ninja clang cargo rustc python3 x86_64-w64-mingw32-gcc x86_64-w64-mingw32-windres; do
    command -v "$c" >/dev/null 2>&1 && ok "$c" || bad "$c missing"
done

echo "== disk =="
df -h "$OUT_ROOT" | awk 'NR==1 || /[0-9]%/{print "  "$0}'

echo "== LoOS source =="
[ -f "$LOOS_ROOT/CMakeLists.txt" ] && ok "CMake project present" || bad "CMakeLists.txt missing"
[ -d "$LOOS_ROOT/apps" ] && ok "desktop app sources present" || bad "apps/ missing"

echo
[ "$fails" -eq 0 ] && echo "pre-flight OK" || { echo "$fails hard failures"; exit 1; }
