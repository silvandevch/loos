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
        add(t, "bin/thread_" + a, open(f"{src}/thread_{a}", "rb").read())
        add(t, "bin/net_" + a, open(f"{src}/net_{a}", "rb").read())
        add(t, "bin/fs_" + a, open(f"{src}/fs_{a}", "rb").read())
        add(t, "bin/cxx_" + a, open(f"{src}/cxx_{a}", "rb").read())
        add(t, "bin/dns_" + a, open(f"{src}/dns_{a}", "rb").read())
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
    mkdir(t, "etc")
    add(t, "etc/hosts", b"127.0.0.1 localhost\n10.1.2.3 printer.lan printer # office\n::5 six.lan\n", 0o644)
    add(t, "etc/services", b"myproto 7777/tcp\nhttps 443/tcp\n", 0o644)
    add(t, "etc/resolv.conf", b"nameserver 127.0.0.1\n", 0o644)
    add(t, "Applications/Hello.app/Contents/Info.plist", b'<?xml version="1.0"?><plist version="1.0"><dict><key>CFBundleExecutable</key><string>Hello</string></dict></plist>', 0o644)
PY
# The cases: boot command line (loos.lin=...), title, log lines to show, QEMU timeout, expected lines.
# Every image is built first; then the QEMU runs go in parallel (PRISM_JOBS at a time, default 4);
# then each log is checked in order.
lins=() titles=() shows=() secs=() wants=()
case_() { lins+=("$1"); titles+=("$2"); shows+=("$3"); secs+=("$4"); shift 4; local IFS=$'\x1f'; wants+=("$*"); }
for prog in /bin/hello_x86_64 /bin/hello_arm64 /Applications/Hello.app; do
    case_ "HOME=/home,$prog" "$prog" "^(prism|heap|fmt|math|env|write|atexit|lin: program finished)" 120 \
        "ctor=42" "heap: abcdefghijklmnopqrstuvwxyz len=26" "fmt: [ 3.14|7   |beef|str|Z] n=21" "math: fib(20)=6765 dot=5.000 div=-176" \
        "env: HOME=/home cmp=1" "write: ok" "atexit handler ran" "lin: program finished, status 1792"
done
case_ /Applications/Greet_x86_64.app "/Applications/Greet_x86_64.app (bundled dylib via @rpath)" "^(prism|greet|app:|lin: program finished)" 120 \
    "greet: hello prism from a bundled dylib" "app: greet_add=1005" "lin: program finished, status 0"
case_ /bin/sig_x86_64 "/bin/sig_x86_64 (signal handlers, sigprocmask, qsort)" "^(prism|sig:|sys:|scan:|num:|misc:|sigtest:|lin: program finished)" 120 \
    "sig: usr1=30 term=15 blocked=1 sorted=13579" "sys: tmp=1 real=/tmp ncpu=0/1 missing=1" "scan: k=4 a=12 w=ab b=31 x10=35 mode=744 wcw=2" \
    "num: d=-125 end=x f=25 mb=2/e9" "misc: fn=01 tm=126/10/15:13 rest=1 tn=42/0/too large fmod=15" "sigtest: logged 5"
for prog in /bin/objc_x86_64 /bin/objc_arm64; do
    case_ "$prog" "$prog (Objective-C runtime, ARC, blocks, exceptions, tagged pointers, locks)" "^(prism:|objc:|load:|initialize:|rt:|exc:)" 180 \
        "rt: tagged=-77 kind=1 class=Tiny weak=1 sync=2 exit=-1 atomic=1" "exc: caught=5 rethrown=9 cleanups=12" \
        "objc: area=12 sq=25 per=20 kinds=101 weak=1 blocks=83 once=1 fwd=42 dyn=7 assoc=1 dealloc=2 class=Square"
done
for prog in /bin/cf_x86_64 /bin/cf_arm64; do
    case_ "$prog" "$prog (CoreFoundation: toll-free strings, numbers, arrays, dictionaries, data, literals)" "^(prism:|const:|str:|num:|arr:|dict:|data:)" 180 \
        "const: class=__NSCFConstantString len=5 eq=1 cf=1" \
        "str: hello, wörld | HELLO, WÖRLD | len=12 | 7-hello-c 2.50 00042 ff | prefix=1 sub=wörld int=-12" \
        "num: 42 2.5 1 v=42 eq=1 id bool=1 cf=7" "arr: pear,apple,fig | apple fig pear | n=3 total=9 has=1 cf=2 42" \
        "dict: one=1 list=3 missing=1 count=50 sum=166650 cf=9 {" "data: <deadbeef 01> len=6 eq=1 rc=2"
done
for prog in /Applications/Found_x86_64.app /Applications/Found_arm64.app; do
    case_ "HOME=/tmp,$prog" "$prog (Foundation: exceptions, sets, dates, run loop, URLs, property lists, bundle)" \
        "^(prism:|exc:|set:|date:|delayed:|runloop:|url:|file:|path:|plist:|bundle:|fm:|proc:|defaults:|modes:|openstep:|archive:|\) same|\*\*\*)" 180 \
        "exc: NSRangeException: *** -[NSConstantArray objectAtIndex:]: index 7 beyond bounds [0 .. 2] | code 42! | 1" \
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
        "*** Terminating app due to uncaught exception 'Custom', reason: 'bye 3'"
done
for prog in /Applications/dl_x86_64/dl /Applications/dl_arm64/dl; do
    case_ "$prog" "$prog (dlopen, dlsym, dladdr)" "^(prism:|dl:|load:)" 180 \
        "dl: before=0 value=152 len=5 labs=9 same=1 in=libplug.dylib clean=1 missing=0 err=1 nosym=0 self=1 close=0"
done
for cmd in /bin/spawn_x86_64,/bin/hello_arm64 /bin/spawn_arm64; do
    case_ "PATH=/bin,$cmd" "${cmd//,/ } (fork, exec of a Mach-O, posix_spawn with a pipe)" "^(prism:|spawn:|lin: program finished)" 180 \
        "spawn: fork=1 exit=7 spawn=0 piped=1 exit2=7 execvp=-1/2 missing=2"
done
for cmd in /bin/thread_x86_64 /bin/thread_arm64; do
    case_ "$cmd" "$cmd (pthreads: mutex, condition variable, keys, errno, thread-locals)" "^(prism:|thread|lin: program finished)" 180 \
        "thread: counter=80000 sum=14 cond=42 tsd=1 errno=1 tlv=1 main_tlv=5 main=1" "thread2: timedwait=60 took_ok=1 detached=1 runs=60"
done
for cmd in /bin/net_x86_64 /bin/net_arm64; do
    case_ "$cmd" "$cmd (sockets, kqueue)" "^(prism:|net:|lin: program finished)" 180 \
        "net: pair=1 opt=1 bound=1 name=1 kq_accept=1 peer=1 kq_read=1 echo=1 client=1 oneshot=1 timer=1 user=1 unsupported=1 addr=1"
done
for cmd in /bin/fs_x86_64 /bin/fs_arm64; do
    case_ "$cmd" "$cmd (statfs, getmntinfo)" "^(prism:|fs:|lin: program finished)" 120 \
        "fs: statfs=1 fstatfs=1 type=apfs on=/ bsize=1 free<=total=1 mnt=1// fsstat=1/apfs"
done
for cmd in /bin/cxx_x86_64 /bin/cxx_arm64; do
    case_ "$cmd" "$cmd (C++ exceptions, RTTI, exception_ptr, dynamic_cast, static guards, terminate)" "^(prism:|cxx:|libc\+\+abi|lin: program finished)" 120 \
        "cxx: int=42 exc=71 mi=2 ptr=2 void=1 str=1 rethrow=9 nested=7 any=1 ep=7 dyn=1 static=1 raii=10 during=1 dtors_ok=1" \
        "cxx: terminate handler ran" "lin: program finished, status 0"
done
for cmd in /bin/dns_x86_64 /bin/dns_arm64; do
    case_ "$cmd loos.net=1" "$cmd (getaddrinfo, hosts, services, DNS)" "^(prism:|dns:|lin: program finished)" 120 \
        "dns: num=2/1/80/16 both=2/443 local=127.0.0.1 hosts=10.1.2.3/printer.lan/22 six=0/::5/28 svc=7777 passive=0.0.0.0 err=8/5/8 ni=printer.lan/https/10.1.2.3 hbn=printer.lan/10.1.2.3 cname=192.0.2.7/real.test aaaa=2001:db8::9 nx=8"
done

n=${#lins[@]}
runs=$(mktemp -d)
for ((i = 0; i < n; i++)); do
    img=$("$here/bios/build.sh" "$out" --module initrd="$tarf" --cmdline "console=ttyS0 loos.test=1 loos.lin=${lins[i]}") || exit 1
    cp "$img" "$runs/$i.img"
done
accel=${QEMU_ACCEL-$(LOOS_KVM=${LOOS_KVM:-auto} "$root/scripts/qemu-accel.sh" args)}
jobs_max=${PRISM_JOBS:-4}
echo "prism test: $n boots, $jobs_max at a time"
for ((i = 0; i < n; i++)); do
    while [ "$(jobs -rp | wc -l)" -ge "$jobs_max" ]; do wait -n; done
    timeout "${secs[i]}" qemu-system-x86_64 $accel -display ${QEMU_DISPLAY:-gtk} -name "prism $i: $(prog=${lins[i]%% *}; echo "${prog##*,}")" -vga std -smp 1 -m 256M \
        -serial "file:$runs/$i.log" -drive "file=$runs/$i.img,format=raw,if=ide" -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot &
done
wait

fail=0
for ((i = 0; i < n; i++)); do
    log=$runs/$i.log
    echo "== ${titles[i]}"
    grep -aE "${shows[i]}" "$log" | grep -v "starting macOS"
    IFS=$'\x1f' read -r -a want <<< "${wants[i]}"
    for w in "${want[@]}"; do
        grep -aqF "$w" "$log" || { echo "prism test FAILED for ${titles[i]%% (*}: missing '$w'"; tail -20 "$log"; fail=1; break; }
    done
    if [ -n "${PRISM_LOGDIR-}" ]; then prog=${lins[i]%% *}; cp "$log" "$PRISM_LOGDIR/$(basename "${prog##*,}").log"; fi
done
rm -rf "$runs"
[ $fail = 0 ] && echo "prism test ok (x86_64 native, arm64 interpreted, universal .app, bundled dylib)"
exit $fail
