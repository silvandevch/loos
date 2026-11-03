#!/usr/bin/env bash
# Builds the BIOS boot image: bios.img = MBR + stage 2 + boot table + kernel ELF (+ modules).
#   build.sh [outdir] [--module NAME=PATH ...] [--cmdline TEXT]
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd); root=$here/../..
out=${LOOS_BOOT_OUT:-/tmp/opencode/loos-boot}
if [ $# -gt 0 ] && [[ $1 != --* ]]; then out=$1; shift; fi
mkdir -p "$out"
cc=${CC:-clang}
"$root/boot/build-kernel.sh" "$out" >/dev/null
$cc --target=i386-none-elf -c "$here/mbr.S" -o "$out/mbr.o"
ld -m elf_i386 -Ttext 0x7c00 -e _start --oformat binary -o "$out/mbr.bin" "$out/mbr.o"
$cc --target=x86_64-unknown-none -c "$here/stage2.S" -o "$out/stage2.o"
cflags="--target=x86_64-unknown-none -std=c11 -ffreestanding -fno-pic -fno-stack-protector -mno-red-zone -mgeneral-regs-only -O2 -Wall -Wextra -Werror"
for f in frontend; do $cc $cflags -c "$here/$f.c" -o "$out/bios_$f.o"; done
for f in loader mmap elf paging rt; do $cc $cflags -c "$root/boot/common/$f.c" -o "$out/bios_$f.o"; done
ld -m elf_x86_64 -T "$here/link.ld" --oformat binary -o "$out/stage2.bin" "$out/stage2.o" "$out/bios_frontend.o" \
    "$out/bios_loader.o" "$out/bios_mmap.o" "$out/bios_elf.o" "$out/bios_paging.o" "$out/bios_rt.o"
python3 "$root/boot/tools/mkbios.py" "$out/bios.img" "$out/mbr.bin" "$out/stage2.bin" "$out/kernel.elf" "$@"
echo "$out/bios.img"
