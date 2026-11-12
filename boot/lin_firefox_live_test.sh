#!/usr/bin/env bash
# Live-rendering proof for Firefox in the Linux layer: once its window is up, take a screenshot,
# type "loos" on the PS/2 keyboard (Firefox focuses the address bar of a new window), take more
# screenshots, and require that the window content changed after the input.
#   lin_firefox_live_test.sh        screenshots land in $LOOS_BOOT_OUT (default out/linfox-live)
set -u
here=$(cd "$(dirname "$0")" && pwd); root=$here/..
out=${LOOS_BOOT_OUT:-$root/out/linfox-live}; mkdir -p "$out"; out=$(cd "$out" && pwd)
sockdir=$(mktemp -d); trap 'rm -rf "$sockdir"' EXIT   # the data drive cannot hold Unix sockets
mon=$sockdir/monitor.sock; log=$out/lin-dyn.log; rm -f "$log" "$out"/shot-*.ppm
hmp() { echo "$1" | socat - "UNIX-CONNECT:$mon" >/dev/null 2>&1; sleep 0.3; }
drive() {
    for _ in $(seq 1 900); do grep -aq "wayland window" "$log" 2>/dev/null && break; sleep 0.5; done
    sleep 10                                   # let Firefox finish its first paints
    hmp "screendump $out/shot-before.ppm"
    for k in l o o s; do hmp "sendkey $k"; done
    sleep 5
    hmp "screendump $out/shot-typed.ppm"
    hmp "sendkey ctrl-a"; hmp "sendkey backspace"
    sleep 5
    hmp "screendump $out/shot-cleared.ppm"
    hmp "quit"
}
drive & driver=$!
LOOS_BOOT_OUT=$out QEMU_DISPLAY=${QEMU_DISPLAY:-gtk} QEMU_EXTRA="-monitor unix:$mon,server,nowait" TIMEOUT=${TIMEOUT:-120} \
    "$here/lin_firefox_test.sh" "${1:-about:blank}" >/dev/null 2>&1
kill $driver 2>/dev/null; wait   # QEMU is gone: stop waiting for a window that never came
ok=1
for s in before typed cleared; do [ -s "$out/shot-$s.ppm" ] || { echo "missing screenshot $s"; ok=0; }; done
if [ $ok = 1 ]; then
    a=$(md5sum < "$out/shot-before.ppm"); b=$(md5sum < "$out/shot-typed.ppm"); c=$(md5sum < "$out/shot-cleared.ppm")
    [ "$a" != "$b" ] || { echo "screen did not change after typing"; ok=0; }
    [ "$b" != "$c" ] || { echo "screen did not change after clearing"; ok=0; }
fi
grep -a "^lin: .*running," "$log" | tail -2
[ $ok = 1 ] && echo "lin firefox live test ok" || { echo "lin firefox live test FAILED"; exit 1; }
