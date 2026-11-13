#!/usr/bin/env bash
# Boots LoOS under QEMU and runs macOS test programs (tests/prism/hello.c) through LoOS Prism:
# the Intel build natively in ring 3, the Apple Silicon build on the arm64 interpreter, and a
# universal binary inside an .app bundle.
set -u
here=$(cd "$(dirname "$0")" && pwd); root=$here/..
out=${1:-${LOOS_BOOT_OUT:-/tmp/opencode/loos-boot}}; mkdir -p "$out"
"$root/tests/prism/build.sh" "$out/prism" >/dev/null || { echo "prism test: cannot build the Mach-O test programs"; exit 1; }
tarf=$out/prism-initrd.tar
python3 - "$out/prism" "$tarf" <<'PY'
import sys, tarfile, io
src, dst = sys.argv[1], sys.argv[2]
def add(t, name, data, mode=0o755):
    i = tarfile.TarInfo(name); i.size = len(data); i.mode = mode; t.addfile(i, io.BytesIO(data))
def mkdir(t, name):
    i = tarfile.TarInfo(name); i.type = tarfile.DIRTYPE; i.mode = 0o755; t.addfile(i)
with tarfile.open(dst, "w", format=tarfile.USTAR_FORMAT) as t:
    for d in ("bin", "Applications", "Applications/Hello.app", "Applications/Hello.app/Contents", "Applications/Hello.app/Contents/MacOS"):
        mkdir(t, d)
    for a in ("x86_64", "arm64"):
        add(t, "bin/hello_" + a, open(f"{src}/hello_{a}", "rb").read())
    add(t, "bin/sig_x86_64", open(f"{src}/sig_x86_64", "rb").read())
    add(t, "Applications/Hello.app/Contents/MacOS/Hello", open(f"{src}/hello_universal", "rb").read())
    g = "Applications/Greet_x86_64.app/Contents"
    for d in ("Applications/Greet_x86_64.app", g, g + "/MacOS", g + "/Frameworks"):
        mkdir(t, d)
    add(t, g + "/MacOS/Greet_x86_64", open(f"{src}/{g[13:]}/MacOS/Greet_x86_64", "rb").read())
    add(t, g + "/Frameworks/libgreet.dylib", open(f"{src}/{g[13:]}/Frameworks/libgreet.dylib", "rb").read())
    add(t, "Applications/Hello.app/Contents/Info.plist", b'<?xml version="1.0"?><plist version="1.0"><dict><key>CFBundleExecutable</key><string>Hello</string></dict></plist>', 0o644)
PY
fail=0
for prog in /bin/hello_x86_64 /bin/hello_arm64 /Applications/Hello.app; do
    img=$("$here/bios/build.sh" "$out" --module initrd="$tarf" --cmdline "console=ttyS0 loos.test=1 loos.lin=HOME=/home,$prog") || exit 1
    log=$(mktemp)
    timeout 120 qemu-system-x86_64 ${QEMU_ACCEL-$(LOOS_KVM=${LOOS_KVM:-off} "$root/scripts/qemu-accel.sh" args)} -display none -vga std -smp 1 -m 256M \
        -serial "file:$log" -drive "file=$img,format=raw,if=ide" -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot
    echo "== $prog"
    grep -aE "^(prism|heap|fmt|math|env|write|atexit|lin: program finished)" "$log"
    for want in "ctor=42" "heap: abcdefghijklmnopqrstuvwxyz len=26" "fmt: [ 3.14|7   |beef|str|Z] n=21" "math: fib(20)=6765 dot=5.000 div=-176" "env: HOME=/home cmp=1" "write: ok" "atexit handler ran" "lin: program finished, status 1792"; do
        grep -aqF "$want" "$log" || { echo "prism test FAILED for $prog: missing '$want'"; tail -20 "$log"; fail=1; break; }
    done
    rm -f "$log"
done
img=$("$here/bios/build.sh" "$out" --module initrd="$tarf" --cmdline "console=ttyS0 loos.test=1 loos.lin=/Applications/Greet_x86_64.app") || exit 1
log=$(mktemp)
timeout 120 qemu-system-x86_64 ${QEMU_ACCEL-$(LOOS_KVM=${LOOS_KVM:-off} "$root/scripts/qemu-accel.sh" args)} -display gtk -vga std -smp 1 -m 256M \
    -serial "file:$log" -drive "file=$img,format=raw,if=ide" -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot
echo "== /Applications/Greet_x86_64.app (bundled dylib via @rpath)"
grep -aE "^(prism|greet|app:|lin: program finished)" "$log"
for want in "greet: hello prism from a bundled dylib" "app: greet_add=1005" "lin: program finished, status 0"; do
    grep -aqF "$want" "$log" || { echo "prism test FAILED for the dylib app: missing '$want'"; tail -20 "$log"; fail=1; break; }
done
rm -f "$log"
img=$("$here/bios/build.sh" "$out" --module initrd="$tarf" --cmdline "console=ttyS0 loos.test=1 loos.lin=/bin/sig_x86_64") || exit 1
log=$(mktemp)
timeout 120 qemu-system-x86_64 ${QEMU_ACCEL-$(LOOS_KVM=${LOOS_KVM:-off} "$root/scripts/qemu-accel.sh" args)} -display none -vga std -smp 1 -m 256M \
    -serial "file:$log" -drive "file=$img,format=raw,if=ide" -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot
echo "== /bin/sig_x86_64 (signal handlers, sigprocmask, qsort)"
grep -aE "^(prism|sig:|lin: program finished)" "$log"
grep -aqF "sig: usr1=30 term=15 blocked=1 sorted=13579" "$log" || { echo "prism test FAILED for signals"; tail -20 "$log"; fail=1; }
rm -f "$log"
[ $fail = 0 ] && echo "prism test ok (x86_64 native, arm64 interpreted, universal .app, bundled dylib)"
exit $fail
