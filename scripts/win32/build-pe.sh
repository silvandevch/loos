#!/usr/bin/env bash
# Build the LoOS user-mode system DLLs as real PE32+ images.
#   scripts/win32/build-pe.sh [outdir]
# Needs: clang (any x86-64 build with the Windows target) and an LLD in link mode.
# LLD is taken from $LLD, then PATH (lld-link / ld.lld), then the local Rust toolchain's rust-lld.
set -euo pipefail
here="$(cd "$(dirname "$0")/../.." && pwd)"
out="${1:-${OUT_ROOT:-$here/../loos/out}/win32}"
src="$here/src/win32"
mkdir -p "$out/obj"
# Serialize concurrent builds (several agents/ctest runs share one output dir; interleaved link steps corrupt the DLLs).
if command -v flock >/dev/null 2>&1; then exec 9>"$out/.build.lock"; flock 9; fi

find_lld() {
    if [ -n "${LLD:-}" ]; then echo "$LLD"; return; fi
    for c in lld-link; do command -v "$c" >/dev/null 2>&1 && { echo "$(command -v $c)"; return; }; done
    # Skip rust-lld copies from toolchains whose LLVM shared library is missing.
    for t in "$HOME"/.rustup/toolchains/*/lib/rustlib/*/bin/rust-lld; do
        [ -x "$t" ] && "$t" -flavor link --version >/dev/null 2>&1 && { echo "$t -flavor link"; return; }
    done
    echo "error: no lld found (install lld, or rustup component rust-lld, or set LLD=)" >&2; exit 1
}
LINK="$(find_lld)"
CC=(clang -target x86_64-pc-windows-msvc -ffreestanding -fno-stack-protector -fno-builtin -O2 -Wall -Wextra -Werror -std=c11
    -I"$src/crt/include" -I"$src/include")

obj() {   # obj <src> <name>
    "${CC[@]}" "$1" -c -o "$out/obj/$2.obj"
    echo "$out/obj/$2.obj"
}

# ---- shared runtime ----
crt=( "$(obj "$src/crt/string.c" crt_string)" )
"${CC[@]}" -x assembler-with-cpp "$src/crt/chkstk.S" -c -o "$out/obj/chkstk.obj"; crt+=( "$out/obj/chkstk.obj" )

# ---- ntdll ----
python3 "$here/scripts/win32/gen_exports.py" check-syscalls "$src/ntdll/syscalls.tbl" "$src/include/loos_nt.h"
python3 "$here/scripts/win32/gen_exports.py" syscalls "$src/ntdll/syscalls.tbl" "$out/syscalls.c"
nt=( "${crt[@]}" "$(obj "$out/syscalls.c" nt_syscalls)" )
for f in errmap version heap sync procparams rtl delayload crt_exports printf ldr_api process; do nt+=( "$(obj "$src/ntdll/$f.c" nt_$f)" ); done
for f in compat/compat ldr/ldr pe/pe apisets/apiset apisets/apiset_table; do
    b="nt_$(basename "$f")"
    nt+=( "$(obj "$src/$f.c" "$b")" )
done
python3 "$here/scripts/win32/gen_exports.py" def ntdll "$out" "$src"/ntdll/errmap.c "$src"/ntdll/version.c "$src"/ntdll/heap.c "$src"/ntdll/sync.c "$src"/ntdll/procparams.c "$src"/ntdll/rtl.c "$src"/ntdll/delayload.c "$src"/ntdll/printf.c "$src"/ntdll/ldr_api.c "$src"/ntdll/process.c "$src/ntdll/syscalls.tbl" "$src/ntdll/extra_exports.txt"
$LINK /dll /nodefaultlib /noentry /machine:x64 /base:0x180000000 /def:"$out/ntdll.def" /implib:"$out/ntdll.lib" /out:"$out/ntdll.dll" "${nt[@]}"

# ---- kernelbase (the implementation; API sets resolve here) and kernel32 (forwarders, as on Windows 10) ----
kb=( "${crt[@]}" )
for f in ansi console createproc env file files2 heap locks mapping mem message misc path paths proc strings sync thread tls version; do kb+=( "$(obj "$src/kernel32/$f.c" kb_$f)" ); done
python3 "$here/scripts/win32/gen_exports.py" def kernelbase "$out" "$src"/kernel32/*.c
$LINK /dll /nodefaultlib /entry:KernelBaseDllMain /machine:x64 /base:0x180100000 /def:"$out/kernelbase.def" /implib:"$out/kernelbase.lib" /out:"$out/kernelbase.dll" "$out/ntdll.lib" "${kb[@]}"

python3 "$here/scripts/win32/gen_exports.py" def --forward-to=KERNELBASE kernel32 "$out" "$src"/kernel32/*.c
echo 'const char loos_kernel32_forwarders_only = 0;' > "$out/k32_empty.c"
$LINK /dll /nodefaultlib /noentry /machine:x64 /base:0x180200000 /def:"$out/kernel32.def" /implib:"$out/kernel32.lib" /out:"$out/kernel32.dll" "$(obj "$out/k32_empty.c" k32_empty)" "$out/kernelbase.lib"

# ---- ucrtbase (the app-facing C runtime; API-set CRT contracts resolve here) ----
ucrt=( "$(obj "$src/ucrt/ucrtbase.c" ucrtbase)" )
python3 "$here/scripts/win32/gen_exports.py" def ucrtbase "$out" "$src/ucrt/ucrtbase.c"
$LINK /dll /nodefaultlib /noentry /machine:x64 /base:0x180300000 /def:"$out/ucrtbase.def" /implib:"$out/ucrtbase.lib" /out:"$out/ucrtbase.dll" "$out/kernel32.lib" "${ucrt[@]}"

ls -l "$out"/*.dll
