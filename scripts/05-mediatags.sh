#!/usr/bin/env bash
# 05-mediatags.sh — create the CD-root media identification tags that setup /
# oscdimg expects in the layout root.
#
# The tree's own oscdimg.cmd assembles the ISO from %_NTPOSTBLD%\%_NTVARIANT%,
# so the tags must exist in that layout root. Since no build has run yet this
# stamps a staging layout by default; pass an explicit directory to stamp a
# real binaries/ layout (e.g. "05-mediatags.sh /home/silvan/LoOS-build/out/cdroot").
source "$(dirname "$0")/../env.sh"
set -uo pipefail

LAYOUT="${1:-$NT_OUT/cdroot}"
mkdir -p "$LAYOUT"

# WIN51  : present on every 2003 media
# WIN51IA: AMD64 / "IA" family media tag
# WIN51IS: Server SKU media tag
for tag in WIN51 WIN51IA WIN51IS; do
    : > "$LAYOUT/$tag"
    log "created $LAYOUT/$tag"
done

cat > "$LAYOUT/README.mediatags" <<'EOF'
Media identification tags for Windows Server 2003 setup.
WIN51 / WIN51IA / WIN51IS must be present at the ISO root for the loader to
accept the media. Copy them into %_NTPOSTBLD%\%_NTVARIANT% before oscdimg.cmd.
EOF

log "media tags written to $LAYOUT"
