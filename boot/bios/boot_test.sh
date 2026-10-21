#!/usr/bin/env bash
# Boots the BIOS image headless under QEMU with 4 CPUs, a boot module and KASLR, and checks the kernel's own report.
set -u
here=$(cd "$(dirname "$0")" && pwd); root=$here/../..
out=${1:-$root/out/boot}
mod=$(mktemp); head -c 40000 /dev/urandom > "$mod"
crc=$(python3 -c "import zlib,sys; print('0x%016x' % zlib.crc32(open('$mod','rb').read()))")
img=$("$here/build.sh" "$out" --module initrd="$mod" --cmdline "console=ttyS0 loos.test=1") || exit 1
log=$(mktemp)
timeout 60 qemu-system-x86_64 ${QEMU_ACCEL:-} -display none -vga std -smp 4 -m 256M -serial "file:$log" -drive "file=$img,format=raw,if=ide" \
    -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot
rc=$?
for want in "LoOS BIOS loader" "LoOS kernel: entered at 0xffffffff" "boot: BIOS, KASLR on" "blob: 3145728 bytes" \
            "module: initrd 40000 bytes, crc32 $crc" "cmdline: console=ttyS0 loos.test=1" "W^X: text r-x, rodata r--, data rw- ok" \
            "ACPI: RSDP 0x" "MADT lists 4 CPUs" "SMP: 4 CPUs online" "framebuffer 1920x1080x32" "desktop: 3 windows at 1920x1080, UI scale 100%" "stage 0 done"; do
    grep -qF -- "$want" "$log" || { echo "missing: $want"; cat "$log"; exit 1; }
done
[ $rc -eq 33 ] || { echo "unexpected qemu exit $rc"; cat "$log"; exit 1; }
echo "bios boot ok"
