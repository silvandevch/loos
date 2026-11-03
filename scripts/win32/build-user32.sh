#!/usr/bin/env bash
# Builds user32.dll and gdi32.dll (win32k service stubs). Prefers clang+lld-link; falls back to mingw-w64 gcc.
set -euo pipefail
here=$(cd "$(dirname "$0")/../.." && pwd); out=${1:-$here/out/win32}; mkdir -p "$out"
src=$here/src/win32
if command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
    for d in user32 gdi32; do
        x86_64-w64-mingw32-gcc -O2 -Wall -Wextra -ffreestanding -fno-stack-protector -nostdlib -shared -e DllMain \
            -I"$src/include" "$src/user32/$d.c" -o "$out/$d.dll" -Wl,--kill-at
    done
else
    echo "mingw-w64 gcc not found; use scripts/win32/build-pe.sh with lld" >&2; exit 1
fi
ls -l "$out"/user32.dll "$out"/gdi32.dll
