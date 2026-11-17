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
        add(t, "bin/cf_" + a, open(f"{src}/cf_{a}", "rb").read())
    add(t, "bin/sig_x86_64", open(f"{src}/sig_x86_64", "rb").read())
    for a in ("x86_64", "arm64"):
        mkdir(t, "Applications/dl_" + a)
        add(t, f"Applications/dl_{a}/dl", open(f"{src}/dl_{a}", "rb").read())
        add(t, f"Applications/dl_{a}/libplug.dylib", open(f"{src}/libplug_{a}.dylib", "rb").read())
    add(t, "Applications/Hello.app/Contents/MacOS/Hello", open(f"{src}/hello_universal", "rb").read())
    g = "Applications/Greet_x86_64.app/Contents"
    for d in ("Applications/Greet_x86_64.app", g, g + "/MacOS", g + "/Frameworks"):
        mkdir(t, d)
    add(t, g + "/MacOS/Greet_x86_64", open(f"{src}/{g[13:]}/MacOS/Greet_x86_64", "rb").read())
    add(t, g + "/Frameworks/libgreet.dylib", open(f"{src}/{g[13:]}/Frameworks/libgreet.dylib", "rb").read())
    for a in ("x86_64", "arm64"):
        f = f"Applications/Found_{a}.app"
        for d in (f, f + "/Contents", f + "/Contents/MacOS", f + "/Contents/Resources", f + "/Contents/Resources/en.lproj"):
            mkdir(t, d)
        for rel in ("MacOS/found", "Info.plist", "Resources/greeting.txt", "Resources/en.lproj/Localizable.strings"):
            add(t, f"{f}/Contents/{rel}", open(f"{src}/Found_{a}.app/Contents/{rel}", "rb").read(), 0o755 if rel.startswith("MacOS") else 0o644)
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
    echo "== $prog (Objective-C runtime, ARC, blocks, exceptions, tagged pointers, locks)"
    grep -aE "^(prism:|objc:|load:|initialize:|rt:|exc:)" "$log" | grep -v "starting macOS"
    for want in "rt: tagged=-77 kind=1 class=Tiny weak=1 sync=2 exit=-1 atomic=1" "exc: caught=5 rethrown=9 cleanups=12" \
        "objc: area=12 sq=25 per=20 kinds=101 weak=1 blocks=83 once=1 fwd=42 dyn=7 assoc=1 dealloc=2 class=Square"; do
        grep -aqF "$want" "$log" || { echo "prism test FAILED for $prog: no \"$want\""; tail -20 "$log"; fail=1; }
    done
    rm -f "$log"
done
for prog in /bin/cf_x86_64 /bin/cf_arm64; do
    img=$("$here/bios/build.sh" "$out" --module initrd="$tarf" --cmdline "console=ttyS0 loos.test=1 loos.lin=$prog") || exit 1
    log=$(mktemp)
    timeout 180 qemu-system-x86_64 ${QEMU_ACCEL-$(LOOS_KVM=${LOOS_KVM:-off} "$root/scripts/qemu-accel.sh" args)} -display none -vga std -smp 1 -m 256M \
        -serial "file:$log" -drive "file=$img,format=raw,if=ide" -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot
    echo "== $prog (CoreFoundation: toll-free strings, numbers, arrays, dictionaries, data, literals)"
    grep -aE "^(prism:|const:|str:|num:|arr:|dict:|data:)" "$log" | grep -v "starting macOS"
    for want in "const: class=__NSCFConstantString len=5 eq=1 cf=1" \
        "str: hello, wörld | HELLO, WÖRLD | len=12 | 7-hello-c 2.50 00042 ff | prefix=1 sub=wörld int=-12" \
        "num: 42 2.5 1 v=42 eq=1 id bool=1 cf=7" "arr: pear,apple,fig | apple fig pear | n=3 total=9 has=1 cf=2 42" \
        "dict: one=1 list=3 missing=1 count=50 sum=166650 cf=9 {" "data: <deadbeef 01> len=6 eq=1 rc=2"; do
        grep -aqF "$want" "$log" || { echo "prism test FAILED for $prog: no \"$want\""; tail -20 "$log"; fail=1; }
    done
    rm -f "$log"
done
for prog in /Applications/Found_x86_64.app /Applications/Found_arm64.app; do
    img=$("$here/bios/build.sh" "$out" --module initrd="$tarf" --cmdline "console=ttyS0 loos.test=1 loos.lin=HOME=/tmp,$prog") || exit 1
    log=$(mktemp)
    timeout 180 qemu-system-x86_64 ${QEMU_ACCEL-$(LOOS_KVM=${LOOS_KVM:-off} "$root/scripts/qemu-accel.sh" args)} -display none -vga std -smp 1 -m 256M \
        -serial "file:$log" -drive "file=$img,format=raw,if=ide" -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot
    echo "== $prog (Foundation: exceptions, sets, dates, run loop, URLs, property lists, bundle)"
    grep -aE "^(prism:|exc:|set:|date:|delayed:|runloop:|url:|file:|path:|plist:|bundle:|fm:|proc:|defaults:|modes:|openstep:|archive:|\) same|\*\*\*)" "$log" | grep -v "starting macOS"
    for want in "exc: NSRangeException: *** -[NSConstantArray objectAtIndex:]: index 7 beyond bounds [0 .. 2] | code 42! | 1" \
        "set: count=3 has=1 n=3 sorted=a,c,d sub=1 cf=1/1" "delayed: hi" "runloop: first=4 ticks=3 cf=2 source=5 entries>=1 took=1 finished=1" \
        "url: https example.com 8443 /docs/a/index.html q=1 top | https://user@example.com:8443/docs/img/logo%20big.png logo big.png | files.example.org -1" \
        "path: libfoo.1.dylib dylib /usr/local/../lib /usr/lib/libfoo.1.dylib libfoo.1 a/b /|x|y|/" \
        "plist: xml=1/100 bin=1/200 file=1 2001-01-01 01:00:00 +0000 <0102ff> on=1 step=1/2" \
        "org.loos.found 1.2 greeting.txt \"Hello from a resource\" Grüezi Found" \
        "fm: made=1 a,a/b,a/b/two.txt,a/one.txt,c,c/b,c/b/two.txt,moved.txt size=5 type=NSFileTypeRegular copied=1 moved=1 dir=1/1 removed=1 left=a,moved.txt missing=4/NSCocoaErrorDomain" \
        "home=/tmp pid=1 os>=11:1/1 uuid=36 same=1 bad=1" \
        "defaults: answer=42 enabled=1 list=2 volume=7 name=default temp=1 file=1 changes=5 pings=2" "modes: default=0 mine=1 common=2" \
        "openstep: {/    k = \"two words\";/    n = (/        1,/        x/    );/}/|hello = \"Grüezi\";" \
        ") same=1 2001-01-01 00:01:00 +0000 t1+t2 __NSCFString 5 https://example.com/a ok refused=1/4864" \
        "*** Terminating app due to uncaught exception 'Custom', reason: 'bye 3'"; do
        grep -aqF "$want" "$log" || { echo "prism test FAILED for $prog: no \"$want\""; tail -20 "$log"; fail=1; }
    done
    [ -n "${PRISM_LOGDIR-}" ] && cp "$log" "$PRISM_LOGDIR/$(basename $prog).log"; rm -f "$log"
done
for prog in /Applications/dl_x86_64/dl /Applications/dl_arm64/dl; do
    img=$("$here/bios/build.sh" "$out" --module initrd="$tarf" --cmdline "console=ttyS0 loos.test=1 loos.lin=$prog") || exit 1
    log=$(mktemp)
    timeout 180 qemu-system-x86_64 ${QEMU_ACCEL-$(LOOS_KVM=${LOOS_KVM:-off} "$root/scripts/qemu-accel.sh" args)} -display none -vga std -smp 1 -m 256M \
        -serial "file:$log" -drive "file=$img,format=raw,if=ide" -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot
    echo "== $prog (dlopen, dlsym, dladdr)"
    grep -aE "^(prism:|dl:|load:)" "$log" | grep -v "starting macOS"
    grep -aqF "dl: before=0 value=152 len=5 labs=9 same=1 in=libplug.dylib clean=1 missing=0 err=1 nosym=0 self=1 close=0" "$log" || { echo "prism test FAILED for $prog"; tail -20 "$log"; fail=1; }
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
