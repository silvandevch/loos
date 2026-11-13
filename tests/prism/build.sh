#!/usr/bin/env bash
# Builds the Prism test programs: $1 = output dir. Needs clang and ld64.lld (shipped with rustup's toolchain).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd); out=${1:?output dir}; mkdir -p "$out"
ld=${LD64:-$(dirname "$(rustc +1.97.1 --print target-libdir)")/bin/gcc-ld/ld64.lld}
for a in x86_64 arm64; do
    clang -target $a-apple-macos11 -O1 -fno-stack-protector -c "$here/hello.c" -o "$out/hello_$a.o"
    "$ld" -arch $a -platform_version macos 11.0 11.0 -L"$here" -lSystem "$out/hello_$a.o" -o "$out/hello_$a" -no_fixup_chains
done
# An app bundle whose executable loads a dylib from Contents/Frameworks through @rpath.
for a in x86_64 arm64; do
    app=$out/Greet_$a.app/Contents; mkdir -p "$app/MacOS" "$app/Frameworks"
    clang -target $a-apple-macos11 -O1 -c "$here/greet.c" -o "$out/greet_$a.o"
    clang -target $a-apple-macos11 -O1 -c "$here/app.c" -o "$out/app_$a.o"
    "$ld" -arch $a -platform_version macos 11.0 11.0 -dylib -install_name @rpath/libgreet.dylib -L"$here" -lSystem "$out/greet_$a.o" -o "$app/Frameworks/libgreet.dylib"
    "$ld" -arch $a -platform_version macos 11.0 11.0 -L"$here" -lSystem "$out/app_$a.o" "$app/Frameworks/libgreet.dylib" -rpath @executable_path/../Frameworks -o "$app/MacOS/Greet_$a"
done
clang -target x86_64-apple-macos11 -O1 -c "$here/sig.c" -o "$out/sig.o"
"$ld" -arch x86_64 -platform_version macos 11.0 11.0 -L"$here" -lSystem "$out/sig.o" -o "$out/sig_x86_64"
"$ld" -arch x86_64 -platform_version macos 13.0 13.0 -L"$here" -lSystem "$out/hello_x86_64.o" -o "$out/hello_chained" -fixup_chains
python3 - "$out" <<'PY'
import struct, sys
o = sys.argv[1]
a, b = open(o + "/hello_x86_64", "rb").read(), open(o + "/hello_arm64", "rb").read()
off1 = 0x4000; off2 = off1 + (len(a) + 0x3fff) // 0x4000 * 0x4000
d = bytearray(off2 + len(b))
d[:48] = struct.pack(">II", 0xcafebabe, 2) + struct.pack(">5I", 0x01000007, 3, off1, len(a), 14) + struct.pack(">5I", 0x0100000c, 0, off2, len(b), 14)
d[off1:off1 + len(a)] = a; d[off2:] = b
open(o + "/hello_universal", "wb").write(d)
PY
echo "$out"
