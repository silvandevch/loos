#!/usr/bin/env bash
# Proves the desktop's double-click path: boots the LoOS desktop with a compat runtime (initrd) and opens one
# file through Files' open logic (loos.open=/programs/<path>, the same code a double-click runs), which must
# reach the matching layer and run the program to its end.
#   open_test.sh <path inside the runtime> [rootfs paths...]     e.g. open_test.sh /usr/bin/echo
#   open_test.sh /bin/mac_hello_x86_64 (needs the Prism samples: ROOTFS from compat-modules.sh)
# EXPECT=<wait status> when the program exits non-zero on purpose (default 0).
# Shows the serial lines that matter and exits non-zero unless the program started and finished.
set -u
here=$(cd "$(dirname "$0")" && pwd); root=$here/..
target=${1:?path inside the runtime}; shift
out=${LOOS_BOOT_OUT:-$root/out/open-test}; mkdir -p "$out"
"$root/tools/lin-rootfs.sh" "$out/rootfs.tar" "$@" >/dev/null || exit 1
img=$("$here/uefi/build.sh" "$out" --module initrd="$out/rootfs.tar" --cmdline "console=ttyS0 loos.test=1 loos.open=/programs$target") || exit 1
log=$out/serial.log; rm -f "$log"
timeout ${TIMEOUT:-120} qemu-system-x86_64 ${QEMU_ACCEL-$(LOOS_KVM=${LOOS_KVM:-auto} "$root/scripts/qemu-accel.sh" args)} -display none -vga std -smp 1 \
    -m ${QEMU_MEM:-4G} -nic user,model=e1000 -serial "file:$log" -bios "${OVMF_CODE:-/usr/share/edk2/ovmf/OVMF_CODE.fd}" \
    -drive "file=$img,format=raw,if=virtio" -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot >/dev/null 2>&1
grep -aE "^(compat|desktop: opening|Starting|No app|Programs run|prism:|lin: (cannot|program finished|\[1)|.*could not start)" "$log" | head -20
grep -aq "^compat: program ended, status ${EXPECT:-0}$" "$log" && echo "open test ok: $target" || { echo "open test FAILED: $target"; exit 1; }
