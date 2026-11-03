#!/usr/bin/env bash
# Rebuild the embedded LoOS installer images from the current LoOS tree and
# regenerate the hash manifest. Requires the LoOS build prerequisites (clang,
# lld, sgdisk, mtools, python3, zstd) and run from anywhere.
set -euo pipefail
APP_ROOT=$(cd "$(dirname "$0")/.." && pwd)
LOOS_ROOT=$(cd "$APP_ROOT/.." && pwd)
WORK="$APP_ROOT/build/payload"
UEFI="$WORK/uefi"; BIOS="$WORK/bios"
rm -rf "$WORK"; mkdir -p "$UEFI" "$BIOS"
"$LOOS_ROOT/boot/uefi/build.sh" "$UEFI" --cmdline "installer" > "$APP_ROOT/build/uefi-build.log" 2>&1
"$LOOS_ROOT/boot/bios/build.sh" "$BIOS" --cmdline "installer" > "$APP_ROOT/build/bios-build.log" 2>&1
zstd -q -19 -f "$UEFI/uefi.img" -o "$APP_ROOT/assets/loos-uefi-installer.img.zst"
zstd -q -19 -f "$BIOS/bios.img" -o "$APP_ROOT/assets/loos-bios-installer.img.zst"
python3 - "$UEFI/uefi.img" "$BIOS/bios.img" "$APP_ROOT/assets" <<'PY'
import hashlib, json, os, sys, time
uefi, bios, assets = sys.argv[1:4]
def info(raw_path, comp_path):
    raw = open(raw_path, 'rb').read()
    comp = open(comp_path, 'rb').read()
    return {
        "raw_bytes": len(raw),
        "raw_sha256": hashlib.sha256(raw).hexdigest(),
        "compressed_sha256": hashlib.sha256(comp).hexdigest(),
        "compression": "zstd",
    }
d = {
    "name": "LoOS installer media",
    "built_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    "source": "LoOS boot/uefi/build.sh --cmdline installer; boot/bios/build.sh --cmdline installer",
    "uefi": info(uefi, os.path.join(assets, 'loos-uefi-installer.img.zst')),
    "bios": info(bios, os.path.join(assets, 'loos-bios-installer.img.zst')),
}
json.dump(d, open(os.path.join(assets, 'payload.json'), 'w'), indent=2)
print(json.dumps(d, indent=2))
PY
