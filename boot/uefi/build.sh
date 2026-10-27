#!/usr/bin/env bash
# Builds the UEFI boot disk: uefi.img = GPT with a FAT32 ESP holding \EFI\BOOT\BOOTX64.EFI, \LOOS\KERNEL.ELF, modules, BOOT.CFG.
#   build.sh [outdir] [--module NAME=PATH ...] [--cmdline TEXT]
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd); root=$here/../..
out=$root/out/boot
if [ $# -gt 0 ] && [[ $1 != --* ]]; then out=$1; shift; fi
mkdir -p "$out"
modules=(); cmdline=""
while [ $# -gt 0 ]; do case $1 in --module) modules+=("$2"); shift 2;; --cmdline) cmdline=$2; shift 2;; *) echo "bad arg $1" >&2; exit 2;; esac; done
cc=${CC:-clang}
"$root/boot/build-kernel.sh" "$out" >/dev/null

find_lld() {
    [ -n "${LLD:-}" ] && { echo "$LLD"; return; }
    command -v lld-link >/dev/null 2>&1 && { command -v lld-link; return; }
    for t in "$HOME"/.rustup/toolchains/*/lib/rustlib/*/bin/rust-lld; do
        if [ -x "$t" ]; then
            echo "$t -flavor link"
            return
        fi
    done
    echo "error: no lld found (install lld, or set LLD=)" >&2; exit 1
}
LINK=$(find_lld)
if [[ "$LINK" == *rust-lld* ]]; then
    # find_lld runs in command substitution, so repeat the environment setup
    # in the parent shell before invoking the linker.
    rust_lld=${LINK%% *}
    rust_lib=$(cd "$(dirname "$rust_lld")/../../.." && pwd)
    export LD_LIBRARY_PATH="$rust_lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi
cflags="--target=x86_64-unknown-windows -std=c11 -ffreestanding -fshort-wchar -fno-stack-protector -fno-stack-check -mno-red-zone -mgeneral-regs-only -O2 -Wall -Wextra -Werror"
objs=()
for f in "$here/main.c" "$root"/boot/common/{loader,mmap,elf,paging,rt}.c; do
    o=$out/uefi_$(basename "${f%.c}").o; $cc $cflags -c "$f" -o "$o"; objs+=("$o")
done
$LINK /subsystem:efi_application /entry:efi_main /nodefaultlib /opt:ref /out:"$out/BOOTX64.EFI" "${objs[@]}" >/dev/null

# ESP contents
cfg=$out/BOOT.CFG
{ echo "kernel=\\LOOS\\KERNEL.ELF"
  for m in "${modules[@]}"; do n=${m%%=*}; echo "module=$n:\\LOOS\\$(echo "$n" | tr a-z A-Z).BIN"; done
  [ -n "$cmdline" ] && echo "cmdline=$cmdline"; true; } > "$cfg"

img=$out/uefi.img
rm -f "$img"; truncate -s 192M "$img"
sgdisk -o -n 1:2048:0 -t 1:ef00 -c 1:LOOS-ESP "$img" >/dev/null
psize=$(sgdisk -i 1 "$img" | awk '/^Partition size:/ {print $3}')
mkfs.fat -F 32 --offset 2048 -S 512 "$img" $(( psize / 2 )) >/dev/null 2>&1
M="mtools -c"; off=$((2048*512))
mmd -i "$img@@$off" ::EFI ::EFI/BOOT ::LOOS
mcopy -i "$img@@$off" "$out/BOOTX64.EFI" ::EFI/BOOT/BOOTX64.EFI
mcopy -i "$img@@$off" "$out/kernel.elf" ::LOOS/KERNEL.ELF
mcopy -i "$img@@$off" "$cfg" ::LOOS/BOOT.CFG
for m in "${modules[@]}"; do n=${m%%=*}; p=${m#*=}; mcopy -i "$img@@$off" "$p" "::LOOS/$(echo "$n" | tr a-z A-Z).BIN"; done
echo "$img"
