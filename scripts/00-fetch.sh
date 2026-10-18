#!/usr/bin/env bash
# 00-fetch.sh — clone/pull the upstream repositories referenced by the build.
# Idempotent: existing checkouts are fast-forwarded instead of re-cloned.
source "$(dirname "$0")/../env.sh"
set -euo pipefail
require git rsync

clone_or_update() {
    local repo="$1" branch="$2" dest="$3"
    if [ -d "$dest/.git" ]; then
        log "updating $dest ($branch)"
        git -C "$dest" fetch --depth 1 origin "$branch"
        git -C "$dest" checkout -q "$branch"
        git -C "$dest" reset --hard -q "origin/$branch"
    else
        log "cloning $repo#$branch -> $dest"
        git clone --depth 1 -b "$branch" "$repo" "$dest"
    fi
}

mkdir -p "$BUILD_ROOT"

# 1. openxp/patchkit — x64 compiler tools + header/source overlay.
clone_or_update "$PK_REPO" "$PK_BRANCH" "$PATCHKIT"

# 2. mrcxlinux/srv03rtm-anika — full pre-patched Server 2003 RTM source tree
#    (contains its own .git; this is the base SDXROOT).
if [ ! -d "$NT_SRC/.git" ]; then
    log "cloning $ANIKA_REPO#$ANIKA_BRANCH -> $NT_SRC (large: ~1.6G .git)"
    git clone --depth 1 -b "$ANIKA_BRANCH" "$ANIKA_REPO" "$NT_SRC"
else
    log "source tree already present at $NT_SRC"
fi

# Integrity: the working tree must match the single upstream commit.
if [ -d "$NT_SRC/.git" ]; then
    dirty=$(git -C "$NT_SRC" status --porcelain | wc -l)
    [ "$dirty" -eq 0 ] || warn "source tree has $dirty local modifications"
    log "source files: $(find "$NT_SRC" -path "$NT_SRC/.git" -prune -o -type f -print | wc -l)"
fi

log "fetch complete"
