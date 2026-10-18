#!/usr/bin/env bash
# 06-stubs.sh — populate the "missing binaries" that the leaked tree cannot
# build (activation/crypto + third-party code was stripped) and/or stage
# clearly-marked placeholders.
#
# Mechanism (from the tree's own tools/missing.cmd): every missing file is
# either copied raw from a retail ISO's I386 dir (pidgen.dll) or `expand -r`'d
# from <name>.xx_ / driver.cab. There are 4308 such entries.
#
# Sources, in priority order:
#   1. $RETAIL_SRC  - a directory containing uncompressed files, e.g. the
#                     community "win2003_x86-missing-binaries" pack, or an
#                     extracted retail ISO I386 tree.
#   2. $RETAIL_ISO  - a retail .iso; extracted with 7z.
#   3. stub mode    - zero-byte, clearly-marked placeholders for the critical
#                     set so a degraded postbuild can run. These are NOT usable
#                     in a real image; they exist to unblock filechk.
source "$(dirname "$0")/../env.sh"
set -uo pipefail
require awk sed

MISSING_CMD="$NT_SRC/tools/missing.cmd"
[ -f "$MISSING_CMD" ] || die "tools/missing.cmd not found under $NT_SRC"

DEST="${1:-$NT_OUT/missing-binaries}"
mkdir -p "$DEST"

# --- 1. Parse the manifest out of missing.cmd ------------------------------
# `copy /Y /B "%_RTLISO%\I386\pidgen.dll" "%_NTPOSTBLD%\"`
# `expand -r "%_RTLISO%\I386\winlogon.ex_" "%_NTPOSTBLD%"`
# `expand -r "%_RTLISO%\I386\driver.cab" /f:"ovcodec2.dll" "%_NTPOSTBLD%"`
manifest="$MISSING_DIR/manifest.tsv"
python3 - "$MISSING_CMD" "$manifest" <<'PY'
import re, sys
src_cmd, out = sys.argv[1], sys.argv[2]
tgt_re = re.compile(r'%_NTPOSTBLD%\\([^"\\]+)"')
src_re = re.compile(r'%_RTLISO%\\I386\\([^"\\]+)"')
rows = []
with open(src_cmd, 'r', errors='replace') as fh:
    for line in fh:
        line = line.rstrip('\n').replace('\r', '')
        if 'expand -r' in line:
            mode = 'expand'
        elif re.search(r'\bcopy ', line):
            mode = 'copy'
        else:
            continue
        t = tgt_re.search(line)
        s = src_re.search(line)
        if t and s:
            rows.append(f"{t.group(1)}\t{s.group(1)}\t{mode}")
with open(out, 'w') as fh:
    fh.write('\n'.join(rows) + ('\n' if rows else ''))
print(len(rows))
PY
n=$(wc -l < "$manifest")
log "parsed $n entries -> $manifest"

# --- 2. Optional source material -------------------------------------------
EXTRACT_DIR=""
if [ -n "${RETAIL_ISO:-}" ] && [ -f "${RETAIL_ISO}" ]; then
    EXTRACT_DIR="$NT_OUT/_retail_iso"
    if [ ! -d "$EXTRACT_DIR/I386" ]; then
        log "extracting retail ISO $RETAIL_ISO"
        mkdir -p "$EXTRACT_DIR"
        7z x -y -o"$EXTRACT_DIR" "$RETAIL_ISO" >/dev/null || warn "7z extraction failed"
    fi
    [ -d "$EXTRACT_DIR/I386" ] && RETAIL_SRC="$EXTRACT_DIR/I386"
fi

copied=0; stubbed=0; failed=0
if [ -n "${RETAIL_SRC:-}" ] && [ -d "${RETAIL_SRC:-/nonexistent}" ]; then
    log "materialising from RETAIL_SRC=$RETAIL_SRC"
    while IFS=$'\t' read -r tgt src mode; do
        if [ -f "$RETAIL_SRC/$tgt" ]; then
            cp -p "$RETAIL_SRC/$tgt" "$DEST/$tgt"; copied=$((copied+1)); continue
        fi
        if [ -f "$RETAIL_SRC/$src" ]; then
            # try wine expand.exe, then 7z for cabinets, else raw copy
            if command -v wine >/dev/null 2>&1 && [ "$mode" = expand ]; then
                wine "$RETAIL_SRC/expand.exe" -r "$RETAIL_SRC/$src" "$DEST" >/dev/null 2>&1 \
                    && { copied=$((copied+1)); continue; }
            fi
            cp -p "$RETAIL_SRC/$src" "$DEST/$tgt"; copied=$((copied+1)); continue
        fi
        failed=$((failed+1))
    done < "$manifest"
else
    warn "no RETAIL_SRC/RETAIL_ISO supplied -> stub mode"
    # Critical set the user called out, plus the media codecs missing.cmd lists.
    for f in winlogon.exe msgina.dll pidgen.dll l3codeca.acm l3codecx.ax ovcodec2.dll wstcodec.sys; do
        printf 'LoOS-PLACEHOLDER %s - replace with the retail binary before building an image.\n' "$f" > "$DEST/$f"
        stubbed=$((stubbed+1))
    done
    warn "created $stubbed placeholder stubs in $DEST (NOT bootable)"
fi

cat > "$DEST/README.md" <<EOF
# missing-binaries staging

Source of truth: $MISSING_CMD (4308 entries).
Parsed manifest: $manifest
This run: copied=$copied stubbed=$stubbed failed=$failed

To populate for real, either:
  export RETAIL_SRC=/path/to/unpacked/binaries-pack   # win2003_x86-missing-binaries
  or
  export RETAIL_ISO=/path/to/windows2003.iso          # retail media
  scripts/06-stubs.sh

Then copy the contents into binaries.<arch><type>/ before tools\\postbuild.cmd.
EOF

log "missing binaries: copied=$copied stubbed=$stubbed failed=$failed -> $DEST"
