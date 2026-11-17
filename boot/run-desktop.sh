#!/usr/bin/env bash
# Builds the kernel + boot image and boots the LoOS desktop in a QEMU window to explore.
#   boot/run-desktop.sh [bios|uefi] [extra kernel cmdline words...]
# Keys: Super = Start, F8 during boot = startup menu, Ctrl+Alt+G releases the mouse.
#   Extra words go to the kernel: loos.look=<id> (loos classic dock tiler paper midnight lean large), loos.night=<0-100>, scale=<pct>.
#   ./loos qemu bios --look tiler   is shorthand for loos.look=tiler.
# Env: LOOS_COMPAT=0 skips the Windows/Linux/macOS app file systems; LOOS_WEBBRIDGE=0 disables the host HTTPS bridge; LOOS_DISPLAY=none runs headless.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd); out=$here/../out/boot
mode=${1:-bios}; shift || true
args=(); while [ $# -gt 0 ]; do case $1 in --look) args+=("loos.look=$2"); shift 2 ;; --night) args+=("loos.night=$2"); shift 2 ;; *) args+=("$1"); shift ;; esac; done
cmdline="console=ttyS0 ${args[*]:-}"
# Windows, Linux and macOS compatibility file systems (boot menu entries + apps). LOOS_COMPAT=0 boots without them.
modargs=()
if [ "${LOOS_COMPAT:-1}" != 0 ]; then
    while IFS= read -r m; do modargs+=(--module "$m"); done < <("$here/compat-modules.sh" "$out")
fi
case $mode in
    bios) img=$("$here/bios/build.sh" "$out" "${modargs[@]}" --cmdline "$cmdline"); fw=() ;;
    uefi) img=$("$here/uefi/build.sh" "$out" "${modargs[@]}" --cmdline "$cmdline"); fw=(-bios "${OVMF_CODE:-/usr/share/edk2/ovmf/OVMF_CODE.fd}") ;;
    *) echo "usage: $0 [bios|uefi] [cmdline words]" >&2; exit 2 ;;
esac
accel=$("$here/../scripts/qemu-accel.sh" args)   # KVM when available (LOOS_KVM=off to force TCG)
# Headless by default (no window); LOOS_DISPLAY=gtk or sdl opens one.
case "${QEMU_DISPLAY:-gtk:-none}}" in
    gtk) display=(-display gtk,zoom-to-fit=on,grab-on-hover=on) ;;
    sdl) display=(-display sdl) ;;
    *) display=(-display none) ;;
esac
# Host-assisted HTTPS for the browser while LoOS has no native network stack. Disable with LOOS_WEBBRIDGE=0.
web_pid=""; web_args=(); web_dir=""; web_sock=""
if [ "${LOOS_WEBBRIDGE:-1}" != 0 ]; then
    web_dir=$(mktemp -d "${TMPDIR:-/tmp}/loos-webbridge.XXXXXX")   # Unix sockets need a local filesystem (not DATA/exFAT)
    web_sock="$web_dir/web.sock"
    python3 "$here/web-bridge.py" "$web_sock" >"$out/webbridge.log" 2>&1 &
    web_pid=$!
    for _ in $(seq 1 100); do [ -S "$web_sock" ] && break; sleep 0.05; done
    if [ -S "$web_sock" ]; then
        web_args=(-chardev "socket,id=webbridge,path=$web_sock,server=off" -device "isa-serial,chardev=webbridge,index=1")
        echo "LoOS: browser web bridge on (host HTTPS over COM2; LOOS_WEBBRIDGE=0 disables)." >&2
    else
        echo "LoOS: web bridge did not start; browser stays offline (see $out/webbridge.log)" >&2
        kill "$web_pid" 2>/dev/null || true; web_pid=""
    fi
fi
cleanup() { [ -n "$web_pid" ] && kill "$web_pid" 2>/dev/null || true; [ -n "$web_dir" ] && rm -rf "$web_dir" || true; }
trap cleanup EXIT INT TERM
echo "LoOS: $mode boot. Super opens Start, F8 at boot opens the startup menu, Ctrl+Alt+G frees the mouse." >&2
qemu-system-x86_64 -name LoOS $accel -smp 4 -m ${QEMU_MEM:-4G} -vga std -display gtk -serial stdio \
    -drive "file=$img,format=raw,if=ide" "${web_args[@]}" "${fw[@]}"
rc=$?
cleanup
exit $rc
