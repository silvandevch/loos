#!/usr/bin/env bash
# Boots with safemode + a deliberate page fault; checks the stop report, the crash dump and QEMU exit 39.
set -u
here=$(cd "$(dirname "$0")" && pwd); root=$here/../..
out=${1:-$root/out/boot}
for kind in pf ud gp manual stack; do
    img=$("$here/build.sh" "$out" --cmdline "console=ttyS0 loos.test=1 bootmenu bootmenu.auto=1 loos.bugcheck=$kind") || exit 1
    log=$(mktemp)
    timeout 60 qemu-system-x86_64 ${QEMU_ACCEL-$(LOOS_KVM=${LOOS_KVM:-off} "$root/scripts/qemu-accel.sh" args)} -display none -vga std -smp 2 -m 256M \
        -serial "file:$log" -drive "file=$img,format=raw,if=ide" -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot
    rc=$?
    case $kind in pf|stack) code="STOP 0x0000000000000050";; ud|gp) code="STOP 0x000000000000001e";; manual) code="STOP 0x00000000000000e2";; esac
    for want in "bootmenu: picked Safe mode" "boot: SAFE MODE" "SMP: 1 CPUs online" "executive: System pid 1, syscalls ok" "$code" "LoOS kernel: bugcheck"; do
        grep -qF -- "$want" "$log" || { echo "$kind missing: $want"; cat "$log"; exit 1; }
    done
    [ $kind = manual ] || grep -qE "dump [1-9][0-9]* bytes" "$log" || { echo "$kind: no dump"; cat "$log"; exit 1; }
    [ $rc -eq 39 ] || { echo "$kind: unexpected qemu exit $rc"; cat "$log"; exit 1; }
done
echo "bugcheck + safe mode ok"
