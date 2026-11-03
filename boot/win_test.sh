#!/usr/bin/env bash
# Boots LoOS under QEMU, runs smss with real PE processes (tests/win/svc.exe under the NT service names) from a tar module.
set -u
here=$(cd "$(dirname "$0")" && pwd); root=$here/..
out=${1:-${LOOS_BOOT_OUT:-/tmp/opencode/loos-boot}}; mkdir -p "$out"
x86_64-w64-mingw32-gcc -nostdlib -nostartfiles -O2 -Wl,-e,start -Wl,--subsystem,console -Wl,--image-base,0x140000000 \
    "$root/tests/win/svc.c" -lkernel32 -o "$out/svc.exe" || exit 1
tarf=$out/winroot.tar
python3 - "$out/svc.exe" "$tarf" <<'PY'
import sys, tarfile, io
data = open(sys.argv[1], "rb").read()
with tarfile.open(sys.argv[2], "w", format=tarfile.USTAR_FORMAT) as t:
    for d in ("Windows", "Windows/System32"):
        i = tarfile.TarInfo(d); i.type = tarfile.DIRTYPE; i.mode = 0o755; t.addfile(i)
    for n in ("autochk", "csrss", "wininit", "services", "lsass", "winlogon"):
        i = tarfile.TarInfo(f"Windows/System32/{n}.exe"); i.size = len(data); i.mode = 0o755; t.addfile(i, io.BytesIO(data))
PY
img=$("$here/bios/build.sh" "$out" --module winroot="$tarf" --cmdline "console=ttyS0 loos.test=1 loos.win=1") || exit 1
log=$(mktemp)
timeout 90 qemu-system-x86_64 ${QEMU_ACCEL-$(LOOS_KVM=${LOOS_KVM:-off} "$root/scripts/qemu-accel.sh" args)} -display none -vga std -smp 1 -m 256M \
    -serial "file:$log" -drive "file=$img,format=raw,if=ide" -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot
grep -E "^(win:|smss:)" "$log"
grep -qF "win: smss ok" "$log" || { echo "win test FAILED"; tail -30 "$log"; exit 1; }
echo "win ring-3 smss test ok"
