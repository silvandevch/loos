#!/usr/bin/env bash
# Boots the UEFI image under QEMU+OVMF with 4 CPUs and checks the kernel's own report (same checks as the BIOS test).
set -u
here=$(cd "$(dirname "$0")" && pwd); root=$here/../..
out=${1:-$root/out/boot}
fw=${OVMF_CODE:-/usr/share/edk2/ovmf/OVMF_CODE.fd}
[ -r "$fw" ] || { echo "OVMF not found ($fw): skipping"; exit 77; }
mod=$(mktemp); head -c 40000 /dev/urandom > "$mod"
crc=$(python3 -c "import zlib; print('0x%016x' % zlib.crc32(open('$mod','rb').read()))")
img=$("$here/build.sh" "$out" --module initrd="$mod" --cmdline "console=ttyS0 loos.test=1") || exit 1
log=$(mktemp)
timeout 120 qemu-system-x86_64 ${QEMU_ACCEL:-} -display none -vga std -smp 4 -m 512M -serial "file:$log" -bios "$fw" \
    -drive "file=$img,format=raw,if=ide" -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot
rc=$?
for want in "LoOS kernel: entered at 0xffffffff" "boot: UEFI, KASLR on" "blob: 3145728 bytes" "module: initrd 40000 bytes, crc32 $crc" \
            "cmdline: console=ttyS0 loos.test=1" "W^X: text r-x, rodata r--, data rw- ok" "ACPI: RSDP 0x" "MADT lists 4 CPUs" \
            "SMP: 4 CPUs online" "desktop: 3 windows at 1920x1080" "stage 0 done"; do
    grep -qF -- "$want" "$log" || { echo "missing: $want"; cat "$log"; exit 1; }
done
grep -qE "framebuffer [0-9]+x[0-9]+x32" "$log" || { echo "missing framebuffer"; cat "$log"; exit 1; }
[ $rc -eq 33 ] || { echo "unexpected qemu exit $rc"; cat "$log"; exit 1; }
echo "uefi boot ok"
