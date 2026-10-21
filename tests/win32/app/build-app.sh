#!/usr/bin/env bash
# Build the Windows-app proof images with MinGW-w64:
#   compat-hello.exe  — an ordinary Win32 app importing GetStdHandle/WriteFile from kernel32.dll
#   k32shim.dll       — exports of the same names, forwarding to host hooks
#   tests/win32/app/build-app.sh <outdir>
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
out="${1:-$here/out}"
CC="${MINGW_CC:-x86_64-w64-mingw32-gcc}"
mkdir -p "$out"

common=(-nostdlib -fno-builtin -fno-stack-protector -fno-asynchronous-unwind-tables
        -fno-ident -O2 -Wall -Werror)

"$CC" -shared "${common[@]}" \
    -o "$out/k32shim.dll" "$here/k32shim.c" \
    -Wl,--image-base,0x180000000 -Wl,--dynamicbase -Wl,-e,DllMain \
    -Wl,--out-implib,"$out/libk32shim.a"

"$CC" "${common[@]}" \
    -o "$out/compat-hello.exe" "$here/compat-hello.c" \
    -lkernel32 \
    -Wl,--image-base,0x140000000 -Wl,--dynamicbase -Wl,--subsystem,console \
    -Wl,-e,app_main

echo "built: $out/k32shim.dll $out/compat-hello.exe"
