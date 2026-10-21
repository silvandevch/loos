#!/usr/bin/env bash
# Builds the stage-0 kernel as a static PIE ELF (relocated and KASLR-slid by the loader): $1 = output dir.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd); root=$here/..
out=${1:-$root/out/boot}; mkdir -p "$out"
cc=${CC:-clang}
kdir=$root/src/kernel/boot
flags="--target=x86_64-unknown-none -ffreestanding -fPIE -fno-stack-protector -mno-red-zone -mgeneral-regs-only -O2 -Wall -Wextra -Werror -nostdlibinc -isystem $root/src/libc/include -I$kdir -I$root/src/shell -I$root/src/security -I$root/src/fs/btrfs -I$root/src/fs/diskfmt"
objs=()
for s in entry.S kclib.c serial.c ktime.c kalloc.c video.c ata.c pci.c blk.c ahci.c nvme.c install.c ps2.c vm.c acpi.c smp.c splash.c shellhost.c kmain.c blob.c; do
    o=$out/k_${s%.*}.o; $cc $flags -c "$kdir/$s" -o "$o"; objs+=("$o")
done
for f in installer session desktop font raster font_data draw compositor inventory launcher taskbar wm ui backend_mem settings apphost util filemgr taskmon; do
    $cc $flags -c "$root/src/shell/$f.c" -o "$out/k_shell_$f.o"; objs+=("$out/k_shell_$f.o")
done
$cc $flags -DLOOS_LIBC_PURE=1 -fno-builtin -I"$root/src/libc/src" -c "$root/src/libc/src/string.c" -o "$out/k_libc_string.o"; objs+=("$out/k_libc_string.o")
for f in fs/btrfs/btrfs fs/btrfs/mkfs fs/diskfmt/gpt fs/diskfmt/fat32; do $cc $flags -c "$root/src/$f.c" -o "$out/k_$(basename $f).o"; objs+=("$out/k_$(basename $f).o"); done
for f in crypto kdf; do $cc $flags -c "$root/src/security/$f.c" -o "$out/k_sec_$f.o"; objs+=("$out/k_sec_$f.o"); done
$cc $flags -c "$root/crates/khal/src/ap_tramp.S" -o "$out/k_aptramp.o"; objs+=("$out/k_aptramp.o")
ld -m elf_x86_64 -pie --no-dynamic-linker -z separate-code -z max-page-size=4096 -z noexecstack -z norelro -e _kstart \
    -o "$out/kernel.elf" "${objs[@]}"
echo "$out/kernel.elf"
