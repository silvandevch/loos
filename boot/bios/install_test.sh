#!/usr/bin/env bash
# End-to-end BIOS installer test under QEMU for every supported disk controller: install unattended from the media to a
# second disk (IDE, AHCI/SATA, NVMe), inspect the written disk with independent tools (btrfs check), then boot the installed
# disk alone and check its stored account. Env: QEMU_ACCEL="-enable-kvm -cpu host" to run under KVM.
set -u
here=$(cd "$(dirname "$0")" && pwd); root=$here/../..
out=${1:-$root/out/boot}
accel=${QEMU_ACCEL:-}
mkdir -p "$out"
work=$(mktemp -d "${TMPDIR:-/tmp}/loos-install.XXXXXX")        # sparse 2 GiB disks live on tmpfs where available
trap 'rm -rf "$work"' EXIT
for t in btrfs dd; do command -v $t >/dev/null || { echo "missing $t: skipping"; exit 77; }; done
img=$("$here/build.sh" "$out" --cmdline "installer installer.auto=1 installer.user=tester installer.pass=correct-horse9 login.test=tester:correct-horse9 loos.test") || exit 1
run_case() {
    local name=$1 target_args=$2 boot_args=$3 want_disk=$4
    local target=$work/install_target_$name.img
    rm -f "$target"; truncate -s 2G "$target"
    local log; log=$(mktemp)
    timeout 300 qemu-system-x86_64 $accel -display none -vga std -smp 2 -m 512M -serial "file:$log" \
        -drive "file=$img,format=raw,if=ide,index=0" $target_args \
        -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot
    local rc=$?
    for want in "$want_disk" "install: media is installer" "install: ok (verified)" "stage 0 done"; do
        grep -qF -- "$want" "$log" || { echo "[$name] installer run: missing '$want'"; cat "$log"; return 1; }
    done
    [ $rc -eq 33 ] || { echo "[$name] installer run: qemu exit $rc"; cat "$log"; return 1; }
    python3 - "$target" "$img" "$out/part_$name.bin" <<'PY' || return 1
import os, struct, sys
t = open(sys.argv[1], "rb"); s = open(sys.argv[2], "rb")
def sector(f, n): f.seek(n * 512); return f.read(512)
mbr = sector(t, 0)
assert mbr[510:512] == b"\x55\xaa", "no MBR signature"
ptype, start, size = mbr[450], struct.unpack("<I", mbr[454:458])[0], struct.unpack("<I", mbr[458:462])[0]
assert ptype == 0x83 and start % 2048 == 0 and start > 7000 and size > 100000, (ptype, start, size)
assert mbr[:446] == sector(s, 0)[:446], "boot code differs"
tab = sector(t, 65) + sector(t, 66)
assert tab[:8] == b"LOOSBTAB"
n = struct.unpack("<I", tab[8:12])[0]
names = [tab[248 + 40 * i:248 + 40 * i + 16].split(b"\0")[0] for i in range(n)]
assert b"bootimg" not in names and names[0] == b"kernel" and b"config" in names, names
assert "installer" not in tab[16:248].split(b"\0")[0].decode().split()
cfg = (b"".join(sector(t, 67 + i) for i in range(8))).split(b"\0")[0].decode()
assert cfg.startswith("LOOSCFG1\n") and "hostname=loos-test" in cfg and "\naccount tester " in cfg, cfg
assert "correct-horse9" not in cfg, "plain password stored"
patched = {0, 65, 66} | set(range(67, 75))
for k in range(0, 75 + 6700):
    if k in patched: continue
    if sector(t, k) != sector(s, k): raise AssertionError("sector %d differs" % k)
# the data partition for btrfs check
t.seek(start * 512); open(sys.argv[3], "wb").write(t.read(min(size, 600 << 20) * 1 if False else size * 512)[:size * 512])
print("boot area ok")
PY
    btrfs check --readonly "$out/part_$name.bin" > "$out/btrfs_$name.log" 2>&1 || { echo "[$name] btrfs check failed"; cat "$out/btrfs_$name.log"; return 1; }
    rm -f "$out/part_$name.bin"
    local log2; log2=$(mktemp)
    timeout 120 qemu-system-x86_64 $accel -display none -vga std -smp 2 -m 512M -serial "file:$log2" $boot_args -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot
    rc=$?
    for want in "install: media is an installed system" "login: wrong password rejected" "login: ok tester" "stage 0 done"; do
        grep -qF -- "$want" "$log2" || { echo "[$name] installed boot: missing '$want'"; cat "$log2"; return 1; }
    done
    [ $rc -eq 33 ] || { echo "[$name] installed boot: qemu exit $rc"; cat "$log2"; return 1; }
    echo "[$name] ok"
}
T=$work
run_case ide "-drive file=$T/install_target_ide.img,format=raw,if=ide,index=1" "-drive file=$T/install_target_ide.img,format=raw,if=ide,index=0" "ide1:" || exit 1
run_case ahci "-device ich9-ahci,id=ahci0 -drive file=$T/install_target_ahci.img,if=none,id=sd0,format=raw -device ide-hd,drive=sd0,bus=ahci0.0" \
    "-device ich9-ahci,id=ahci0 -drive file=$T/install_target_ahci.img,if=none,id=sd0,format=raw -device ide-hd,drive=sd0,bus=ahci0.0" "sata0:" || exit 1
run_case nvme "-drive file=$T/install_target_nvme.img,if=none,id=nv0,format=raw -device nvme,drive=nv0,serial=loos0" \
    "-drive file=$T/install_target_nvme.img,if=none,id=nv0,format=raw -device nvme,drive=nv0,serial=loos0,bootindex=0" "nvme0:" || exit 1
echo "install ok"
