#!/usr/bin/env bash
# Installs to an IDE disk, boots it with a deliberate page fault (dump saved to the on-disk area), then boots it
# again and checks the kernel reports the saved dump once.
set -u
here=$(cd "$(dirname "$0")" && pwd); root=$here/../..
out=${1:-$root/out/boot}
work=$(mktemp -d "${TMPDIR:-/tmp}/loos-dump.XXXXXX"); trap 'rm -rf "$work"' EXIT
q() { timeout 300 qemu-system-x86_64 ${QEMU_ACCEL:-} -display gtk -vga std -smp 2 -m 512M -serial "file:$1" "${@:2}" \
        -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot; }
img=$("$here/build.sh" "$out" --cmdline "installer installer.auto=1 installer.user=tester installer.pass=correct-horse9 loos.test") || exit 1
disk=$work/disk.img; truncate -s 2G "$disk"
q "$work/1.log" -drive "file=$img,format=raw,if=ide,index=0" -drive "file=$disk,format=raw,if=ide,index=1"
grep -qF "install: ok (verified)" "$work/1.log" || { echo "install failed"; cat "$work/1.log"; exit 1; }
patch() {  # rewrite the installed boot-table command line
    python3 - "$disk" "$1" <<'PY'
import sys
f = open(sys.argv[1], "r+b"); f.seek(65 * 512 + 16); f.write(sys.argv[2].encode().ljust(232, b"\0"))
PY
}
patch "loos.test loos.bugcheck=pf"
q "$work/2.log" -drive "file=$disk,format=raw,if=ide"
for want in "crashdump: area on" "STOP 0x0000000000000050" "dump saved to disk"; do
    grep -qF -- "$want" "$work/2.log" || { echo "crash boot missing: $want"; cat "$work/2.log"; exit 1; }
done
patch "loos.test"
q "$work/3.log" -drive "file=$disk,format=raw,if=ide"
grep -qF "crashdump: the last session stopped with 0x0000000000000050 PAGE_FAULT_IN_NONPAGED_AREA" "$work/3.log" || { echo "no report"; cat "$work/3.log"; exit 1; }
q "$work/4.log" -drive "file=$disk,format=raw,if=ide"
grep -qF "the last session stopped" "$work/4.log" && { echo "reported twice"; exit 1; }
echo "crash dump on disk ok"
