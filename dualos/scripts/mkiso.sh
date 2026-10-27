#!/usr/bin/env bash
# usage: mkiso.sh kernel32.elf kernel64.elf out.iso
set -euo pipefail
k32=$1 k64=$2 iso=$3
here=$(cd "$(dirname "$0")/.." && pwd)
grub2-file --is-x86-multiboot2 "$k32" || { echo "kernel32 is not a valid Multiboot2 image" >&2; exit 1; }
grub2-file --is-x86-multiboot2 "$k64" || { echo "kernel64 is not a valid Multiboot2 image" >&2; exit 1; }
# Mode-B image must contain no SSE/MMX register use (Pentium baseline).
if objdump -d "$k32" | grep -Eq '%?(xmm|mm)[0-7]'; then echo "kernel32 contains SIMD instructions" >&2; exit 1; fi
stage=$(mktemp -d); trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/boot/grub"
cp "$here/boot/grub.cfg" "$stage/boot/grub/grub.cfg"
cp "$k32" "$stage/boot/kernel32.elf"
cp "$k64" "$stage/boot/kernel64.elf"
mkdir -p "$(dirname "$iso")"
grub2-mkrescue -o "$iso" "$stage" 2>/dev/null
echo "built $iso"
