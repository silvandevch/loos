#!/usr/bin/env bash
# Builds a UEFI installer-style disk with diskfmt_tool and checks it with the host's own tools: sgdisk (GPT), fsck.fat (ESP),
# mtools (file contents), btrfs check (data partition). Usage: diskfmt_test.sh path/to/diskfmt_tool workdir
set -eu
tool=$1; work=$2; mkdir -p "$work"
for t in sgdisk fsck.fat mcopy btrfs dd cmp; do command -v $t >/dev/null || { echo "missing $t: skipping"; exit 77; }; done
head -c 150000 /dev/urandom > "$work/efi.bin"; head -c 3300000 /dev/urandom > "$work/kern.bin"; echo "kernel=\\LOOS\\KERNEL.ELF" > "$work/boot.cfg"
for size in 700 1500; do
    read -r esp_off data_off < <("$tool" "$work/d.img" $size 300 BOOTX64.EFI="$work/efi.bin" KERNEL.ELF="$work/kern.bin" BOOT.CFG="$work/boot.cfg")
    sgdisk -v "$work/d.img" | grep -q "No problems found"
    dd if="$work/d.img" of="$work/esp.img" bs=1M skip=$((esp_off / 1048576)) count=300 status=none
    fsck.fat -n "$work/esp.img" > "$work/fsck.log" 2>&1 || { cat "$work/fsck.log"; exit 1; }
    mcopy -o -i "$work/esp.img" ::/LOOS/KERNEL.ELF "$work/kern.out"; cmp "$work/kern.bin" "$work/kern.out"
    mcopy -o -i "$work/esp.img" ::/EFI/BOOT/BOOTX64.EFI "$work/efi.out"; cmp "$work/efi.bin" "$work/efi.out"
    mcopy -o -i "$work/esp.img" ::/LOOS/BOOT.CFG "$work/cfg.out"; cmp "$work/boot.cfg" "$work/cfg.out"
    dd if="$work/d.img" of="$work/data.img" bs=1M skip=$((data_off / 1048576)) status=none
    btrfs check --readonly "$work/data.img" > "$work/btrfs.log" 2>&1 || { cat "$work/btrfs.log"; exit 1; }
done
echo "diskfmt ok"
