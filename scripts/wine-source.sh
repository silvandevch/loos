#!/usr/bin/env bash
# Prepare a writable Wine build copy; NT_SRC is the read-only study tree.
prepare_wine_source() {
    [ -f "$NT_SRC/tools/razzle.cmd" ] || die "missing razzle.cmd in study tree: $NT_SRC"
    if awk -v p="$NT_BUILD_SRC" 'tolower($0) ~ "fuse.fuse-overlayfs " tolower(p) " " { found=1 } END { exit !found }' /proc/self/mountinfo; then
        return
    fi
    root="$(dirname "$NT_BUILD_SRC")"
    mkdir -p "$root/upper" "$root/work" "$NT_BUILD_SRC"
    if command -v fuse-overlayfs >/dev/null 2>&1; then
        log "mounting writable build overlay over read-only study tree: $NT_BUILD_SRC"
        fuse-overlayfs -o "lowerdir=$NT_SRC,upperdir=$root/upper,workdir=$root/work" "$NT_BUILD_SRC" \
            || die "could not mount Wine build overlay"
    else
        require rsync
        log "copying study source to writable build tree: $NT_BUILD_SRC"
        rsync -a --exclude='/.git/' "$NT_SRC/" "$NT_BUILD_SRC/" || die "could not copy study source"
    fi
    [ -f "$NT_BUILD_SRC/tools/razzle.cmd" ] || die "build view is incomplete: $NT_BUILD_SRC"
}
