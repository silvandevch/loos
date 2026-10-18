#!/usr/bin/env bash
# LoOS — build workspace environment.
#
# Two roots keep disk use friendly:
#   SRC_ROOT  the big source tree + patchkit  -> fast internal disk (NVMe)
#   OUT_ROOT  build output, ISOs, VM disks    -> external DATA drive (355G free)
#
# Override either by exporting the variable, or by writing a small
# .loosrc (next to this file) with lines like:
#     SRC_ROOT=/some/path
#     OUT_ROOT=/run/media/silvan/DATA/loos
# .loosrc is gitignored; the `loos` CLI writes it for you (`loos relocate`).

export LOOS_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ---- config file (optional) -------------------------------------------------
if [ -f "$LOOS_ROOT/.loosrc" ]; then
    # shellcheck disable=SC1091
    . "$LOOS_ROOT/.loosrc"
fi

# ---- external DATA drive ----------------------------------------------------
export LOOS_DATA="${LOOS_DATA:-/run/media/silvan/DATA}"

# ---- roots ------------------------------------------------------------------
# Source: keep the existing fast NVMe tree if present, else fall back to DATA.
if [ -z "${SRC_ROOT:-}" ]; then
    if [ -d "/home/silvan/LoOS-build/srv03rtm" ]; then
        SRC_ROOT="/home/silvan/LoOS-build"
    else
        SRC_ROOT="$LOOS_DATA/loos/src"
    fi
fi
export SRC_ROOT
# Output: big and disposable -> DATA by default.
export OUT_ROOT="${OUT_ROOT:-$LOOS_DATA/loos/out}"

export NT_SRC="${NT_SRC:-$SRC_ROOT/srv03rtm}"        # SDXROOT source tree
export PATCHKIT="${PATCHKIT:-$SRC_ROOT/patchkit}"    # openxp/patchkit (amd64 branch)
export NT_OUT="${NT_OUT:-$OUT_ROOT}"                 # build/binaries/ISO output
export NT_BUILD_SRC="${NT_BUILD_SRC:-$OUT_ROOT/nt-source-overlay/srv03rtm}" # writable Wine build view

# Back-compat alias (older scripts/tools may reference BUILD_ROOT).
export BUILD_ROOT="$SRC_ROOT"

# ---- upstream git targets ---------------------------------------------------
export PK_REPO="https://github.com/theopenxp/patchkit"
export PK_BRANCH="amd64_patches"
export ANIKA_REPO="https://github.com/mrcxlinux/srv03rtm-anika"
export ANIKA_BRANCH="main"

# ---- workspace subdirs ------------------------------------------------------
export CERTS_DIR="$LOOS_ROOT/certs"
export SCRIPTS_DIR="$LOOS_ROOT/scripts"
export MISSING_DIR="$LOOS_ROOT/missing"
export LOGS_DIR="$LOOS_ROOT/logs"
export BACKUP_DIR="$LOOS_ROOT/backups"

mkdir -p "$NT_OUT" "$CERTS_DIR" "$MISSING_DIR" "$LOOS_ROOT/build" "$LOGS_DIR" "$BACKUP_DIR"

# ---- helpers ----------------------------------------------------------------
log()  { printf '\033[1;36m[%s]\033[0m %s\n' "$(date +%H:%M:%S)" "$*"; }
warn() { printf '\033[1;33m[WARN]\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31m[FAIL]\033[0m %s\n' "$*" >&2; exit 1; }

require() {
    for c in "$@"; do command -v "$c" >/dev/null 2>&1 || die "missing dependency: $c"; done
}

# df of a path's mount, one line
disk_of() { df -h "$1" 2>/dev/null | awk 'NR==2{printf "%s free of %s (%s used)", $4, $2, $5}'; }
