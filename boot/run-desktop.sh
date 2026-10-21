#!/usr/bin/env bash
# Boots the LoOS desktop in a QEMU window (mouse + keyboard work: Super opens the launcher).
#   boot/run-desktop.sh [bios|uefi]
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd); out=$here/../out/boot
mode=${1:-bios}
case $mode in
    bios) img=$("$here/bios/build.sh" "$out"); fw=() ;;
    uefi) img=$("$here/uefi/build.sh" "$out"); fw=(-bios "${OVMF_CODE:-/usr/share/edk2/ovmf/OVMF_CODE.fd}") ;;
    *) echo "usage: $0 [bios|uefi]" >&2; exit 2 ;;
esac
exec qemu-system-x86_64 -smp 4 -m 512M -vga std -serial stdio -drive "file=$img,format=raw,if=ide" "${fw[@]}"
