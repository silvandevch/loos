#!/usr/bin/env bash
# Build the pe_exec demo PE images (losshost.dll + loos-echo.exe) with MinGW-w64.
#   tests/win32/exec/build-demo.sh <outdir>
# Freestanding: no CRT, DLL entry is DllMain, EXE entry is pe_main, relocations
# kept so the LoOS loader exercises its relocate path.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
out="${1:-$here/out}"
CC="${MINGW_CC:-x86_64-w64-mingw32-gcc}"
mkdir -p "$out"

common=(-nostdlib -fno-builtin -fno-stack-protector -fno-asynchronous-unwind-tables
        -fno-ident -O2 -Wall -Werror)

"$CC" -shared "${common[@]}" \
    -o "$out/losshost.dll" "$here/losshost.c" \
    -Wl,--image-base,0x180000000 -Wl,--dynamicbase -Wl,-e,DllMain \
    -Wl,--out-implib,"$out/liblosshost.a"

"$CC" "${common[@]}" \
    -o "$out/loos-echo.exe" "$here/loos-echo.c" \
    -L"$out" -llosshost \
    -Wl,--image-base,0x140000000 -Wl,--dynamicbase -Wl,--subsystem,console \
    -Wl,-e,pe_main

echo "built: $out/losshost.dll $out/loos-echo.exe"
