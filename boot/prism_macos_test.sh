#!/usr/bin/env bash
# Runs real macOS command-line tools (copied from a macOS machine, never stored in the repo) on LoOS
# through Prism under QEMU, and compares their output with what macOS printed for the same command.
#   PRISM_MAC=user@host   macOS machine reachable by ssh (key login); default user@192.168.1.100
#   prism_macos_test.sh [out-dir]
set -u
here=$(cd "$(dirname "$0")" && pwd); root=$here/..
out=${1:-${LOOS_BOOT_OUT:-/tmp/opencode/loos-boot}}/prism-macos; mkdir -p "$out/bin"
mac=${PRISM_MAC:-user@192.168.1.100}
# command line (comma separated, as loos.lin takes it) | expected output line
cases='/bin/pwd|/
/usr/bin/printenv,HOME|/home
/usr/bin/dirname,/a/b/c|/a/b
/usr/bin/cksum,/tmp/f|3015617425 6 /tmp/f
/usr/bin/sum,/tmp/f|36979 1 /tmp/f
/usr/bin/xxd,/tmp/f|00000000: 6865 6c6c 6f0a                           hello.
/usr/bin/getopt,ab:,-a,-b,x,y| -a -b x -- y'
tools=$(echo "$cases" | cut -d'|' -f1 | cut -d, -f1 | sort -u)
for t in $tools; do
    [ -s "$out/bin/$(basename "$t")" ] || scp -q "$mac:$t" "$out/bin/$(basename "$t")" || { echo "prism macos test: cannot copy $t from $mac"; exit 1; }
done
tarf=$out/initrd.tar
python3 - "$out/bin" "$tarf" $tools <<'PY'
import sys, tarfile, io, os
src, dst, tools = sys.argv[1], sys.argv[2], sys.argv[3:]
def add(t, name, data, mode=0o755):
    i = tarfile.TarInfo(name); i.size = len(data); i.mode = mode; t.addfile(i, io.BytesIO(data))
def mkdir(t, name):
    i = tarfile.TarInfo(name); i.type = tarfile.DIRTYPE; i.mode = 0o755; t.addfile(i)
with tarfile.open(dst, "w", format=tarfile.USTAR_FORMAT) as t:
    for d in ("bin", "usr", "usr/bin", "tmp", "home"):
        mkdir(t, d)
    for p in tools:
        add(t, p.lstrip("/"), open(os.path.join(src, os.path.basename(p)), "rb").read())
    add(t, "tmp/f", b"hello\n", 0o644)
PY
fail=0; pass=0
while IFS='|' read -r cmd want; do
    img=$("$here/bios/build.sh" "$out" --module initrd="$tarf" --cmdline "console=ttyS0 loos.test=1 loos.lin=HOME=/home,$cmd") || exit 1
    log=$(mktemp)
    timeout 120 qemu-system-x86_64 ${QEMU_ACCEL-$(LOOS_KVM=${LOOS_KVM:-off} "$root/scripts/qemu-accel.sh" args)} -display none -vga std -smp 1 -m 256M \
        -serial "file:$log" -drive "file=$img,format=raw,if=ide" -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot
    if grep -aqxF -- "$want" "$log" && grep -aqF "lin: program finished, status 0" "$log"; then
        echo "ok   ${cmd//,/ }"; pass=$((pass + 1))
    else
        echo "FAIL ${cmd//,/ } (want '$want')"; grep -aE "^(prism|lin: )" "$log" | tail -6; fail=1
    fi
    rm -f "$log"
done <<< "$cases"
echo "prism macos test: $pass passed"
exit $fail
