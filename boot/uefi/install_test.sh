#!/usr/bin/env bash
# End-to-end UEFI installer test under QEMU+OVMF for every supported disk controller (IDE, AHCI, NVMe): unattended install,
# then the host's own tools check the result (sgdisk, fsck.fat, mtools, btrfs check) and the installed disk boots alone.
# Env: QEMU_ACCEL="-enable-kvm -cpu host" for KVM; CONTROLLERS="ide ahci nvme" to select.
set -u
here=$(cd "$(dirname "$0")" && pwd); root=$here/../..
out=${1:-$root/out/boot}
accel=${QEMU_ACCEL:-}
fw=${OVMF_CODE:-/usr/share/edk2/ovmf/OVMF_CODE.fd}
[ -r "$fw" ] || { echo "OVMF not found ($fw): skipping"; exit 77; }
for t in btrfs sgdisk fsck.fat mcopy dd cmp; do command -v $t >/dev/null || { echo "missing $t: skipping"; exit 77; }; done
mkdir -p "$out"
work=$(mktemp -d "${TMPDIR:-/tmp}/loos-uefi-install.XXXXXX"); trap 'rm -rf "$work"' EXIT
mod=$work/initrd.bin; head -c 30000 /dev/urandom > "$mod"
img=$("$here/build.sh" "$out" --module initrd="$mod" --cmdline "installer installer.auto=1 installer.user=tester installer.pass=correct-horse9 login.test=tester:correct-horse9 loos.test") || exit 1
media=$work/media.img; cp "$img" "$media"
run_case() {
    local name=$1 target_args=$2 boot_args=$3 want_disk=$4
    local target=$work/target_$name.img
    rm -f "$target"; truncate -s 2G "$target"
    local log; log=$(mktemp)
    timeout 400 qemu-system-x86_64 $accel -display none -vga std -smp 2 -m 1G -serial "file:$log" -bios "$fw" \
        -drive "file=$media,format=raw,if=ide,index=0" $target_args -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot
    local rc=$?
    for want in "$want_disk" "boot: UEFI" "install: media is installer" "install: ok (verified)" "stage 0 done"; do
        grep -qF -- "$want" "$log" || { echo "[$name] installer run: missing '$want'"; cat "$log"; return 1; }
    done
    [ $rc -eq 33 ] || { echo "[$name] installer run: qemu exit $rc"; cat "$log"; return 1; }
    sgdisk -v "$target" | grep -q "No problems found" || { echo "[$name] GPT check failed"; sgdisk -v "$target"; return 1; }
    local esp_start esp_size data_start
    esp_start=$(sgdisk -i 1 "$target" | awk '/First sector:/ {print $3}'); esp_size=$(sgdisk -i 1 "$target" | awk '/Partition size:/ {print $3}')
    data_start=$(sgdisk -i 2 "$target" | awk '/First sector:/ {print $3}')
    dd if="$target" of="$work/esp.img" bs=512 skip="$esp_start" count="$esp_size" status=none
    fsck.fat -n "$work/esp.img" > "$work/fsck.log" 2>&1 || { echo "[$name] fsck.fat failed"; cat "$work/fsck.log"; return 1; }
    mcopy -o -i "$work/esp.img" ::/EFI/BOOT/BOOTX64.EFI "$work/e.out"
    mcopy -o -i "$work/esp.img" ::/LOOS/BOOT.CFG "$work/cfg.out"
    mcopy -o -i "$work/esp.img" ::/LOOS/INITRD.BIN "$work/i.out"; cmp "$mod" "$work/i.out" || { echo "[$name] module differs"; return 1; }
    mcopy -o -i "$work/esp.img" ::/LOOS/CONFIG.BIN "$work/c.out"
    grep -q "^module=config:" "$work/cfg.out" && ! grep -qE "^cmdline=.*(^| )installer( |$)" "$work/cfg.out" && ! grep -q "efi:" "$work/cfg.out" || { echo "[$name] BOOT.CFG wrong:"; cat "$work/cfg.out"; return 1; }
    head -c 9 "$work/c.out" | grep -q "LOOSCFG1" && ! grep -q "correct-horse9" "$work/c.out" || { echo "[$name] CONFIG.BIN wrong"; return 1; }
    dd if="$target" of="$work/data.img" bs=512 skip="$data_start" status=none
    btrfs check --readonly "$work/data.img" > "$work/btrfs.log" 2>&1 || { echo "[$name] btrfs check failed"; cat "$work/btrfs.log"; return 1; }
    local log2; log2=$(mktemp)
    timeout 300 qemu-system-x86_64 $accel -display none -vga std -smp 2 -m 1G -serial "file:$log2" -bios "$fw" $boot_args \
        -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot
    rc=$?
    for want in "boot: UEFI" "install: media is an installed system" "login: wrong password rejected" "login: ok tester" "module: initrd 30000 bytes" "stage 0 done"; do
        grep -qF -- "$want" "$log2" || { echo "[$name] installed boot: missing '$want'"; cat "$log2"; return 1; }
    done
    [ $rc -eq 33 ] || { echo "[$name] installed boot: qemu exit $rc"; cat "$log2"; return 1; }
    echo "[$name] ok"
}
T=$work
for c in ${CONTROLLERS:-ide ahci nvme}; do
    case $c in
    ide)  run_case ide "-drive file=$T/target_ide.img,format=raw,if=ide,index=1" "-drive file=$T/target_ide.img,format=raw,if=ide,index=0" "ide1:" || exit 1 ;;
    ahci) run_case ahci "-device ich9-ahci,id=ahci0 -drive file=$T/target_ahci.img,if=none,id=sd0,format=raw -device ide-hd,drive=sd0,bus=ahci0.0" \
            "-device ich9-ahci,id=ahci0 -drive file=$T/target_ahci.img,if=none,id=sd0,format=raw -device ide-hd,drive=sd0,bus=ahci0.0" "sata0:" || exit 1 ;;
    nvme) run_case nvme "-drive file=$T/target_nvme.img,if=none,id=nv0,format=raw -device nvme,drive=nv0,serial=loos0" \
            "-drive file=$T/target_nvme.img,if=none,id=nv0,format=raw -device nvme,drive=nv0,serial=loos0,bootindex=0" "nvme0:" || exit 1 ;;
    esac
done
echo "uefi install ok"
