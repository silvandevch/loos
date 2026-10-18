#!/usr/bin/env bash
# 11-resolve-patchkit-conflicts.sh — resolve the 321 overlay conflicts.
#
# 02-overlay-patchkit.sh copies only additive paths; the paths that would
# overwrite the base tree are skipped by default. This takes the patchkit
# (openxp x64) version for every conflicted path, backing up the base version
# and normalising text line-endings to the tree convention.
#
#   bash scripts/11-resolve-patchkit-conflicts.sh --dry-run   # preview
#   bash scripts/11-resolve-patchkit-conflicts.sh             # apply
source "$(dirname "$0")/../env.sh"
set -uo pipefail
require python3

[ -d "$NT_SRC/.git" ] || die "source tree not found at $NT_SRC"
[ -d "$PATCHKIT" ]     || die "patchkit not found at $PATCHKIT"

python3 "$(dirname "$0")/resolve_patchkit_conflicts.py" "$@"
rc=$?

# append to the overlay provenance log
{
  echo "resolve $(date +%Y%m%d-%H%M%S) mode=${1:-apply} rc=$rc"
} >> "$LOGS_DIR/overlay.log"
exit $rc
