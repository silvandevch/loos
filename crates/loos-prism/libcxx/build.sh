#!/usr/bin/env bash
# Builds LLVM's libc++ (fetched by fetch.sh) as a universal macOS dylib, /usr/lib/libc++.1.dylib,
# for Prism: x86_64 + arm64, macOS ABI, against Prism's own C headers (crates/loos-prism/sdk) and
# its C++ ABI runtime (libc/cxxabi.c provides what libc++abi would). Output: $1 (a file path).
# Needs clang and rustup's ld64.lld. C-library imports are resolved by the loader at run time
# (flat lookup into Prism's libSystem), so the link does not need a libSystem stub.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd); prism=$(cd "$here/.." && pwd)
out=${1:?output dylib path}
src=$("$here/fetch.sh")/libcxx
ld=${LD64:-$(dirname "$(rustc +1.97.1 --print target-libdir)")/bin/gcc-ld/ld64.lld}
work=${LIBCXX_WORK:-$(dirname "$out")/libcxx-build}
mkdir -p "$work/gen"
cp "$here/__config_site" "$work/gen/__config_site"
sed 's/@_LIBCPP_ASSERTION_HANDLER_CONTENT@//' "$src/vendor/llvm/default_assertion_handler.in" > "$work/gen/__assertion_handler"

# Every source file of the library except: the Windows / IBM support, the time zone database,
# and operator new / delete and the exception classes (Prism's C++ ABI runtime has those, as
# libc++abi does on macOS).
mapfile -t files < <(cd "$src/src" && ls *.cpp filesystem/*.cpp ryu/*.cpp 2>/dev/null | grep -vE '^(new\.cpp|tz\.cpp|tzdb.*\.cpp|time_zone.*\.cpp|support/)')

jobs=${LIBCXX_JOBS:-$(nproc)}
slices=()
for arch in x86_64 arm64; do
    objdir=$work/$arch; mkdir -p "$objdir"
    flags=(-target "$arch-apple-macos11" -std=c++26 -O2 -fPIC -fvisibility-inlines-hidden
           -nostdinc++ -nostdlibinc -isystem "$work/gen" -isystem "$src/include" -isystem "$prism/sdk/include"
           -I "$src/src" -isystem "$src/../libcxxabi/include" -I "$src/../libc" -DNDEBUG -D_LIBCPP_BUILDING_LIBRARY -D_LIBCPP_REMOVE_TRANSITIVE_INCLUDES
           -DLIBCXX_BUILDING_LIBCXXABI -D_LIBCPP_HAS_NO_PRAGMA_SYSTEM_HEADER
           -fno-stack-protector -fno-vectorize -fno-slp-vectorize -Wno-user-defined-literals -Wno-macro-redefined)
    [ "$arch" = x86_64 ] && flags+=(-march=x86-64)
    objs=()
    for f in "${files[@]}" "$here/prism_stdexcept.cpp"; do
        [ "${f:0:1}" = / ] && in=$f || in=$src/src/$f
        o=$objdir/$(basename "${f//\//_}").o; objs+=("$o")
        if [ ! -f "$o" ] || [ "$in" -nt "$o" ] || [ "$prism/sdk/include" -nt "$o" ]; then
            while [ "$(jobs -rp | wc -l)" -ge "$jobs" ]; do wait -n; done
            clang++ "${flags[@]}" -c "$in" -o "$o" &
        fi
    done
    fail=0; while [ "$(jobs -rp | wc -l)" -gt 0 ]; do wait -n || fail=1; done
    [ $fail = 0 ] || { echo "libc++: compile failed ($arch)" >&2; exit 1; }
    # Link in a temp directory: the linker writes through a memory mapping, and on some filesystems
    # pages of it read back as zeros (the same reason build.rs links there).
    tmp=$(mktemp -d)
    "$ld" -arch "$arch" -platform_version macos 11.0 11.0 -dylib -install_name /usr/lib/libc++.1.dylib \
        -undefined dynamic_lookup -no_fixup_chains "${objs[@]}" -o "$tmp/libc++.1.dylib"
    cp "$tmp/libc++.1.dylib" "$objdir/libc++.1.dylib"; rm -rf "$tmp"
    slices+=("$arch=$objdir/libc++.1.dylib")
done
python3 -I - "$out" "${slices[@]}" <<'PY'
# A universal (fat) Mach-O from the two thin slices, 16 KiB aligned.
import struct, sys
out, parts = sys.argv[1], [p.split("=", 1) for p in sys.argv[2:]]
cpu = {"x86_64": (0x01000007, 3), "arm64": (0x0100000c, 0)}
blobs = [open(p, "rb").read() for _, p in parts]
off, hdr, body = 0x4000, struct.pack(">II", 0xcafebabe, len(parts)), b""
offs = []
for b in blobs:
    offs.append(off); off += (len(b) + 0x3fff) // 0x4000 * 0x4000
for (arch, _), b, o in zip(parts, blobs, offs):
    hdr += struct.pack(">IIIII", *cpu[arch], o, len(b), 14)
data = bytearray(off)
data[:len(hdr)] = hdr
for b, o in zip(blobs, offs):
    data[o:o + len(b)] = b
open(out, "wb").write(bytes(data[:offs[-1] + len(blobs[-1])]))
PY
echo "$out"
