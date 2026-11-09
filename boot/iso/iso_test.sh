#!/usr/bin/env bash
# Boots the install ISO as a CD-ROM under QEMU+OVMF (NVMe target, like a VMware NVMe disk), installs unattended, then boots the
# installed disk alone. Env: QEMU_ACCEL="-enable-kvm -cpu host".
set -u
here=$(cd "$(dirname "$0")" && pwd); root=$here/../..
out=${1:-$root/out/boot}; accel=${QEMU_ACCEL:-}
fw=${OVMF_CODE:-/usr/share/edk2/ovmf/OVMF_CODE.fd}
[ -r "$fw" ] || { echo "OVMF not found: skipping"; exit 77; }
work=$(mktemp -d "${TMPDIR:-/tmp}/loos-iso.XXXXXX"); trap 'rm -rf "$work"' EXIT
iso=$("$here/build.sh" "$out" --cmdline "installer installer.auto=0 installer.user=tester installer.pass=correct-horse9 login.test=tester:correct-horse9 loos.test") || exit 1
truncate -s 4G "$work/disk.img"
nvme="-drive file=$work/disk.img,if=none,id=nv0,format=raw -device nvme,drive=nv0,serial=loos0"
log=$work/a.log
timeout 400 qemu-system-x86_64 $accel -display ${QEMU_DISPLAY:-gtk} -vga std -smp 2 -m 2G -serial "file:$log" -bios "$fw" \
    -drive "file=$iso,format=raw,if=none,id=cd0,readonly=on" -device ide-cd,drive=cd0,bootindex=0 $nvme -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot
rc=$?
for want in "boot: UEFI" "install: media is installer" "install: ok (verified)"; do grep -qF -- "$want" "$log" || { echo "ISO install: missing '$want'"; cat "$log"; exit 1; }; done
[ $rc -eq 33 ] || { echo "ISO install: qemu exit $rc"; cat "$log"; exit 1; }
log=$work/b.log
timeout 200 qemu-system-x86_64 $accel -display ${QEMU_DISPLAY:-gtk} -vga std -smp 2 -m 2G -serial "file:$log" -bios "$fw" $nvme,bootindex=0 \
    -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot
rc=$?
for want in "install: media is an installed system" "login: ok tester"; do grep -qF -- "$want" "$log" || { echo "installed boot: missing '$want'"; cat "$log"; exit 1; }; done
echo "iso ok"
