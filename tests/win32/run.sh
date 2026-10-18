#!/usr/bin/env bash
# Host-side tests for the user-mode Win32 pieces. Uses clang+ASan/UBSan when
# available, plain gcc otherwise. Real-PE checks run against any wine DLLs found.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"; src="$here/../../src/win32"
out="$(mktemp -d)"; trap 'rm -rf "$out"' EXIT
cc=gcc; san=""
if command -v clang >/dev/null && echo 'int main(){}' | clang -x c - -fsanitize=address,undefined -o "$out/t" 2>/dev/null; then
    cc=clang; san="-fsanitize=address,undefined -fno-sanitize-recover=all"
fi
flags="-std=c99 -D_POSIX_C_SOURCE=200809L -g -Wall -Wextra -Werror $san -I$src/ldr -I$src/pe -I$src/apisets -I$src/compat"
apis=("$src/apisets/apiset.c" "$src/apisets/apiset_table.c")

$cc $flags "$here/apisets/apiset_test.c" "${apis[@]}" -o "$out/apiset_test"
$cc $flags "$here/pe/pe_test.c" "$src/pe/pe.c" "${apis[@]}" -o "$out/pe_test"
cmp=("$src/compat/compat.c")
$cc $flags "$here/compat/compat_test.c" "${cmp[@]}" -o "$out/compat_test"
k32=("$src/kernel32/version.c" "$src/ntdll/version.c" "${cmp[@]}" "$src/kernel32/path.c" "$src/kernel32/file.c" "$src/kernel32/mem.c" "$src/kernel32/sync.c" "$src/kernel32/proc.c" "$src/ntdll/errmap.c")
$cc ${flags/c99/c11} -DLOOS_HOST_TEST -I"$src/include" "$here/kernel32/k32_test.c" "$here/kernel32/nt_fake.c" "${k32[@]}" -o "$out/k32_test"
$cc ${flags/c99/c11} -DLOOS_HOST_TEST -DFAKE_NO_LDR -I"$src/include" "$here/ntdll/ldr_api_test.c" "$here/kernel32/nt_fake.c" "$here/common/pebuild.c" "$src/ntdll/ldr_api.c" "$src/ntdll/version.c" "$src/ntdll/errmap.c" "$src/ldr/ldr.c" "$src/pe/pe.c" "${cmp[@]}" "${apis[@]}" -o "$out/ldr_api_test"
$cc ${flags/c99/c11} -DLOOS_HOST_TEST -DFAKE_NO_LDR -I"$src/include" "$here/ntdll/heap_test.c" "$here/kernel32/nt_fake.c" "$src/ntdll/heap.c" -o "$out/heap_test"
$cc $flags "$here/ldr/ldr_test.c" "$here/common/pebuild.c" "${cmp[@]}" "$src/ldr/ldr.c" "$src/pe/pe.c" "${apis[@]}" -o "$out/ldr_test"

"$out/apiset_test"
"$out/compat_test"
real=(); for d in /usr/lib64/wine*/wine/x86_64-windows /usr/lib64/wine*/wine/i386-windows; do
    for n in kernel32 ntdll user32; do [ -f "$d/$n.dll" ] && real+=("$d/$n.dll"); done; done
"$out/pe_test" "${real[@]:0:6}"
"$out/ldr_test"
"$out/k32_test"
"$out/ldr_api_test"
"$out/heap_test"

# ---- real linked system DLLs (needs clang + an LLD) ----
if command -v clang >/dev/null && { command -v lld-link >/dev/null || ls "$HOME"/.rustup/toolchains/*/lib/rustlib/*/bin/rust-lld >/dev/null 2>&1 || [ -n "${LLD:-}" ]; }; then
    "$here/../../scripts/win32/build-pe.sh" "$out/pe" >/dev/null
    $cc -std=gnu11 -g -Wall -Wextra -Werror $san -I"$src/ldr" -I"$src/pe" -I"$src/apisets" "$here/pe/artifacts_test.c" "$src/ldr/ldr.c" "$src/pe/pe.c" "${apis[@]}" -o "$out/artifacts_test"
    "$out/artifacts_test" "$out/pe"
else
    echo "artifacts: SKIPPED (no clang/lld; see scripts/win32/build-pe.sh)"
fi
