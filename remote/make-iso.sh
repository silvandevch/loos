#!/usr/bin/env bash
# make-iso.sh — Linux-only fallback ISO builder (no Wine, no oscdimg).
# Use when you only have a postbuild layout directory (e.g. copied off a VM)
# and want to burn it here.
#
#   remote/make-iso.sh <layout-dir> <out.iso> [boot-image]
#
# The layout dir is the SKU CD tree (what oscdimg.cmd would pass as
# "%_NTPOSTBLD%\<variant>"). Boot image defaults to the tree's etfsboot.com.
# El Torito parameters mirror Microsoft's oscdimg "-n -b etfsboot.com".
source "$(dirname "$0")/../env.sh"
set -uo pipefail
require mkisofs

LAYOUT="${1:?usage: make-iso.sh <layout-dir> <out.iso> [boot-image]}"
OUT="${2:?missing output iso path}"
BOOT="${3:-$NT_SRC/base/ntsetup/opktools/wpebins/x86/etfsboot.com}"

[ -d "$LAYOUT" ] || die "layout dir not found: $LAYOUT"
[ -f "$BOOT" ]   || die "boot image not found: $BOOT (install mkisofs/xorriso or pass one)"

# mkisofs needs the boot image inside the tree root for a no-emulation entry.
copied=0
if [ ! -f "$LAYOUT/$(basename "$BOOT")" ]; then
    cp "$BOOT" "$LAYOUT/$(basename "$BOOT")"; copied=1
fi
cleanup() { [ "$copied" = 1 ] && rm -f "$LAYOUT/$(basename "$BOOT")"; }
trap cleanup EXIT

log "building ISO from $LAYOUT"
mkisofs -b "$(basename "$BOOT")" -no-emul-boot -boot-load-size 4 -boot-info-table \
        -J -R -N -V "LOOS" -o "$OUT" "$LAYOUT"
log "ISO: $OUT ($(du -h "$OUT" | cut -f1))"
