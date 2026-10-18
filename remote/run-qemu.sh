#!/usr/bin/env bash
# run-qemu.sh — boot the built ISO in QEMU, with a data disk for installation.
#
#   ISO=out/loos.iso remote/run-qemu.sh
#   remote/run-qemu.sh out/loos.iso
#
# Env: QEMU_MEM QEMU_CPUS QEMU_DISK QEMU_DISK_SIZE QEMU_ACCEL(kvm|tcg) ISO
set -uo pipefail
QEMU="$(command -v qemu-system-x86_64)" || { echo "qemu-system-x86_64 not found"; exit 1; }

ISO="${1:-${ISO:-}}"
[ -z "$ISO" ] && { echo "usage: ISO=<iso> run-qemu.sh   (or pass path as arg1)"; exit 1; }
[ -f "$ISO" ] || { echo "ISO not found: $ISO"; exit 1; }

MEM="${QEMU_MEM:-4096}"
CPUS="${QEMU_CPUS:-4}"
DISK="${QEMU_DISK:-$(dirname "$ISO")/loos.qcow2}"
DISK_SIZE="${QEMU_DISK_SIZE:-20G}"
ACCEL="${QEMU_ACCEL:-kvm}"

if [ ! -f "$DISK" ]; then
    echo "[qemu] creating data disk $DISK ($DISK_SIZE)"
    qemu-img create -f qcow2 "$DISK" "$DISK_SIZE" >/dev/null
fi

ACCEL_ARGS=()
if [ "$ACCEL" = "kvm" ] && [ -w /dev/kvm ]; then
    ACCEL_ARGS=(-enable-kvm -cpu host)
else
    echo "[qemu] KVM unavailable — falling back to TCG (slow)"
    ACCEL_ARGS=(-cpu qemu64)
fi

echo "[qemu] booting $ISO  (mem=${MEM}MB cpus=$CPUS disk=$DISK)"
exec "$QEMU" \
    "${ACCEL_ARGS[@]}" \
    -m "$MEM" -smp "$CPUS" \
    -machine pc,accel="${ACCEL}" \
    -cdrom "$ISO" \
    -drive file="$DISK",if=ide,format=qcow2 \
    -boot order=d,menu=on \
    -vga std -usb \
    -netdev user,id=n0 -device e1000,netdev=n0 \
    -rtc base=localtime \
    "$@"
