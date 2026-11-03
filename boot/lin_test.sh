#!/usr/bin/env bash
# Boots LoOS under QEMU and runs the freestanding Linux test program (tests/lin/lbox.c) in ring 3 from a tar initrd.
set -u
here=$(cd "$(dirname "$0")" && pwd); root=$here/..
out=${1:-${LOOS_BOOT_OUT:-/tmp/opencode/loos-boot}}; mkdir -p "$out"
clang --target=x86_64-linux-gnu -static -nostdlib -O2 -ffreestanding -fno-stack-protector -fno-pic -mno-red-zone \
    "$root/tests/lin/lbox.c" -o "$out/lbox" 2>/dev/null || exit 1
tarf=$out/lin-initrd.tar
python3 - "$out/lbox" "$tarf" <<'PY'
import sys, tarfile, io
with tarfile.open(sys.argv[2], "w", format=tarfile.USTAR_FORMAT) as t:
    for d in ("bin", "etc"):
        i = tarfile.TarInfo(d); i.type = tarfile.DIRTYPE; i.mode = 0o755; t.addfile(i)
    data = open(sys.argv[1], "rb").read()
    i = tarfile.TarInfo("bin/lbox"); i.size = len(data); i.mode = 0o755; t.addfile(i, io.BytesIO(data))
    m = b"LoOS says hello from the initrd\n"
    i = tarfile.TarInfo("etc/motd"); i.size = len(m); i.mode = 0o644; t.addfile(i, io.BytesIO(m))
PY
img=$("$here/bios/build.sh" "$out" --module initrd="$tarf" --cmdline "console=ttyS0 loos.test=1 loos.lin=/bin/lbox,selftest") || exit 1
log=$(mktemp)
timeout 90 qemu-system-x86_64 ${QEMU_ACCEL-$(LOOS_KVM=${LOOS_KVM:-off} "$root/scripts/qemu-accel.sh" args)} -display none -vga std -smp 1 -m 256M \
    -serial "file:$log" -drive "file=$img,format=raw,if=ide" -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot
cat "$log" | grep -E "^(lin:|ok  |FAIL|LBOX)" 
grep -qF "LBOX: all ok" "$log" || { echo "lin test FAILED"; tail -30 "$log"; exit 1; }
grep -qF "FAIL " "$log" && { echo "lin test FAILED"; exit 1; }
echo "lin ring-3 test ok"
