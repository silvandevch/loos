#!/usr/bin/env bash
# Fetches LLVM's libc++ and libc++abi sources (Apache-2.0 WITH LLVM-exception; see THIRD-PARTY.md)
# at the release matching the host clang, into $LIBCXX_SRC (default: out/third_party/llvm-project).
# Sparse and shallow: only the directories the Prism libc++ build reads.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd); root=$(cd "$here/../../.." && pwd)
tag=${LLVM_TAG:-llvmorg-23.1.5}
dst=${LIBCXX_SRC:-$root/out/third_party/llvm-project}
if [ -f "$dst/libcxx/include/__config" ] && [ "$(git -C "$dst" describe --tags 2>/dev/null)" = "$tag" ]; then
    echo "$dst"; exit 0
fi
rm -rf "$dst"; mkdir -p "$(dirname "$dst")"
git clone -q --depth 1 --filter=blob:none --sparse --branch "$tag" https://github.com/llvm/llvm-project.git "$dst" >&2
git -C "$dst" sparse-checkout set libcxx/include libcxx/src libcxx/vendor libcxxabi/include libc/shared libc/src/__support libc/hdr libc/include/llvm-libc-macros libc/include/llvm-libc-types runtimes >&2
echo "$dst"
