#!/usr/bin/env bash
# Builds the LoOS UEFI install ISO (El Torito EFI image + hybrid GPT, so it can also be dd'd to a USB stick).
#   build.sh [outdir] [--cmdline TEXT]     -> prints the ISO path (outdir/loos.iso)
# Default command line is "installer" (interactive wizard). VMware/VirtualBox/QEMU/real hardware: see docs/INSTALL.md.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd); root=$here/../..
out=$root/out/boot
if [ $# -gt 0 ] && [[ $1 != --* ]]; then out=$1; shift; fi
cmdline="installer"
while [ $# -gt 0 ]; do case $1 in --cmdline) cmdline=$2; shift 2;; *) echo "bad arg $1" >&2; exit 2;; esac; done
for t in xorriso mformat mcopy mmd; do command -v $t >/dev/null || { echo "missing $t (dnf/apt install xorriso mtools)" >&2; exit 1; }; done
mkdir -p "$out"
# Compatibility-layer file systems carried on the media (and copied to the installed system): sample Windows and Linux apps.
winrt=$out/winroot.tar; linrt=$out/initrd.tar
python3 - "$root" "$winrt" <<'PY'
import sys, tarfile
root, dst = sys.argv[1:3]
with tarfile.open(dst, "w", format=tarfile.USTAR_FORMAT) as t:
    for d in ("Windows", "Windows/System32", "Users", "Users/user"):
        i = tarfile.TarInfo(d); i.type = tarfile.DIRTYPE; i.mode = 0o755; t.addfile(i)
    for src, name in (("tests/win/hello_gui.exe", "hello_gui.exe"), ("apps/loos-notepad/notepad.exe", "notepad.exe"),
                      ("apps/loos-cmd/cmd.exe", "cmd.exe"), ("tests/win/nettest.exe", "nettest.exe"), ("tests/win/selftest.exe", "selftest.exe")):
        t.add(f"{root}/{src}", arcname=f"Windows/System32/{name}")
PY
clang --target=x86_64-linux-gnu -static -nostdlib -O2 -ffreestanding -fno-stack-protector -fno-pic -mno-red-zone "$root/tests/lin/lbox.c" -o "$out/lbox"
clang --target=x86_64-linux-gnu -static -nostdlib -O2 -ffreestanding -fno-stack-protector -fno-pic -mno-red-zone -I"$root/src/lin" \
    "$root/tests/lin/gui.c" "$root/tests/lin/looswin_client.c" -o "$out/gui"
python3 - "$out" "$linrt" <<'PY'
import io, sys, tarfile
out, dst = sys.argv[1:3]
with tarfile.open(dst, "w", format=tarfile.USTAR_FORMAT) as t:
    for d in ("bin", "etc"):
        i = tarfile.TarInfo(d); i.type = tarfile.DIRTYPE; i.mode = 0o755; t.addfile(i)
    for n in ("lbox", "gui"):
        t.add(f"{out}/{n}", arcname=f"bin/{n}")
    m = b"LoOS says hello from the initrd\n"
    i = tarfile.TarInfo("etc/motd"); i.size = len(m); i.mode = 0o644; t.addfile(i, io.BytesIO(m))
PY
"$root/boot/uefi/build.sh" "$out" --module winroot="$winrt" --module initrd="$linrt" --cmdline "$cmdline" >/dev/null
stage=$out/iso-root; rm -rf "$stage"; mkdir -p "$stage/EFI/BOOT" "$stage/LOOS"
# El Torito EFI system image: a bare FAT volume with the same layout as the ESP the firmware would see on a disk.
efiimg=$stage/efiboot.img
kb=$(( ( $(stat -c %s "$out/kernel.elf") + $(stat -c %s "$out/BOOTX64.EFI") + $(stat -c %s "$winrt") + $(stat -c %s "$linrt") ) / 1024 + 8192 ))
truncate -s "$(( (kb + 1023) / 1024 * 1024 ))K" "$efiimg"
mformat -i "$efiimg" -v LOOSBOOT ::
mmd -i "$efiimg" ::EFI ::EFI/BOOT ::LOOS
mcopy -i "$efiimg" "$out/BOOTX64.EFI" ::EFI/BOOT/BOOTX64.EFI
mcopy -i "$efiimg" "$out/kernel.elf" ::LOOS/KERNEL.ELF
mcopy -i "$efiimg" "$out/BOOT.CFG" ::LOOS/BOOT.CFG
mcopy -i "$efiimg" "$winrt" ::LOOS/WINROOT.BIN
mcopy -i "$efiimg" "$linrt" ::LOOS/INITRD.BIN
cp "$out/BOOTX64.EFI" "$stage/EFI/BOOT/"; cp "$out/kernel.elf" "$stage/LOOS/KERNEL.ELF"; cp "$out/BOOT.CFG" "$stage/LOOS/"; cp "$winrt" "$stage/LOOS/WINROOT.BIN"; cp "$linrt" "$stage/LOOS/INITRD.BIN"
args=(-as mkisofs -iso-level 3 -full-iso9660-filenames -volid LOOS_INSTALL -appid "LoOS installer" -o "$out/loos.iso"
      -eltorito-alt-boot -e efiboot.img -no-emul-boot -isohybrid-gpt-basdat)
xorriso "${args[@]}" "$stage" >/dev/null 2>"$out/iso.log" || { cat "$out/iso.log" >&2; exit 1; }
echo "$out/loos.iso"
