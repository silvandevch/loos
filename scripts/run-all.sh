#!/usr/bin/env bash
# run-all.sh — full offline prep pipeline (fetch, overlay, patch, certs, tags,
# stubs). Stops on first hard failure. Then use 09-build-all.sh to compile.
source "$(dirname "$0")/../env.sh"
set -euo pipefail
S="$LOOS_ROOT/scripts"

log "############## LoOS prep pipeline ##############"
bash "$S/00-fetch.sh"
bash "$S/01-structure.sh"
bash "$S/02-overlay-patchkit.sh"          # additive; add --force to override
bash "$S/03-patch-oobe.sh"
bash "$S/04-certs.sh"
bash "$S/05-mediatags.sh"
bash "$S/06-stubs.sh"                      # stub mode unless RETAIL_SRC/RETAIL_ISO
log "############## prep complete ##############"
log "next: scripts/10-verify-env.sh && scripts/09-build-all.sh"
