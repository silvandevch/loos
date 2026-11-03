#!/usr/bin/env bash
# Builds the stage-0 kernel as a static PIE ELF (relocated and KASLR-slid by the loader): $1 = output dir.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd); root=$here/..
out=${1:-${LOOS_BOOT_OUT:-/tmp/opencode/loos-boot}}; mkdir -p "$out"
cc=${CC:-clang}
kdir=$root/src/kernel/boot
flags="--target=x86_64-unknown-none -ffreestanding -fPIE -fno-stack-protector -mno-red-zone -mgeneral-regs-only -fno-omit-frame-pointer -O2 -Wall -Wextra -Werror -nostdlibinc -isystem $root/src/libc/include -I$kdir -I$root/src/security -I$root/src/fs/btrfs -I$root/src/fs/diskfmt"
objs=()
for s in entry.S trap.S usermode.S user.c bugcheck.c bootmenu.c kclib.c serial.c ktime.c kalloc.c video.c ata.c pci.c blk.c rustblk.c install.c ps2.c vm.c acpi.c smp.c splash.c shellhost.c kmain.c blob.c; do
    o=$out/k_${s%.*}.o; $cc $flags -c "$kdir/$s" -o "$o"; objs+=("$o")
done
$cc $flags -DLOOS_LIBC_PURE=1 -fno-builtin -I"$root/src/libc/src" -c "$root/src/libc/src/string.c" -o "$out/k_libc_string.o"; objs+=("$out/k_libc_string.o")
for f in fs/btrfs/btrfs fs/btrfs/mkfs fs/diskfmt/gpt fs/diskfmt/fat32; do $cc $flags -c "$root/src/$f.c" -o "$out/k_$(basename $f).o"; objs+=("$out/k_$(basename $f).o"); done
for f in crypto kdf; do $cc $flags -c "$root/src/security/$f.c" -o "$out/k_sec_$f.o"; objs+=("$out/k_sec_$f.o"); done
$cc $flags -c "$root/crates/khal/src/ap_tramp.S" -o "$out/k_aptramp.o"; objs+=("$out/k_aptramp.o")
# Rust executive (no_std staticlib, crates/kffi)
rtarget=${CARGO_TARGET_DIR:-${LOOS_RUST_TARGET:-/tmp/opencode/loos-target}}
cargo ${LOOS_RUST_TOOLCHAIN-+1.97.1} build --release --offline -q -p loos-kffi --target x86_64-unknown-none \
    --manifest-path "$root/crates/Cargo.toml" --target-dir "$rtarget" >&2
objs+=("$rtarget/x86_64-unknown-none/release/libloos_kffi.a")
ld -m elf_x86_64 -pie --no-dynamic-linker -z separate-code -z max-page-size=4096 -z noexecstack -z norelro -e _kstart \
    -o "$out/kernel.elf" "${objs[@]}"
echo "$out/kernel.elf"
