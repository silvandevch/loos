#!/usr/bin/env bash
# Boots with safemode + a deliberate page fault; checks the stop report, the crash dump and QEMU exit 39.
# Additionally holds each crash screen in the GUI window for $HOLD seconds (default 15).
set -u
here=$(cd "$(dirname "$0")" && pwd); root=$here/../..
out=${1:-$root/out/boot}
hold=${HOLD:-15}

for kind in pf ud gp manual stack; do
    case $kind in
        pf|stack) code="STOP 0x0000000000000050";;
        ud|gp)    code="STOP 0x000000000000001e";;
        manual)   code="STOP 0x00000000000000e2";;
    esac
    img=$("$here/build.sh" "$out" --cmdline "console=ttyS0 loos.test=1 bootmenu bootmenu.auto=1 loos.bugcheck=$kind") || exit 1

    # ---- 1) verification run: headless, with isa-debug-exit -> real QEMU exit code 39 ----
    vlog=$(mktemp)
    timeout 60 qemu-system-x86_64 ${QEMU_ACCEL-$(LOOS_KVM=${LOOS_KVM:-off} "$root/scripts/qemu-accel.sh" args)} -display none -vga std -smp 2 -m 256M \
        -serial "file:$vlog" -drive "file=$img,format=raw,if=ide" -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot
    rc=$?
    for want in "bootmenu: picked Safe mode" "boot: SAFE MODE" "SMP: 1 CPUs online" "executive: System pid 1, syscalls ok" "$code" "LoOS kernel: bugcheck"; do
        grep -qF -- "$want" "$vlog" || { echo "$kind missing: $want"; cat "$vlog"; exit 1; }
    done
    [ $kind = manual ] || grep -qE "dump [1-9][0-9]* bytes" "$vlog" || { echo "$kind: no dump"; cat "$vlog"; exit 1; }
    [ $rc -eq 39 ] || { echo "$kind: unexpected qemu exit $rc"; cat "$vlog"; exit 1; }
    rm -f "$vlog"
    echo "$kind: verified (qemu exit $rc)"

    # ---- 2) viewing run: GUI, NO isa-debug-exit, keep the crash screen up for $HOLD seconds ----
    glog=$(mktemp)
    qemu-system-x86_64 ${QEMU_ACCEL-$(LOOS_KVM=${LOOS_KVM:-off} "$root/scripts/qemu-accel.sh" args)} -display gtk -vga std -smp 2 -m 256M \
        -serial "file:$glog" -drive "file=$img,format=raw,if=ide" -no-reboot -no-shutdown &
    qpid=$!
    ( sleep 60; kill -TERM "$qpid" 2>/dev/null; sleep 2; kill -KILL "$qpid" 2>/dev/null ) & wd=$!

    while kill -0 "$qpid" 2>/dev/null; do
        grep -qF "LoOS kernel: bugcheck" "$glog" 2>/dev/null && break
        sleep 0.05
    done
    if kill -0 "$qpid" 2>/dev/null; then
        echo "  -> holding $kind crash screen for ${hold}s"
        sleep "$hold"
        kill "$qpid" 2>/dev/null
    fi
    wait "$qpid" 2>/dev/null
    kill "$wd" 2>/dev/null; wait "$wd" 2>/dev/null
    rm -f "$glog"
done
echo "bugcheck + safe mode ok"