#!/usr/bin/env bash
# 04-certs.sh — regenerate the test-signing certificate chain (the "timebomb"
# certs that expired Oct 2021) and fold new fingerprints back into the tree.
#
# Runs the tree's certutil/generate.sh (pure openssl, so it works on Linux),
# but first fixes two real defects in that script:
#   1. CRLF line endings (it ships from the Windows tree) break bash.
#   2. Line 103 is missing a space between two -i paths, so the
#      ds/win32/ntcrypto/mincrypt/vercert.cpp public-key patch silently fails.
#
# Output: certs/{testroot,testpca,vbl03ca}.cer + driver.pfx, and the regenerated
# source files (crypto.c, strongname.cpp, vercert.cpp, policy.cpp, defview.cpp,
# server.c, checktest*.cmd, crypto.cmd) copied back into the tree with backups.
source "$(dirname "$0")/../env.sh"
set -uo pipefail
require openssl perl sed

SRC_CERTUTIL="$NT_SRC/certutil"
[ -f "$SRC_CERTUTIL/generate.sh" ] || die "certutil/generate.sh not found under $NT_SRC"

scratch="$(mktemp -d /tmp/opencode/certs.XXXXXX)"
trap 'rm -rf "$scratch"' EXIT
cp -a "$SRC_CERTUTIL/." "$scratch/"

log "normalising CRLF in generate.sh"
sed -i 's/\r$//' "$scratch/generate.sh"

log "fixing missing space before second -i in generate.sh:103"
sed -i 's|lib/vercert.cpp"-i |lib/vercert.cpp" -i |' "$scratch/generate.sh"
grep -q 'lib/vercert.cpp" -i ' "$scratch/generate.sh" || warn "expected -i fix not applied"

log "running certutil/generate.sh"
( cd "$scratch" && bash generate.sh ) > "$LOGS_DIR/certs-generate.log" 2>&1
rc=$?
[ "$rc" -eq 0 ] || { tail -30 "$LOGS_DIR/certs-generate.log"; die "generate.sh failed (rc=$rc)"; }

GEN="$scratch/srv03rtm.certs"
[ -d "$GEN" ] || die "generate.sh produced no srv03rtm.certs"

log "collecting certs -> $CERTS_DIR"
mkdir -p "$CERTS_DIR/tools"
for f in testroot.cer testpca.cer vbl03ca.cer driver.pfx; do
    [ -f "$GEN/tools/$f" ] && cp -p "$GEN/tools/$f" "$CERTS_DIR/tools/$f"
done

stamp="$(date +%Y%m%d-%H%M%S)"
bk="$BACKUP_DIR/certs-$stamp"
applied=0
log "applying regenerated source files into tree"
while IFS= read -r rel; do
    case "$rel" in tools/*.cer|tools/*.pfx) continue;; esac
    dst="$NT_SRC/$rel"
    mkdir -p "$(dirname "$dst")"
    [ -e "$dst" ] && { mkdir -p "$bk/$(dirname "$rel")"; cp -p "$dst" "$bk/$rel"; }
    cp -p "$GEN/$rel" "$dst"
    applied=$((applied+1))
done < <(cd "$GEN" && find . -type f | sed 's|^\./||')

{ echo "certs $stamp applied=$applied"; echo "backup=$bk"; } >> "$LOGS_DIR/certs.log"

log "verifying driver.pfx"
openssl pkcs12 -in "$CERTS_DIR/tools/driver.pfx" -nodes -passin pass: -nokeys >/dev/null 2>&1 \
    && log "driver.pfx OK" || warn "driver.pfx could not be read"
log "certs regenerated ($applied source files applied); backups: $bk"
