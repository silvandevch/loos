#!/usr/bin/env bash
# Print the environment needed to inject liblooswin into a Lin process.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
lib=${LOOS_LIBLOOSWIN:-${OUT_ROOT:-$root/out}/cmake/host-clang/src/lin/liblooswin.so}
test -r "$lib" || { echo "liblooswin not built: $lib" >&2; exit 1; }
printf 'LD_PRELOAD=%q\n' "$lib"
