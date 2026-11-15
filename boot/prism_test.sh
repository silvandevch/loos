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
    for d in ("bin", "tmp", "Applications", "Applications/Hello.app", "Applications/Hello.app/Contents", "Applications/Hello.app/Contents/MacOS"):
        mkdir(t, d)
    for a in ("x86_64", "arm64"):
        add(t, "bin/hello_" + a, open(f"{src}/hello_{a}", "rb").read())
    for a in ("x86_64", "arm64"):
        add(t, "bin/spawn_" + a, open(f"{src}/spawn_{a}", "rb").read())
    for a in ("x86_64", "arm64"):
        add(t, "bin/objc_" + a, open(f"{src}/objc_{a}", "rb").read())
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
timeout 120 qemu-system-x86_64 ${QEMU_ACCEL-$(LOOS_KVM=${LOOS_KVM:-off} "$root/scripts/qemu-accel.sh" args)} -display none -vga std -smp 1 -m 256M \
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
grep -aE "^(prism|sig:|sys:|scan:|num:|misc:|sigtest:|lin: program finished)" "$log"
grep -aqF "sig: usr1=30 term=15 blocked=1 sorted=13579" "$log" || { echo "prism test FAILED for signals"; tail -20 "$log"; fail=1; }
grep -aqF "sys: tmp=1 real=/tmp ncpu=0/1 missing=1" "$log" || { echo "prism test FAILED for realpath/mkstemp/sysctl"; grep -a "^sys:" "$log"; fail=1; }
grep -aqF "scan: k=4 a=12 w=ab b=31 x10=35 mode=744 wcw=2" "$log" || { echo "prism test FAILED for sscanf/setmode/wcwidth"; grep -a "^scan:" "$log"; fail=1; }
grep -aqF "num: d=-125 end=x f=25 mb=2/e9" "$log" || { echo "prism test FAILED for strtod/mbrtowc"; grep -a "^num:" "$log"; fail=1; }
grep -aqF "misc: fn=01 tm=126/10/15:13 rest=1 tn=42/0/too large fmod=15" "$log" || { echo "prism test FAILED for fnmatch/strptime/strtonum/fmod"; grep -a "^misc:" "$log"; fail=1; }
grep -aqF "sigtest: logged 5" "$log" || { echo "prism test FAILED for syslog"; fail=1; }
rm -f "$log"
for prog in /bin/objc_x86_64 /bin/objc_arm64; do
    img=$("$here/bios/build.sh" "$out" --module initrd="$tarf" --cmdline "console=ttyS0 loos.test=1 loos.lin=$prog") || exit 1
    log=$(mktemp)
    timeout 180 qemu-system-x86_64 ${QEMU_ACCEL-$(LOOS_KVM=${LOOS_KVM:-off} "$root/scripts/qemu-accel.sh" args)} -display none -vga std -smp 1 -m 256M \
        -serial "file:$log" -drive "file=$img,format=raw,if=ide" -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot
    echo "== $prog (Objective-C runtime, ARC, blocks)"
    grep -aE "^(prism:|objc:|load:|initialize:)" "$log" | grep -v "starting macOS"
    grep -aqF "objc: area=12 sq=25 per=20 kinds=101 weak=1 blocks=83 once=1 fwd=42 dyn=7 assoc=1 dealloc=2 class=Square" "$log" || { echo "prism test FAILED for $prog"; tail -20 "$log"; fail=1; }
    rm -f "$log"
done
for cmd in /bin/spawn_x86_64,/bin/hello_arm64 /bin/spawn_arm64; do
    img=$("$here/bios/build.sh" "$out" --module initrd="$tarf" --cmdline "console=ttyS0 loos.test=1 loos.lin=PATH=/bin,$cmd") || exit 1
    log=$(mktemp)
    timeout 180 qemu-system-x86_64 ${QEMU_ACCEL-$(LOOS_KVM=${LOOS_KVM:-off} "$root/scripts/qemu-accel.sh" args)} -display none -vga std -smp 1 -m 256M \
        -serial "file:$log" -drive "file=$img,format=raw,if=ide" -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot
    echo "== ${cmd//,/ } (fork, exec of a Mach-O, posix_spawn with a pipe)"
    grep -aE "^(prism:|spawn:|lin: program finished)" "$log" | grep -v "starting macOS"
    grep -aqF "spawn: fork=1 exit=7 spawn=0 piped=1 exit2=7 execvp=-1/2 missing=2" "$log" || { echo "prism test FAILED for $cmd"; tail -20 "$log"; fail=1; }
    rm -f "$log"
done
[ $fail = 0 ] && echo "prism test ok (x86_64 native, arm64 interpreted, universal .app, bundled dylib)"
exit $fail
