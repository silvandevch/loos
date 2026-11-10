#!/usr/bin/env bash
# Boots LoOS under QEMU and runs a dynamically linked glibc program (host /usr/bin/echo + ld.so + libc) from a tar rootfs.
#   lin_dyn_test.sh [program,args...]    default: /usr/bin/echo,dynamic-linking-works
set -u
here=$(cd "$(dirname "$0")" && pwd); root=$here/..
out=${LOOS_BOOT_OUT:-$root/out/boot}; mkdir -p "$out"
cmd=${1:-/usr/bin/echo,dynamic-linking-works}
"$root/tools/lin-rootfs.sh" "$out/lin-rootfs.tar" ${ROOTFS_BINS:-/usr/bin/echo /usr/bin/true /usr/bin/ls /usr/bin/cat} >/dev/null || exit 1
# LIN_BOOT=uefi boots through OVMF, whose loader reads big rootfs images far faster than the BIOS path.
boot=${LIN_BOOT:-bios}; fw=()
[ "$boot" = uefi ] && fw=(-bios "${OVMF_CODE:-/usr/share/edk2/ovmf/OVMF_CODE.fd}")
img=$("$here/$boot/build.sh" "$out" --module initrd="$out/lin-rootfs.tar" --cmdline "console=ttyS0 loos.test=1 loos.net=1 loos.lin=$cmd") || exit 1
log=$out/lin-dyn.log; rm -f "$log"
timeout ${TIMEOUT:-120} qemu-system-x86_64 ${QEMU_ACCEL-$(LOOS_KVM=${LOOS_KVM:-off} "$root/scripts/qemu-accel.sh" args)} -display ${QEMU_DISPLAY:-gtk} -vga std -smp 1 -m ${QEMU_MEM:-4G} -nic user,model=e1000 \
    -serial "file:$log" "${fw[@]}" -drive "file=$img,format=raw,if=${LIN_DISK_IF:-ide}" -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot ${QEMU_EXTRA:-}
grep -aE "^lin:" "$log" | head -20
grep -aqF "program finished, status 0" "$log" && echo "lin dynamic test ok" || { echo "lin dynamic test FAILED"; tail -15 "$log"; exit 1; }
